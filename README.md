# pattern-emit

Match a text against a pattern like `gr(a|e)y` without shipping a regex engine. At compile time this crate turns each constant pattern into a small table of bytes; the program you ship only walks the table.

`build` reads a pattern as Go's `regexp` (RE2) does; `build_ecma` reads it as ECMA-262 does in Unicode mode with no flags, which is JSON Schema's `pattern`. Where the Rust parser, `regex-syntax`, would read one differently, compile rewrites it when the dialect defines it exactly, and otherwise refuses it and says why.

## Use

Compile side (feature `build`), once per pattern:

```rust,ignore
let table = pattern_emit::build(r"gr(a|e)y")?;      // Vec<u8>, or a Refusal saying why not
let source = pattern_emit::byte_string(&table);     // b"\x00..." to paste into generated code
```

The generated code, which needs no feature:

```rust,ignore
static P0: pattern_emit::Pattern = pattern_emit::Pattern::new(b"...");
let hit = P0.is_match("a grey day")?;               // true: matches anywhere, as RE2's MatchString
```

`load` checks the table and keeps it, through `spin`'s `Once`, so `is_match` never does; a table nothing loaded gets its check on its first search. The search side is `no_std` with no allocator. A damaged table, or a search that cannot finish, is an `Err(Unusable)`, never a panic or a trap.

## The guard

The table is a sparse DFA from `regex-automata`, so both sides must use one `regex-automata` version: its byte format changes between versions.

Written as RE2 defines them, whatever the Rust parser would do:

- `\w`, `\d`, `\s`, `\b`, `\B` are ASCII. `\s` leaves out `\v`.
- Case folding is Unicode simple folding (`(?i)k` matches the Kelvin sign).

Refused, with the RE2 reading named:

- flags `x`, `u`, `R`;
- class set operations (`&&`, `--`, `~~`) and classes inside classes, which RE2 reads as literal characters;
- `\<`, `\>`, `\b{...}`, `\u` and `\U` escapes, `\p{name=value}` and property names RE2 lacks (`\p{Emoji}`);
- nested repetition (`a**`, `a{2}{3}`) and counts past 1000.

A test replays Go's recorded answers: the guard refuses every pattern Go rejects, and every table agrees with Go on every input. `tests/fixture/guard.txt` lists which patterns Go accepts and the guard declines.

## The ECMA guard

Written as ECMA-262 defines them:

- `.` leaves out `\n`, `\r`, U+2028, and U+2029.
- `\s` is WhiteSpace and LineTerminator (`\v`, U+00A0, U+FEFF, the Unicode spaces); `\w`, `\d`, `\b`, `\B` are ASCII.
- `^` and `$` anchor the whole text.

Refused, with the ECMA reading named:

- inline flags (`(?i)`, `(?i:…)`), `\A`, `\z`, `\Q…\E`, `\a`, `\<`, `\>`, `\cX`, octal and `\0`;
- POSIX classes (`[[:alpha:]]` is literal characters in ECMA), class set operations, and classes inside classes;
- `\x{…}` and `\U` escapes, escapes of characters that are not syntax, a bare `{`, `}`, or `]`, and `]` first in a class;
- `\pN` without braces, `\p{^…}`, and every property but a general category by short name (`\p{Lu}`): long names, scripts, binary properties like `\p{Emoji}`;
- `(?P<name>…)`, group names beyond ASCII identifiers, and a name used twice;
- lookaround and backreferences, which ECMA has and a table cannot answer;
- a repeated repeat or assertion (`a**`, `^*`).

The guard refuses a surrogate pair written as two `\u` escapes. Categories follow regex-syntax's Unicode version, which may differ from a JavaScript engine's for newly assigned characters.

The same test replays `tests/fixture/ecma.json`, recorded with Bun (JavaScriptCore) over about 500 patterns and 160 inputs; `ecma.txt` lists which patterns ECMA accepts and the guard declines. V8 differs on one answer, `\B` between the halves of a surrogate pair, against the standard.

## Size and cost

A table may take `TABLE_CAP` bytes: 64 KiB. Ordinary patterns take 1 to 2 KiB; `\p{Lu}` takes about 30 KiB, `.{20}` about 17 KiB. What grows a table is Unicode classes (`\pL` takes about 98 KiB, over the cap), a long `.{n}`, and counted repetition after an unbounded prefix such as `(a|b)*a(a|b){10}`. Building stops early on such a blowup instead of running long.

In a WebAssembly module the search costs about 10 KB of code once, against about 67 KB for `regex-lite`. A search runs 6 to 10 times faster. The check at load runs WebAssembly instructions in proportion to the table: 40 to 340 per byte up to 10 KiB. Tables of 30 KiB and more run up to 805 per byte.

Captures and replacement are not supported; this answers "does it match".
