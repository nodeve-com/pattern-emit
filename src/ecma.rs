//! The ECMA-262 guard (Unicode mode, no flags): refuse what regex-syntax reads differently from
//! JavaScript's `RegExp` with the `u` flag, and rewrite what ECMA defines and regex-syntax reads
//! differently (`.`, `\s`, `\w`, `\d`, `\b`).

use crate::guard::{apply, beside_dash, boundary, refuse, CATEGORIES};
use crate::Refusal;
use regex_syntax::ast::{
    self, visit, AssertionKind, Ast, ClassPerl, ClassPerlKind, ClassSetItem, ClassUnicode,
    ClassUnicodeKind, ErrorKind, GroupKind, HexLiteralKind, Literal, LiteralKind, Span,
    SpecialLiteralKind, Visitor,
};

/// ECMA's WhiteSpace and LineTerminator, as a class body.
const SPACE: &str = r"\t\n\x0B\f\r\x20\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}";

/// Characters ECMA lets a backslash escape outside a class.
const SYNTAX: &str = r"^$\.*+?()[]{}|/";

/// ECMA's general categories beyond the shared ones.
const EXTRA: &[&str] = &["Cn", "LC"];

/// `pattern` with `.`, Perl classes, and word boundaries written as ECMA-262 defines them, or why
/// not.
pub(crate) fn rewrite(pattern: &str) -> Result<String, Refusal> {
    let ast = ast::parse::Parser::new().parse(pattern).map_err(|e| {
        let at = e.span().start.offset;
        let why = match e.kind() {
            ErrorKind::UnsupportedLookAround => {
                "lookaround: ECMA-262 has it, a table cannot answer it".to_owned()
            }
            ErrorKind::UnsupportedBackreference
                if pattern
                    .get(e.span().start.offset..e.span().end.offset)
                    .is_some_and(|t| t == "\\0") =>
            {
                "`\\0`: ECMA-262 reads NUL, regex-syntax a backreference; write `\\x00`".to_owned()
            }
            ErrorKind::UnsupportedBackreference => {
                "backreference: ECMA-262 has it, a table cannot answer it".to_owned()
            }
            kind => format!("not an ECMA-262 pattern, or one only another dialect has: {kind}"),
        };
        Refusal(format!("{why} (at byte {at})"))
    })?;
    let guard = Guard {
        pattern,
        edits: Vec::new(),
        names: Vec::new(),
    };
    apply(pattern, visit(&ast, guard)?)
}

struct Guard<'p> {
    pattern: &'p str,
    edits: Vec<(Span, String)>,
    names: Vec<String>,
}

impl Guard<'_> {
    fn perl(&mut self, c: &ClassPerl, in_class: bool) -> Result<(), Refusal> {
        if in_class && beside_dash(self.pattern, &c.span) {
            return refuse(
                "a class like `\\d` beside `-` in a bracket: ECMA-262 rejects it as a range \
                 endpoint, regex-syntax reads a literal dash",
            );
        }
        let pos = match c.kind {
            ClassPerlKind::Digit => "0-9",
            ClassPerlKind::Space => SPACE,
            ClassPerlKind::Word => "0-9A-Z_a-z",
        };
        let text = match (in_class && !c.negated, c.negated) {
            (true, _) => pos.to_owned(),
            (false, true) => format!("[^{pos}]"),
            (false, false) => format!("[{pos}]"),
        };
        self.edits.push((c.span, text));
        Ok(())
    }

    fn unicode(&self, c: &ClassUnicode) -> Result<(), Refusal> {
        let text = self.pattern.get(c.span.start.offset..c.span.end.offset);
        match &c.kind {
            ClassUnicodeKind::Named(n)
                if (CATEGORIES.contains(&n.as_str()) || EXTRA.contains(&n.as_str()))
                    && !text.is_some_and(|t| t.contains("{^")) =>
            {
                Ok(())
            }
            _ => refuse(
                "`\\p`: ECMA-262 needs braces and a property name spelled exactly; only general \
                 categories by short name (`\\p{Lu}`) are supported, not `\\pL`, `\\p{^Lu}`, \
                 `\\p{Letter}`, `\\p{Script=Greek}`, or binary properties (`\\p{Emoji}`)",
            ),
        }
    }

    /// A literal as ECMA-262 in Unicode mode reads it.
    fn literal(&self, l: &Literal, in_class: bool) -> Result<(), Refusal> {
        match l.kind {
            LiteralKind::Verbatim => verbatim(l.c, in_class),
            LiteralKind::Meta | LiteralKind::Superfluous => escape(l.c, in_class),
            LiteralKind::HexFixed(HexLiteralKind::X | HexLiteralKind::UnicodeShort)
            | LiteralKind::HexBrace(HexLiteralKind::UnicodeShort) => Ok(()),
            LiteralKind::HexBrace(HexLiteralKind::X) => {
                refuse("`\\x{…}`: ECMA-262 has `\\xHH` and `\\u{…}`")
            }
            LiteralKind::HexFixed(HexLiteralKind::UnicodeLong)
            | LiteralKind::HexBrace(HexLiteralKind::UnicodeLong) => {
                refuse("`\\U`: ECMA-262 has `\\uHHHH` and `\\u{…}`")
            }
            LiteralKind::Special(ref kind) => special(kind),
            LiteralKind::Octal => refuse("octal escapes: ECMA-262 rejects them in Unicode mode"),
        }
    }

    /// A group: flags are refused, and a named group must be `(?<name>…)` with a unique name.
    fn group(&mut self, g: &ast::Group) -> Result<(), Refusal> {
        match &g.kind {
            GroupKind::NonCapturing(f) => flags(f),
            GroupKind::CaptureIndex(_) => Ok(()),
            GroupKind::CaptureName {
                starts_with_p,
                name,
            } => self.capture_name(*starts_with_p, &name.name),
        }
    }

    fn capture_name(&mut self, starts_with_p: bool, name: &str) -> Result<(), Refusal> {
        if starts_with_p {
            return refuse("`(?P<name>…)`: ECMA-262 writes a named group `(?<name>…)`");
        }
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit());
        if !plain || self.names.iter().any(|n| n == name) {
            return refuse("group name: only ASCII identifiers, each used once, are supported");
        }
        self.names.push(name.to_owned());
        Ok(())
    }

    fn assertion(&mut self, a: &ast::Assertion) -> Result<(), Refusal> {
        match a.kind {
            AssertionKind::StartLine | AssertionKind::EndLine => Ok(()),
            AssertionKind::WordBoundary | AssertionKind::NotWordBoundary => {
                self.edits.push(boundary(a));
                Ok(())
            }
            AssertionKind::StartText | AssertionKind::EndText => refuse(
                "`\\A`, `\\z`: ECMA-262 has no such escape; `^` and `$` anchor the whole text",
            ),
            _ => refuse("`\\<`, `\\>`, `\\b{…}`: ECMA-262 rejects these escapes"),
        }
    }
}

/// A literal character outside a class, or inside one, that ECMA-262 reads only as written.
fn verbatim(c: char, in_class: bool) -> Result<(), Refusal> {
    if !in_class && matches!(c, '{' | '}' | ']') {
        return refuse(format!(
            "a bare `{c}`: ECMA-262 in Unicode mode rejects it unescaped; regex-syntax reads a literal"
        ));
    }
    if in_class && c == ']' {
        return refuse(
            "`]` first in a bracket: ECMA-262 ends the class there (`[]` is empty), \
             regex-syntax reads a literal",
        );
    }
    Ok(())
}

/// A backslash before `c`: ECMA-262 escapes only syntax characters, and `-` in a class.
fn escape(c: char, in_class: bool) -> Result<(), Refusal> {
    if SYNTAX.contains(c) || (in_class && c == '-') {
        return Ok(());
    }
    refuse(format!(
        "escape `\\{c}`: ECMA-262 in Unicode mode escapes only syntax characters \
         (and `-` in a class)"
    ))
}

fn special(kind: &SpecialLiteralKind) -> Result<(), Refusal> {
    match kind {
        SpecialLiteralKind::Tab
        | SpecialLiteralKind::LineFeed
        | SpecialLiteralKind::CarriageReturn
        | SpecialLiteralKind::VerticalTab
        | SpecialLiteralKind::FormFeed => Ok(()),
        _ => refuse("`\\a`: ECMA-262 has no bell escape"),
    }
}

fn repetition(r: &ast::Repetition) -> Result<(), Refusal> {
    match *r.ast {
        Ast::Repetition(_) => refuse("nested repetition (`a**`, `a{2}{3}`): ECMA-262 rejects it"),
        Ast::Assertion(_) => refuse("a repeated assertion (`^*`): ECMA-262 rejects it"),
        _ => Ok(()),
    }
}

fn flags(f: &ast::Flags) -> Result<(), Refusal> {
    match f.items.is_empty() {
        true => Ok(()),
        false => refuse(
            "inline flags `(?i)`: ECMA-262 has none; JSON Schema's `pattern` runs with no flags",
        ),
    }
}

impl Visitor for Guard<'_> {
    type Output = Vec<(Span, String)>;
    type Err = Refusal;

    fn visit_pre(&mut self, ast: &Ast) -> Result<(), Refusal> {
        match ast {
            Ast::Literal(l) => self.literal(l, false),
            Ast::Group(g) => self.group(g),
            Ast::Dot(span) => {
                self.edits
                    .push((**span, r"[^\n\r\x{2028}\x{2029}]".to_owned()));
                Ok(())
            }
            Ast::Assertion(a) => self.assertion(a),
            Ast::Flags(f) => flags(&f.flags),
            Ast::ClassUnicode(c) => self.unicode(c),
            Ast::ClassPerl(c) => self.perl(c, false),
            Ast::Repetition(r) => repetition(r),
            _ => Ok(()),
        }
    }

    fn visit_class_set_item_pre(&mut self, item: &ClassSetItem) -> Result<(), Refusal> {
        match item {
            ClassSetItem::Perl(c) => self.perl(c, true),
            ClassSetItem::Unicode(c) => self.unicode(c),
            ClassSetItem::Bracketed(_) => refuse(
                "a class inside a class: ECMA-262 reads `[a[b]]` as the class `[a[b]` then a bare `]`",
            ),
            ClassSetItem::Ascii(_) => refuse(
                "POSIX class `[[:alpha:]]`: ECMA-262 reads the characters `[:alph` literally",
            ),
            ClassSetItem::Literal(l) => self.literal(l, true),
            ClassSetItem::Range(r) => {
                self.literal(&r.start, true)?;
                self.literal(&r.end, true)
            }
            _ => Ok(()),
        }
    }

    fn visit_class_set_binary_op_pre(&mut self, _: &ast::ClassSetBinaryOp) -> Result<(), Refusal> {
        refuse("class set operations `&&`, `--`, `~~`: ECMA-262 reads them as literal characters")
    }

    fn finish(self) -> Result<Self::Output, Refusal> {
        Ok(self.edits)
    }
}
