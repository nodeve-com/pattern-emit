//! The RE2 guard: refuse what regex-syntax reads differently from Go's `regexp`, and rewrite what
//! it reads differently but RE2 defines (Perl classes and `\b` are ASCII).

use crate::Refusal;
use regex_syntax::ast::{
    self, visit, AssertionKind, Ast, ClassPerl, ClassPerlKind, ClassSetItem, ClassUnicode,
    ClassUnicodeKind, FlagsItemKind, HexLiteralKind, LiteralKind, RepetitionKind, RepetitionRange,
    Span, Visitor,
};

/// Go's most repeats of repeats, and the most one `{n}` may count.
const MAX_REPEAT: u32 = 1000;

/// General-category names both dialects share.
pub(crate) const CATEGORIES: &[&str] = &[
    "C", "Cc", "Cf", "Co", "L", "Ll", "Lm", "Lo", "Lt", "Lu", "M", "Mc", "Me", "Mn", "N", "Nd",
    "Nl", "No", "P", "Pc", "Pd", "Pe", "Pf", "Pi", "Po", "Ps", "S", "Sc", "Sk", "Sm", "So", "Z",
    "Zl", "Zp", "Zs",
];

/// Go's category names beyond the shared ones.
const EXTRA: &[&str] = &["Any", "Cs"];

/// An edit making the word boundary at `a` ASCII, as Go and ECMA define it.
pub(crate) fn boundary(a: &ast::Assertion) -> (Span, String) {
    let text = if a.kind == AssertionKind::WordBoundary {
        r"(?-u:\b)"
    } else {
        r"(?-u:\B)"
    };
    (a.span, text.to_owned())
}

pub(crate) fn refuse<T>(why: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal(why.into()))
}

/// `pattern` with Perl classes and word boundaries written as RE2 defines them, or why not.
pub(crate) fn rewrite(pattern: &str) -> Result<String, Refusal> {
    let ast = ast::parse::Parser::new().parse(pattern).map_err(|e| {
        let at = e.span().start.offset;
        Refusal(format!("not an RE2 pattern: {} at byte {at}", e.kind()))
    })?;
    let guard = Guard {
        pattern,
        edits: Vec::new(),
        repeats: vec![MAX_REPEAT],
    };
    apply(pattern, visit(&ast, guard)?)
}

/// `pattern` with each span replaced by its text.
pub(crate) fn apply(pattern: &str, mut edits: Vec<(Span, String)>) -> Result<String, Refusal> {
    let mut out = pattern.to_owned();
    edits.sort_by_key(|(span, _)| core::cmp::Reverse(span.start.offset));
    for (span, text) in edits {
        if !out.is_char_boundary(span.start.offset) || !out.is_char_boundary(span.end.offset) {
            return refuse("pattern's spans do not fall on characters");
        }
        out.replace_range(span.start.offset..span.end.offset, &text);
    }
    Ok(out)
}

struct Guard<'p> {
    pattern: &'p str,
    edits: Vec<(Span, String)>,
    /// Go's budget: how many repeats a nested `{n}` may still multiply into.
    repeats: Vec<u32>,
}

impl Guard<'_> {
    fn perl(&mut self, c: &ClassPerl, in_class: bool) -> Result<(), Refusal> {
        let pos = match c.kind {
            ClassPerlKind::Digit => "0-9",
            ClassPerlKind::Space => r"\t\n\f\r\x20",
            ClassPerlKind::Word => "0-9A-Z_a-z",
        };
        let text = match (in_class, c.negated) {
            (true, false) => pos.to_owned(),
            (true, true) => format!("[^{pos}]"),
            (false, false) => format!("[{pos}]"),
            (false, true) => format!("[^{pos}]"),
        };
        if in_class {
            self.class_neighbours(&c.span)?;
        }
        self.edits.push((c.span, text));
        Ok(())
    }

    /// A class beside `-` in a bracket is a range endpoint in some readings: refuse it.
    fn class_neighbours(&self, span: &Span) -> Result<(), Refusal> {
        if beside_dash(self.pattern, span) {
            return refuse(
                "a class like `\\d` beside `-` in a bracket: RE2 reads it as a bad range, \
                 regex-syntax as a literal dash",
            );
        }
        Ok(())
    }

    fn unicode(&self, c: &ClassUnicode) -> Result<(), Refusal> {
        match &c.kind {
            ClassUnicodeKind::OneLetter(l) if "CLMNPSZ".contains(*l) => Ok(()),
            ClassUnicodeKind::Named(n) if CATEGORIES.contains(&n.as_str()) || EXTRA.contains(&n.as_str()) || script(n) => Ok(()),
            ClassUnicodeKind::NamedValue { .. } => refuse(
                "`\\p{name=value}`: RE2 has only `\\p{Category}` and `\\p{Script}`, no property forms",
            ),
            _ => refuse(
                "unicode class name: RE2 knows only its categories and scripts, spelled as \
                 Unicode does (`\\p{Greek}`, not `\\p{greek}` or `\\p{Emoji}`)",
            ),
        }
    }

    fn assertion(&mut self, a: &ast::Assertion) -> Result<(), Refusal> {
        match a.kind {
            AssertionKind::StartLine
            | AssertionKind::EndLine
            | AssertionKind::StartText
            | AssertionKind::EndText => Ok(()),
            AssertionKind::WordBoundary | AssertionKind::NotWordBoundary => {
                self.edits.push(boundary(a));
                Ok(())
            }
            _ => refuse(
                "`\\<`, `\\>`, `\\b{…}`: RE2 reads them as escaped literals, regex-syntax as \
                 word boundaries",
            ),
        }
    }

    /// Checks a repeat against Go's budget: a nested count may multiply into at most `MAX_REPEAT`.
    fn repetition(&mut self, r: &ast::Repetition) -> Result<(), Refusal> {
        if matches!(*r.ast, Ast::Repetition(_)) {
            return refuse("nested repetition (`a**`, `a{2}{3}`): RE2 rejects stacked operators");
        }
        let left = self.repeats.last().copied().unwrap_or(0);
        let Some(n) = repeat_count(&r.op.kind) else {
            self.repeats.push(left);
            return Ok(());
        };
        if n > left {
            return refuse(format!(
                "repeat count {n} past RE2's limit of {MAX_REPEAT}, for nested counts their product"
            ));
        }
        self.repeats.push(left.checked_div(n).unwrap_or(left));
        Ok(())
    }
}

/// The count a `{n}`, `{n,}`, or `{n,m}` repetition takes, or `None` for `*`, `+`, and `?`.
fn repeat_count(kind: &RepetitionKind) -> Option<u32> {
    match kind {
        RepetitionKind::Range(
            RepetitionRange::Exactly(n)
            | RepetitionRange::AtLeast(n)
            | RepetitionRange::Bounded(_, n),
        ) => Some(*n),
        _ => None,
    }
}

/// Whether the class at `span` sits beside a `-` that could make it a range endpoint.
pub(crate) fn beside_dash(pattern: &str, span: &Span) -> bool {
    let bytes = pattern.as_bytes();
    let before = span.start.offset.checked_sub(1).and_then(|i| bytes.get(i));
    let before2 = span.start.offset.checked_sub(2).and_then(|i| bytes.get(i));
    let after = bytes.get(span.end.offset);
    let after2 = bytes.get(span.end.offset.saturating_add(1));
    let dash_after = after == Some(&b'-') && after2 != Some(&b']');
    let dash_before = before == Some(&b'-') && !matches!(before2, Some(b'[' | b'^'));
    dash_after || dash_before
}

/// Whether `name` is a one-word script, spelled exactly (`Greek`): regex-syntax matches names
/// loosely, and Go rejects any with an underscore or inner capital.
fn script(name: &str) -> bool {
    let mut chars = name.chars();
    let exact = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && !chars.as_str().is_empty()
        && chars.all(|c| c.is_ascii_lowercase());
    exact
        && regex_syntax::Parser::new()
            .parse(&format!("\\p{{Script={name}}}"))
            .is_ok()
}

fn flags(f: &ast::Flags) -> Result<(), Refusal> {
    for item in &f.items {
        if let FlagsItemKind::Flag(flag) = item.kind {
            match flag {
                ast::Flag::IgnoreWhitespace => return refuse("flag `x`: RE2 has no extended mode"),
                ast::Flag::Unicode => {
                    return refuse("flag `u`: RE2 is always Unicode and has no switch for it")
                }
                ast::Flag::CRLF => {
                    return refuse("flag `R`: RE2 has no CRLF mode; `\\n` alone ends a line")
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl Visitor for Guard<'_> {
    type Output = Vec<(Span, String)>;
    type Err = Refusal;

    fn finish(self) -> Result<Self::Output, Refusal> {
        Ok(self.edits)
    }

    fn visit_pre(&mut self, ast: &Ast) -> Result<(), Refusal> {
        match ast {
            Ast::Flags(f) => flags(&f.flags),
            Ast::Group(g) => match &g.kind {
                ast::GroupKind::NonCapturing(f) => flags(f),
                _ => Ok(()),
            },
            Ast::Literal(l) => match l.kind {
                LiteralKind::HexFixed(
                    HexLiteralKind::UnicodeShort | HexLiteralKind::UnicodeLong,
                ) => refuse("`\\u` and `\\U` escapes: RE2 has none; write `\\x{…}`"),
                _ => Ok(()),
            },
            Ast::Assertion(a) => self.assertion(a),
            Ast::ClassUnicode(c) => self.unicode(c),
            Ast::ClassPerl(c) => self.perl(c, false),
            Ast::Repetition(r) => self.repetition(r),
            _ => Ok(()),
        }
    }

    fn visit_post(&mut self, ast: &Ast) -> Result<(), Refusal> {
        if matches!(ast, Ast::Repetition(_)) {
            self.repeats.pop();
        }
        Ok(())
    }

    fn visit_class_set_item_pre(&mut self, item: &ClassSetItem) -> Result<(), Refusal> {
        match item {
            ClassSetItem::Bracketed(_) => {
                refuse("a class inside a class: RE2 reads `[a[b]]` as the class `[a[b]` then a `]`")
            }
            ClassSetItem::Perl(c) => self.perl(c, true),
            ClassSetItem::Unicode(c) => self.unicode(c),
            _ => Ok(()),
        }
    }

    fn visit_class_set_binary_op_pre(&mut self, _: &ast::ClassSetBinaryOp) -> Result<(), Refusal> {
        refuse("class set operations `&&`, `--`, `~~`: RE2 reads them as literal characters")
    }
}
