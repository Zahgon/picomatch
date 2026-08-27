// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The syntax tree of a regular expression.

use super::charset::CharSet;
use crate::js::JsString;

/// A parsed pattern.
#[derive(Clone, Debug)]
pub struct Pattern {
    /// The root of the tree.
    pub root: Node,
    /// The number of capture groups, not counting capture 0.
    pub groups: usize,
    /// The named groups in source order, with their capture numbers.
    pub names: Vec<(JsString, usize)>,
}

impl Pattern {
    /// `$^`.
    pub fn never() -> Pattern {
        Pattern {
            root: Node::Sequence(vec![Node::End, Node::Start]),
            groups: 0,
            names: Vec::new(),
        }
    }
}

/// A node of the tree.
#[derive(Clone, Debug)]
pub enum Node {
    /// One literal code unit.
    Unit(u16),
    /// `.`.
    AnyUnit,
    /// A character class; `negated` is the leading `^`, which applies after any case folding.
    Class {
        /// The members as written.
        set: CharSet,
        /// Whether the class was written with `^`.
        negated: bool,
    },
    /// Terms in source order.
    Sequence(Vec<Node>),
    /// Alternatives in source order.
    Alternation(Vec<Node>),
    /// A capture group with its number.
    Capture(usize, Box<Node>),
    /// A quantified atom.
    Repeat(Repeat),
    /// `^`.
    Start,
    /// `$`.
    End,
    /// `\b`, or `\B` when negated.
    WordBoundary {
        /// Whether this is `\B`.
        negated: bool,
    },
    /// A lookahead or lookbehind.
    Look {
        /// The asserted pattern.
        body: Box<Node>,
        /// Whether this is a lookbehind.
        behind: bool,
        /// Whether the assertion is negative.
        negated: bool,
    },
    /// A backreference to a capture group.
    BackReference(usize),
}

/// A quantified atom.
#[derive(Clone, Debug)]
pub struct Repeat {
    /// The repeated atom.
    pub body: Box<Node>,
    /// The fewest iterations.
    pub min: u32,
    /// The most iterations; `None` is unbounded.
    pub max: Option<u32>,
    /// Whether more iterations are tried first.
    pub greedy: bool,
    /// The capture groups inside the atom, which every iteration resets: `first..end`.
    pub captures: (usize, usize),
}
