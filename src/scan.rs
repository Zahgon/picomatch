// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Scans a glob pattern into its base path and glob, without building a regular expression.

use crate::js::{JsString, Object, Value};
use crate::options::Options;
use crate::utils;

const ASTERISK: u16 = b'*' as u16;
const AT: u16 = b'@' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const COMMA: u16 = b',' as u16;
const DOT: u16 = b'.' as u16;
const EXCLAMATION: u16 = b'!' as u16;
const SLASH: u16 = b'/' as u16;
const LEFT_BRACE: u16 = b'{' as u16;
const LEFT_PAREN: u16 = b'(' as u16;
const LEFT_BRACKET: u16 = b'[' as u16;
const PLUS: u16 = b'+' as u16;
const QUESTION: u16 = b'?' as u16;
const RIGHT_BRACE: u16 = b'}' as u16;
const RIGHT_PAREN: u16 = b')' as u16;
const RIGHT_BRACKET: u16 = b']' as u16;

fn is_path_separator(code: Option<u16>) -> bool {
    code == Some(SLASH) || code == Some(BACKSLASH)
}

/// `true` as a value; token members come into existence in the order the scan sets them.
fn yes() -> Value {
    Value::Bool(true)
}

fn new_token() -> Object {
    let mut token = Object::new();
    token.set_str("value", Value::String(JsString::new()));
    token.set_str("depth", Value::Number(0.0));
    token.set_str("isGlob", Value::Bool(false));
    token
}

/// Sets a token's depth: one segment, or unbounded for a globstar. A prefix token keeps depth 0.
fn set_depth(token: &mut Object) {
    if token.get_str("isPrefix").is_some_and(Value::truthy) {
        return;
    }
    let globstar = token.get_str("isGlobstar").is_some_and(Value::truthy);
    token.set_str("depth", Value::Number(if globstar { f64::INFINITY } else { 1.0 }));
}

fn token_depth(token: &Object) -> f64 {
    token.get_str("depth").and_then(Value::as_number).unwrap_or(f64::NAN)
}

/// Adds to the state's `maxDepth`, which is absent (and turns to `NaN`) unless `tokens` is exactly `true`.
fn add_depth(state: &mut Object, depth: f64) {
    let current = state.get_str("maxDepth").and_then(Value::as_number).unwrap_or(f64::NAN);
    state.set_str("maxDepth", Value::Number(current + depth));
}

/// The position of the cursor over the pattern's code units.
struct Cursor<'a> {
    units: &'a [u16],
    /// `str.length - 1`.
    length: isize,
    index: isize,
    code: Option<u16>,
    prev: Option<u16>,
}

impl Cursor<'_> {
    fn eos(&self) -> bool {
        self.index >= self.length
    }

    fn at(&self, index: isize) -> Option<u16> {
        usize::try_from(index).ok().and_then(|i| self.units.get(i).copied())
    }

    fn peek(&self) -> Option<u16> {
        self.at(self.index + 1)
    }

    fn advance(&mut self) -> Option<u16> {
        self.prev = self.code;
        self.index += 1;
        self.at(self.index)
    }

    /// The `while (eos() !== true && (code = advance()))` head of the inner loops: NUL ends them, as a falsy code.
    fn advance_truthy(&mut self) -> bool {
        if self.eos() {
            return false;
        }
        self.code = self.advance();
        self.code.is_some_and(|code| code != 0)
    }
}

/// The flags and positions a scan gathers.
#[derive(Default)]
struct Found {
    start: usize,
    last_index: isize,
    is_brace: bool,
    is_bracket: bool,
    is_glob: bool,
    is_extglob: bool,
    is_globstar: bool,
    brace_escaped: bool,
    backslashes: bool,
    negated: bool,
    negated_extglob: bool,
    finished: bool,
}

/// Skips a balanced parenthesis body with `scanToEnd`. Returns with `finished` set when the closing parenthesis is
/// found.
fn skip_parens(cursor: &mut Cursor<'_>, found: &mut Found, token: &mut Object, mut parens: i64) {
    while cursor.advance_truthy() {
        if cursor.code == Some(BACKSLASH) {
            found.backslashes = true;
            token.set_str("backslashes", yes());
            cursor.advance();
            continue;
        }
        if cursor.code == Some(LEFT_PAREN) {
            parens += 1;
            continue;
        }
        if cursor.code == Some(RIGHT_PAREN) {
            parens -= 1;
            if parens == 0 {
                found.finished = true;
                break;
            }
        }
    }
}

/// Scans a pattern; the result has the members of the library's `scan` state, in its order.
pub fn scan(input: &JsString, options: &Options) -> Object {
    let scan_to_end = options.is_true("parts") || options.is_true("tokens") || options.is_true("scanToEnd");
    let units = input.units();
    let mut slashes: Vec<usize> = Vec::new();
    let mut tokens: Vec<Object> = Vec::new();
    let mut parts: Vec<JsString> = Vec::new();
    let mut found = Found::default();
    let mut braces = 0i64;
    let mut token = new_token();
    let mut cursor = Cursor {
        units,
        length: units.len() as isize - 1,
        index: -1,
        code: None,
        prev: None,
    };

    while cursor.index < cursor.length {
        cursor.code = cursor.advance();

        if cursor.code == Some(BACKSLASH) {
            found.backslashes = true;
            token.set_str("backslashes", yes());
            cursor.code = cursor.advance();
            if cursor.code == Some(LEFT_BRACE) {
                found.brace_escaped = true;
            }
            continue;
        }

        if found.brace_escaped || cursor.code == Some(LEFT_BRACE) {
            braces += 1;
            while cursor.advance_truthy() {
                if cursor.code == Some(BACKSLASH) {
                    found.backslashes = true;
                    token.set_str("backslashes", yes());
                    cursor.advance();
                    continue;
                }
                if cursor.code == Some(LEFT_BRACE) {
                    braces += 1;
                    continue;
                }
                if !found.brace_escaped && cursor.code == Some(DOT) {
                    cursor.code = cursor.advance();
                    if cursor.code == Some(DOT) {
                        found.is_brace = true;
                        found.is_glob = true;
                        token.set_str("isBrace", yes());
                        token.set_str("isGlob", yes());
                        found.finished = true;
                        if scan_to_end {
                            continue;
                        }
                        break;
                    }
                }
                if !found.brace_escaped && cursor.code == Some(COMMA) {
                    found.is_brace = true;
                    found.is_glob = true;
                    token.set_str("isBrace", yes());
                    token.set_str("isGlob", yes());
                    found.finished = true;
                    if scan_to_end {
                        continue;
                    }
                    break;
                }
                if cursor.code == Some(RIGHT_BRACE) {
                    braces -= 1;
                    if braces == 0 {
                        found.brace_escaped = false;
                        found.is_brace = true;
                        token.set_str("isBrace", yes());
                        found.finished = true;
                        break;
                    }
                }
            }
            if scan_to_end {
                continue;
            }
            break;
        }

        if cursor.code == Some(SLASH) {
            slashes.push(cursor.index as usize);
            tokens.push(std::mem::replace(&mut token, new_token()));
            if found.finished {
                continue;
            }
            if cursor.prev == Some(DOT) && cursor.index == found.start as isize + 1 {
                found.start += 2;
                continue;
            }
            found.last_index = cursor.index + 1;
            continue;
        }

        if !options.is_true("noext") {
            let code = cursor.code;
            let is_extglob_char = [PLUS, AT, ASTERISK, QUESTION, EXCLAMATION].map(Some).contains(&code);
            if is_extglob_char && cursor.peek() == Some(LEFT_PAREN) {
                found.is_glob = true;
                found.is_extglob = true;
                token.set_str("isGlob", yes());
                token.set_str("isExtglob", yes());
                found.finished = true;
                if code == Some(EXCLAMATION) && cursor.index == found.start as isize {
                    found.negated_extglob = true;
                }
                if scan_to_end {
                    skip_parens(&mut cursor, &mut found, &mut token, 0);
                    continue;
                }
                break;
            }
        }

        if cursor.code == Some(ASTERISK) {
            if cursor.prev == Some(ASTERISK) {
                found.is_globstar = true;
                token.set_str("isGlobstar", yes());
            }
            found.is_glob = true;
            token.set_str("isGlob", yes());
            found.finished = true;
            if scan_to_end {
                continue;
            }
            break;
        }

        if cursor.code == Some(QUESTION) {
            found.is_glob = true;
            token.set_str("isGlob", yes());
            found.finished = true;
            if scan_to_end {
                continue;
            }
            break;
        }

        if cursor.code == Some(LEFT_BRACKET) {
            while !cursor.eos() {
                let next = cursor.advance();
                if next.is_none_or(|unit| unit == 0) {
                    break;
                }
                if next == Some(BACKSLASH) {
                    found.backslashes = true;
                    token.set_str("backslashes", yes());
                    cursor.advance();
                    continue;
                }
                if next == Some(RIGHT_BRACKET) {
                    found.is_bracket = true;
                    found.is_glob = true;
                    token.set_str("isBracket", yes());
                    token.set_str("isGlob", yes());
                    found.finished = true;
                    break;
                }
            }
            if scan_to_end {
                continue;
            }
            break;
        }

        if !options.is_true("nonegate") && cursor.code == Some(EXCLAMATION) && cursor.index == found.start as isize {
            found.negated = true;
            token.set_str("negated", yes());
            found.start += 1;
            continue;
        }

        if !options.is_true("noparen") && cursor.code == Some(LEFT_PAREN) {
            found.is_glob = true;
            token.set_str("isGlob", yes());
            if scan_to_end {
                skip_parens(&mut cursor, &mut found, &mut token, 1);
                continue;
            }
            break;
        }

        if found.is_glob {
            found.finished = true;
            if scan_to_end {
                continue;
            }
            break;
        }
    }

    if options.is_true("noext") {
        found.is_extglob = false;
        found.is_glob = false;
    }

    let start = found.start.min(units.len());
    let prefix = JsString::from_units(units[..start].to_vec());
    let rest = &units[start..];
    let last_index = found.last_index - found.start as isize;
    let (mut base, mut glob) = if !units.is_empty() && found.is_glob && last_index > 0 {
        let split = (last_index as usize).min(rest.len());
        (rest[..split].to_vec(), rest[split..].to_vec())
    } else if found.is_glob {
        (Vec::new(), rest.to_vec())
    } else {
        (rest.to_vec(), Vec::new())
    };

    if !base.is_empty() && base != [SLASH] && base != rest && is_path_separator(base.last().copied()) {
        base.pop();
    }

    if options.is_true("unescape") {
        if !glob.is_empty() {
            glob = utils::remove_backslashes(&JsString::from_units(glob)).into_units();
        }
        if !base.is_empty() && found.backslashes {
            base = utils::remove_backslashes(&JsString::from_units(base)).into_units();
        }
    }

    let mut state = Object::new();
    state.set_str("prefix", Value::String(prefix.clone()));
    state.set_str("input", Value::String(input.clone()));
    state.set_str("start", Value::Number(found.start as f64));
    state.set_str("base", Value::String(JsString::from_units(base)));
    state.set_str("glob", Value::String(JsString::from_units(glob)));
    state.set_str("isBrace", Value::Bool(found.is_brace));
    state.set_str("isBracket", Value::Bool(found.is_bracket));
    state.set_str("isGlob", Value::Bool(found.is_glob));
    state.set_str("isExtglob", Value::Bool(found.is_extglob));
    state.set_str("isGlobstar", Value::Bool(found.is_globstar));
    state.set_str("negated", Value::Bool(found.negated));
    state.set_str("negatedExtglob", Value::Bool(found.negated_extglob));

    let want_tokens = options.is_true("tokens");
    if want_tokens {
        state.set_str("maxDepth", Value::Number(0.0));
        if !is_path_separator(cursor.code) {
            tokens.push(token);
        }
    }

    if options.is_true("parts") || want_tokens {
        let tokens_truthy = options.truthy("tokens");
        let mut prev_index: Option<usize> = None;

        for (idx, &i) in slashes.iter().enumerate() {
            let n = prev_index.map_or(found.start, |prev| prev + 1);
            let value = JsString::from_units(units[n.min(i)..i].to_vec());
            if tokens_truthy {
                if idx == 0 && found.start != 0 {
                    tokens[idx].set_str("isPrefix", yes());
                    tokens[idx].set_str("value", Value::String(prefix.clone()));
                } else {
                    tokens[idx].set_str("value", Value::String(value.clone()));
                }
                set_depth(&mut tokens[idx]);
                add_depth(&mut state, token_depth(&tokens[idx]));
            }
            if i >= found.start {
                parts.push(value);
                prev_index = Some(i);
            }
        }

        let n = prev_index.map_or(found.start, |prev| prev + 1);
        let value = input.slice_from(n);
        parts.push(value.clone());

        if tokens_truthy && prev_index.is_some_and(|prev| prev != 0 && prev + 1 < units.len()) {
            if let Some(last) = tokens.last_mut() {
                last.set_str("value", Value::String(value));
                set_depth(last);
                let depth = token_depth(last);
                add_depth(&mut state, depth);
            }
        }

        if want_tokens {
            state.set_str("tokens", Value::Array(tokens.into_iter().map(Value::Object).collect()));
        }
        let numbers = slashes.iter().map(|&i| Value::Number(i as f64)).collect();
        state.set_str("slashes", Value::Array(numbers));
        state.set_str("parts", Value::Array(parts.into_iter().map(Value::String).collect()));
    }

    state
}
