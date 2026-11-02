use crate::{ecma, guard};
use core::fmt;
use regex_automata::dfa::{dense, sparse, Automaton};
use regex_automata::nfa::thompson;
use regex_automata::{dfa::StartKind, Input, MatchKind};

/// The most bytes one pattern's table may take: 64 KiB. Every table the research measured for the
/// patterns people write is under 16 KiB; Unicode classes and counted repetition are what pass it.
pub const TABLE_CAP: usize = 64 * 1024;

/// Determinization and the NFA stop growing at this many bytes, so a blowup fails fast.
const BUILD_LIMIT: usize = 1 << 20;

/// Why a pattern has no table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub(crate) String);

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for Refusal {}

const HOW_TO_SHRINK: &str = "Unicode classes (`\\pL`, `\\p{Greek}`), a long `.{n}`, and counted \
repetition after an unbounded prefix are what grow a table; narrow or drop them";

/// The table for `pattern`, read as RE2 reads it: a minimized sparse DFA of the unanchored search,
/// as bytes [`crate::Pattern::new`] takes.
///
/// A pattern Go's `regexp` rejects, a form where regex-syntax and RE2 differ, or a table over
/// [`TABLE_CAP`] is a [`Refusal`] saying why.
pub fn build(pattern: &str) -> Result<Vec<u8>, Refusal> {
    table(&guard::rewrite(pattern)?)
}

/// The table for `pattern`, read as ECMA-262 reads it in Unicode mode with no flags, the dialect
/// of JSON Schema's `pattern`: as [`build`], with the ECMA guard.
///
/// A pattern ECMA rejects, a form ECMA reads differently from regex-syntax (inline flags, `\A`,
/// POSIX classes, `\x{…}`), one it has and a table cannot answer (lookaround, backreferences), or
/// a table over [`TABLE_CAP`] is a [`Refusal`] saying why.
pub fn build_ecma(pattern: &str) -> Result<Vec<u8>, Refusal> {
    table(&ecma::rewrite(pattern)?)
}

/// Whether `pattern`, read as [`build_ecma`] reads it, matches anywhere in `text`: for a compiler
/// that must know which names a pattern reaches. The table is built, searched, and dropped.
pub fn matches_ecma(pattern: &str, text: &str) -> Result<bool, Refusal> {
    let bytes = build_ecma(pattern)?;
    let (dfa, _) = sparse::DFA::from_bytes(&bytes)
        .map_err(|e| Refusal(format!("table failed its check: {e}")))?;
    dfa.try_search_fwd(&Input::new(text).earliest(true))
        .map(|m| m.is_some())
        .map_err(|e| Refusal(format!("search gave up: {e}")))
}

fn table(rewritten: &str) -> Result<Vec<u8>, Refusal> {
    let dense = dense::Builder::new()
        .configure(
            dense::Config::new()
                .start_kind(StartKind::Unanchored)
                .match_kind(MatchKind::All)
                .minimize(true)
                .determinize_size_limit(Some(BUILD_LIMIT))
                .dfa_size_limit(Some(BUILD_LIMIT)),
        )
        .thompson(thompson::Config::new().nfa_size_limit(Some(BUILD_LIMIT)))
        .build(rewritten)
        .map_err(|e| {
            let nfa_size = std::error::Error::source(&e)
                .and_then(|s| s.downcast_ref::<thompson::BuildError>())
                .is_some_and(|n| n.size_limit().is_some());
            if e.is_size_limit_exceeded() || nfa_size {
                Refusal(format!(
                    "pattern's table passes {BUILD_LIMIT} bytes while building; {HOW_TO_SHRINK}"
                ))
            } else {
                Refusal(format!("no table: {}", first_line(&e.to_string())))
            }
        })?;
    let bytes = dense
        .to_sparse()
        .map_err(|e| Refusal(format!("table not built: {e}")))?
        .to_bytes_little_endian();
    if bytes.len() > TABLE_CAP {
        return Err(Refusal(format!(
            "pattern's table is {} bytes, over the {TABLE_CAP} cap; {HOW_TO_SHRINK}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// `bytes` as a Rust byte-string literal, `b"\x01…"`, for emitted code.
pub fn byte_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    out.push_str("b\"");
    for &b in bytes {
        match b {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x20..=0x7e => out.push(char::from(b)),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out.push('"');
    out
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}
