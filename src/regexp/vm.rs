// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The backtracking machine. Backtracking is explicit, so the length of the input does not grow the native stack;
//! only nested lookarounds recurse.

use super::charset::canonicalize;
use super::compiler::{ClassTest, Inst, Program};
use super::Capture;

const UNSET: i64 = -1;

/// An entry of the backtrack stack.
enum Frame {
    /// Resume here when everything after the choice fails.
    Branch { pc: usize, position: usize },
    /// Put a register back on the way down.
    Restore { register: usize, value: i64 },
}

struct Machine<'a> {
    program: &'a Program,
    input: &'a [u16],
    registers: Vec<i64>,
    stack: Vec<Frame>,
}

/// Runs a program with the match starting at `start`; the captures on success.
pub fn run(program: &Program, input: &[u16], start: usize) -> Option<Vec<Capture>> {
    let mut machine = Machine {
        program,
        input,
        registers: vec![UNSET; program.registers],
        stack: Vec::new(),
    };
    machine.execute(0, start).then(|| machine.captures())
}

fn is_line_terminator(unit: u16) -> bool {
    matches!(unit, 0x0A | 0x0D | 0x2028 | 0x2029)
}

fn is_word_unit(unit: u16) -> bool {
    matches!(unit, 0x30..=0x39 | 0x41..=0x5A | 0x5F | 0x61..=0x7A)
}

impl Machine<'_> {
    fn captures(&self) -> Vec<Capture> {
        (0..self.program.captures)
            .map(|capture| {
                let (start, end) = (self.registers[2 * capture], self.registers[2 * capture + 1]);
                (start != UNSET && end != UNSET).then_some((start as usize, end as usize))
            })
            .collect()
    }

    fn set_register(&mut self, register: usize, value: i64) {
        let old = std::mem::replace(&mut self.registers[register], value);
        self.stack.push(Frame::Restore { register, value: old });
    }

    /// The unit to consume from `position`, and where that leaves the position.
    fn read(&self, position: usize, back: bool) -> Option<(u16, usize)> {
        if back {
            let next = position.checked_sub(1)?;
            Some((self.input[next], next))
        } else {
            Some((*self.input.get(position)?, position + 1))
        }
    }

    fn class_matches(&self, class: usize, unit: u16) -> bool {
        match &self.program.classes[class] {
            ClassTest::Exact(set) => set.contains(unit),
            ClassTest::Folded { set, negated } => set.contains(unit) != *negated,
        }
    }

    fn units_equal(&self, a: u16, b: u16) -> bool {
        a == b || (self.program.flags.ignore_case && canonicalize(a) == canonicalize(b))
    }

    fn at_word_boundary(&self, position: usize) -> bool {
        let before = position > 0 && is_word_unit(self.input[position - 1]);
        let after = self.input.get(position).is_some_and(|&unit| is_word_unit(unit));
        before != after
    }

    fn back_reference(&self, capture: usize, position: usize, back: bool) -> Option<usize> {
        let (start, end) = (self.registers[2 * capture], self.registers[2 * capture + 1]);
        if start == UNSET || end == UNSET {
            return Some(position);
        }
        let text = &self.input[start as usize..end as usize];
        let begin = if back {
            position.checked_sub(text.len())?
        } else {
            position
        };
        let candidate = self.input.get(begin..begin + text.len())?;
        text.iter()
            .zip(candidate)
            .all(|(&a, &b)| self.units_equal(a, b))
            .then_some(if back { begin } else { begin + text.len() })
    }

    /// Pops to the next branch above `base`, restoring registers on the way.
    fn backtrack(&mut self, base: usize) -> Option<(usize, usize)> {
        while self.stack.len() > base {
            match self.stack.pop() {
                Some(Frame::Branch { pc, position }) => return Some((pc, position)),
                Some(Frame::Restore { register, value }) => self.registers[register] = value,
                None => break,
            }
        }
        None
    }

    /// Runs from `pc` until the program or the current assertion succeeds. On failure the stack is back at its
    /// height at the call and the registers are as they were.
    fn execute(&mut self, mut pc: usize, mut position: usize) -> bool {
        let base = self.stack.len();
        let program = self.program;
        loop {
            let advanced = match &program.insts[pc] {
                Inst::Unit { unit, back } => self
                    .read(position, *back)
                    .filter(|&(read, _)| self.units_equal(read, *unit))
                    .map(|(_, next)| (pc + 1, next)),
                Inst::Class { class, back } => self
                    .read(position, *back)
                    .filter(|&(read, _)| self.class_matches(*class, read))
                    .map(|(_, next)| (pc + 1, next)),
                Inst::Dot { back } => self
                    .read(position, *back)
                    .filter(|&(read, _)| program.flags.dot_all || !is_line_terminator(read))
                    .map(|(_, next)| (pc + 1, next)),
                Inst::Split { first, second } => {
                    self.stack.push(Frame::Branch { pc: *second, position });
                    Some((*first, position))
                }
                Inst::Jump(target) => Some((*target, position)),
                Inst::GroupStart { slot } => {
                    self.set_register(*slot, position as i64);
                    Some((pc + 1, position))
                }
                Inst::GroupEnd { capture, slot, back } => {
                    let start = self.registers[*slot];
                    let end = position as i64;
                    let (low, high) = if *back { (end, start) } else { (start, end) };
                    self.set_register(2 * capture, low);
                    self.set_register(2 * capture + 1, high);
                    Some((pc + 1, position))
                }
                Inst::ClearCaptures { first, end } => {
                    for capture in *first..*end {
                        self.set_register(2 * capture, UNSET);
                        self.set_register(2 * capture + 1, UNSET);
                    }
                    Some((pc + 1, position))
                }
                Inst::ResetCounter { counter } => {
                    self.set_register(*counter, 0);
                    Some((pc + 1, position))
                }
                Inst::LoopHead {
                    counter,
                    min,
                    max,
                    greedy,
                    body,
                    exit,
                } => {
                    let count = self.registers[*counter];
                    if count < i64::from(*min) {
                        Some((*body, position))
                    } else if max.is_some_and(|max| count >= i64::from(max)) {
                        Some((*exit, position))
                    } else if *greedy {
                        self.stack.push(Frame::Branch { pc: *exit, position });
                        Some((*body, position))
                    } else {
                        self.stack.push(Frame::Branch { pc: *body, position });
                        Some((*exit, position))
                    }
                }
                Inst::LoopBody { position: slot } => {
                    self.set_register(*slot, position as i64);
                    Some((pc + 1, position))
                }
                Inst::LoopTail {
                    counter,
                    position: slot,
                    min,
                    head,
                } => {
                    let count = self.registers[*counter];
                    if count >= i64::from(*min) && self.registers[*slot] == position as i64 {
                        None
                    } else {
                        self.set_register(*counter, count + 1);
                        Some((*head, position))
                    }
                }
                Inst::Start => {
                    let holds =
                        position == 0 || (program.flags.multiline && is_line_terminator(self.input[position - 1]));
                    holds.then_some((pc + 1, position))
                }
                Inst::End => {
                    let holds = position == self.input.len()
                        || (program.flags.multiline && is_line_terminator(self.input[position]));
                    holds.then_some((pc + 1, position))
                }
                Inst::WordBoundary { negated } => {
                    (self.at_word_boundary(position) != *negated).then_some((pc + 1, position))
                }
                Inst::Look { negated, next } => {
                    let height = self.stack.len();
                    let succeeded = self.execute(pc + 1, position);
                    if succeeded && *negated {
                        while self.backtrack(height).is_some() {}
                    } else if succeeded {
                        self.drop_branches(height);
                    }
                    (succeeded != *negated).then_some((*next, position))
                }
                Inst::BackReference { capture, back } => self
                    .back_reference(*capture, position, *back)
                    .map(|next| (pc + 1, next)),
                Inst::LookEnd | Inst::Match => return true,
            };
            match advanced.or_else(|| self.backtrack(base)) {
                Some((next_pc, next_position)) => {
                    pc = next_pc;
                    position = next_position;
                }
                None => return false,
            }
        }
    }

    /// Makes an assertion atomic: its choices go, but its register changes stay undoable.
    fn drop_branches(&mut self, height: usize) {
        let mut kept = height;
        for index in height..self.stack.len() {
            if matches!(self.stack[index], Frame::Restore { .. }) {
                self.stack.swap(kept, index);
                kept += 1;
            }
        }
        self.stack.truncate(kept);
    }
}
