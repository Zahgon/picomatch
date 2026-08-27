// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The syntax of JavaScript regular expressions without the `u` and `v` flags, Annex B included. A pattern this parser
//! rejects is one `new RegExp` rejects.

use super::ast::{Node, Pattern, Repeat};
use super::charset::CharSet;
use super::SyntaxError;
use crate::js::JsString;

/// V8's stand-in for an unbounded or overflowing quantifier number.
const QUANTIFIER_INFINITY: u32 = i32::MAX as u32;

/// How deep groups may nest; the parser and the compiler recurse.
const MAX_NESTING: usize = 100_000;

const NOTHING_TO_REPEAT: SyntaxError = SyntaxError("Nothing to repeat");
const INVALID_GROUP: SyntaxError = SyntaxError("Invalid group");
const INVALID_NAMED_REFERENCE: SyntaxError = SyntaxError("Invalid named reference");
const INVALID_CAPTURE_NAME: SyntaxError = SyntaxError("Invalid capture group name");

/// Parses a pattern.
pub fn parse(source: &[u16]) -> Result<Pattern, SyntaxError> {
    let (total_groups, has_named_groups) = count_groups(source);
    let mut parser = Parser {
        src: source,
        pos: 0,
        total_groups,
        has_named_groups,
        next_group: 1,
        names: Vec::new(),
        references: Vec::new(),
        depth: 0,
    };
    let root = parser.disjunction()?;
    if parser.pos < source.len() {
        return Err(SyntaxError("Unmatched ')'"));
    }
    let mut root = root;
    parser.resolve_references(&mut root)?;
    Ok(Pattern {
        root,
        groups: total_groups,
        names: parser.names,
    })
}

const fn ch(c: char) -> u16 {
    c as u16
}

/// Counts the capture groups ahead of parsing: a `\N` escape depends on how many groups the whole pattern has.
fn count_groups(src: &[u16]) -> (usize, bool) {
    let mut count = 0;
    let mut named = false;
    let mut in_class = false;
    let mut i = 0;
    while i < src.len() {
        let unit = src[i];
        if unit == ch('\\') {
            i += 2;
            continue;
        }
        if in_class {
            in_class = unit != ch(']');
        } else if unit == ch('[') {
            in_class = true;
        } else if unit == ch('(') {
            if src.get(i + 1) != Some(&ch('?')) {
                count += 1;
            } else if src.get(i + 2) == Some(&ch('<'))
                && src.get(i + 3) != Some(&ch('='))
                && src.get(i + 3) != Some(&ch('!'))
            {
                count += 1;
                named = true;
            }
        }
        i += 1;
    }
    (count, named)
}

/// A class member: one unit, or a whole class escape such as `\d`.
enum ClassAtom {
    Unit(u16),
    Set(CharSet),
}

struct Parser<'a> {
    src: &'a [u16],
    pos: usize,
    total_groups: usize,
    has_named_groups: bool,
    next_group: usize,
    names: Vec<(JsString, usize)>,
    /// Named backreferences waiting for the names: `\k<name>` may come before its group.
    references: Vec<JsString>,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u16> {
        self.src.get(self.pos + offset).copied()
    }

    fn eat(&mut self, unit: char) -> bool {
        if self.peek() == Some(ch(unit)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn disjunction(&mut self) -> Result<Node, SyntaxError> {
        let mut alternatives = vec![self.alternative()?];
        while self.eat('|') {
            alternatives.push(self.alternative()?);
        }
        Ok(match alternatives.len() {
            1 => alternatives.remove(0),
            _ => Node::Alternation(alternatives),
        })
    }

    fn alternative(&mut self) -> Result<Node, SyntaxError> {
        let mut terms = Vec::new();
        while let Some(unit) = self.peek() {
            if unit == ch('|') || unit == ch(')') {
                break;
            }
            terms.push(self.term()?);
        }
        Ok(Node::Sequence(terms))
    }

    fn term(&mut self) -> Result<Node, SyntaxError> {
        let first_capture = self.next_group;
        let start = self.pos;
        let unit = self.src[start];
        self.pos += 1;
        let atom = match char::from_u32(u32::from(unit)) {
            Some('^') => return Ok(Node::Start),
            Some('$') => return Ok(Node::End),
            Some('(') => match self.group()? {
                Node::Look { behind: true, .. } if self.at_quantifier()? => return Err(NOTHING_TO_REPEAT),
                node @ Node::Look { behind: true, .. } => return Ok(node),
                node => node,
            },
            Some('.') => Node::AnyUnit,
            Some('[') => self.class()?,
            Some('\\') => match self.peek() {
                None => return Err(SyntaxError("\\ at end of pattern")),
                Some(next) if next == ch('b') || next == ch('B') => {
                    self.pos += 1;
                    return Ok(Node::WordBoundary {
                        negated: next == ch('B'),
                    });
                }
                Some(_) => self.atom_escape()?,
            },
            Some('*' | '+' | '?') => return Err(NOTHING_TO_REPEAT),
            Some('{') => {
                self.pos = start;
                if self.interval()?.is_some() {
                    return Err(NOTHING_TO_REPEAT);
                }
                self.pos = start + 1;
                Node::Unit(unit)
            }
            _ => Node::Unit(unit),
        };
        self.quantified(atom, first_capture)
    }

    /// Whether a quantifier starts here.
    fn at_quantifier(&mut self) -> Result<bool, SyntaxError> {
        match self.peek() {
            Some(unit) if unit == ch('*') || unit == ch('+') || unit == ch('?') => Ok(true),
            Some(unit) if unit == ch('{') => {
                let start = self.pos;
                let found = self.interval()?.is_some();
                self.pos = start;
                Ok(found)
            }
            _ => Ok(false),
        }
    }

    /// `{n}`, `{n,}` or `{n,m}` at the current position; anything else leaves the position alone.
    fn interval(&mut self) -> Result<Option<(u32, Option<u32>)>, SyntaxError> {
        let start = self.pos;
        self.pos += 1;
        let Some(min) = self.decimal() else {
            self.pos = start;
            return Ok(None);
        };
        let max = if self.eat('}') {
            return Ok(Some((min, Some(min))));
        } else if self.eat(',') {
            if self.eat('}') {
                return Ok(Some((min, None)));
            }
            match self.decimal() {
                Some(max) if self.eat('}') => max,
                _ => {
                    self.pos = start;
                    return Ok(None);
                }
            }
        } else {
            self.pos = start;
            return Ok(None);
        };
        if min > max {
            return Err(SyntaxError("numbers out of order in quantifier"));
        }
        Ok(Some((min, Some(max))))
    }

    fn decimal(&mut self) -> Option<u32> {
        let start = self.pos;
        let mut value = 0u32;
        while let Some(digit) = self.peek().and_then(decimal_digit) {
            value = value.saturating_mul(10).saturating_add(digit).min(QUANTIFIER_INFINITY);
            self.pos += 1;
        }
        (self.pos > start).then_some(value)
    }

    fn quantified(&mut self, atom: Node, first_capture: usize) -> Result<Node, SyntaxError> {
        let (min, max) = match self.peek() {
            Some(unit) if unit == ch('*') => {
                self.pos += 1;
                (0, None)
            }
            Some(unit) if unit == ch('+') => {
                self.pos += 1;
                (1, None)
            }
            Some(unit) if unit == ch('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some(unit) if unit == ch('{') => match self.interval()? {
                Some(bounds) => bounds,
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        let greedy = !self.eat('?');
        let max = max.filter(|&max| max != QUANTIFIER_INFINITY);
        Ok(Node::Repeat(Repeat {
            body: Box::new(atom),
            min,
            max,
            greedy,
            captures: (first_capture, self.next_group),
        }))
    }

    fn group(&mut self) -> Result<Node, SyntaxError> {
        if !self.eat('?') {
            let number = self.take_group_number();
            return self.close_group(|body| Node::Capture(number, body));
        }
        let kind = self.peek().ok_or(INVALID_GROUP)?;
        self.pos += 1;
        match char::from_u32(u32::from(kind)) {
            Some(':') => self.close_group(|body| *body),
            Some('=') => self.close_group(|body| look(body, false, false)),
            Some('!') => self.close_group(|body| look(body, false, true)),
            Some('<') => {
                if self.eat('=') {
                    return self.close_group(|body| look(body, true, false));
                }
                if self.eat('!') {
                    return self.close_group(|body| look(body, true, true));
                }
                let name = self.group_name()?;
                if self.names.iter().any(|(known, _)| *known == name) {
                    return Err(SyntaxError("Duplicate capture group name"));
                }
                let number = self.take_group_number();
                self.names.push((name, number));
                self.close_group(|body| Node::Capture(number, body))
            }
            _ => Err(INVALID_GROUP),
        }
    }

    fn take_group_number(&mut self) -> usize {
        let number = self.next_group;
        self.next_group += 1;
        number
    }

    fn close_group(&mut self, wrap: impl FnOnce(Box<Node>) -> Node) -> Result<Node, SyntaxError> {
        if self.depth >= MAX_NESTING {
            return Err(SyntaxError("Regular expression too large"));
        }
        self.depth += 1;
        let body = self.disjunction();
        self.depth -= 1;
        let body = body?;
        if !self.eat(')') {
            return Err(SyntaxError("Unterminated group"));
        }
        Ok(wrap(Box::new(body)))
    }

    /// A group name after `<`, through the closing `>`.
    fn group_name(&mut self) -> Result<JsString, SyntaxError> {
        let mut name = Vec::new();
        loop {
            let code = self.name_code_point()?;
            if code == u32::from(ch('>')) {
                break;
            }
            if !is_name_char(code, name.is_empty()) {
                return Err(INVALID_CAPTURE_NAME);
            }
            match char::from_u32(code) {
                Some(c) => name.extend(c.encode_utf16(&mut [0; 2]).iter()),
                None => return Err(INVALID_CAPTURE_NAME),
            }
        }
        if name.is_empty() {
            return Err(INVALID_CAPTURE_NAME);
        }
        Ok(JsString::from_units(name))
    }

    /// The next code point of a group name, joining surrogate pairs and reading `\u` escapes.
    fn name_code_point(&mut self) -> Result<u32, SyntaxError> {
        let unit = self.peek().ok_or(INVALID_CAPTURE_NAME)?;
        self.pos += 1;
        if unit == ch('\\') {
            if !self.eat('u') {
                return Err(SyntaxError("Invalid Unicode escape"));
            }
            return self.unicode_name_escape();
        }
        if let (0xD800..=0xDBFF, Some(low @ 0xDC00..=0xDFFF)) = (unit, self.peek()) {
            self.pos += 1;
            return Ok(0x10000 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(low) - 0xDC00));
        }
        Ok(u32::from(unit))
    }

    fn unicode_name_escape(&mut self) -> Result<u32, SyntaxError> {
        const INVALID: SyntaxError = SyntaxError("Invalid Unicode escape");
        if self.eat('{') {
            let mut value = 0u32;
            let mut digits = 0;
            while let Some(digit) = self.peek().and_then(hex_digit) {
                value = value.saturating_mul(16).saturating_add(digit);
                digits += 1;
                self.pos += 1;
            }
            if digits == 0 || value > 0x10_FFFF || !self.eat('}') {
                return Err(INVALID);
            }
            return Ok(value);
        }
        let lead = self.fixed_hex(4).ok_or(INVALID)?;
        if (0xD800..=0xDBFF).contains(&lead) && self.peek() == Some(ch('\\')) && self.peek_at(1) == Some(ch('u')) {
            let start = self.pos;
            self.pos += 2;
            if let Some(trail @ 0xDC00..=0xDFFF) = self.fixed_hex(4) {
                return Ok(0x10000 + ((lead - 0xD800) << 10) + (trail - 0xDC00));
            }
            self.pos = start;
        }
        Ok(lead)
    }

    fn fixed_hex(&mut self, count: usize) -> Option<u32> {
        let mut value = 0;
        for offset in 0..count {
            value = value * 16 + self.peek_at(offset).and_then(hex_digit)?;
        }
        self.pos += count;
        Some(value)
    }

    fn atom_escape(&mut self) -> Result<Node, SyntaxError> {
        let unit = self.src[self.pos];
        if let Some(digit) = decimal_digit(unit).filter(|&digit| digit != 0) {
            let start = self.pos;
            let mut value = 0usize;
            while let Some(digit) = self.peek().and_then(decimal_digit) {
                value = value.saturating_mul(10).saturating_add(digit as usize);
                self.pos += 1;
            }
            if value <= self.total_groups {
                return Ok(Node::BackReference(value));
            }
            self.pos = start;
            if digit >= 8 {
                self.pos += 1;
                return Ok(Node::Unit(unit));
            }
            return Ok(Node::Unit(self.legacy_octal()));
        }
        if unit == ch('k') {
            if !self.has_named_groups {
                self.pos += 1;
                return Ok(Node::Unit(unit));
            }
            self.pos += 1;
            if !self.eat('<') {
                return Err(INVALID_NAMED_REFERENCE);
            }
            let name = self.group_name().map_err(|_| INVALID_NAMED_REFERENCE)?;
            self.references.push(name);
            // The number is filled in once every name is known.
            return Ok(Node::BackReference(usize::MAX - (self.references.len() - 1)));
        }
        if let Some(set) = self.class_escape() {
            return Ok(Node::Class { set, negated: false });
        }
        Ok(Node::Unit(self.character_escape(false)?))
    }

    /// `\d`, `\D`, `\s`, `\S`, `\w` or `\W` at the current position.
    fn class_escape(&mut self) -> Option<CharSet> {
        let set = match char::from_u32(u32::from(self.peek()?))? {
            'd' => CharSet::digits(),
            'D' => CharSet::digits().inverted(),
            's' => CharSet::space(),
            'S' => CharSet::space().inverted(),
            'w' => CharSet::word(),
            'W' => CharSet::word().inverted(),
            _ => return None,
        };
        self.pos += 1;
        Some(set)
    }

    /// The unit a character escape stands for; the position is after the backslash.
    fn character_escape(&mut self, in_class: bool) -> Result<u16, SyntaxError> {
        let unit = self.peek().ok_or(SyntaxError("\\ at end of pattern"))?;
        let Some(c) = char::from_u32(u32::from(unit)) else {
            self.pos += 1;
            return Ok(unit);
        };
        Ok(match c {
            'f' | 'n' | 'r' | 't' | 'v' => {
                self.pos += 1;
                match c {
                    'f' => 0x0C,
                    'n' => 0x0A,
                    'r' => 0x0D,
                    't' => 0x09,
                    _ => 0x0B,
                }
            }
            'c' => match self.peek_at(1) {
                Some(letter) if is_ascii_letter(letter) => {
                    self.pos += 2;
                    letter % 32
                }
                Some(extra) if in_class && (decimal_digit(extra).is_some() || extra == ch('_')) => {
                    self.pos += 2;
                    extra % 32
                }
                // The backslash stands for itself and the `c` is read again.
                _ => ch('\\'),
            },
            '0'..='7' => self.legacy_octal(),
            'x' => {
                self.pos += 1;
                self.fixed_hex(2).map_or(ch('x'), |value| value as u16)
            }
            'u' => {
                self.pos += 1;
                self.fixed_hex(4).map_or(ch('u'), |value| value as u16)
            }
            'k' if in_class && self.has_named_groups => return Err(SyntaxError("Invalid escape")),
            _ => {
                self.pos += 1;
                unit
            }
        })
    }

    /// A legacy octal escape: up to three digits, at most `\377`.
    fn legacy_octal(&mut self) -> u16 {
        let mut value = 0;
        let mut digits = 0;
        while digits < 3 {
            match self.peek().and_then(decimal_digit) {
                Some(digit) if digit < 8 && value * 8 + digit <= 0o377 => {
                    value = value * 8 + digit;
                    digits += 1;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        value as u16
    }

    fn class(&mut self) -> Result<Node, SyntaxError> {
        let negated = self.eat('^');
        let mut set = CharSet::default();
        loop {
            let unit = self.peek().ok_or(SyntaxError("Unterminated character class"))?;
            if unit == ch(']') {
                self.pos += 1;
                break;
            }
            let first = self.class_atom()?;
            if self.peek() != Some(ch('-')) {
                add_atom(&mut set, first);
                continue;
            }
            self.pos += 1;
            match self.peek() {
                None => return Err(SyntaxError("Unterminated character class")),
                Some(next) if next == ch(']') => {
                    add_atom(&mut set, first);
                    set.add_range(ch('-'), ch('-'));
                }
                Some(_) => match (first, self.class_atom()?) {
                    (ClassAtom::Unit(low), ClassAtom::Unit(high)) => {
                        if low > high {
                            return Err(SyntaxError("Range out of order in character class"));
                        }
                        set.add_range(low, high);
                    }
                    (first, second) => {
                        add_atom(&mut set, first);
                        set.add_range(ch('-'), ch('-'));
                        add_atom(&mut set, second);
                    }
                },
            }
        }
        set.normalize();
        Ok(Node::Class { set, negated })
    }

    fn class_atom(&mut self) -> Result<ClassAtom, SyntaxError> {
        let unit = self.src[self.pos];
        self.pos += 1;
        if unit != ch('\\') {
            return Ok(ClassAtom::Unit(unit));
        }
        if let Some(set) = self.class_escape() {
            return Ok(ClassAtom::Set(set));
        }
        if self.eat('b') {
            return Ok(ClassAtom::Unit(0x08));
        }
        self.character_escape(true).map(ClassAtom::Unit)
    }

    /// Replaces the placeholders of named backreferences with capture numbers.
    fn resolve_references(&self, node: &mut Node) -> Result<(), SyntaxError> {
        match node {
            Node::BackReference(number) if *number > self.total_groups => {
                let name = &self.references[usize::MAX - *number];
                let (_, resolved) = self
                    .names
                    .iter()
                    .find(|(known, _)| known == name)
                    .ok_or(INVALID_NAMED_REFERENCE)?;
                *number = *resolved;
            }
            Node::Sequence(items) | Node::Alternation(items) => {
                for item in items {
                    self.resolve_references(item)?;
                }
            }
            Node::Capture(_, body) | Node::Look { body, .. } => self.resolve_references(body)?,
            Node::Repeat(repeat) => self.resolve_references(&mut repeat.body)?,
            _ => {}
        }
        Ok(())
    }
}

fn look(body: Box<Node>, behind: bool, negated: bool) -> Node {
    Node::Look { body, behind, negated }
}

fn add_atom(set: &mut CharSet, atom: ClassAtom) {
    match atom {
        ClassAtom::Unit(unit) => set.add_range(unit, unit),
        ClassAtom::Set(other) => set.add_set(&other),
    }
}

fn decimal_digit(unit: u16) -> Option<u32> {
    char::from_u32(u32::from(unit))?.to_digit(10)
}

fn hex_digit(unit: u16) -> Option<u32> {
    char::from_u32(u32::from(unit))?.to_digit(16)
}

fn is_ascii_letter(unit: u16) -> bool {
    (0x41..=0x5A).contains(&unit) || (0x61..=0x7A).contains(&unit)
}

/// Whether a code point may appear in a group name: an approximation of ID_Start and ID_Continue by Rust's Unicode
/// tables, with `$`, `_`, ZWNJ and ZWJ.
fn is_name_char(code: u32, first: bool) -> bool {
    let Some(c) = char::from_u32(code) else {
        return false;
    };
    if c == '$' || c == '_' {
        return true;
    }
    if first {
        c.is_alphabetic()
    } else {
        c.is_alphanumeric() || code == 0x200C || code == 0x200D
    }
}
