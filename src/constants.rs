// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Regular expression fragments and limits, for POSIX paths and for Windows paths.

/// The longest pattern the parser takes.
pub const MAX_LENGTH: f64 = 65536.0;

/// The recursion limit of repeated extglobs when the option does not set one.
pub const DEFAULT_MAX_EXTGLOB_RECURSION: f64 = 0.0;

/// The fragments the parser builds regular expressions from.
#[derive(Clone, Copy, Debug)]
pub struct GlobChars {
    /// A literal dot.
    pub dot_literal: &'static str,
    /// A literal plus sign.
    pub plus_literal: &'static str,
    /// A path separator.
    pub slash_literal: &'static str,
    /// At least one character follows.
    pub one_char: &'static str,
    /// Any character but a path separator.
    pub qmark: &'static str,
    /// `.` or `..` as a whole path segment.
    pub dots_slash: &'static str,
    /// Not a leading dot.
    pub no_dot: &'static str,
    /// Not a `.` or `..` segment, anchored at a segment start.
    pub no_dots: &'static str,
    /// Not an empty or `.` segment.
    pub no_dot_slash: &'static str,
    /// Not a `.` or `..` segment.
    pub no_dots_slash: &'static str,
    /// Any character but a dot or a path separator.
    pub qmark_no_dot: &'static str,
    /// Any run of characters within a path segment.
    pub star: &'static str,
    /// The start of a path segment.
    pub start_anchor: &'static str,
}

const POSIX_CHARS: GlobChars = GlobChars {
    dot_literal: "\\.",
    plus_literal: "\\+",
    slash_literal: "\\/",
    one_char: "(?=.)",
    qmark: "[^/]",
    dots_slash: "\\.{1,2}(?:\\/|$)",
    no_dot: "(?!\\.)",
    no_dots: "(?!(?:^|\\/)\\.{1,2}(?:\\/|$))",
    no_dot_slash: "(?!\\.{0,1}(?:\\/|$))",
    no_dots_slash: "(?!\\.{1,2}(?:\\/|$))",
    qmark_no_dot: "[^.\\/]",
    star: "[^/]*?",
    start_anchor: "(?:^|\\/)",
};

const WINDOWS_CHARS: GlobChars = GlobChars {
    dot_literal: "\\.",
    plus_literal: "\\+",
    slash_literal: "[\\\\/]",
    one_char: "(?=.)",
    qmark: "[^\\\\/]",
    dots_slash: "\\.{1,2}(?:[\\\\/]|$)",
    no_dot: "(?!\\.)",
    no_dots: "(?!(?:^|[\\\\/])\\.{1,2}(?:[\\\\/]|$))",
    no_dot_slash: "(?!\\.{0,1}(?:[\\\\/]|$))",
    no_dots_slash: "(?!\\.{1,2}(?:[\\\\/]|$))",
    qmark_no_dot: "[^.\\\\/]",
    star: "[^\\\\/]*?",
    start_anchor: "(?:^|[\\\\/])",
};

/// The fragments for the platform the `windows` option selects.
pub fn glob_chars(windows: bool) -> &'static GlobChars {
    if windows {
        &WINDOWS_CHARS
    } else {
        &POSIX_CHARS
    }
}

/// The class body of a POSIX bracket name.
pub fn posix_regex_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "alnum" => "a-zA-Z0-9",
        "alpha" => "a-zA-Z",
        "ascii" => "\\x00-\\x7F",
        "blank" => " \\t",
        "cntrl" => "\\x00-\\x1F\\x7F",
        "digit" => "0-9",
        "graph" => "\\x21-\\x7E",
        "lower" => "a-z",
        "print" => "\\x20-\\x7E ",
        "punct" => "\\-!\"#$%&'()\\*+,./:;<=>?@[\\\\\\]^_`{|}~",
        "space" => " \\t\\r\\n\\v\\f",
        "upper" => "A-Z",
        "word" => "A-Za-z0-9_",
        "xdigit" => "A-Fa-f0-9",
        _ => return None,
    })
}

/// The pattern an equivalent, simpler glob stands for.
pub fn replacement(input: &str) -> Option<&'static str> {
    match input {
        "***" => Some("*"),
        "**/**" | "**/**/**" => Some("**"),
        _ => None,
    }
}

/// The extglobs that compile to their own group; `@(...)` is a plain group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtglobKind {
    /// `!(...)`.
    Negate,
    /// `?(...)`.
    Qmark,
    /// `+(...)`.
    Plus,
    /// `*(...)`.
    Star,
}

impl ExtglobKind {
    /// The opening fragment.
    pub fn open(self) -> &'static str {
        match self {
            ExtglobKind::Negate => "(?:(?!(?:",
            _ => "(?:",
        }
    }

    /// The closing fragment.
    pub fn close(self, chars: &GlobChars) -> String {
        match self {
            ExtglobKind::Negate => format!(")){})", chars.star),
            ExtglobKind::Qmark => String::from(")?"),
            ExtglobKind::Plus => String::from(")+"),
            ExtglobKind::Star => String::from(")*"),
        }
    }

    /// The token type of the extglob's prefix character.
    pub fn token_type(self) -> &'static str {
        match self {
            ExtglobKind::Negate => "negate",
            ExtglobKind::Qmark => "qmark",
            ExtglobKind::Plus => "plus",
            ExtglobKind::Star => "star",
        }
    }
}
