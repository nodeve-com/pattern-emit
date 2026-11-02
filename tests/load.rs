//! `Pattern::load` checks a table once; a damaged one is `Unusable` there and at every search.

use pattern_emit::{build, Pattern};

fn pattern(mut table: Vec<u8>, damage: bool) -> Pattern {
    if damage {
        table.iter_mut().take(8).for_each(|b| *b = 0xff);
    }
    Pattern::new(Box::leak(table.into_boxed_slice()))
}

#[test]
fn a_table_loads_and_then_searches() {
    let p = pattern(build("gr(a|e)y").unwrap(), false);
    assert_eq!(p.load(), Ok(()));
    assert_eq!(p.load(), Ok(()));
    assert_eq!(p.is_match("a grey day"), Ok(true));
    assert_eq!(p.is_match("a green day"), Ok(false));
}

#[test]
fn a_table_nothing_loaded_is_checked_by_its_first_search() {
    let p = pattern(build("gr(a|e)y").unwrap(), false);
    assert_eq!(p.is_match("gray"), Ok(true));
}

#[test]
fn a_damaged_table_is_unusable_at_load_and_at_every_search() {
    let p = pattern(build("gr(a|e)y").unwrap(), true);
    let why = p.load().unwrap_err();
    assert_eq!(why.to_string(), "pattern table failed its check");
    assert_eq!(p.load(), Err(why));
    assert_eq!(p.is_match("grey"), Err(why));
}
