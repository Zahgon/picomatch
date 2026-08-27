// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The analysis of repeated extglobs: `+(...)` and `*(...)` bodies prone to catastrophic backtracking are compiled to
//! a literal, or to one flat character class when every branch reduces to single characters.

use crate::constants::DEFAULT_MAX_EXTGLOB_RECURSION;
use crate::js::string::is_js_whitespace;
use crate::js::{JsString, Value};
use crate::options::Options;
use crate::regexp::RegExp;
use crate::utils;

const BACKSLASH: u16 = b'\\' as u16;
const QUOTE: u16 = b'"' as u16;
const LEFT_PAREN: u16 = b'(' as u16;
const RIGHT_PAREN: u16 = b')' as u16;
const LEFT_BRACKET: u16 = b'[' as u16;
const RIGHT_BRACKET: u16 = b']' as u16;
const PIPE: u16 = b'|' as u16;
const PLUS: u16 = b'+' as u16;
const STAR: u16 = b'*' as u16;

/// What the analysis decides about an extglob body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Whether the extglob must not compile to a repeated group.
    pub risky: bool,
    /// The flat replacement, when there is one.
    pub safe_output: Option<JsString>,
}

impl Analysis {
    fn risky() -> Analysis {
        Analysis {
            risky: true,
            safe_output: None,
        }
    }
}

/// `String.prototype.trim`.
pub fn trim(text: &[u16]) -> &[u16] {
    let start = text.iter().position(|&unit| !is_js_whitespace(unit));
    let Some(start) = start else {
        return &[];
    };
    let end = text
        .iter()
        .rposition(|&unit| !is_js_whitespace(unit))
        .map_or(start, |e| e + 1);
    &text[start..end]
}

fn split_top_level(input: &[u16]) -> Vec<Vec<u16>> {
    let mut parts = Vec::new();
    let mut bracket = 0;
    let mut paren = 0;
    let mut quote = false;
    let mut value = Vec::new();
    let mut escaped = false;

    for &unit in input {
        if escaped {
            value.push(unit);
            escaped = false;
            continue;
        }
        if unit == BACKSLASH {
            value.push(unit);
            escaped = true;
            continue;
        }
        if unit == QUOTE {
            quote = !quote;
            value.push(unit);
            continue;
        }
        if !quote {
            if unit == LEFT_BRACKET {
                bracket += 1;
            } else if unit == RIGHT_BRACKET && bracket > 0 {
                bracket -= 1;
            } else if bracket == 0 {
                if unit == LEFT_PAREN {
                    paren += 1;
                } else if unit == RIGHT_PAREN && paren > 0 {
                    paren -= 1;
                } else if unit == PIPE && paren == 0 {
                    parts.push(std::mem::take(&mut value));
                    continue;
                }
            }
        }
        value.push(unit);
    }

    parts.push(value);
    parts
}

fn is_plain_branch(branch: &[u16]) -> bool {
    let mut escaped = false;
    for &unit in branch {
        if escaped {
            escaped = false;
            continue;
        }
        if unit == BACKSLASH {
            escaped = true;
            continue;
        }
        let special = char::from_u32(u32::from(unit)).is_some_and(|c| "?*+@!()[]{}".contains(c));
        if special {
            return false;
        }
    }
    true
}

/// Whether `body` is `@(...)` around one plain alternative.
fn is_simple_at_group(value: &[u16]) -> bool {
    let inner_ok = |unit: &u16| !char::from_u32(u32::from(*unit)).is_some_and(|c| "\\()[]{}|".contains(c));
    value.len() > 3
        && value.starts_with(&[u16::from(b'@'), LEFT_PAREN])
        && value[value.len() - 1] == RIGHT_PAREN
        && value[2..value.len() - 1].iter().all(inner_ok)
}

/// `value.replace(/\\(.)/g, '$1')`.
fn unescape(value: &[u16]) -> Vec<u16> {
    match RegExp::literal(r"\\(.)", "g") {
        Some(regex) => regex.replace_all(value, |found| {
            let (start, end) = found.captures[1].unwrap_or_default();
            value[start..end].to_vec()
        }),
        None => value.to_vec(),
    }
}

/// The text of a branch that is plain once its `@(...)` wrappers are gone.
fn normalize_simple_branch(branch: &[u16]) -> Option<Vec<u16>> {
    let mut value = trim(branch).to_vec();
    while is_simple_at_group(&value) {
        value = value[2..value.len() - 1].to_vec();
    }
    is_plain_branch(&value).then(|| unescape(&value))
}

fn has_repeated_char_prefix_overlap(branches: &[Vec<u16>]) -> bool {
    let values: Vec<Vec<u16>> = branches
        .iter()
        .filter_map(|branch| normalize_simple_branch(branch))
        .filter(|value| !value.is_empty())
        .collect();

    for (i, a) in values.iter().enumerate() {
        for b in &values[i + 1..] {
            let first = a[0];
            if a.iter().any(|&unit| unit != first) || b.iter().any(|&unit| unit != first) {
                continue;
            }
            if a == b || a.starts_with(b) || b.starts_with(a) {
                return true;
            }
        }
    }
    false
}

/// A `+(...)` or `*(...)` at the start of a pattern.
struct RepeatedExtglob {
    kind: u16,
    body: Vec<u16>,
    end: usize,
}

fn parse_repeated_extglob(pattern: &[u16], require_end: bool) -> Option<RepeatedExtglob> {
    let kind = *pattern.first()?;
    if (kind != PLUS && kind != STAR) || pattern.get(1) != Some(&LEFT_PAREN) {
        return None;
    }

    let mut bracket = 0;
    let mut paren = 0;
    let mut quote = false;
    let mut escaped = false;

    for (i, &unit) in pattern.iter().enumerate().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        if unit == BACKSLASH {
            escaped = true;
            continue;
        }
        if unit == QUOTE {
            quote = !quote;
            continue;
        }
        if quote {
            continue;
        }
        if unit == LEFT_BRACKET {
            bracket += 1;
            continue;
        }
        if unit == RIGHT_BRACKET && bracket > 0 {
            bracket -= 1;
            continue;
        }
        if bracket > 0 {
            continue;
        }
        if unit == LEFT_PAREN {
            paren += 1;
            continue;
        }
        if unit == RIGHT_PAREN {
            paren -= 1;
            if paren == 0 {
                if require_end && i != pattern.len() - 1 {
                    return None;
                }
                return Some(RepeatedExtglob {
                    kind,
                    body: pattern[2..i].to_vec(),
                    end: i,
                });
            }
        }
    }
    None
}

fn build_char_class_star(chars: &[Vec<u16>]) -> JsString {
    let mut source = if let [only] = chars {
        utils::escape_regex(only)
    } else {
        let mut class = JsString::from("[");
        for unit in chars {
            class.push_js(&utils::escape_regex(unit));
        }
        class.push_str("]");
        class
    };
    source.push_str("*");
    source
}

fn star_extglob_sequence_chars(pattern: &[u16]) -> Option<Vec<Vec<u16>>> {
    let mut index = 0;
    let mut chars = Vec::new();

    while index < pattern.len() {
        let found = parse_repeated_extglob(&pattern[index..], false)?;
        if found.kind != STAR {
            return None;
        }
        let branches = split_top_level(&found.body);
        let [only] = branches.as_slice() else {
            return None;
        };
        let branch = normalize_simple_branch(trim(only))?;
        if branch.len() != 1 {
            return None;
        }
        chars.push(branch);
        index += found.end + 1;
    }

    (!chars.is_empty()).then_some(chars)
}

fn repeated_extglob_recursion(pattern: &[u16]) -> u32 {
    let mut depth = 0;
    let mut found = parse_repeated_extglob(trim(pattern), true);
    while let Some(current) = found {
        depth += 1;
        found = parse_repeated_extglob(trim(&current.body), true);
    }
    depth
}

/// Whether a branch is one or more `*` and `?` characters.
fn is_all_wildcards(branch: &[u16]) -> bool {
    !branch.is_empty() && branch.iter().all(|&unit| unit == STAR || unit == u16::from(b'?'))
}

/// Analyzes the body of an extglob.
pub fn analyze_repeated_extglob(body: &[u16], options: &Options) -> Analysis {
    let setting = options.get("maxExtglobRecursion");
    if matches!(setting, Value::Bool(false)) {
        return Analysis::default();
    }
    let max = setting.as_number().unwrap_or(DEFAULT_MAX_EXTGLOB_RECURSION);

    let branches: Vec<Vec<u16>> = split_top_level(body)
        .iter()
        .map(|branch| trim(branch).to_vec())
        .collect();

    if branches.len() > 1
        && (branches.iter().any(Vec::is_empty)
            || branches.iter().any(|branch| is_all_wildcards(branch))
            || has_repeated_char_prefix_overlap(&branches))
    {
        return Analysis::risky();
    }

    let mut safe_chars: Vec<Vec<u16>> = Vec::new();
    let mut saw_star_sequence = false;
    let mut combinable = true;

    for branch in &branches {
        if let Some(chars) = star_extglob_sequence_chars(branch) {
            saw_star_sequence = true;
            safe_chars.extend(chars);
            continue;
        }

        if let Some(literal) = normalize_simple_branch(branch).filter(|literal| literal.len() == 1) {
            safe_chars.push(literal);
            continue;
        }

        combinable = false;

        if f64::from(repeated_extglob_recursion(branch)) > max {
            return Analysis::risky();
        }
    }

    if !saw_star_sequence {
        return Analysis::default();
    }
    if !combinable {
        return Analysis::risky();
    }
    let mut unique: Vec<Vec<u16>> = Vec::new();
    for unit in safe_chars {
        if !unique.contains(&unit) {
            unique.push(unit);
        }
    }
    Analysis {
        risky: true,
        safe_output: Some(build_char_class_star(&unique)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(body: &str) -> Analysis {
        let body: Vec<u16> = body.encode_utf16().collect();
        analyze_repeated_extglob(&body, &Options::default())
    }

    #[test]
    fn plain_alternatives_are_safe() {
        assert_eq!(analyze("a|b"), Analysis::default());
    }

    #[test]
    fn overlapping_repeated_characters_are_risky() {
        assert!(analyze("a|aa").risky);
        assert!(analyze("|a").risky);
        assert!(analyze("*|?").risky);
    }

    #[test]
    fn star_sequences_flatten_to_a_class() {
        assert_eq!(analyze("*(a)|*(b)").safe_output, Some(JsString::from("[ab]*")));
        assert_eq!(analyze("*(a)").safe_output, Some(JsString::from("a*")));
    }

    #[test]
    fn trim_uses_javascript_white_space() {
        let text: Vec<u16> = "\u{feff} a\u{a0}".encode_utf16().collect();
        assert_eq!(trim(&text), [u16::from(b'a')]);
    }
}
