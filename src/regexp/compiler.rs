// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Lowers a syntax tree to a program for the backtracking machine.

use super::ast::{Node, Pattern, Repeat};
use super::charset::{CharSet, FoldedSet};
use super::Flags;
use crate::js::JsString;

/// How a class tests a unit.
#[derive(Clone, Debug)]
pub enum ClassTest {
    /// Directly.
    Exact(CharSet),
    /// Through case folding; `negated` turns the answer around, since a negated class is the complement of the
    /// folded test rather than the folded test of the complement.
    Folded {
        /// The canonical members.
        set: FoldedSet,
        /// Whether the class was written with `^`.
        negated: bool,
    },
}

/// One instruction. `back` means the instruction reads backwards, inside a lookbehind.
#[derive(Clone, Debug)]
pub enum Inst {
    /// Consume one given unit.
    Unit { unit: u16, back: bool },
    /// Consume one unit of a class.
    Class { class: usize, back: bool },
    /// Consume one unit other than a line terminator.
    Dot { back: bool },
    /// Try `first`, and `second` on failure.
    Split { first: usize, second: usize },
    /// Continue at another instruction.
    Jump(usize),
    /// Remember where a capture group begins.
    GroupStart { slot: usize },
    /// Set a capture from the remembered start and the current position.
    GroupEnd { capture: usize, slot: usize, back: bool },
    /// Reset the captures `first..end`.
    ClearCaptures { first: usize, end: usize },
    /// Set a counter register to zero.
    ResetCounter { counter: usize },
    /// Decide whether a loop runs another iteration.
    LoopHead {
        counter: usize,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        body: usize,
        exit: usize,
    },
    /// Remember where an iteration begins.
    LoopBody { position: usize },
    /// End an iteration: reject an empty one past the minimum, count it, and go back to the head.
    LoopTail {
        counter: usize,
        position: usize,
        min: u32,
        head: usize,
    },
    /// `^`.
    Start,
    /// `$`.
    End,
    /// `\b` or `\B`.
    WordBoundary { negated: bool },
    /// Run the assertion that follows; continue at `next`.
    Look { negated: bool, next: usize },
    /// The assertion's body succeeded.
    LookEnd,
    /// Compare with an earlier capture.
    BackReference { capture: usize, back: bool },
    /// The whole pattern matched.
    Match,
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Program {
    /// The instructions.
    pub insts: Vec<Inst>,
    /// The classes the instructions refer to.
    pub classes: Vec<ClassTest>,
    /// The number of captures, capture 0 included.
    pub captures: usize,
    /// The number of registers: two per capture, then start slots, counters and iteration positions.
    pub registers: usize,
    /// The named groups.
    pub names: Vec<(JsString, usize)>,
    /// The flags the program was compiled for.
    pub flags: Flags,
}

/// Compiles a pattern.
pub fn compile(pattern: &Pattern, flags: Flags) -> Program {
    let captures = pattern.groups + 1;
    let mut compiler = Compiler {
        program: Program {
            insts: Vec::new(),
            classes: Vec::new(),
            captures,
            registers: captures * 3,
            names: pattern.names.clone(),
            flags,
        },
    };
    compiler.emit(Inst::GroupStart { slot: 2 * captures });
    compiler.node(&pattern.root, false);
    compiler.emit(Inst::GroupEnd {
        capture: 0,
        slot: 2 * captures,
        back: false,
    });
    compiler.emit(Inst::Match);
    compiler.program
}

struct Compiler {
    program: Program,
}

impl Compiler {
    fn emit(&mut self, inst: Inst) -> usize {
        self.program.insts.push(inst);
        self.program.insts.len() - 1
    }

    fn next_pc(&self) -> usize {
        self.program.insts.len()
    }

    fn register(&mut self) -> usize {
        self.program.registers += 1;
        self.program.registers - 1
    }

    fn start_slot(&self, capture: usize) -> usize {
        2 * self.program.captures + capture
    }

    fn node(&mut self, node: &Node, back: bool) {
        match node {
            Node::Unit(unit) => {
                self.emit(Inst::Unit { unit: *unit, back });
            }
            Node::AnyUnit => {
                self.emit(Inst::Dot { back });
            }
            Node::Class { set, negated } => {
                let class = self.class(set, *negated);
                self.emit(Inst::Class { class, back });
            }
            Node::Sequence(items) => {
                if back {
                    items.iter().rev().for_each(|item| self.node(item, back));
                } else {
                    items.iter().for_each(|item| self.node(item, back));
                }
            }
            Node::Alternation(items) => self.alternation(items, back),
            Node::Capture(number, body) => {
                let slot = self.start_slot(*number);
                self.emit(Inst::GroupStart { slot });
                self.node(body, back);
                self.emit(Inst::GroupEnd {
                    capture: *number,
                    slot,
                    back,
                });
            }
            Node::Repeat(repeat) => self.repeat(repeat, back),
            Node::Start => {
                self.emit(Inst::Start);
            }
            Node::End => {
                self.emit(Inst::End);
            }
            Node::WordBoundary { negated } => {
                self.emit(Inst::WordBoundary { negated: *negated });
            }
            Node::Look { body, behind, negated } => {
                let look = self.emit(Inst::Look {
                    negated: *negated,
                    next: 0,
                });
                self.node(body, *behind);
                self.emit(Inst::LookEnd);
                let next = self.next_pc();
                self.program.insts[look] = Inst::Look {
                    negated: *negated,
                    next,
                };
            }
            Node::BackReference(capture) => {
                self.emit(Inst::BackReference {
                    capture: *capture,
                    back,
                });
            }
        }
    }

    fn class(&mut self, set: &CharSet, negated: bool) -> usize {
        let test = if self.program.flags.ignore_case {
            ClassTest::Folded {
                set: FoldedSet::new(set),
                negated,
            }
        } else if negated {
            ClassTest::Exact(set.inverted())
        } else {
            ClassTest::Exact(set.clone())
        };
        self.program.classes.push(test);
        self.program.classes.len() - 1
    }

    fn alternation(&mut self, items: &[Node], back: bool) {
        let mut jumps = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if i + 1 == items.len() {
                self.node(item, back);
                break;
            }
            let split = self.emit(Inst::Split { first: 0, second: 0 });
            self.node(item, back);
            jumps.push(self.emit(Inst::Jump(0)));
            let second = self.next_pc();
            self.program.insts[split] = Inst::Split {
                first: split + 1,
                second,
            };
        }
        let end = self.next_pc();
        for jump in jumps {
            self.program.insts[jump] = Inst::Jump(end);
        }
    }

    fn repeat(&mut self, repeat: &Repeat, back: bool) {
        let counter = self.register();
        let position = self.register();
        self.emit(Inst::ResetCounter { counter });
        let head = self.emit(Inst::Jump(0));
        let body = self.emit(Inst::LoopBody { position });
        let (first, end) = repeat.captures;
        if first < end {
            self.emit(Inst::ClearCaptures { first, end });
        }
        self.node(&repeat.body, back);
        self.emit(Inst::LoopTail {
            counter,
            position,
            min: repeat.min,
            head,
        });
        let exit = self.next_pc();
        self.program.insts[head] = Inst::LoopHead {
            counter,
            min: repeat.min,
            max: repeat.max,
            greedy: repeat.greedy,
            body,
            exit,
        };
    }
}
