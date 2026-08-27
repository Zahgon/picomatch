// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Sets of UTF-16 code units for character classes, and the case folding of the `i` flag.

use std::sync::OnceLock;

use crate::js::string::is_js_whitespace;

const MAX_UNIT: u32 = 0xFFFF;

/// A set of code units as sorted, disjoint inclusive ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharSet {
    ranges: Vec<(u16, u16)>,
}

impl CharSet {
    /// Adds an inclusive range.
    pub fn add_range(&mut self, low: u16, high: u16) {
        self.ranges.push((low, high));
    }

    /// Adds every member of another set.
    pub fn add_set(&mut self, other: &CharSet) {
        self.ranges.extend_from_slice(&other.ranges);
    }

    /// Sorts the ranges and merges the ones that touch.
    pub fn normalize(&mut self) {
        self.ranges.sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::with_capacity(self.ranges.len());
        for &(low, high) in &self.ranges {
            match merged.last_mut() {
                Some(last) if u32::from(low) <= u32::from(last.1) + 1 => last.1 = last.1.max(high),
                _ => merged.push((low, high)),
            }
        }
        self.ranges = merged;
    }

    /// The complement of a normalized set.
    pub fn inverted(&self) -> CharSet {
        let mut out = CharSet::default();
        let mut next = 0u32;
        for &(low, high) in &self.ranges {
            if u32::from(low) > next {
                out.add_range(next as u16, low - 1);
            }
            next = u32::from(high) + 1;
        }
        if next <= MAX_UNIT {
            out.add_range(next as u16, MAX_UNIT as u16);
        }
        out
    }

    /// Whether a normalized set holds a unit.
    pub fn contains(&self, unit: u16) -> bool {
        self.ranges
            .binary_search_by(|&(low, high)| {
                if high < unit {
                    std::cmp::Ordering::Less
                } else if low > unit {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    /// The members of the set.
    pub fn units(&self) -> impl Iterator<Item = u16> + '_ {
        self.ranges.iter().flat_map(|&(low, high)| low..=high)
    }

    /// `\d`.
    pub fn digits() -> CharSet {
        CharSet {
            ranges: vec![(0x30, 0x39)],
        }
    }

    /// `\w`.
    pub fn word() -> CharSet {
        CharSet {
            ranges: vec![(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)],
        }
    }

    /// `\s`.
    pub fn space() -> CharSet {
        let mut set = CharSet::default();
        for unit in (0..=MAX_UNIT as u16).filter(|&unit| is_js_whitespace(unit)) {
            set.add_range(unit, unit);
        }
        set.normalize();
        set
    }
}

/// The result of ECMAScript's `Canonicalize` without the `u` flag: the upper case of a unit when that is one code
/// unit, and never a non-ASCII unit folding to an ASCII one.
pub fn canonicalize(unit: u16) -> u16 {
    if unit < 0x80 {
        return if (0x61..=0x7A).contains(&unit) {
            unit - 0x20
        } else {
            unit
        };
    }
    fold_table()[usize::from(unit)]
}

fn fold_table() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| (0..=MAX_UNIT).map(fold_unit).collect())
}

fn fold_unit(code: u32) -> u16 {
    let unit = code as u16;
    let Some(ch) = char::from_u32(code) else {
        return unit;
    };
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => {
            let folded = u32::from(single);
            if folded > MAX_UNIT || (code >= 0x80 && folded < 0x80) {
                unit
            } else {
                folded as u16
            }
        }
        _ => unit,
    }
}

/// A set tested through case folding: one bit per canonical unit.
#[derive(Clone, Debug)]
pub struct FoldedSet {
    bits: Vec<u64>,
}

impl FoldedSet {
    /// The canonical units of a set's members.
    pub fn new(set: &CharSet) -> FoldedSet {
        let mut bits = vec![0u64; 1024];
        for unit in set.units() {
            let folded = canonicalize(unit);
            bits[usize::from(folded >> 6)] |= 1 << (folded & 63);
        }
        FoldedSet { bits }
    }

    /// Whether some member folds to the same unit as `unit`.
    pub fn contains(&self, unit: u16) -> bool {
        let folded = canonicalize(unit);
        self.bits[usize::from(folded >> 6)] & (1 << (folded & 63)) != 0
    }
}
