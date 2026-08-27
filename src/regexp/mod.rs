// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! A JavaScript `RegExp` for the source strings the library generates and passes through.
//!
//! Matching works on UTF-16 code units with the ECMAScript backtracking semantics, including the Annex B syntax that
//! non-Unicode patterns allow. A pattern goes through three stages: [`parser`] checks the syntax exactly as
//! `new RegExp` does and builds a tree, [`compiler`] lowers the tree to a program, and [`vm`] runs the program.
//! The `u` and `v` flags are the one part of `RegExp` this module leaves out.

mod ast;
mod charset;
mod compiler;
mod parser;
mod vm;

use crate::js::JsString;

use compiler::Program;

/// Why a pattern or its flags were rejected, with the text `new RegExp` reports after the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxError(pub &'static str);

/// The flags of a regular expression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `g`: matching starts at `lastIndex`, which then moves on.
    pub global: bool,
    /// `i`: code units compare by their canonical upper case.
    pub ignore_case: bool,
    /// `m`: `^` and `$` also match at line terminators.
    pub multiline: bool,
    /// `s`: `.` also matches line terminators.
    pub dot_all: bool,
    /// `y`: matching happens at `lastIndex` only.
    pub sticky: bool,
}

impl Flags {
    /// Parses a flags string; every flag may appear once.
    pub fn parse(text: &[u16]) -> Result<Flags, SyntaxError> {
        let mut flags = Flags::default();
        let mut has_indices = false;
        for &unit in text {
            let slot = match char::from_u32(u32::from(unit)) {
                Some('g') => &mut flags.global,
                Some('i') => &mut flags.ignore_case,
                Some('m') => &mut flags.multiline,
                Some('s') => &mut flags.dot_all,
                Some('y') => &mut flags.sticky,
                Some('d') => &mut has_indices,
                _ => return Err(SyntaxError("Invalid flags")),
            };
            if std::mem::replace(slot, true) {
                return Err(SyntaxError("Invalid flags"));
            }
        }
        Ok(flags)
    }
}

/// One capture: a range of code units of the input.
pub type Capture = Option<(usize, usize)>;

/// A successful match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// Capture 0 is the whole match; a capture that took no part is `None`.
    pub captures: Vec<Capture>,
    /// The named groups in source order, with their capture numbers.
    pub names: Vec<(JsString, usize)>,
}

impl Match {
    /// Where the match starts.
    pub fn index(&self) -> usize {
        self.captures[0].map_or(0, |(start, _)| start)
    }

    /// Where the match ends.
    pub fn end(&self) -> usize {
        self.captures[0].map_or(0, |(_, end)| end)
    }
}

/// A compiled regular expression with its `lastIndex`.
#[derive(Clone, Debug)]
pub struct RegExp {
    program: Program,
    flags: Flags,
    last_index: usize,
}

impl RegExp {
    /// `new RegExp(source, flags)`.
    pub fn new(source: &[u16], flags: &[u16]) -> Result<RegExp, SyntaxError> {
        let flags = Flags::parse(flags)?;
        let pattern = parser::parse(source)?;
        Ok(RegExp {
            program: compiler::compile(&pattern, flags),
            flags,
            last_index: 0,
        })
    }

    /// A regular expression from a source and flags known to be valid; `None` otherwise. Callers
    /// pass fixed sources that always compile, so their `None` arms are never taken.
    pub fn literal(source: &str, flags: &str) -> Option<RegExp> {
        let source: Vec<u16> = source.encode_utf16().collect();
        let flags: Vec<u16> = flags.encode_utf16().collect();
        RegExp::new(&source, &flags).ok()
    }

    /// `/$^/`, the expression the library falls back to.
    pub fn never() -> RegExp {
        RegExp {
            program: compiler::compile(&ast::Pattern::never(), Flags::default()),
            flags: Flags::default(),
            last_index: 0,
        }
    }

    /// `regex.exec(input)`: the `g` and `y` flags read and move `lastIndex`.
    pub fn exec(&mut self, input: &[u16]) -> Option<Match> {
        let stateful = self.flags.global || self.flags.sticky;
        let start = if stateful { self.last_index } else { 0 };
        if start > input.len() {
            self.last_index = 0;
            return None;
        }
        let found = self.search(input, start);
        if stateful {
            self.last_index = found.as_ref().map_or(0, Match::end);
        }
        found
    }

    /// The first match at or after `start` (at `start` only with `y`); `lastIndex` stays as it is.
    pub fn search(&self, input: &[u16], start: usize) -> Option<Match> {
        let last = if self.flags.sticky { start } else { input.len() };
        (start..=last).find_map(|index| {
            let captures = vm::run(&self.program, input, index)?;
            Some(Match {
                captures,
                names: self.program.names.clone(),
            })
        })
    }

    /// `input.replace(regex, replacer)` for a global expression; the replacer gets the match.
    pub fn replace_all(&self, input: &[u16], mut replacer: impl FnMut(&Match) -> Vec<u16>) -> Vec<u16> {
        let mut out = Vec::with_capacity(input.len());
        let mut copied = 0;
        let mut index = 0;
        while index <= input.len() {
            let Some(found) = self.search(input, index) else {
                break;
            };
            out.extend_from_slice(&input[copied..found.index()]);
            out.extend(replacer(&found));
            copied = found.end();
            index = if found.end() == found.index() {
                found.end() + 1
            } else {
                found.end()
            };
        }
        out.extend_from_slice(input.get(copied..).unwrap_or_default());
        out
    }
}

#[cfg(test)]
mod tests;
