//! Generators for the Kafka contract's laws.
//!
//! Most of the texts a law draws are topic names, which Kafka allows only
//! from ASCII letters, digits, dots, underscores and dashes, and most of the
//! 32-bit integers it draws are partition counts, which are small. Drawn
//! from all texts and all integers, almost no case would pass the laws'
//! refinements. So three draws in four are names and small counts; the
//! fourth is drawn from the whole type, so invalid names and out-of-range
//! counts are still tried. A drawn text is never empty: the laws that need
//! the empty name write it out.
#![allow(dead_code)]
use proptest::prelude::*;

/// Topic-name-like texts, three times in four, and any text otherwise.
pub fn names() -> proptest::strategy::BoxedStrategy<std::string::String> {
    prop_oneof![
        3 => "[a-zA-Z0-9._-]{1,40}",
        1 => ".+",
    ]
    .boxed()
}

/// Counts: the laws draw partition counts and record budgets, which are
/// small. Three in four are drawn from 1 to 16; the rest from all of Int32,
/// so out-of-range counts are still tried.
pub fn counts() -> proptest::strategy::BoxedStrategy<i32> {
    prop_oneof![
        3 => 1i32..=16,
        1 => any::<i32>(),
    ]
    .boxed()
}
