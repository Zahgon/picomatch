// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! JavaScript strings: sequences of UTF-16 code units, which may hold unpaired surrogates.
//!
//! Every length, index and comparison the library makes counts UTF-16 code units, so strings are kept in that form
//! and converted to UTF-8 only when they are written out.

use std::fmt;

/// A JavaScript string value.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsString(Vec<u16>);

impl JsString {
    /// The empty string.
    pub fn new() -> Self {
        JsString(Vec::new())
    }

    /// Wraps UTF-16 code units.
    pub fn from_units(units: Vec<u16>) -> Self {
        JsString(units)
    }

    /// A string of one code unit.
    pub fn from_unit(unit: u16) -> Self {
        JsString(vec![unit])
    }

    /// The code units of the string.
    pub fn units(&self) -> &[u16] {
        &self.0
    }

    /// Consumes the string, returning its code units.
    pub fn into_units(self) -> Vec<u16> {
        self.0
    }

    /// The length in UTF-16 code units, as `String.prototype.length`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends another string.
    pub fn push_js(&mut self, other: &JsString) {
        self.0.extend_from_slice(&other.0);
    }

    /// Appends UTF-16 code units.
    pub fn push_units(&mut self, units: &[u16]) {
        self.0.extend_from_slice(units);
    }

    /// Appends a Rust string.
    pub fn push_str(&mut self, text: &str) {
        self.0.extend(text.encode_utf16());
    }

    /// The code unit at `index`, if any.
    pub fn unit_at(&self, index: usize) -> Option<u16> {
        self.0.get(index).copied()
    }

    /// `String.prototype.substring`: the bounds are clamped to the string and swapped when reversed.
    pub fn substring(&self, start: f64, end: f64) -> JsString {
        JsString(substring(&self.0, start, end).to_vec())
    }

    /// `String.prototype.slice` with one argument: the code units from `start` on.
    pub fn slice_from(&self, start: usize) -> JsString {
        JsString(self.0.get(start..).unwrap_or_default().to_vec())
    }

    /// `String.prototype.split` with a one code unit separator.
    pub fn split_unit(&self, separator: u16) -> Vec<JsString> {
        self.0
            .split(|unit| *unit == separator)
            .map(|part| JsString(part.to_vec()))
            .collect()
    }

    /// `String.prototype.split` with a string separator.
    pub fn split_str(&self, separator: &str) -> Vec<JsString> {
        let separator: Vec<u16> = separator.encode_utf16().collect();
        split_units(&self.0, &separator)
            .into_iter()
            .map(|part| JsString(part.to_vec()))
            .collect()
    }

    /// Whether the string starts with the given ASCII text.
    pub fn starts_with_str(&self, prefix: &str) -> bool {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        self.0.starts_with(&prefix)
    }

    /// Whether the string equals the given text.
    pub fn eq_str(&self, text: &str) -> bool {
        self.0.iter().copied().eq(text.encode_utf16())
    }

    /// Joins strings with a separator, as `Array.prototype.join`.
    pub fn join(parts: &[JsString], separator: &str) -> JsString {
        let separator: Vec<u16> = separator.encode_utf16().collect();
        let mut out = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(&separator);
            }
            out.extend_from_slice(&part.0);
        }
        JsString(out)
    }

    /// Replaces every occurrence of `from` with `to`.
    pub fn replace_all(&self, from: &str, to: &str) -> JsString {
        let from: Vec<u16> = from.encode_utf16().collect();
        let to: Vec<u16> = to.encode_utf16().collect();
        let parts = split_units(&self.0, &from);
        let mut out = Vec::with_capacity(self.0.len());
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(&to);
            }
            out.extend_from_slice(part);
        }
        JsString(out)
    }

    /// The string as UTF-8, with every unpaired surrogate replaced by U+FFFD.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }

    /// The string as UTF-8 bytes in the WTF-8 sense a JavaScript runtime writes them: unpaired surrogates become
    /// U+FFFD, as `Buffer.from(string, 'utf8')` does.
    pub fn to_utf8(&self) -> String {
        self.to_string_lossy()
    }

    /// Whether the string is a canonical array index ("0" .. "4294967294"), the keys JavaScript objects order
    /// first and numerically.
    pub fn array_index(&self) -> Option<u32> {
        array_index(&self.0)
    }
}

impl From<&str> for JsString {
    fn from(text: &str) -> Self {
        JsString(text.encode_utf16().collect())
    }
}

impl From<String> for JsString {
    fn from(text: String) -> Self {
        JsString(text.encode_utf16().collect())
    }
}

impl fmt::Debug for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

impl fmt::Display for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

/// `String.prototype.substring` on code units.
pub fn substring(units: &[u16], start: f64, end: f64) -> &[u16] {
    let len = units.len() as f64;
    let clamp = |value: f64| {
        if value.is_nan() {
            0.0
        } else {
            value.trunc().clamp(0.0, len)
        }
    };
    let (a, b) = (clamp(start), clamp(end));
    let (from, to) = if a <= b { (a, b) } else { (b, a) };
    &units[from as usize..to as usize]
}

/// The canonical array index a key denotes, if it is one.
pub fn array_index(units: &[u16]) -> Option<u32> {
    if units.is_empty() || units.len() > 10 {
        return None;
    }
    if units[0] == u16::from(b'0') {
        return if units.len() == 1 { Some(0) } else { None };
    }
    let mut value: u64 = 0;
    for &unit in units {
        if !(u16::from(b'0')..=u16::from(b'9')).contains(&unit) {
            return None;
        }
        value = value * 10 + u64::from(unit - u16::from(b'0'));
    }
    if value < u64::from(u32::MAX) {
        Some(value as u32)
    } else {
        None
    }
}

/// Splits code units at every occurrence of a non-empty separator.
pub fn split_units<'a>(units: &'a [u16], separator: &[u16]) -> Vec<&'a [u16]> {
    let mut parts = Vec::new();
    if separator.is_empty() {
        parts.push(units);
        return parts;
    }
    let mut start = 0;
    let mut i = 0;
    while i + separator.len() <= units.len() {
        if &units[i..i + separator.len()] == separator {
            parts.push(&units[start..i]);
            i += separator.len();
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&units[start..]);
    parts
}

/// `String.prototype.indexOf`: the first occurrence of `needle` at or after `from`.
pub fn index_of(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    let from = from.min(haystack.len());
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// `String.prototype.lastIndexOf`: the last occurrence of `needle` starting at or before `from`.
pub fn last_index_of(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let last = (haystack.len() - needle.len()).min(from);
    (0..=last).rev().find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// Whether a code unit is JavaScript white space or a line terminator (`\s` in a regular expression, and what
/// `String.prototype.trim` and `parseInt` skip).
pub fn is_js_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x0009..=0x000D
            | 0x0020
            | 0x00A0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_clamps_and_swaps() {
        let s = JsString::from("abcdef");
        assert_eq!(s.substring(4.0, 2.0), JsString::from("cd"));
        assert_eq!(s.substring(-3.0, 2.0), JsString::from("ab"));
        assert_eq!(s.substring(2.0, 99.0), JsString::from("cdef"));
    }

    #[test]
    fn array_indices() {
        assert_eq!(JsString::from("0").array_index(), Some(0));
        assert_eq!(JsString::from("4294967294").array_index(), Some(4_294_967_294));
        assert_eq!(JsString::from("4294967295").array_index(), None);
        assert_eq!(JsString::from("01").array_index(), None);
        assert_eq!(JsString::from("-1").array_index(), None);
    }

    #[test]
    fn index_searches() {
        let hay: Vec<u16> = "abcabc".encode_utf16().collect();
        let needle: Vec<u16> = "bc".encode_utf16().collect();
        assert_eq!(index_of(&hay, &needle, 0), Some(1));
        assert_eq!(index_of(&hay, &needle, 2), Some(4));
        assert_eq!(last_index_of(&hay, &needle, 3), Some(1));
        assert_eq!(last_index_of(&hay, &needle, 9), Some(4));
    }
}
