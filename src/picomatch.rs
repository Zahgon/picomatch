// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The public matching API: matchers built from globs, `test`, `matchBase`, `makeRe` and `toRegex`.

use crate::js::{JsError, JsString, Value};
use crate::options::{Format, Options};
use crate::parse::{self, ParseState};
use crate::regexp::{Match, RegExp};
use crate::scan;
use crate::utils;

/// What a match attempt leaves in the result's `match` member.
#[derive(Clone, Debug)]
pub enum MatchValue {
    /// The attempt never got to compare (an empty input).
    Undefined,
    /// A plain answer, without `regex.exec`.
    Bool(bool),
    /// `regex.exec` found nothing.
    Null,
    /// `regex.exec` found a match in the given text.
    Exec(Match, JsString),
}

impl MatchValue {
    fn is_match(&self) -> bool {
        matches!(self, MatchValue::Bool(true) | MatchValue::Exec(..))
    }
}

/// The result of `picomatch.test`.
#[derive(Clone, Debug)]
pub struct TestResult {
    /// Whether the input matched.
    pub is_match: bool,
    /// The input after `format`.
    pub output: JsString,
    /// The match.
    pub found: MatchValue,
}

/// The result of calling a matcher with `returnObject`.
#[derive(Clone, Debug)]
pub enum MatcherResult {
    /// A matcher over several patterns answers a boolean.
    Bool(bool),
    /// A matcher over one pattern answers the test result, with ignores applied.
    Detail(TestResult),
}

impl MatcherResult {
    /// Whether the input matched.
    pub fn is_match(&self) -> bool {
        match self {
            MatcherResult::Bool(answer) => *answer,
            MatcherResult::Detail(detail) => detail.is_match,
        }
    }
}

/// One call of `onResult`, `onIgnore` or `onMatch`.
#[derive(Clone, Debug)]
pub struct Event {
    /// `result`, `ignore` or `match`.
    pub kind: &'static str,
    /// The input.
    pub input: JsString,
    /// `isMatch` at the time of the call.
    pub is_match: bool,
    /// The input after `format`.
    pub output: JsString,
}

/// A matcher for one pattern.
#[derive(Debug)]
pub struct SingleMatcher {
    glob: JsString,
    regex: RegExp,
    options: Option<Options>,
    posix: bool,
    ignored: Option<Box<Matcher>>,
}

/// The function `picomatch(glob, options)` returns.
#[derive(Debug)]
pub enum Matcher {
    /// A string pattern.
    Single(SingleMatcher),
    /// An array of patterns: the first one that matches wins.
    Any(Vec<Matcher>),
}

impl Matcher {
    /// Builds a matcher from a pattern value. `options` is `None` when the caller gave none.
    pub fn new(glob: &Value, options: Option<&Options>) -> Result<Matcher, JsError> {
        // The JSON parser bounds how deeply arrays nest, so this recursion ends within the stack.
        if let Value::Array(items) = glob {
            let matchers = items.iter().map(|item| Matcher::new(item, options));
            return matchers.collect::<Result<Vec<_>, _>>().map(Matcher::Any);
        }

        let glob = glob
            .as_string()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| JsError::type_error("Expected pattern to be a non-empty string"))?;

        let defaults = Options::default();
        let opts = options.unwrap_or(&defaults);
        let regex = make_re(&Value::String(glob.clone()), opts)?;

        let ignored = if opts.truthy("ignore") {
            let mut ignore_options = opts.clone();
            ignore_options.set("ignore", Value::Null);
            ignore_options.events = false;
            Some(Box::new(Matcher::new(&opts.get("ignore"), Some(&ignore_options))?))
        } else {
            None
        };

        Ok(Matcher::Single(SingleMatcher {
            glob: glob.clone(),
            regex,
            posix: opts.truthy("windows"),
            options: options.cloned(),
            ignored,
        }))
    }

    /// Calls the matcher on an input; callback events go to `events`.
    pub fn call(&mut self, input: &Value, events: &mut Vec<Event>) -> Result<MatcherResult, JsError> {
        match self {
            Matcher::Single(single) => single.call(input, events).map(MatcherResult::Detail),
            Matcher::Any(matchers) => {
                for matcher in matchers {
                    if matcher.call(input, events)?.is_match() {
                        return Ok(MatcherResult::Bool(true));
                    }
                }
                Ok(MatcherResult::Bool(false))
            }
        }
    }
}

impl SingleMatcher {
    fn call(&mut self, input: &Value, events: &mut Vec<Event>) -> Result<TestResult, JsError> {
        let defaults = Options::default();
        let opts = self.options.as_ref().unwrap_or(&defaults);
        let mut result = test(input, &mut self.regex, opts, Some(&self.glob), self.posix)?;
        let log = opts.events;
        let event = |kind, result: &TestResult, input: &JsString| Event {
            kind,
            input: input.clone(),
            is_match: result.is_match,
            output: result.output.clone(),
        };
        let text = input.as_string().cloned().unwrap_or_default();

        if log {
            events.push(event("result", &result, &text));
        }
        if !result.is_match {
            return Ok(result);
        }

        let is_ignored = match &mut self.ignored {
            Some(matcher) => matcher.call(input, events)?.is_match(),
            None => false,
        };
        if is_ignored {
            if log {
                events.push(event("ignore", &result, &text));
            }
            result.is_match = false;
            return Ok(result);
        }

        if log {
            events.push(event("match", &result, &text));
        }
        Ok(result)
    }
}

fn apply_format(format: Format, input: &JsString) -> JsString {
    match format {
        Format::Slashes => {
            let slashes: Vec<u16> = input
                .units()
                .iter()
                .map(|&u| if u == u16::from(b'\\') { u16::from(b'/') } else { u })
                .collect();
            let text = JsString::from_units(slashes);
            if text.starts_with_str("./") {
                text.slice_from(2)
            } else {
                text
            }
        }
        Format::Lower => to_lower_case(input),
    }
}

/// `String.prototype.toLowerCase`: lone surrogates stay as they are.
fn to_lower_case(input: &JsString) -> JsString {
    let mut out = Vec::with_capacity(input.len());
    let mut run = String::new();
    for decoded in char::decode_utf16(input.units().iter().copied()) {
        match decoded {
            Ok(c) => run.push(c),
            Err(error) => {
                out.extend(run.to_lowercase().encode_utf16());
                run.clear();
                out.push(error.unpaired_surrogate());
            }
        }
    }
    out.extend(run.to_lowercase().encode_utf16());
    JsString::from_units(out)
}

/// `picomatch.test(input, regex, options, { glob, posix })`.
pub fn test(
    input: &Value,
    regex: &mut RegExp,
    options: &Options,
    glob: Option<&JsString>,
    posix: bool,
) -> Result<TestResult, JsError> {
    let Some(input) = input.as_string() else {
        return Err(JsError::type_error("Expected input to be a string"));
    };

    if input.is_empty() {
        return Ok(TestResult {
            is_match: false,
            output: JsString::new(),
            found: MatchValue::Undefined,
        });
    }

    let format = |text: &JsString| match options.format {
        Some(format) => Some(apply_format(format, text)),
        None if posix => Some(utils::to_posix_slashes(text.units())),
        None => None,
    };

    let mut matched = glob == Some(input);
    let mut output = if matched {
        format(input).unwrap_or_else(|| input.clone())
    } else {
        input.clone()
    };

    if !matched {
        output = format(input).unwrap_or_else(|| input.clone());
        matched = glob == Some(&output);
    }

    let mut found = MatchValue::Bool(matched);
    if !matched || options.is_true("capture") {
        found = if options.is_true("matchBase") || options.is_true("basename") {
            MatchValue::Bool(match_base_with(input, regex, posix))
        } else {
            match regex.exec(output.units()) {
                Some(result) => MatchValue::Exec(result, output.clone()),
                None => MatchValue::Null,
            }
        };
    }

    Ok(TestResult {
        is_match: found.is_match(),
        output,
        found,
    })
}

fn match_base_with(input: &JsString, regex: &mut RegExp, posix: bool) -> bool {
    let name = utils::basename(input.units(), posix).to_js_string();
    regex.exec(name.units()).is_some()
}

/// `picomatch.matchBase(input, glob, options)`.
pub fn match_base(input: &JsString, glob: &Value, options: &Options) -> Result<bool, JsError> {
    let mut regex = make_re(glob, options)?;
    Ok(match_base_with(input, &mut regex, options.truthy("windows")))
}

/// `picomatch.scan(input, options)`.
pub fn scan(input: &JsString, options: &Options) -> Value {
    Value::Object(scan::scan(input, options))
}

/// `picomatch.parse(pattern, options)` for one pattern.
pub fn parse(pattern: &Value, options: &Options) -> Result<ParseState, JsError> {
    let mut options = options.clone();
    options.set("fastpaths", Value::Bool(false));
    parse::parse(pattern, &options)
}

/// `picomatch.makeRe(input, options)`.
pub fn make_re(input: &Value, options: &Options) -> Result<RegExp, JsError> {
    let input = input
        .as_string()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| JsError::type_error("Expected a non-empty string"))?;

    let first = input.unit_at(0);
    let fast = if !options.is_false("fastpaths") && (first == Some(u16::from(b'.')) || first == Some(u16::from(b'*'))) {
        parse::fastpaths(input, options)?
    } else {
        None
    };

    let (output, negated) = match fast.filter(|source| !source.is_empty()) {
        Some(output) => (output, false),
        None => {
            let state = parse::parse_str(input, options)?;
            (state.output, state.negated)
        }
    };
    Ok(compile_re(&output, negated, options))
}

fn compile_re(output: &JsString, negated: bool, options: &Options) -> RegExp {
    let contains = options.truthy("contains");
    let mut source = JsString::from(if contains { "" } else { "^" });
    source.push_str("(?:");
    source.push_js(output);
    source.push_str(")");
    if !contains {
        source.push_str("$");
    }
    if negated {
        let mut wrapped = JsString::from("^(?!");
        wrapped.push_js(&source);
        wrapped.push_str(").*$");
        source = wrapped;
    }
    to_regex(&source, options)
}

/// `picomatch.toRegex(source, options)`: an invalid source or invalid flags give `/$^/`.
pub fn to_regex(source: &JsString, options: &Options) -> RegExp {
    let flags = if options.truthy("flags") {
        options.get("flags").to_js_string()
    } else if options.truthy("nocase") {
        JsString::from("i")
    } else {
        JsString::new()
    };
    RegExp::new(source.units(), flags.units()).unwrap_or_else(|_| RegExp::never())
}
