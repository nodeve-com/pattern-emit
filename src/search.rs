use core::fmt;
use regex_automata::dfa::{sparse::DFA, Automaton};
use regex_automata::Input;
use spin::Once;

/// Why a pattern can't answer: its table failed the load check, or a search gave up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unusable(&'static str);

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl core::error::Error for Unusable {}

/// One pattern's table, checked once and kept.
///
/// `static P: Pattern = Pattern::new(&[..]);` with the bytes `build` returned.
pub struct Pattern {
    bytes: &'static [u8],
    dfa: Once<Result<DFA<&'static [u8]>, Unusable>>,
}

impl Pattern {
    pub const fn new(bytes: &'static [u8]) -> Self {
        Pattern {
            bytes,
            dfa: Once::new(),
        }
    }

    /// Checks the table and keeps it, so a search never does. A caller checks each table once,
    /// before its first search.
    pub fn load(&self) -> Result<(), Unusable> {
        self.dfa().map(|_| ())
    }

    fn dfa(&self) -> Result<&DFA<&'static [u8]>, Unusable> {
        self.dfa
            .call_once(|| {
                DFA::from_bytes(self.bytes)
                    .map(|(dfa, _)| dfa)
                    .map_err(|_| Unusable("pattern table failed its check"))
            })
            .as_ref()
            .map_err(|e| *e)
    }

    /// Whether the pattern matches anywhere in `s`, as RE2's `MatchString` does.
    ///
    /// A corrupt table or a search the table can't finish is an `Err`, never a panic. A table
    /// [`Pattern::load`] has not checked is checked now.
    pub fn is_match(&self, s: &str) -> Result<bool, Unusable> {
        self.dfa()?
            .try_search_fwd(&Input::new(s).earliest(true))
            .map(|m| m.is_some())
            .map_err(|_| Unusable("pattern search gave up"))
    }
}
