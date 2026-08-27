// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The `picomatch` command: JSON requests on standard input, one JSON response per line on standard output.

use crate::js::{json, JsError, JsString, Object, Value};
use crate::options::{request_error, Options};
use crate::parse::ParseState;
use crate::picomatch::{self, Event, MatchValue, Matcher, MatcherResult, TestResult};

/// The usage text, for any command-line argument.
pub const USAGE: &str = "usage: picomatch < requests.jsonl\n\
\n\
Reads one JSON request per line on standard input and writes one JSON response per line.\n\
Requests: {\"op\":\"isMatch\",\"patterns\":P,\"inputs\":[...]}, {\"op\":\"match\",\"patterns\":P,\"inputs\":[...]},\n\
\x20         {\"op\":\"test\",\"glob\":G,\"inputs\":[...]}, {\"op\":\"matchBase\",\"glob\":G,\"input\":S},\n\
\x20         {\"op\":\"scan\",\"input\":S}, {\"op\":\"parse\",\"pattern\":P}, {\"op\":\"toRegex\",\"source\":S,\"inputs\":[...]}\n\
with optional \"options\" and \"events\".\n";

fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut out = Object::new();
    for (key, value) in entries {
        out.set_str(key, value);
    }
    Value::Object(out)
}

fn slice_value(text: &JsString, range: (usize, usize)) -> Value {
    Value::String(JsString::from_units(text.units()[range.0..range.1].to_vec()))
}

fn describe_exec(found: &crate::regexp::Match, text: &JsString) -> Value {
    let captures = found
        .captures
        .iter()
        .map(|capture| capture.map_or(Value::Undefined, |range| slice_value(text, range)))
        .collect();
    let mut entries = vec![
        ("captures", Value::Array(captures)),
        ("index", Value::Number(found.index() as f64)),
    ];
    if !found.names.is_empty() {
        let groups: Object = found
            .names
            .iter()
            .map(|(name, number)| {
                let value = found.captures[*number].map_or(Value::Undefined, |range| slice_value(text, range));
                (name.clone(), value)
            })
            .collect();
        entries.push(("groups", Value::Object(groups)));
    }
    object(entries)
}

fn describe_match(found: &MatchValue) -> Value {
    match found {
        MatchValue::Undefined => Value::Undefined,
        MatchValue::Bool(answer) => Value::Bool(*answer),
        MatchValue::Null => Value::Null,
        MatchValue::Exec(result, text) => describe_exec(result, text),
    }
}

fn describe_test(result: &TestResult) -> Value {
    object([
        ("isMatch", Value::Bool(result.is_match)),
        ("output", Value::String(result.output.clone())),
        ("match", describe_match(&result.found)),
    ])
}

fn describe(result: &MatcherResult) -> Value {
    match result {
        MatcherResult::Bool(answer) => Value::Bool(*answer),
        MatcherResult::Detail(detail) => describe_test(detail),
    }
}

fn public_state(state: &ParseState) -> Value {
    let tokens = state
        .tokens
        .iter()
        .map(|token| {
            let kind = ("type", Value::str(token.kind));
            if token.kind == "bracket" {
                object([kind])
            } else {
                object([kind, ("value", Value::String(token.value.clone()))])
            }
        })
        .collect();
    let mut out = Object::new();
    out.set_str("input", Value::String(state.input.clone()));
    out.set_str("index", Value::Number(state.index as f64));
    out.set_str("start", Value::Number(state.start as f64));
    out.set_str("dot", Value::Bool(state.dot));
    out.set_str("consumed", Value::String(state.consumed.clone()));
    out.set_str("prefix", Value::String(state.prefix.clone()));
    out.set_str("backtrack", Value::Bool(state.backtrack));
    out.set_str("negated", Value::Bool(state.negated));
    out.set_str("brackets", Value::Number(state.brackets as f64));
    out.set_str("braces", Value::Number(state.braces as f64));
    out.set_str("parens", Value::Number(state.parens as f64));
    out.set_str("quotes", Value::Number(state.quotes as f64));
    out.set_str("globstar", Value::Bool(state.globstar));
    out.set_str("tokens", Value::Array(tokens));
    if state.negated_extglob {
        out.set_str("negatedExtglob", Value::Bool(true));
    }
    Value::Object(out)
}

fn event_values(events: Vec<Event>) -> Value {
    let rows = events.into_iter().map(|event| {
        Value::Array(vec![
            Value::str(event.kind),
            Value::String(event.input),
            Value::Bool(event.is_match),
            Value::String(event.output),
        ])
    });
    Value::Array(rows.collect())
}

fn member(request: &Object, name: &str) -> Value {
    request.get_str(name).cloned().unwrap_or_default()
}

fn string_member(request: &Object, name: &str) -> Result<JsString, JsError> {
    match request.get_str(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(request_error(format!("{name} must be a string"))),
    }
}

fn input_list(request: &Object) -> Result<Vec<Value>, JsError> {
    match request.get_str("inputs") {
        Some(Value::Array(items)) => Ok(items.clone()),
        _ => Err(request_error("inputs must be an array")),
    }
}

fn build_options(request: &Object, events: bool) -> Result<Option<Options>, JsError> {
    let spec = member(request, "options");
    if spec.is_undefined() && !events {
        return Ok(None);
    }
    let mut options = match &spec {
        Value::Undefined => Options::default(),
        Value::Object(object) => Options::from_request(object)?,
        _ => return Err(request_error("options must be an object")),
    };
    options.events = events;
    Ok(Some(options))
}

/// The results of a matcher on the inputs, with the events the callbacks logged when `events` is on.
fn run_matcher(
    request: &Object,
    options: Option<&Options>,
    events: bool,
    describe_result: fn(&MatcherResult) -> Value,
) -> Result<Value, JsError> {
    let inputs = input_list(request)?;
    let mut matcher = Matcher::new(&member(request, "patterns"), options)?;
    let mut log = Vec::new();
    let mut results = Vec::with_capacity(inputs.len());
    for input in &inputs {
        results.push(describe_result(&matcher.call(input, &mut log)?));
    }
    let mut entries = vec![("results", Value::Array(results))];
    if events {
        entries.push(("events", event_values(log)));
    }
    Ok(object(entries))
}

fn run_test(request: &Object, options: &Options) -> Result<Value, JsError> {
    let inputs = input_list(request)?;
    let glob = member(request, "glob");
    let mut regex = picomatch::make_re(&glob, options)?;
    let mut results = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let result = picomatch::test(input, &mut regex, options, glob.as_string(), options.truthy("windows"))?;
        results.push(describe_test(&result));
    }
    Ok(object([("results", Value::Array(results))]))
}

fn run_parse(request: &Object, options: &Options) -> Result<Value, JsError> {
    let pattern = member(request, "pattern");
    if let Value::Array(items) = &pattern {
        let states: Result<Vec<Value>, JsError> = items
            .iter()
            .map(|item| Ok(public_state(&picomatch::parse(item, options)?)))
            .collect();
        return Ok(object([("states", Value::Array(states?))]));
    }
    Ok(public_state(&picomatch::parse(&pattern, options)?))
}

fn run_to_regex(request: &Object, options: &Options) -> Result<Value, JsError> {
    let inputs = input_list(request)?;
    let source = string_member(request, "source")?;
    let mut regex = picomatch::to_regex(&source, options);
    let mut results = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let Value::String(text) = input else {
            return Err(request_error("input must be a string"));
        };
        results.push(
            regex
                .exec(text.units())
                .map_or(Value::Null, |found| describe_exec(&found, text)),
        );
    }
    Ok(object([("results", Value::Array(results))]))
}

fn handle(request: &Value) -> Result<Value, JsError> {
    let Value::Object(request) = request else {
        return Err(request_error("a request must be a JSON object"));
    };
    let events = member(request, "events").truthy();
    let options = build_options(request, events)?;
    let defaults = Options::default();
    let opts = options.as_ref().unwrap_or(&defaults);
    let op = member(request, "op");
    match op.as_string().map(JsString::to_string_lossy).as_deref() {
        Some("isMatch") => run_matcher(request, options.as_ref(), events, |result| {
            Value::Bool(result.is_match())
        }),
        Some("match") => run_matcher(request, options.as_ref(), events, describe),
        Some("test") => run_test(request, opts),
        Some("matchBase") => {
            let input = string_member(request, "input")?;
            let answer = picomatch::match_base(&input, &member(request, "glob"), opts)?;
            Ok(object([("result", Value::Bool(answer))]))
        }
        Some("scan") => Ok(picomatch::scan(&string_member(request, "input")?, opts)),
        Some("parse") => run_parse(request, opts),
        Some("toRegex") => run_to_regex(request, opts),
        _ => {
            let shown = json::stringify(&op).unwrap_or_else(|| JsString::from("undefined"));
            let mut message = JsString::from("unknown op: ");
            message.push_js(&shown);
            Err(request_error(message))
        }
    }
}

/// The response line of one request line.
fn respond(line: &[u16]) -> (JsString, bool) {
    let handled = json::parse(line).and_then(|request| handle(&request));
    let failed = handled.is_err();
    let response = handled.unwrap_or_else(|error| {
        object([(
            "error",
            object([
                ("name", Value::str(&error.name)),
                ("message", Value::String(error.message)),
            ]),
        )])
    });
    (json::stringify(&response).unwrap_or_default(), failed)
}

/// Answers all of standard input. Returns the bytes for standard output and the exit status.
pub fn run(input: &[u8]) -> (Vec<u8>, u8) {
    let text = String::from_utf8_lossy(input);
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut output = String::new();
    let mut failed = false;
    for line in units.split(|&unit| unit == u16::from(b'\n')) {
        if line.is_empty() {
            continue;
        }
        let (response, line_failed) = respond(line);
        failed |= line_failed;
        output.push_str(&response.to_utf8());
        output.push('\n');
    }
    (output.into_bytes(), u8::from(failed))
}
