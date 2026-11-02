#![doc = include_str!("../README.md")]
// The search side is core-only; std comes with the compile side.
#![cfg_attr(not(feature = "build"), no_std)]

mod search;
pub use search::{Pattern, Unusable};

#[cfg(feature = "build")]
mod ecma;
#[cfg(feature = "build")]
mod guard;
#[cfg(feature = "build")]
mod table;
#[cfg(feature = "build")]
pub use table::{build, build_ecma, byte_string, matches_ecma, Refusal, TABLE_CAP};
