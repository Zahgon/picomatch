// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Options as the library reads them. Most are handed over from JSON untouched, because the library tests some with
//! `=== true` and others for truthiness. The two function-valued options are chosen by name.

use crate::js::{JsError, JsString, Object, Value};

/// `options.format`: how an input is normalised before matching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Backslashes to slashes, then one leading `./` removed.
    Slashes,
    /// `toLowerCase`.
    Lower,
}

/// `options.expandRange`: how a brace range becomes regular expression source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpandRange {
    /// `([a-b])`.
    Group,
    /// `(a|b)`.
    Alternate,
}

impl ExpandRange {
    /// Calls the function with the range's end points. The options object is passed after them, so it takes the
    /// place of a missing end point, and whatever is still missing reads as `undefined`.
    pub fn expand(self, args: &[JsString]) -> JsString {
        let options = JsString::from("[object Object]");
        let undefined = JsString::from("undefined");
        let (a, b) = match args {
            [] => (&options, &undefined),
            [only] => (only, &options),
            [a, b, ..] => (a, b),
        };
        let (open, middle, close) = match self {
            ExpandRange::Group => ("([", "-", "])"),
            ExpandRange::Alternate => ("(", "|", ")"),
        };
        let mut out = JsString::from(open);
        out.push_js(a);
        out.push_str(middle);
        out.push_js(b);
        out.push_str(close);
        out
    }
}

/// The options of one call.
#[derive(Clone, Debug, Default)]
pub struct Options {
    values: Object,
    /// `format`.
    pub format: Option<Format>,
    /// `expandRange`.
    pub expand_range: Option<ExpandRange>,
    /// Whether the event callbacks are set.
    pub events: bool,
}

const PLAIN_OPTIONS: [&str; 31] = [
    "basename",
    "bash",
    "capture",
    "contains",
    "dot",
    "fastpaths",
    "flags",
    "ignore",
    "keepQuotes",
    "literalBrackets",
    "matchBase",
    "maxExtglobRecursion",
    "maxLength",
    "nobrace",
    "nobracket",
    "nocase",
    "noext",
    "noextglob",
    "noglobstar",
    "nonegate",
    "noparen",
    "parts",
    "posix",
    "prepend",
    "regex",
    "scanToEnd",
    "strictBrackets",
    "strictSlashes",
    "tokens",
    "unescape",
    "windows",
];

impl Options {
    /// Options from a request's `options` member. Members the command does not know are dropped.
    pub fn from_request(spec: &Object) -> Result<Options, JsError> {
        let mut options = Options::default();
        for key in PLAIN_OPTIONS {
            if let Some(value) = spec.get_str(key) {
                options.values.set_str(key, value.clone());
            }
        }
        if let Some(format) = spec.get_str("format") {
            options.format = Some(match format.to_js_string() {
                name if name.eq_str("slashes") => Format::Slashes,
                name if name.eq_str("lower") => Format::Lower,
                _ => return Err(request_error("format must be \"slashes\" or \"lower\"")),
            });
        }
        if let Some(expand) = spec.get_str("expandRange") {
            options.expand_range = Some(match expand.to_js_string() {
                name if name.eq_str("group") => ExpandRange::Group,
                name if name.eq_str("alternate") => ExpandRange::Alternate,
                _ => return Err(request_error("expandRange must be \"group\" or \"alternate\"")),
            });
        }
        Ok(options)
    }

    /// The value of an option; `undefined` when it is absent.
    pub fn get(&self, key: &str) -> Value {
        self.values.get_str(key).cloned().unwrap_or_default()
    }

    /// Sets an option.
    pub fn set(&mut self, key: &str, value: Value) {
        self.values.set_str(key, value);
    }

    /// `if (options[key])`.
    pub fn truthy(&self, key: &str) -> bool {
        self.values.get_str(key).is_some_and(Value::truthy)
    }

    /// `options[key] === true`.
    pub fn is_true(&self, key: &str) -> bool {
        self.values.get_str(key).is_some_and(Value::is_true)
    }

    /// `options[key] === false`.
    pub fn is_false(&self, key: &str) -> bool {
        matches!(self.values.get_str(key), Some(Value::Bool(false)))
    }
}

/// An error of the command's front end.
pub fn request_error(message: impl Into<JsString>) -> JsError {
    JsError::new("RequestError", message)
}
