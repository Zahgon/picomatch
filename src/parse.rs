// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Parses a glob pattern into tokens and the source of a regular expression.
//!
//! Tokens live in an arena, because a token stays reachable through `prev` links after it leaves the token list. The
//! list holds arena indices.

use crate::constants::{self, ExtglobKind, GlobChars, MAX_LENGTH};
use crate::extglob::analyze_repeated_extglob;
use crate::js::number::number_to_string;
use crate::js::string::index_of;
use crate::js::{JsError, JsString, Value};
use crate::options::Options;
use crate::regexp::RegExp;
use crate::utils;

/// A token of the parsed pattern.
#[derive(Clone, Debug, Default)]
struct Token {
    kind: &'static str,
    value: JsString,
    output: Option<JsString>,
    prev: Option<usize>,
    extglob: bool,
    star: bool,
    posix: bool,
    comma: bool,
    dots: bool,
    output_index: usize,
    tokens_index: usize,
}

impl Token {
    fn new(kind: &'static str, value: &JsString) -> Token {
        Token {
            kind,
            value: value.clone(),
            ..Token::default()
        }
    }

    fn with_output(kind: &'static str, value: &JsString, output: impl Into<JsString>) -> Token {
        Token {
            output: Some(output.into()),
            ..Token::new(kind, value)
        }
    }

    fn extglob(mut self) -> Token {
        self.extglob = true;
        self
    }
}

/// An open extglob.
#[derive(Clone, Debug)]
struct Extglob {
    kind: ExtglobKind,
    close: JsString,
    inner: JsString,
    prev: usize,
    parens: i64,
    output: JsString,
    start_index: isize,
    tokens_index: usize,
}

/// A token of the public parse state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateToken {
    /// The token type.
    pub kind: &'static str,
    /// The token value.
    pub value: JsString,
}

/// The state a parse ends with.
#[derive(Clone, Debug, Default)]
pub struct ParseState {
    /// The pattern, after `REPLACEMENTS`.
    pub input: JsString,
    /// The last index read.
    pub index: isize,
    /// Where the glob proper starts.
    pub start: isize,
    /// `options.dot === true`.
    pub dot: bool,
    /// The token values taken so far.
    pub consumed: JsString,
    /// The regular expression source.
    pub output: JsString,
    /// The `./` prefix.
    pub prefix: JsString,
    /// Whether the output had to be rebuilt from the tokens.
    pub backtrack: bool,
    /// Whether the pattern is negated.
    pub negated: bool,
    /// Whether the pattern starts with a negated extglob.
    pub negated_extglob: bool,
    /// Open brackets.
    pub brackets: i64,
    /// Open braces.
    pub braces: i64,
    /// Open parentheses.
    pub parens: i64,
    /// Open double quotes.
    pub quotes: i64,
    /// Whether the pattern has a globstar.
    pub globstar: bool,
    /// The tokens.
    pub tokens: Vec<StateToken>,
}

fn syntax_error(kind: &str, ch: char) -> JsError {
    JsError::new(
        "SyntaxError",
        format!("Missing {kind}: \"{ch}\" - use \"\\\\{ch}\" to match literal characters"),
    )
}

fn max_length(options: &Options) -> f64 {
    options
        .get("maxLength")
        .as_number()
        .map_or(MAX_LENGTH, |limit| MAX_LENGTH.min(limit))
}

fn check_length(length: usize, max: f64) -> Result<(), JsError> {
    if length as f64 > max {
        return Err(JsError::new(
            "SyntaxError",
            format!(
                "Input length: {length}, exceeds maximum allowed length: {}",
                number_to_string(max)
            ),
        ));
    }
    Ok(())
}

fn replacement(input: &JsString) -> JsString {
    constants::replacement(&input.to_string_lossy())
        .filter(|_| input.units().iter().all(|&unit| unit < 0x80))
        .map_or_else(|| input.clone(), JsString::from)
}

/// `/regex/.test(text)` for the parser's fixed expressions.
fn test(source: &str, text: &[u16]) -> bool {
    RegExp::literal(source, "").is_some_and(|regex| regex.search(text, 0).is_some())
}

/// `/[!=<:]/.test(next)`; a missing character reads as the text "undefined".
fn is_lookaround_char(next: Option<u16>) -> bool {
    next.and_then(|unit| char::from_u32(u32::from(unit)))
        .is_some_and(|c| "!=<:".contains(c))
}

fn unit(c: char) -> u16 {
    c as u16
}

fn is(value: &JsString, c: char) -> bool {
    value.units() == [unit(c)]
}

/// `value.slice(1)`.
fn tail(value: &JsString) -> JsString {
    value.slice_from(1)
}

/// `text.slice(0, -count)`: a count of zero leaves nothing, as `slice(0, -0)` does.
fn drop_last(text: &JsString, count: usize) -> JsString {
    if count == 0 {
        return JsString::new();
    }
    let keep = text.len().saturating_sub(count);
    JsString::from_units(text.units()[..keep].to_vec())
}

/// Sorts strings as `Array.prototype.sort` does by default: by UTF-16 code units.
fn sorted_units(mut args: Vec<JsString>) -> Vec<JsString> {
    args.sort();
    args
}

fn expand_range(args: Vec<JsString>, options: &Options) -> JsString {
    if let Some(custom) = options.expand_range {
        return custom.expand(&args);
    }
    let args = sorted_units(args);
    let mut value = JsString::from("[");
    value.push_js(&JsString::join(&args, "-"));
    value.push_str("]");
    if RegExp::new(value.units(), &[]).is_ok() {
        return value;
    }
    let escaped: Vec<JsString> = args.iter().map(|arg| utils::escape_regex(arg.units())).collect();
    JsString::join(&escaped, "..")
}

struct Parser<'a> {
    input: JsString,
    len: isize,
    options: &'a Options,
    opts: Options,
    chars: &'static GlobChars,
    capture: &'static str,
    star: JsString,
    state: ParseState,
    arena: Vec<Token>,
    tokens: Vec<usize>,
    extglobs: Vec<Extglob>,
    braces: Vec<usize>,
    stack: Vec<&'static str>,
    prev: usize,
}

const BOS: usize = 0;

/// Parses a pattern. `options` are the caller's; the parser works on a copy.
pub fn parse(input: &Value, options: &Options) -> Result<ParseState, JsError> {
    let Some(input) = input.as_string() else {
        return Err(JsError::type_error("Expected a string"));
    };
    parse_str(input, options)
}

/// Parses a pattern given as a string.
pub fn parse_str(input: &JsString, options: &Options) -> Result<ParseState, JsError> {
    let input = replacement(input);
    let mut opts = options.clone();
    check_length(input.len(), max_length(&opts))?;

    let bos_output = if opts.truthy("prepend") {
        opts.get("prepend").to_js_string()
    } else {
        JsString::new()
    };
    let bos = Token::with_output("bos", &JsString::new(), bos_output);

    let capture = if opts.truthy("capture") { "" } else { "?:" };
    let chars = constants::glob_chars(opts.is_true("windows"));

    let mut star = if opts.is_true("bash") {
        globstar(&opts, chars, capture)
    } else {
        JsString::from(chars.star)
    };
    if opts.truthy("capture") {
        star = parenthesized(&star);
    }

    if let Value::Bool(noext) = opts.get("noext") {
        opts.set("noextglob", Value::Bool(noext));
    }

    let mut state = ParseState {
        input: input.clone(),
        index: -1,
        dot: opts.is_true("dot"),
        ..ParseState::default()
    };
    let body = utils::remove_prefix(&input, &mut state.prefix);

    let mut parser = Parser {
        len: body.len() as isize,
        input: body,
        options,
        opts,
        chars,
        capture,
        star,
        state,
        arena: vec![bos],
        tokens: vec![BOS],
        extglobs: Vec::new(),
        braces: Vec::new(),
        stack: Vec::new(),
        prev: BOS,
    };

    if parser.fast_path() {
        return Ok(parser.finish());
    }
    parser.tokenize()?;
    parser.close_unbalanced()?;
    parser.rebuild_output();
    Ok(parser.finish())
}

fn parenthesized(text: &JsString) -> JsString {
    let mut out = JsString::from("(");
    out.push_js(text);
    out.push_str(")");
    out
}

fn globstar(opts: &Options, chars: &GlobChars, capture: &str) -> JsString {
    let tail = if opts.truthy("dot") {
        chars.dots_slash
    } else {
        chars.dot_literal
    };
    JsString::from(format!("({capture}(?:(?!{}{tail})[\\s\\S])*?)", chars.start_anchor))
}

impl Parser<'_> {
    fn finish(self) -> ParseState {
        let Parser {
            mut state,
            arena,
            tokens,
            ..
        } = self;
        state.tokens = tokens
            .iter()
            .map(|&index| StateToken {
                kind: arena[index].kind,
                value: arena[index].value.clone(),
            })
            .collect();
        state
    }

    fn eos(&self) -> bool {
        self.state.index >= self.len - 1
    }

    fn unit_at(&self, index: isize) -> Option<u16> {
        usize::try_from(index).ok().and_then(|i| self.input.unit_at(i))
    }

    fn peek(&self, n: isize) -> Option<u16> {
        self.unit_at(self.state.index + n)
    }

    fn peek_is(&self, n: isize, c: char) -> bool {
        self.peek(n) == Some(unit(c))
    }

    fn advance(&mut self) -> JsString {
        self.state.index += 1;
        self.unit_at(self.state.index)
            .map_or_else(JsString::new, JsString::from_unit)
    }

    fn remaining(&self) -> JsString {
        self.input.slice_from((self.state.index + 1).max(0) as usize)
    }

    fn consume(&mut self, value: &JsString, count: isize) {
        self.state.consumed.push_js(value);
        self.state.index += count;
    }

    fn append_value(&mut self, value: &JsString) {
        self.state.output.push_js(value);
        self.state.consumed.push_js(value);
    }

    fn prev_token(&self) -> &Token {
        &self.arena[self.prev]
    }

    fn prev_token_mut(&mut self) -> &mut Token {
        &mut self.arena[self.prev]
    }

    fn prev_output(&self) -> JsString {
        self.prev_token().output.clone().unwrap_or_default()
    }

    fn increment(&mut self, kind: &'static str) {
        *self.counter(kind) += 1;
        self.stack.push(kind);
    }

    fn decrement(&mut self, kind: &'static str) {
        *self.counter(kind) -= 1;
        self.stack.pop();
    }

    fn counter(&mut self, kind: &str) -> &mut i64 {
        match kind {
            "brackets" => &mut self.state.brackets,
            "braces" => &mut self.state.braces,
            _ => &mut self.state.parens,
        }
    }

    fn negate(&mut self) {
        let mut count = 1;
        while self.peek_is(1, '!') && (!self.peek_is(2, '(') || self.peek_is(3, '?')) {
            self.advance();
            self.state.start += 1;
            count += 1;
        }
        if count % 2 == 0 {
            return;
        }
        self.state.negated = true;
        self.state.start += 1;
    }

    /// Adds a token, merging runs of plain text; the arena index of the new token, if one was added.
    fn push(&mut self, token: Token) -> Option<usize> {
        if self.prev_token().kind == "globstar" {
            let is_brace = self.state.braces > 0 && (token.kind == "comma" || token.kind == "brace");
            let is_extglob =
                token.extglob || (!self.extglobs.is_empty() && (token.kind == "pipe" || token.kind == "paren"));
            if token.kind != "slash" && token.kind != "paren" && !is_brace && !is_extglob {
                let length = self.prev_output().len();
                self.state.output = drop_last(&self.state.output, length);
                let star = self.star.clone();
                let prev = self.prev_token_mut();
                prev.kind = "star";
                prev.value = JsString::from("*");
                prev.output = Some(star.clone());
                self.state.output.push_js(&star);
            }
        }

        if token.kind != "paren" {
            if let Some(open) = self.extglobs.last_mut() {
                open.inner.push_js(&token.value);
            }
        }

        let output = token.output.as_ref().unwrap_or(&token.value).clone();
        if !token.value.is_empty() || token.output.as_ref().is_some_and(|text| !text.is_empty()) {
            self.state.output.push_js(&output);
            self.state.consumed.push_js(&token.value);
        }

        if self.prev_token().kind == "text" && token.kind == "text" {
            let prev = self.prev_token_mut();
            let mut merged = match prev.output.take().filter(|text| !text.is_empty()) {
                Some(text) => text,
                None => prev.value.clone(),
            };
            merged.push_js(&token.value);
            prev.output = Some(merged);
            prev.value.push_js(&token.value);
            return None;
        }

        let mut token = token;
        token.prev = Some(self.prev);
        self.arena.push(token);
        let index = self.arena.len() - 1;
        self.tokens.push(index);
        self.prev = index;
        Some(index)
    }

    fn extglob_open(&mut self, kind: ExtglobKind, value: &JsString) {
        let mut extglob = Extglob {
            kind,
            close: JsString::from(kind.close(self.chars)),
            inner: JsString::new(),
            prev: self.prev,
            parens: self.state.parens,
            output: self.state.output.clone(),
            start_index: self.state.index,
            tokens_index: self.tokens.len(),
        };
        let mut output = JsString::from(if self.opts.truthy("capture") { "(" } else { "" });
        output.push_str(kind.open());

        self.increment("parens");
        let prefix_output = if self.state.output.is_empty() {
            self.chars.one_char
        } else {
            ""
        };
        self.push(Token::with_output(kind.token_type(), value, prefix_output));
        let paren = self.advance();
        self.push(Token::with_output("paren", &paren, output).extglob());
        extglob.inner = JsString::new();
        self.extglobs.push(extglob);
    }

    fn extglob_close(&mut self, mut extglob: Extglob, value: &JsString) -> Result<(), JsError> {
        let start = extglob.start_index.max(0) as usize;
        let end = (self.state.index + 1).max(0) as usize;
        let units = self.input.units();
        let end = end.min(units.len());
        let literal = JsString::from_units(units[start.min(end)..end].to_vec());
        let body = units[(start + 2).min(end)..self.state.index.max(0) as usize].to_vec();
        let analysis = analyze_repeated_extglob(&body, &self.opts);

        if matches!(extglob.kind, ExtglobKind::Plus | ExtglobKind::Star) && analysis.risky {
            let safe_output = analysis.safe_output.map(|safe| {
                let mut text = JsString::from(if extglob.output.is_empty() {
                    self.chars.one_char
                } else {
                    ""
                });
                text.push_js(&if self.opts.truthy("capture") {
                    parenthesized(&safe)
                } else {
                    safe
                });
                text
            });
            let open_output = safe_output.unwrap_or_else(|| utils::escape_regex(literal.units()));
            // A brace range may have removed the extglob's tokens; the original reads an undefined token here.
            let Some(&open) = self.tokens.get(extglob.tokens_index) else {
                return Err(JsError::type_error(
                    "Cannot set properties of undefined (setting 'type')",
                ));
            };
            self.arena[open].kind = "text";
            self.arena[open].value = literal;
            self.arena[open].output = Some(open_output.clone());

            for &index in &self.tokens[extglob.tokens_index + 1..] {
                self.arena[index].value = JsString::new();
                self.arena[index].output = Some(JsString::new());
            }

            self.state.output = extglob.output.clone();
            self.state.output.push_js(&open_output);
            self.state.backtrack = true;

            self.push(Token::with_output("paren", value, "").extglob());
            self.decrement("parens");
            return Ok(());
        }

        let mut output = extglob.close.clone();
        if self.opts.truthy("capture") {
            output.push_str(")");
        }

        if extglob.kind == ExtglobKind::Negate {
            let mut extglob_star = self.star.clone();
            if extglob.inner.len() > 1 && index_of(extglob.inner.units(), &[unit('/')], 0).is_some() {
                extglob_star = globstar(&self.opts, self.chars, self.capture);
            }

            let remaining = self.remaining();
            let only_parens = !remaining.is_empty() && remaining.units().iter().all(|&u| u == unit(')'));
            if extglob_star != self.star || self.eos() || only_parens {
                let mut close = JsString::from(")$))");
                close.push_js(&extglob_star);
                output = close.clone();
                extglob.close = close;
            }

            if index_of(extglob.inner.units(), &[unit('*')], 0).is_some()
                && !remaining.is_empty()
                && test(r"^\.[^\\/.]+$", remaining.units())
            {
                let mut nested = self.options.clone();
                nested.set("fastpaths", Value::Bool(false));
                let expression = parse_str(&remaining, &nested)?.output;
                let mut close = JsString::from(")");
                close.push_js(&expression);
                close.push_str(")");
                close.push_js(&extglob_star);
                close.push_str(")");
                output = close;
            }

            if self.arena[extglob.prev].kind == "bos" {
                self.state.negated_extglob = true;
            }
        }

        self.push(Token::with_output("paren", value, output).extglob());
        self.decrement("parens");
        Ok(())
    }

    /// The path for patterns with nothing to tokenize; whether it applied.
    fn fast_path(&mut self) -> bool {
        if self.opts.is_false("fastpaths") || test(r#"(^[*!]|[/()[\]{}"])"#, self.input.units()) {
            return false;
        }
        let input = self.input.clone();
        let chars = self.chars;
        let qmark_no_dot = if self.opts.truthy("dot") {
            chars.qmark
        } else {
            chars.qmark_no_dot
        };
        let star = self.star.clone();
        let mut backslashes = false;

        let mut output = match RegExp::literal(r"(\\?)((\W)(\3*))", "g") {
            Some(regex) => regex.replace_all(input.units(), |found| {
                let text = |group: usize| {
                    let (start, end) = found.captures[group].unwrap_or_default();
                    &input.units()[start..end]
                };
                let (whole, esc, all, first, rest) = (text(0), text(1), text(2), text(3), text(4));
                let mut out = JsString::new();
                let repeat = |out: &mut JsString, fragment: &str, count: usize| {
                    for _ in 0..count {
                        out.push_str(fragment);
                    }
                };
                if first == [unit('\\')] {
                    backslashes = true;
                    return whole.to_vec();
                }
                if first == [unit('?')] {
                    if !esc.is_empty() {
                        out.push_units(esc);
                        out.push_units(first);
                        repeat(&mut out, chars.qmark, rest.len());
                    } else if found.index() == 0 {
                        out.push_str(qmark_no_dot);
                        repeat(&mut out, chars.qmark, rest.len());
                    } else {
                        repeat(&mut out, chars.qmark, all.len());
                    }
                    return out.into_units();
                }
                if first == [unit('.')] {
                    repeat(&mut out, chars.dot_literal, all.len());
                    return out.into_units();
                }
                if first == [unit('*')] {
                    if !esc.is_empty() {
                        out.push_units(esc);
                        out.push_units(first);
                        if !rest.is_empty() {
                            out.push_js(&star);
                        }
                    } else {
                        out.push_js(&star);
                    }
                    return out.into_units();
                }
                if !esc.is_empty() {
                    return whole.to_vec();
                }
                out.push_str("\\");
                out.push_units(whole);
                out.into_units()
            }),
            None => input.units().to_vec(),
        };

        if backslashes {
            if self.opts.is_true("unescape") {
                output.retain(|&u| u != unit('\\'));
            } else if let Some(regex) = RegExp::literal(r"\\+", "g") {
                let source = output.clone();
                output = regex.replace_all(&source, |found| {
                    let length = found.end() - found.index();
                    let kept = if length % 2 == 0 { 2 } else { 1 };
                    vec![unit('\\'); kept]
                });
            }
        }

        if output == input.units() && self.opts.is_true("contains") {
            self.state.output = input;
            return true;
        }

        self.state.output = utils::wrap_output(&output, self.state.negated, self.options.truthy("contains"));
        true
    }

    fn tokenize(&mut self) -> Result<(), JsError> {
        while !self.eos() {
            let value = self.advance();
            if value.units() == [0] {
                continue;
            }
            self.token(value)?;
        }
        Ok(())
    }

    fn token(&mut self, mut value: JsString) -> Result<(), JsError> {
        if is(&value, '\\') {
            let next = self.peek(1);
            if next == Some(unit('/')) && !self.opts.is_true("bash") {
                return Ok(());
            }
            if next == Some(unit('.')) || next == Some(unit(';')) {
                return Ok(());
            }
            if next.is_none() {
                value.push_str("\\");
                self.push(Token::new("text", &value));
                return Ok(());
            }

            // collapse slashes to reduce potential for exploits
            let run = self
                .remaining()
                .units()
                .iter()
                .take_while(|&&u| u == unit('\\'))
                .count();
            if run > 2 {
                self.state.index += run as isize;
                if run % 2 != 0 {
                    value.push_str("\\");
                }
            }

            if self.opts.is_true("unescape") {
                value = self.advance();
            } else {
                let next = self.advance();
                value.push_js(&next);
            }

            if self.state.brackets == 0 {
                self.push(Token::new("text", &value));
                return Ok(());
            }
        }

        if self.state.brackets > 0 {
            let prev_value = self.prev_token().value.clone();
            if !is(&value, ']') || prev_value.eq_str("[") || prev_value.eq_str("[^") {
                self.bracket_body(value);
                return Ok(());
            }
        }

        if self.state.quotes == 1 && !is(&value, '"') {
            let value = utils::escape_regex(value.units());
            self.prev_token_mut().value.push_js(&value);
            self.append_value(&value);
            return Ok(());
        }

        let first = value
            .units()
            .first()
            .map_or('\0', |&u| char::from_u32(u32::from(u)).unwrap_or('\u{fffd}'));
        match first {
            '"' if value.len() == 1 => {
                self.state.quotes = if self.state.quotes == 1 { 0 } else { 1 };
                if self.opts.is_true("keepQuotes") {
                    self.push(Token::new("text", &value));
                }
                Ok(())
            }
            '(' if value.len() == 1 => {
                self.increment("parens");
                self.push(Token::new("paren", &value));
                Ok(())
            }
            ')' if value.len() == 1 => self.close_paren(&value),
            '[' if value.len() == 1 => self.open_bracket(value),
            ']' if value.len() == 1 => self.close_bracket(value),
            '{' if value.len() == 1 && !self.opts.is_true("nobrace") => {
                self.open_brace(&value);
                Ok(())
            }
            '}' if value.len() == 1 => {
                self.close_brace(value);
                Ok(())
            }
            '|' if value.len() == 1 => {
                self.push(Token::new("text", &value));
                Ok(())
            }
            ',' if value.len() == 1 => {
                self.comma(&value);
                Ok(())
            }
            '/' if value.len() == 1 => {
                self.slash(&value);
                Ok(())
            }
            '.' if value.len() == 1 => {
                self.dot(&value);
                Ok(())
            }
            '?' if value.len() == 1 => {
                self.question_mark(&value);
                Ok(())
            }
            '!' if value.len() == 1 && self.exclamation(&value) => Ok(()),
            '+' if value.len() == 1 => {
                self.plus(&value);
                Ok(())
            }
            '@' if value.len() == 1 => {
                if !self.opts.is_true("noextglob") && self.peek_is(1, '(') && !self.peek_is(2, '?') {
                    self.push(Token::with_output("at", &value, "").extglob());
                } else {
                    self.push(Token::new("text", &value));
                }
                Ok(())
            }
            '*' if value.len() == 1 => {
                self.star(&value);
                Ok(())
            }
            _ => {
                self.plain_text(value);
                Ok(())
            }
        }
    }

    /// One unit inside a character class.
    fn bracket_body(&mut self, mut value: JsString) {
        if !self.opts.is_false("posix") && is(&value, ':') {
            let prev_value = self.prev_token().value.clone();
            let inner = tail(&prev_value);
            if index_of(inner.units(), &[unit('[')], 0).is_some() {
                self.prev_token_mut().posix = true;

                if index_of(inner.units(), &[unit(':')], 0).is_some() {
                    let idx = prev_value.units().iter().rposition(|&u| u == unit('[')).unwrap_or(0);
                    let pre = JsString::from_units(prev_value.units()[..idx].to_vec());
                    let rest = prev_value.slice_from(idx + 2);
                    let posix = (rest.units().iter().all(|&u| u < 0x80))
                        .then(|| constants::posix_regex_source(&rest.to_string_lossy()))
                        .flatten();
                    if let Some(posix) = posix {
                        let mut replaced = pre;
                        replaced.push_str(posix);
                        self.prev_token_mut().value = replaced;
                        self.state.backtrack = true;
                        self.advance();

                        let bos_empty = self.arena[BOS].output.as_ref().is_none_or(JsString::is_empty);
                        if bos_empty && self.tokens.iter().position(|&t| t == self.prev) == Some(1) {
                            self.arena[BOS].output = Some(JsString::from(self.chars.one_char));
                        }
                        return;
                    }
                }
            }
        }

        if (is(&value, '[') && !self.peek_is(1, ':')) || (is(&value, '-') && self.peek_is(1, ']')) {
            value = escaped(&value);
        }

        let prev_value = self.prev_token().value.clone();
        if is(&value, ']') && (prev_value.eq_str("[") || prev_value.eq_str("[^")) {
            value = escaped(&value);
        }

        if is(&value, '!') && prev_value.eq_str("[") && !self.opts.is_true("literalBrackets") {
            let remaining = self.remaining();
            let is_lone_negation =
                self.peek_is(1, ']') && index_of(remaining.slice_from(1).units(), &[unit(']')], 0).is_none();
            if !is_lone_negation {
                // POSIX bracket negation: [!abc] is equivalent to [^abc]
                value = JsString::from("^");
            }
        }

        self.prev_token_mut().value.push_js(&value);
        self.append_value(&value);
    }

    fn close_paren(&mut self, value: &JsString) -> Result<(), JsError> {
        if self.state.parens == 0 && self.opts.is_true("strictBrackets") {
            return Err(syntax_error("opening", '('));
        }

        if let Some(extglob) = self.extglobs.last() {
            if self.state.parens == extglob.parens + 1 {
                if let Some(extglob) = self.extglobs.pop() {
                    return self.extglob_close(extglob, value);
                }
            }
        }

        let output = if self.state.parens != 0 { ")" } else { "\\)" };
        self.push(Token::with_output("paren", value, output));
        self.decrement("parens");
        Ok(())
    }

    fn open_bracket(&mut self, mut value: JsString) -> Result<(), JsError> {
        let nobracket = self.opts.is_true("nobracket");
        if nobracket || index_of(self.remaining().units(), &[unit(']')], 0).is_none() {
            if !nobracket && self.opts.is_true("strictBrackets") {
                return Err(syntax_error("closing", ']'));
            }
            value = escaped(&value);
        } else {
            self.increment("brackets");
        }
        self.push(Token::new("bracket", &value));
        Ok(())
    }

    fn close_bracket(&mut self, mut value: JsString) -> Result<(), JsError> {
        let prev_is_lone_bracket = self.prev_token().kind == "bracket" && self.prev_token().value.len() == 1;
        if self.opts.is_true("nobracket") || prev_is_lone_bracket {
            self.push(Token::with_output("text", &value, escaped(&value)));
            return Ok(());
        }

        if self.state.brackets == 0 {
            if self.opts.is_true("strictBrackets") {
                return Err(syntax_error("opening", '['));
            }
            self.push(Token::with_output("text", &value, escaped(&value)));
            return Ok(());
        }

        self.decrement("brackets");

        let prev_value = tail(&self.prev_token().value);
        if !self.prev_token().posix
            && prev_value.units().first() == Some(&unit('^'))
            && index_of(prev_value.units(), &[unit('/')], 0).is_none()
        {
            value = JsString::from("/]");
        }

        self.prev_token_mut().value.push_js(&value);
        self.append_value(&value);

        // when literal brackets are explicitly disabled
        // assume we should match with a regex character class
        if self.opts.is_false("literalBrackets") || utils::has_regex_chars(prev_value.units()) {
            return Ok(());
        }

        let whole = self.prev_token().value.clone();
        let escaped_value = utils::escape_regex(whole.units());
        self.state.output = drop_last(&self.state.output, whole.len());

        // when literal brackets are explicitly enabled
        // assume we should escape the brackets to match literal characters
        if self.opts.is_true("literalBrackets") {
            self.state.output.push_js(&escaped_value);
            self.prev_token_mut().value = escaped_value;
            return Ok(());
        }

        // when the user specifies nothing, try to match both
        let mut both = JsString::from("(");
        both.push_str(self.capture);
        both.push_js(&escaped_value);
        both.push_str("|");
        both.push_js(&whole);
        both.push_str(")");
        self.state.output.push_js(&both);
        self.prev_token_mut().value = both;
        Ok(())
    }

    fn open_brace(&mut self, value: &JsString) {
        self.increment("braces");
        let mut open = Token::with_output("brace", value, "(");
        open.output_index = self.state.output.len();
        open.tokens_index = self.tokens.len();
        if let Some(index) = self.push(open) {
            self.braces.push(index);
        }
    }

    fn close_brace(&mut self, mut value: JsString) {
        let brace = self.braces.last().copied();
        let Some(brace) = brace.filter(|_| !self.opts.is_true("nobrace")) else {
            let output = value.clone();
            self.push(Token::with_output("text", &value, output));
            return;
        };

        let mut output = JsString::from(")");

        if self.arena[brace].dots {
            let mut range = Vec::new();
            while let Some(index) = self.tokens.pop() {
                let token = &self.arena[index];
                if token.kind == "brace" {
                    break;
                }
                if token.kind != "dots" {
                    range.insert(0, token.value.clone());
                }
            }
            output = expand_range(range, &self.opts);
            self.state.backtrack = true;
        }

        if !self.arena[brace].comma && !self.arena[brace].dots {
            let keep = self.arena[brace].output_index.min(self.state.output.len());
            let out = JsString::from_units(self.state.output.units()[..keep].to_vec());
            let from = self.arena[brace].tokens_index.min(self.tokens.len());
            let toks = self.tokens[from..].to_vec();
            self.arena[brace].value = JsString::from("\\{");
            self.arena[brace].output = Some(JsString::from("\\{"));
            value = JsString::from("\\}");
            output = value.clone();
            self.state.output = out;
            for index in toks {
                let token = &self.arena[index];
                let text = token
                    .output
                    .as_ref()
                    .filter(|text| !text.is_empty())
                    .unwrap_or(&token.value);
                self.state.output.push_js(text);
            }
        }

        self.push(Token::with_output("brace", &value, output));
        self.decrement("braces");
        self.braces.pop();
    }

    fn comma(&mut self, value: &JsString) {
        let mut output = value.clone();
        if let Some(&brace) = self.braces.last() {
            if self.stack.last() == Some(&"braces") {
                self.arena[brace].comma = true;
                output = JsString::from("|");
            }
        }
        self.push(Token::with_output("comma", value, output));
    }

    fn slash(&mut self, value: &JsString) {
        // if the beginning of the glob is "./", advance the start
        // to the current index, and don't add the "./" characters
        // to the state. This greatly simplifies lookbehinds when
        // checking for BOS characters like "!" and "." (not "./")
        if self.prev_token().kind == "dot" && self.state.index == self.state.start + 1 {
            self.state.start = self.state.index + 1;
            self.state.consumed = JsString::new();
            self.state.output = JsString::new();
            self.tokens.pop();
            self.prev = BOS;
            return;
        }
        self.push(Token::with_output("slash", value, self.chars.slash_literal));
    }

    fn dot(&mut self, value: &JsString) {
        let chars = self.chars;
        if self.state.braces > 0 && self.prev_token().kind == "dot" {
            let brace = self.braces.last().copied();
            let prev = self.prev_token_mut();
            if prev.value.eq_str(".") {
                prev.output = Some(JsString::from(chars.dot_literal));
            }
            prev.kind = "dots";
            prev.output.get_or_insert_with(JsString::new).push_js(value);
            prev.value.push_js(value);
            if let Some(brace) = brace {
                self.arena[brace].dots = true;
            }
            return;
        }

        let kind = self.prev_token().kind;
        if self.state.braces + self.state.parens == 0 && kind != "bos" && kind != "slash" {
            self.push(Token::with_output("text", value, chars.dot_literal));
            return;
        }
        self.push(Token::with_output("dot", value, chars.dot_literal));
    }

    fn question_mark(&mut self, value: &JsString) {
        let is_group = self.prev_token().value.eq_str("(");
        if !is_group && !self.opts.is_true("noextglob") && self.peek_is(1, '(') && !self.peek_is(2, '?') {
            self.extglob_open(ExtglobKind::Qmark, value);
            return;
        }

        if self.prev_token().kind == "paren" {
            let next = self.peek(1);
            let prev_is_open = self.prev_token().value.eq_str("(");
            let escaped_output = (prev_is_open && !is_lookaround_char(next))
                || (next == Some(unit('<')) && !test(r"<([!=]|\w+>)", self.remaining().units()));
            let output = if escaped_output { escaped(value) } else { value.clone() };
            self.push(Token::with_output("text", value, output));
            return;
        }

        let kind = self.prev_token().kind;
        if !self.opts.is_true("dot") && (kind == "slash" || kind == "bos") {
            self.push(Token::with_output("qmark", value, self.chars.qmark_no_dot));
            return;
        }
        self.push(Token::with_output("qmark", value, self.chars.qmark));
    }

    /// The `!` cases that consume the token; anything else is plain text.
    fn exclamation(&mut self, value: &JsString) -> bool {
        if !self.opts.is_true("noextglob")
            && self.peek_is(1, '(')
            && (!self.peek_is(2, '?') || !is_lookaround_char(self.peek(3)))
        {
            self.extglob_open(ExtglobKind::Negate, value);
            return true;
        }
        if !self.opts.is_true("nonegate") && self.state.index == 0 {
            self.negate();
            return true;
        }
        false
    }

    fn plus(&mut self, value: &JsString) {
        if !self.opts.is_true("noextglob") && self.peek_is(1, '(') && !self.peek_is(2, '?') {
            self.extglob_open(ExtglobKind::Plus, value);
            return;
        }

        if self.prev_token().value.eq_str("(") || self.opts.is_false("regex") {
            self.push(Token::with_output("plus", value, self.chars.plus_literal));
            return;
        }

        let kind = self.prev_token().kind;
        if kind == "bracket" || kind == "paren" || kind == "brace" || self.state.parens > 0 {
            self.push(Token::new("plus", value));
            return;
        }

        self.push(Token::new("plus", &JsString::from(self.chars.plus_literal)));
    }

    fn plain_text(&mut self, mut value: JsString) {
        if is(&value, '$') || is(&value, '^') {
            value = escaped(&value);
        }

        let remaining = self.remaining();
        let run = remaining
            .units()
            .iter()
            .take_while(|&&u| !char::from_u32(u32::from(u)).is_some_and(|c| "@![].,$*+?^{}()|\\/".contains(c)))
            .count();
        if run > 0 {
            value.push_units(&remaining.units()[..run]);
            self.state.index += run as isize;
        }

        self.push(Token::new("text", &value));
    }

    fn star(&mut self, value: &JsString) {
        let prev = self.prev_token();
        if prev.kind == "globstar" || prev.star {
            let star = self.star.clone();
            let prev = self.prev_token_mut();
            prev.kind = "star";
            prev.star = true;
            prev.value.push_js(value);
            prev.output = Some(star);
            self.state.backtrack = true;
            self.state.globstar = true;
            self.consume(value, 0);
            return;
        }

        let rest = self.remaining();
        let opens_extglob =
            rest.units().first() == Some(&unit('(')) && rest.units().get(1).is_some_and(|&u| u != unit('?'));
        if !self.opts.is_true("noextglob") && opens_extglob {
            self.extglob_open(ExtglobKind::Star, value);
            return;
        }

        if self.prev_token().kind == "star" {
            self.second_star(value, rest);
            return;
        }

        let mut token = Token::with_output("star", value, self.star.clone());

        if self.opts.is_true("bash") {
            let kind = self.prev_token().kind;
            if kind == "bos" || kind == "slash" {
                let mut output = JsString::from(if self.opts.truthy("dot") { "" } else { self.chars.no_dot });
                output.push_str(".*?");
                token.output = Some(output);
            } else {
                token.output = Some(JsString::from(".*?"));
            }
            self.push(token);
            return;
        }

        let kind = self.prev_token().kind;
        if (kind == "bracket" || kind == "paren") && self.opts.is_true("regex") {
            token.output = Some(value.clone());
            self.push(token);
            return;
        }

        if self.state.index == self.state.start || kind == "slash" || kind == "dot" {
            let lookahead = if kind == "dot" {
                self.chars.no_dot_slash
            } else if self.opts.is_true("dot") {
                self.chars.no_dots_slash
            } else if self.opts.truthy("dot") {
                ""
            } else {
                self.chars.no_dot
            };
            self.extend_prev_output(lookahead);
            if !self.peek_is(1, '*') {
                self.extend_prev_output(self.chars.one_char);
            }
        }

        self.push(token);
    }

    /// Appends to the state's output and to the previous token's; a token without an output reads as `undefined`.
    fn extend_prev_output(&mut self, text: &str) {
        self.state.output.push_str(text);
        let prev = self.prev_token_mut();
        prev.output
            .get_or_insert_with(|| JsString::from("undefined"))
            .push_str(text);
    }

    /// A star right after a star: a globstar, or a run of plain stars.
    fn second_star(&mut self, value: &JsString, mut rest: JsString) {
        if self.opts.is_true("noglobstar") {
            self.consume(value, 0);
            return;
        }

        let Some(prior) = self.prev_token().prev else {
            return;
        };
        let before = self.arena[prior].prev;
        let prior_kind = self.arena[prior].kind;
        let is_start = prior_kind == "slash" || prior_kind == "bos";
        let after_star = before.is_some_and(|b| matches!(self.arena[b].kind, "star" | "globstar"));
        let first_after = rest.units().first().copied();

        if self.opts.is_true("bash") && (!is_start || first_after.is_some_and(|u| u != unit('/'))) {
            self.push(Token::with_output("star", value, ""));
            return;
        }

        let is_brace = self.state.braces > 0 && (prior_kind == "comma" || prior_kind == "brace");
        let is_extglob = !self.extglobs.is_empty() && (prior_kind == "pipe" || prior_kind == "paren");
        if !is_start && prior_kind != "paren" && !is_brace && !is_extglob {
            self.push(Token::with_output("star", value, ""));
            return;
        }

        // strip consecutive `/**/`
        while rest.starts_with_str("/**") {
            if self
                .unit_at(self.state.index + 4)
                .is_some_and(|after| after != unit('/'))
            {
                break;
            }
            rest = rest.slice_from(3);
            self.consume(&JsString::from("/**"), 3);
        }

        // A globstar followed only by balanced closing parens is at the logical end of patterns like
        // `test(/utils/**)` and `test?(/utils/**)`. Treat it as EOS so the trailing `/**` can match its
        // parent path, except in negated extglobs where that would change the exclusion semantics.
        let closes_all = self.state.parens > 0
            && rest.len() as i64 == self.state.parens
            && rest.units().iter().all(|&u| u == unit(')'))
            && !self.extglobs.iter().any(|extglob| extglob.kind == ExtglobKind::Negate);
        let is_end = self.eos() || closes_all;
        let rest_first = rest.units().first().copied();

        let global = globstar(&self.opts, self.chars, self.capture);
        let slash = self.chars.slash_literal;

        if prior_kind == "bos" && self.eos() {
            self.make_globstar(value, global.clone());
            self.state.output = global;
            self.state.globstar = true;
            self.consume(value, 0);
            return;
        }

        let prior_after_bos = before.is_some_and(|b| self.arena[b].kind != "bos");
        if prior_kind == "slash" && prior_after_bos && !after_star && is_end {
            let mut output = global;
            output.push_str(if self.opts.truthy("strictSlashes") { ")" } else { "|$)" });
            self.rewrite_slash_star(prior, value, output);
            self.consume(value, 0);
            return;
        }

        if prior_kind == "slash" && prior_after_bos && rest_first == Some(unit('/')) {
            let mut output = global;
            output.push_str(slash);
            output.push_str("|");
            output.push_str(slash);
            if rest.len() > 1 {
                output.push_str("|$");
            }
            output.push_str(")");
            self.rewrite_slash_star(prior, value, output);
            let mut taken = value.clone();
            taken.push_js(&self.advance());
            self.consume(&taken, 0);
            self.push(Token::with_output("slash", &JsString::from("/"), ""));
            return;
        }

        if prior_kind == "bos" && rest_first == Some(unit('/')) {
            let mut output = JsString::from("(?:^|");
            output.push_str(slash);
            output.push_str("|");
            output.push_js(&global);
            output.push_str(slash);
            output.push_str(")");
            self.make_globstar(value, output.clone());
            self.state.output = output;
            self.state.globstar = true;
            let mut taken = value.clone();
            taken.push_js(&self.advance());
            self.consume(&taken, 0);
            self.push(Token::with_output("slash", &JsString::from("/"), ""));
            return;
        }

        // remove single star from output
        let length = self.prev_output().len();
        self.state.output = drop_last(&self.state.output, length);

        // reset previous token to globstar
        self.make_globstar(value, global.clone());

        // reset output with globstar
        self.state.output.push_js(&global);
        self.state.globstar = true;
        self.consume(value, 0);
    }

    fn make_globstar(&mut self, value: &JsString, output: JsString) {
        let prev = self.prev_token_mut();
        prev.kind = "globstar";
        prev.value.push_js(value);
        prev.output = Some(output);
    }

    /// Turns `/` and `*` into the optional group of a trailing or inner globstar.
    fn rewrite_slash_star(&mut self, prior: usize, value: &JsString, output: JsString) {
        let prior_output = self.arena[prior].output.clone().unwrap_or_default();
        let length = prior_output.len() + self.prev_output().len();
        self.state.output = drop_last(&self.state.output, length);

        let mut opened = JsString::from("(?:");
        opened.push_js(&prior_output);
        self.arena[prior].output = Some(opened.clone());

        self.make_globstar(value, output.clone());
        self.state.output.push_js(&opened);
        self.state.output.push_js(&output);
        self.state.globstar = true;
    }

    fn close_unbalanced(&mut self) -> Result<(), JsError> {
        while self.state.brackets > 0 {
            if self.opts.is_true("strictBrackets") {
                return Err(syntax_error("closing", ']'));
            }
            self.escape_last_open('[');
            self.decrement("brackets");
        }
        while self.state.parens > 0 {
            if self.opts.is_true("strictBrackets") {
                return Err(syntax_error("closing", ')'));
            }
            self.escape_last_open('(');
            self.decrement("parens");
        }
        while self.state.braces > 0 {
            if self.opts.is_true("strictBrackets") {
                return Err(syntax_error("closing", '}'));
            }
            self.escape_last_open('{');
            self.decrement("braces");
        }

        let kind = self.prev_token().kind;
        if !self.opts.is_true("strictSlashes") && (kind == "star" || kind == "bracket") {
            let mut output = JsString::from(self.chars.slash_literal);
            output.push_str("?");
            self.push(Token::with_output("maybe_slash", &JsString::new(), output));
        }
        Ok(())
    }

    fn escape_last_open(&mut self, open: char) {
        self.state.output = JsString::from_units(utils::escape_last(self.state.output.units(), unit(open), None));
    }

    /// Rebuilds the output from the tokens if any step had to rewrite earlier ones.
    fn rebuild_output(&mut self) {
        if !self.state.backtrack {
            return;
        }
        self.state.output = JsString::new();
        for &index in &self.tokens {
            let token = &self.arena[index];
            self.state.output.push_js(token.output.as_ref().unwrap_or(&token.value));
        }
    }
}

fn escaped(value: &JsString) -> JsString {
    let mut out = JsString::from("\\");
    out.push_js(value);
    out
}

/// The result of the fast paths: the source for a few very common patterns, or `None` for the full parser.
pub fn fastpaths(input: &JsString, options: &Options) -> Result<Option<JsString>, JsError> {
    check_length(input.len(), max_length(options))?;
    let input = replacement(input);
    let chars = constants::glob_chars(options.is_true("windows"));

    let mut star = if options.is_true("bash") {
        String::from(".*?")
    } else {
        String::from(chars.star)
    };
    if options.truthy("capture") {
        star = format!("({star})");
    }

    let fast = FastPath {
        options,
        chars,
        nodot: if options.truthy("dot") {
            chars.no_dots
        } else {
            chars.no_dot
        },
        slash_dot: if options.truthy("dot") {
            chars.no_dots_slash
        } else {
            chars.no_dot
        },
        capture: if options.truthy("capture") { "" } else { "?:" },
        star,
    };

    let mut prefix = JsString::new();
    let body = utils::remove_prefix(&input, &mut prefix);
    let Some(mut source) = fast.create(body.units()) else {
        return Ok(None);
    };
    if !options.is_true("strictSlashes") {
        source.push_str(chars.slash_literal);
        source.push_str("?");
    }
    Ok(Some(source))
}

struct FastPath<'a> {
    options: &'a Options,
    chars: &'static GlobChars,
    nodot: &'static str,
    slash_dot: &'static str,
    capture: &'static str,
    star: String,
}

impl FastPath<'_> {
    fn globstar(&self) -> String {
        if self.options.is_true("noglobstar") {
            return self.star.clone();
        }
        let tail = if self.options.truthy("dot") {
            self.chars.dots_slash
        } else {
            self.chars.dot_literal
        };
        format!("({}(?:(?!{}{tail})[\\s\\S])*?)", self.capture, self.chars.start_anchor)
    }

    fn create(&self, text: &[u16]) -> Option<JsString> {
        let (dot, slash, one) = (self.chars.dot_literal, self.chars.slash_literal, self.chars.one_char);
        let (nodot, slash_dot, star) = (self.nodot, self.slash_dot, &self.star);
        let globstar = self.globstar();
        let leading = format!("(?:{nodot}{globstar}{slash})?");
        // A name with unpaired surrogates cannot be one of the fixed forms, and falls through with the others.
        let source = match String::from_utf16(text).as_deref() {
            Ok("*") => format!("{nodot}{one}{star}"),
            Ok(".*") => format!("{dot}{one}{star}"),
            Ok("*.*") => format!("{nodot}{star}{dot}{one}{star}"),
            Ok("*/*") => format!("{nodot}{star}{slash}{one}{slash_dot}{star}"),
            Ok("**") => format!("{nodot}{globstar}"),
            Ok("**/*") => format!("{leading}{slash_dot}{one}{star}"),
            Ok("**/*.*") => format!("{leading}{slash_dot}{star}{dot}{one}{star}"),
            Ok("**/.*") => format!("{leading}{dot}{one}{star}"),
            _ => return self.with_extension(text),
        };
        Some(JsString::from(source))
    }

    fn with_extension(&self, text: &[u16]) -> Option<JsString> {
        let mut regex = RegExp::literal(r"^(.*?)\.(\w+)$", "")?;
        let found = regex.exec(text)?;
        let (stem_start, stem_end) = found.captures[1]?;
        let (ext_start, ext_end) = found.captures[2]?;
        let stem = &text[stem_start..stem_end];

        // A bare `**` here is adjacent to a literal extension in the same path
        // segment (e.g. `**.js`), so it must act as a single star rather than a
        // globstar. Bail out of the fast path and let the full parser handle it.
        if stem == [unit('*'), unit('*')] {
            return None;
        }

        let mut source = self.create(stem)?;
        source.push_str(self.chars.dot_literal);
        source.push_units(&text[ext_start..ext_end]);
        Some(source)
    }
}
