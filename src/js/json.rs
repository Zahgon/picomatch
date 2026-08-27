// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! `JSON.parse` and `JSON.stringify` with the results and the error messages of Node.js 22 (V8).

use super::error::JsError;
use super::number::number_to_string;
use super::string::JsString;
use super::value::{Object, Value};

/// Sources shorter than this are quoted whole in "Unexpected token" messages.
const MIN_SOURCE_LENGTH_FOR_CONTEXT: usize = 21;
/// How deep arrays and objects may nest; the parser and the serializer recurse.
const MAX_NESTING: usize = 100_000;
/// The characters of context on each side of the unexpected token in longer sources.
const MAX_CONTEXT_CHARACTERS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    String,
    Number,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    True,
    False,
    Null,
    Whitespace,
    Colon,
    Comma,
    Illegal,
    Eos,
}

fn token_of(unit: u16) -> Token {
    match unit {
        0x22 => Token::String,
        0x2D | 0x30..=0x39 => Token::Number,
        0x7B => Token::LBrace,
        0x7D => Token::RBrace,
        0x5B => Token::LBrack,
        0x5D => Token::RBrack,
        0x74 => Token::True,
        0x66 => Token::False,
        0x6E => Token::Null,
        0x20 | 0x09 | 0x0D | 0x0A => Token::Whitespace,
        0x3A => Token::Colon,
        0x2C => Token::Comma,
        _ => Token::Illegal,
    }
}

/// The messages V8 reports with a position.
#[derive(Clone, Copy)]
enum Message {
    ExpectedPropNameOrRBrace,
    ExpectedColonAfterPropertyName,
    ExpectedCommaOrRBrace,
    ExpectedDoubleQuotedPropertyName,
    ExpectedCommaOrRBrack,
    UnterminatedString,
    BadEscapedCharacter,
    BadUnicodeEscape,
    BadControlCharacter,
    NoNumberAfterMinusSign,
    UnterminatedFractionalNumber,
    ExponentPartMissingNumber,
    UnexpectedNonWhitespace,
}

impl Message {
    fn text(self) -> &'static str {
        match self {
            Message::ExpectedPropNameOrRBrace => "Expected property name or '}' in JSON",
            Message::ExpectedColonAfterPropertyName => "Expected ':' after property name in JSON",
            Message::ExpectedCommaOrRBrace => "Expected ',' or '}' after property value in JSON",
            Message::ExpectedDoubleQuotedPropertyName => "Expected double-quoted property name in JSON",
            Message::ExpectedCommaOrRBrack => "Expected ',' or ']' after array element in JSON",
            Message::UnterminatedString => "Unterminated string in JSON",
            Message::BadEscapedCharacter => "Bad escaped character in JSON",
            Message::BadUnicodeEscape => "Bad Unicode escape in JSON",
            Message::BadControlCharacter => "Bad control character in string literal in JSON",
            Message::NoNumberAfterMinusSign => "No number after minus sign in JSON",
            Message::UnterminatedFractionalNumber => "Unterminated fractional number in JSON",
            Message::ExponentPartMissingNumber => "Exponent part is missing a number in JSON",
            Message::UnexpectedNonWhitespace => "Unexpected non-whitespace character after JSON",
        }
    }
}

/// `JSON.parse(source)`.
pub fn parse(source: &[u16]) -> Result<Value, JsError> {
    let mut parser = Parser {
        src: source,
        pos: 0,
        depth: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.pos < source.len() {
        return Err(parser.report(parser.peek(), Some(Message::UnexpectedNonWhitespace)));
    }
    Ok(value)
}

/// `JSON.parse` of Rust text.
pub fn parse_str(source: &str) -> Result<Value, JsError> {
    let units: Vec<u16> = source.encode_utf16().collect();
    parse(&units)
}

struct Parser<'a> {
    src: &'a [u16],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Token {
        match self.src.get(self.pos) {
            Some(&unit) => token_of(unit),
            None => Token::Eos,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek() == Token::Whitespace {
            self.pos += 1;
        }
    }

    fn syntax_error(message: impl Into<JsString>) -> JsError {
        JsError::new("SyntaxError", message)
    }

    /// The error for `token` at the current position: the explicit message when there is one, otherwise the
    /// message V8 derives from the token.
    fn report(&self, token: Token, message: Option<Message>) -> JsError {
        if let Some(message) = message {
            return self.positioned(message.text());
        }
        match token {
            Token::Eos => Parser::syntax_error("Unexpected end of JSON input"),
            Token::Number => self.positioned("Unexpected number in JSON"),
            Token::String => self.positioned("Unexpected string in JSON"),
            _ => self.unexpected_token(),
        }
    }

    fn positioned(&self, text: &str) -> JsError {
        let (mut line, mut line_start, mut i) = (1, 0, 0);
        while i < self.pos {
            match self.src[i] {
                0x0D => {
                    if self.src.get(i + 1) == Some(&0x0A) && i + 1 < self.pos {
                        i += 1;
                    }
                    line += 1;
                    line_start = i + 1;
                }
                0x0A => {
                    line += 1;
                    line_start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        let column = self.pos - line_start + 1;
        Parser::syntax_error(format!("{text} at position {} (line {line} column {column})", self.pos))
    }

    fn unexpected_token(&self) -> JsError {
        let src = self.src;
        for whole in ["NaN", "Infinity", "undefined", "[object Object]"] {
            if JsString::from_units(src.to_vec()).eq_str(whole) {
                return Parser::syntax_error(format!("\"{whole}\" is not valid JSON"));
            }
        }
        let mut message = JsString::from("Unexpected token '");
        message.push_units(&src[self.pos..=self.pos]);
        message.push_str("', ");
        let len = src.len();
        if len < MIN_SOURCE_LENGTH_FOR_CONTEXT {
            message.push_str("\"");
            message.push_units(src);
            message.push_str("\"");
        } else if self.pos < MAX_CONTEXT_CHARACTERS {
            message.push_str("\"");
            message.push_units(&src[..self.pos + MAX_CONTEXT_CHARACTERS]);
            message.push_str("\"...");
        } else if self.pos < len - MAX_CONTEXT_CHARACTERS {
            message.push_str("...\"");
            message.push_units(&src[self.pos - MAX_CONTEXT_CHARACTERS..self.pos + MAX_CONTEXT_CHARACTERS]);
            message.push_str("\"...");
        } else {
            message.push_str("...\"");
            message.push_units(&src[self.pos - MAX_CONTEXT_CHARACTERS..]);
            message.push_str("\"");
        }
        message.push_str(" is not valid JSON");
        Parser::syntax_error(message)
    }

    fn expect(&mut self, token: Token, message: Message) -> Result<(), JsError> {
        self.skip_whitespace();
        if self.peek() == token {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.report(self.peek(), Some(message)))
        }
    }

    fn parse_value(&mut self) -> Result<Value, JsError> {
        self.skip_whitespace();
        match self.peek() {
            Token::String => Ok(Value::String(self.scan_string()?)),
            Token::Number => self.scan_number(),
            Token::LBrace => self.nested(Parser::parse_object),
            Token::LBrack => self.nested(Parser::parse_array),
            Token::True => self.scan_literal("true", Value::Bool(true)),
            Token::False => self.scan_literal("false", Value::Bool(false)),
            Token::Null => self.scan_literal("null", Value::Null),
            token => Err(self.report(token, None)),
        }
    }

    /// Parses a nested container, refusing depths the native stack could not take.
    fn nested(&mut self, parse: fn(&mut Self) -> Result<Value, JsError>) -> Result<Value, JsError> {
        if self.depth >= MAX_NESTING {
            return Err(JsError::new("RangeError", "Maximum call stack size exceeded"));
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn parse_object(&mut self) -> Result<Value, JsError> {
        self.pos += 1;
        let mut object = Object::new();
        self.skip_whitespace();
        if self.peek() == Token::RBrace {
            self.pos += 1;
            return Ok(Value::Object(object));
        }
        let mut key_message = Message::ExpectedPropNameOrRBrace;
        loop {
            self.skip_whitespace();
            if self.peek() != Token::String {
                return Err(self.report(self.peek(), Some(key_message)));
            }
            let key = self.scan_string()?;
            self.expect(Token::Colon, Message::ExpectedColonAfterPropertyName)?;
            let value = self.parse_value()?;
            object.set(key, value);
            self.skip_whitespace();
            if self.peek() == Token::Comma {
                self.pos += 1;
                key_message = Message::ExpectedDoubleQuotedPropertyName;
                continue;
            }
            self.expect(Token::RBrace, Message::ExpectedCommaOrRBrace)?;
            return Ok(Value::Object(object));
        }
    }

    fn parse_array(&mut self) -> Result<Value, JsError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Token::RBrack {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_whitespace();
            if self.peek() == Token::Comma {
                self.pos += 1;
                continue;
            }
            self.expect(Token::RBrack, Message::ExpectedCommaOrRBrack)?;
            return Ok(Value::Array(items));
        }
    }

    fn scan_literal(&mut self, literal: &str, value: Value) -> Result<Value, JsError> {
        let expected: Vec<u16> = literal.encode_utf16().collect();
        let remaining = self.src.len() - self.pos;
        if remaining >= expected.len() && self.src[self.pos..self.pos + expected.len()] == expected[..] {
            self.pos += expected.len();
            return Ok(value);
        }
        self.pos += 1;
        for &unit in expected.iter().skip(1).take(remaining - 1) {
            if self.src[self.pos] != unit {
                return Err(self.report(token_of(self.src[self.pos]), None));
            }
            self.pos += 1;
        }
        Err(self.report(Token::Eos, None))
    }

    fn is_digit(&self) -> bool {
        matches!(self.src.get(self.pos), Some(0x30..=0x39))
    }

    fn scan_number(&mut self) -> Result<Value, JsError> {
        let start = self.pos;
        if self.src[self.pos] == u16::from(b'-') {
            self.pos += 1;
        }
        match self.src.get(self.pos) {
            Some(0x30) => {
                self.pos += 1;
                if self.is_digit() {
                    return Err(self.report(Token::Number, None));
                }
            }
            Some(0x31..=0x39) => {
                while self.is_digit() {
                    self.pos += 1;
                }
            }
            _ => return Err(self.report(Token::Number, Some(Message::NoNumberAfterMinusSign))),
        }
        if self.src.get(self.pos) == Some(&u16::from(b'.')) {
            self.pos += 1;
            if !self.is_digit() {
                return Err(self.report(Token::Number, Some(Message::UnterminatedFractionalNumber)));
            }
            while self.is_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.src.get(self.pos), Some(0x65 | 0x45)) {
            self.pos += 1;
            if matches!(self.src.get(self.pos), Some(0x2B | 0x2D)) {
                self.pos += 1;
            }
            if !self.is_digit() {
                return Err(self.report(Token::Number, Some(Message::ExponentPartMissingNumber)));
            }
            while self.is_digit() {
                self.pos += 1;
            }
        }
        let text: String = self.src[start..self.pos].iter().map(|&u| char::from(u as u8)).collect();
        // The scan above accepted only valid number syntax, so this parse cannot fail.
        Ok(Value::Number(text.parse::<f64>().unwrap_or(f64::NAN)))
    }

    fn hex_digit(&self) -> Option<u16> {
        self.src
            .get(self.pos)
            .and_then(|&u| char::from_u32(u32::from(u)))
            .and_then(|c| c.to_digit(16))
            .map(|d| d as u16)
    }

    fn scan_string(&mut self) -> Result<JsString, JsError> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(&unit) = self.src.get(self.pos) else {
                return Err(self.report(Token::Eos, Some(Message::UnterminatedString)));
            };
            match unit {
                0x22 => {
                    self.pos += 1;
                    return Ok(JsString::from_units(out));
                }
                0x5C => {
                    self.pos += 1;
                    let Some(&escape) = self.src.get(self.pos) else {
                        return Err(self.report(Token::Eos, None));
                    };
                    let decoded = match escape {
                        0x22 => 0x22,
                        0x5C => 0x5C,
                        0x2F => 0x2F,
                        0x62 => 0x08,
                        0x66 => 0x0C,
                        0x6E => 0x0A,
                        0x72 => 0x0D,
                        0x74 => 0x09,
                        0x75 => {
                            self.pos += 1;
                            let mut code = 0u16;
                            for _ in 0..4 {
                                let Some(digit) = self.hex_digit() else {
                                    return Err(self.report(self.peek(), Some(Message::BadUnicodeEscape)));
                                };
                                code = code << 4 | digit;
                                self.pos += 1;
                            }
                            out.push(code);
                            continue;
                        }
                        // V8 classifies only Latin-1 units as escape characters; any other unit is an illegal token.
                        0x100.. => return Err(self.unexpected_token()),
                        _ => return Err(self.report(self.peek(), Some(Message::BadEscapedCharacter))),
                    };
                    out.push(decoded);
                    self.pos += 1;
                }
                0x00..=0x1F => return Err(self.report(Token::Illegal, Some(Message::BadControlCharacter))),
                _ => {
                    out.push(unit);
                    self.pos += 1;
                }
            }
        }
    }
}

/// `JSON.stringify(value)`; `None` where JavaScript returns `undefined`.
pub fn stringify(value: &Value) -> Option<JsString> {
    let mut out = JsString::new();
    write_value(&mut out, value).then_some(out)
}

/// Appends `text` as a JSON string literal.
pub fn quote(out: &mut JsString, text: &[u16]) {
    out.push_str("\"");
    let mut i = 0;
    while i < text.len() {
        let unit = text[i];
        match unit {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x00..=0x1F => out.push_str(&format!("\\u{unit:04x}")),
            0xD800..=0xDBFF => {
                if matches!(text.get(i + 1), Some(0xDC00..=0xDFFF)) {
                    out.push_units(&text[i..i + 2]);
                    i += 1;
                } else {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            0xDC00..=0xDFFF => out.push_str(&format!("\\u{unit:04x}")),
            _ => out.push_units(&[unit]),
        }
        i += 1;
    }
    out.push_str("\"");
}

/// Appends the serialization of `value`; false when the value serializes to nothing (`undefined`, functions).
fn write_value(out: &mut JsString, value: &Value) -> bool {
    match value {
        Value::Undefined => return false,
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_finite() {
                out.push_str(&number_to_string(*n));
            } else {
                out.push_str("null");
            }
        }
        Value::String(s) => quote(out, s.units()),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return true;
            }
            out.push_str("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",");
                }
                if !write_value(out, item) {
                    out.push_str("null");
                }
            }
            out.push_str("]");
        }
        Value::Object(object) => {
            out.push_str("{");
            let mut first = true;
            for (key, item) in object.iter() {
                if matches!(item, Value::Undefined) {
                    continue;
                }
                if !first {
                    out.push_str(",");
                }
                first = false;
                quote(out, key.units());
                out.push_str(":");
                write_value(out, item);
            }
            out.push_str("}");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(source: &str) -> String {
        match parse_str(source) {
            Ok(_) => String::from("OK"),
            Err(e) => e.message.to_string_lossy(),
        }
    }

    #[test]
    fn parse_errors_match_v8() {
        assert_eq!(message("[1,]"), "Unexpected token ']', \"[1,]\" is not valid JSON");
        assert_eq!(
            message("{\"a\":1"),
            "Expected ',' or '}' after property value in JSON at position 6 (line 1 column 7)"
        );
        assert_eq!(
            message("\"\\u12x4\""),
            "Bad Unicode escape in JSON at position 5 (line 1 column 6)"
        );
        assert_eq!(
            message("tr1"),
            "Unexpected number in JSON at position 2 (line 1 column 3)"
        );
        assert_eq!(
            message("\"\\\u{ff}\""),
            "Bad escaped character in JSON at position 2 (line 1 column 3)"
        );
        assert_eq!(
            message("\"\\\u{100}\""),
            "Unexpected token '\u{100}', \"\"\\\u{100}\"\" is not valid JSON"
        );
        assert_eq!(message("fals"), "Unexpected end of JSON input");
        assert_eq!(message("falsx"), "Unexpected token 'x', \"falsx\" is not valid JSON");
        assert_eq!(message("[object Object]"), "\"[object Object]\" is not valid JSON");
        assert_eq!(
            message("[1,1,1,1,1,@1111111111"),
            "Unexpected token '@', ...\"1,1,1,1,1,@111111111\"... is not valid JSON"
        );
        assert_eq!(
            message("xxxxxxxxxxxxxxxxxxxxx"),
            "Unexpected token 'x', \"xxxxxxxxxx\"... is not valid JSON"
        );
        assert_eq!(
            message("[1 :]"),
            "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)"
        );
        assert_eq!(
            message("-"),
            "No number after minus sign in JSON at position 1 (line 1 column 2)"
        );
        assert_eq!(
            message("[]\r\r\nx"),
            "Unexpected non-whitespace character after JSON at position 5 (line 3 column 1)"
        );
    }

    #[test]
    fn stringifies_like_javascript() {
        let value = parse_str("{\"b\":[1,-0,1e21,\"\\u0001\\ud800\"],\"1\":{},\"a\":[]}").ok();
        let compact = value.as_ref().and_then(stringify).map(|s| s.to_string_lossy());
        assert_eq!(
            compact.as_deref(),
            Some("{\"1\":{},\"b\":[1,0,1e+21,\"\\u0001\\ud800\"],\"a\":[]}")
        );
    }
}
