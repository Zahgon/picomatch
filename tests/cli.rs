// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The `picomatch` command, driven through its binary.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_picomatch"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the command starts");
    let mut stdin = child.stdin.take().expect("standard input is piped");
    stdin.write_all(input.as_bytes()).expect("the request is written");
    drop(stdin);
    child.wait_with_output().expect("the command ends")
}

fn answer(input: &str) -> String {
    String::from_utf8(run(&[], input).stdout).expect("responses are UTF-8")
}

fn is_match(pattern: &str, input: &str, options: &str) -> bool {
    let request =
        format!("{{\"op\":\"isMatch\",\"patterns\":{pattern:?},\"inputs\":[{input:?}],\"options\":{options}}}\n");
    let response = answer(&request);
    assert!(response.starts_with("{\"results\":"), "{request} -> {response}");
    response.contains("[true]")
}

#[test]
fn stars_stay_in_their_segment() {
    assert!(is_match("*.js", "a.js", "{}"));
    assert!(!is_match("*.js", "a/b.js", "{}"));
    assert!(is_match("**/*.js", "a/b/c.js", "{}"));
    assert!(!is_match("*", ".a", "{}"));
    assert!(is_match("*", ".a", "{\"dot\":true}"));
}

#[test]
fn braces_and_ranges() {
    assert!(is_match("a/{b,c}/d", "a/c/d", "{}"));
    assert!(is_match("{1..3}", "2", "{}"));
    assert!(!is_match("{1..3}", "4", "{}"));
    assert!(is_match("{1..3}", "2", "{\"expandRange\":\"group\"}"));
}

#[test]
fn extglobs_and_negation() {
    assert!(is_match("!(a|b)", "c", "{}"));
    assert!(!is_match("!(a|b)", "a", "{}"));
    assert!(is_match("+(a|b)", "abab", "{}"));
    assert!(is_match("!a", "b", "{}"));
    assert!(!is_match("!a", "a", "{}"));
    assert!(is_match("!a", "!a", "{\"nonegate\":true}"));
}

#[test]
fn posix_brackets() {
    assert!(is_match("[[:alpha:]]", "a", "{}"));
    assert!(!is_match("[[:alpha:]]", "1", "{}"));
}

#[test]
fn windows_paths() {
    assert!(is_match("a/*", "a\\b", "{\"windows\":true}"));
    assert!(!is_match("a/*", "a\\b", "{}"));
}

#[test]
fn wildcards_count_utf16_code_units() {
    assert!(!is_match("?", "😀", "{}"));
    assert!(is_match("??", "😀", "{}"));
}

#[test]
fn an_invalid_regex_never_matches() {
    assert!(!is_match("(?<n>a)(?<n>b)", "ab", "{}"));
}

#[test]
fn scan_reports_base_and_glob() {
    let response = answer("{\"op\":\"scan\",\"input\":\"!./foo/*.js\"}\n");
    assert_eq!(
        response,
        "{\"prefix\":\"!./\",\"input\":\"!./foo/*.js\",\"start\":3,\"base\":\"foo\",\"glob\":\"*.js\",\
         \"isBrace\":false,\"isBracket\":false,\"isGlob\":true,\"isExtglob\":false,\"isGlobstar\":false,\
         \"negated\":true,\"negatedExtglob\":false}\n"
    );
}

#[test]
fn parse_reports_tokens_without_regex_source() {
    let response = answer("{\"op\":\"parse\",\"pattern\":\"a/*\"}\n");
    assert!(response.contains("{\"type\":\"slash\",\"value\":\"/\"}"), "{response}");
    assert!(!response.contains("output"), "{response}");
}

#[test]
fn captures_and_events() {
    let request = "{\"op\":\"match\",\"patterns\":\"*.js\",\"inputs\":[\"a.js\"],\"options\":{\"capture\":true},\"events\":true}\n";
    assert_eq!(
        answer(request),
        "{\"results\":[{\"isMatch\":true,\"output\":\"a.js\",\"match\":{\"captures\":[\"a.js\",\"a\"],\"index\":0}}],\
         \"events\":[[\"result\",\"a.js\",true,\"a.js\"],[\"match\",\"a.js\",true,\"a.js\"]]}\n"
    );
}

#[test]
fn ignore_turns_a_match_off() {
    let request = "{\"op\":\"isMatch\",\"patterns\":\"*\",\"inputs\":[\"a\",\"b\"],\"options\":{\"ignore\":\"a\"}}\n";
    assert_eq!(answer(request), "{\"results\":[false,true]}\n");
}

#[test]
fn errors_are_responses() {
    let output = run(&[], "nope\n{\"op\":\"isMatch\",\"patterns\":\"\",\"inputs\":[]}\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"error\":{\"name\":\"SyntaxError\",\"message\":\"Unexpected token 'o', \\\"nope\\\" is not valid JSON\"}}\n\
         {\"error\":{\"name\":\"TypeError\",\"message\":\"Expected pattern to be a non-empty string\"}}\n"
    );
}

#[test]
fn strict_brackets_report_the_missing_character() {
    let request = "{\"op\":\"isMatch\",\"patterns\":\"(a\",\"inputs\":[],\"options\":{\"strictBrackets\":true}}\n";
    assert!(answer(request).contains("Missing closing: \\\")\\\""));
}

#[test]
fn a_brace_range_over_an_open_extglob_is_an_error_response() {
    let request = "{\"op\":\"isMatch\",\"patterns\":\"{+(|a..b})\",\"inputs\":[\"a\"]}\n";
    assert!(answer(request).contains("TypeError"));
}

#[test]
fn arguments_are_a_usage_error() {
    let output = run(&["--help"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("usage: picomatch"));
}
