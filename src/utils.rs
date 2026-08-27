// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! String helpers shared by the scanner, the parser and the matcher.

use crate::js::string::last_index_of;
use crate::js::{JsString, Value};
use crate::regexp::RegExp;

const BACKSLASH: u16 = b'\\' as u16;

fn is_regex_char_unit(unit: u16) -> bool {
    matches!(
        char::from_u32(u32::from(unit)),
        Some('-' | '*' | '+' | '?' | '.' | '^' | '$' | '{' | '}' | '(' | '|' | ')' | '[' | ']')
    )
}

/// Whether a string holds a character with a meaning in regular expressions.
pub fn has_regex_chars(text: &[u16]) -> bool {
    text.iter().any(|&unit| is_regex_char_unit(unit))
}

/// Puts a backslash before every character with a meaning in regular expressions.
pub fn escape_regex(text: &[u16]) -> JsString {
    let mut out = Vec::with_capacity(text.len());
    for &unit in text {
        if is_regex_char_unit(unit) {
            out.push(BACKSLASH);
        }
        out.push(unit);
    }
    JsString::from_units(out)
}

/// Turns backslashes into forward slashes, except those before a character that needs escaping.
pub fn to_posix_slashes(text: &[u16]) -> JsString {
    const KEPT: &str = "*+?^${}(|)[]";
    let mut out = text.to_vec();
    for (i, unit) in text.iter().enumerate() {
        let follower = text.get(i + 1).and_then(|&next| char::from_u32(u32::from(next)));
        if *unit == BACKSLASH && !follower.is_some_and(|c| KEPT.contains(c)) {
            out[i] = u16::from(b'/');
        }
    }
    JsString::from_units(out)
}

/// Removes the backslashes that escape something, leaving bracket expressions alone.
pub fn remove_backslashes(text: &JsString) -> JsString {
    let Some(regex) = RegExp::literal(r"(?:\[.*?[^\\]\]|\\(?=.))", "g") else {
        return text.clone();
    };
    JsString::from_units(regex.replace_all(text.units(), |found| {
        let (start, end) = found.captures[0].unwrap_or_default();
        let matched = &text.units()[start..end];
        if matched == [BACKSLASH] {
            Vec::new()
        } else {
            matched.to_vec()
        }
    }))
}

/// Puts a backslash before the last unescaped occurrence of `unit` at or before `last_index`.
pub fn escape_last(input: &[u16], unit: u16, last_index: Option<usize>) -> Vec<u16> {
    let from = last_index.unwrap_or(input.len());
    let Some(index) = last_index_of(input, &[unit], from) else {
        return input.to_vec();
    };
    if index > 0 && input[index - 1] == BACKSLASH {
        return escape_last(input, unit, Some(index - 1));
    }
    let mut out = input[..index].to_vec();
    out.push(BACKSLASH);
    out.extend_from_slice(&input[index..]);
    out
}

/// Takes a leading `./` off a pattern; the prefix goes to `prefix`.
pub fn remove_prefix(input: &JsString, prefix: &mut JsString) -> JsString {
    if input.starts_with_str("./") {
        *prefix = JsString::from("./");
        input.slice_from(2)
    } else {
        input.clone()
    }
}

/// Wraps generated source with its anchors and, for a negated pattern, the negation.
pub fn wrap_output(input: &[u16], negated: bool, contains: bool) -> JsString {
    let mut out = JsString::new();
    if !contains {
        out.push_str("^");
    }
    out.push_str("(?:");
    out.push_units(input);
    out.push_str(")");
    if !contains {
        out.push_str("$");
    }
    if negated {
        let mut wrapped = JsString::from("(?:^(?!");
        wrapped.push_js(&out);
        wrapped.push_str(").*$)");
        return wrapped;
    }
    out
}

/// The last segment of a path, skipping one trailing separator; `undefined` for an empty path.
pub fn basename(path: &[u16], windows: bool) -> Value {
    let is_separator = |unit: &u16| *unit == u16::from(b'/') || (windows && *unit == BACKSLASH);
    let segments: Vec<&[u16]> = path.split(is_separator).collect();
    let last = segments[segments.len() - 1];
    if !last.is_empty() {
        return Value::String(JsString::from_units(last.to_vec()));
    }
    match segments.len().checked_sub(2) {
        Some(index) => Value::String(JsString::from_units(segments[index].to_vec())),
        None => Value::Undefined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> JsString {
        JsString::from(value)
    }

    #[test]
    fn escapes_regex_characters() {
        assert_eq!(escape_regex(text("a.b*c").units()), text("a\\.b\\*c"));
    }

    #[test]
    fn keeps_escapes_of_special_characters_in_posix_slashes() {
        assert_eq!(to_posix_slashes(text("a\\b\\*").units()), text("a/b\\*"));
    }

    #[test]
    fn removes_backslashes_outside_brackets() {
        assert_eq!(remove_backslashes(&text("a\\b[\\c]")), text("ab[\\c]"));
    }

    #[test]
    fn escapes_the_last_unescaped_occurrence() {
        let out = escape_last(text("(a\\(").units(), u16::from(b'('), None);
        assert_eq!(JsString::from_units(out), text("\\(a\\("));
    }

    #[test]
    fn basename_skips_one_trailing_separator() {
        assert_eq!(basename(text("a/b/").units(), false).to_js_string(), text("b"));
        assert_eq!(basename(text("a\\b").units(), true).to_js_string(), text("b"));
        assert!(basename(&[], false).is_undefined());
    }
}
