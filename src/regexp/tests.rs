// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

use super::{Capture, RegExp};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn exec(source: &str, flags: &str, input: &str) -> Option<Vec<Capture>> {
    let mut regex = RegExp::new(&units(source), &units(flags)).expect("valid pattern");
    regex.exec(&units(input)).map(|found| found.captures)
}

fn is_match(source: &str, input: &str) -> bool {
    exec(source, "", input).is_some()
}

fn is_valid(source: &str) -> bool {
    RegExp::new(&units(source), &[]).is_ok()
}

#[test]
fn rejects_what_new_regexp_rejects() {
    for source in [
        "(",
        ")",
        "[",
        "\\",
        "*",
        "a**",
        "a{2,1}",
        "^*",
        "\\b+",
        "(?<=a)*",
        "(?<n>a)(?<n>b)",
        "[z-a]",
        "(?x)",
        "(?i:a)",
        "(?<1a>b)",
        "(?<n>a)\\k<m>",
        "{1}",
    ] {
        assert!(!is_valid(source), "{source}");
    }
}

#[test]
fn accepts_annex_b_syntax() {
    for source in [
        "]", "}", "{", "a{", "a{,1}", "a{1", "(?=a)*", "\\c", "\\1", "\\8", "[\\d-a]", "\\u{41}", "\\k", "\\p{L}",
        "a{2}?",
    ] {
        assert!(is_valid(source), "{source}");
    }
}

#[test]
fn matches_code_units() {
    assert!(!is_match("^.$", "😀"));
    assert!(is_match("^..$", "😀"));
    assert!(is_match("^[^/]$", "é"));
}

#[test]
fn line_terminators_stop_dot() {
    for input in ["\n", "\r", "\u{2028}", "\u{2029}"] {
        assert!(!is_match("^.$", input));
        assert!(exec("^.$", "s", input).is_some());
    }
}

#[test]
fn folds_case_without_crossing_into_ascii() {
    assert!(exec("^\u{e9}$", "i", "\u{c9}").is_some());
    assert!(exec("^s$", "i", "\u{17f}").is_none());
    assert!(exec("^k$", "i", "\u{212a}").is_none());
    assert!(exec("^[^a]$", "i", "A").is_none());
    assert!(exec("^[a-z]+$", "i", "ABC").is_some());
}

#[test]
fn resets_captures_every_iteration() {
    let input = "ab";
    assert_eq!(
        exec("(?:(a)|(b))+", "", input),
        Some(vec![Some((0, 2)), None, Some((1, 2))])
    );
}

#[test]
fn rejects_empty_iterations_past_the_minimum() {
    assert_eq!(exec("(a*)*", "", "b"), Some(vec![Some((0, 0)), None]));
    assert_eq!(exec("(a*)+", "", "b"), Some(vec![Some((0, 0)), Some((0, 0))]));
}

#[test]
fn lookbehind_reads_backwards() {
    assert_eq!(
        exec("(?<=(\\d+)(\\d+))$", "", "1053"),
        Some(vec![Some((4, 4)), Some((0, 1)), Some((1, 4))])
    );
    assert!(is_match("(?<!a)b", "cb"));
    assert!(!is_match("(?<!a)b", "ab"));
}

#[test]
fn backreferences_to_unset_groups_match_empty() {
    assert!(is_match("^\\1(a)$", "a"));
    assert!(is_match("^(?:(a)|b)\\1$", "b"));
    assert!(is_match("^(?<n>a)\\k<n>$", "aa"));
}

#[test]
fn legacy_escapes() {
    assert!(is_match("^\\101$", "A"));
    assert!(is_match("^\\8$", "8"));
    assert!(is_match("^\\cA$", "\u{1}"));
    assert!(is_match("^\\c$", "\\c"));
    assert!(is_match("^[\\b]$", "\u{8}"));
}

#[test]
fn lookahead_is_atomic() {
    assert!(!is_match("^(?=(a+))a*b\\1", "baaabac"));
    assert_eq!(exec("(?=(a+))a*b\\1", "", "baaabac").map(|c| c[0]), Some(Some((3, 6))));
}

#[test]
fn global_and_sticky_move_last_index() {
    let mut regex = RegExp::new(&units("a"), &units("g")).expect("valid pattern");
    let input = units("aa");
    assert!(regex.exec(&input).is_some());
    assert!(regex.exec(&input).is_some());
    assert!(regex.exec(&input).is_none());
    assert!(regex.exec(&input).is_some());
}
