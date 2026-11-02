//! The guards against the engines they match, without them.
//!
//! `fixture/go.json` was recorded once with Go 1.26's `regexp` (RE2) and `fixture/ecma.json` with
//! Node's `new RegExp(pattern, "u")` (ECMA-262, Unicode mode, no flags; its version is in the
//! file): per pattern, whether it compiles and, if so, whether `test` or `MatchString` finds it
//! in each input ("1"/"0" in input order). `fixture/guard.txt` and `fixture/ecma.txt` list each
//! pattern the engine accepts as `pass` or `refuse` (the guard declines it); set `UPDATE=1` to
//! rewrite them after a change on purpose.
#![cfg(feature = "build")]
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use pattern_emit::{build, build_ecma, Pattern, Refusal};
use std::fmt::Write;

/// Replays one engine's recorded answers (`key` per pattern) against `builder`.
fn replay(file: &str, key: &str, list: &str, builder: fn(&str) -> Result<Vec<u8>, Refusal>) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixture/");
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}{file}")).unwrap()).unwrap();
    let inputs: Vec<&str> = fixture["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    let mut listing = String::new();
    let mut bad = Vec::new();
    for entry in fixture["patterns"].as_array().unwrap() {
        let pattern = entry["p"].as_str().unwrap();
        let table = builder(pattern);
        let Some(answers) = entry[key].as_str() else {
            if table.is_ok() {
                bad.push(format!("{pattern:?}: the engine rejects it, build accepts"));
            }
            continue;
        };
        match table {
            Err(why) => {
                writeln!(listing, "refuse\t{pattern:?}\t{why}").unwrap();
                continue;
            }
            Ok(bytes) => {
                let p = Pattern::new(Box::leak(bytes.into_boxed_slice()));
                for (i, input) in inputs.iter().enumerate() {
                    let want = answers.as_bytes().get(i) == Some(&b'1');
                    if p.is_match(input) != Ok(want) {
                        bad.push(format!("{pattern:?} on {input:?}: the engine says {want}"));
                    }
                }
                writeln!(listing, "pass\t{pattern:?}").unwrap();
            }
        }
    }
    assert!(bad.is_empty(), "disagreements:\n{}", bad.join("\n"));

    let path = format!("{dir}{list}");
    if std::env::var_os("UPDATE").is_some() {
        std::fs::write(&path, &listing).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        listing,
        "{list} is stale; UPDATE=1"
    );
}

#[test]
fn guard_agrees_with_go() {
    replay("go.json", "go", "guard.txt", build);
}

#[test]
fn ecma_guard_agrees_with_node() {
    replay("ecma.json", "ecma", "ecma.txt", build_ecma);
}

#[test]
fn refusals_say_why() {
    for (pattern, why) in [
        (r"(?x)a b", "flag `x`"),
        (r"[a&&b]", "&&"),
        (r"\<", "word boundar"),
        (r"a**", "nested repetition"),
        (r"a{1001}", "limit of 1000"),
        (r"\pL+", "cap"),
        (r"(", "not an RE2 pattern"),
    ] {
        let e = build(pattern).unwrap_err().to_string();
        assert!(e.contains(why), "{pattern}: {e}");
    }
}

#[test]
fn ecma_refusals_say_why() {
    for (pattern, why) in [
        (r"(?i)a", "inline flags"),
        (r"\Aa", "`\\A`"),
        (r"[[:alpha:]]", "POSIX class"),
        (r"\Qa\E", "not an ECMA-262 pattern"),
        (r"\x{41}", "`\\x{…}`"),
        (r"\pL", "general categories"),
        (r"(?P<n>a)", "(?<name>"),
        (r"(?=a)", "lookaround"),
        (r"(a)\1", "backreference"),
        (r"\0", "write `\\x00`"),
        (r"a{", "not an ECMA-262 pattern"),
        (r"a}", "bare"),
        (r"[]a]", "first in a bracket"),
        (r"\a", "escape"),
        (r"^*", "repeated assertion"),
    ] {
        let e = build_ecma(pattern).unwrap_err().to_string();
        assert!(e.contains(why), "{pattern}: {e}");
    }
}

#[test]
fn bad_tables_are_errors_not_panics() {
    let good = build("a+b").unwrap();
    let leak = |b: Vec<u8>| Pattern::new(Box::leak(b.into_boxed_slice()));
    assert!(leak(good.clone()).is_match("xaab") == Ok(true));
    assert!(leak(good[..good.len() / 2].to_vec())
        .is_match("ab")
        .is_err());
    assert!(leak(vec![0xff; 64]).is_match("ab").is_err());
    assert!(leak(Vec::new()).is_match("ab").is_err());
    let mut flipped = good;
    for b in flipped.iter_mut().skip(40) {
        *b = !*b;
    }
    assert!(leak(flipped).is_match("ab").is_err());
}
