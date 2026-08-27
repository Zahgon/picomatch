// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Errors as JavaScript reports them: the constructor name and the message.

use std::fmt;

use super::string::JsString;

/// A thrown JavaScript error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError {
    /// The error's `name`: `Error`, `TypeError`, `RangeError`, `SyntaxError`, `URIError`, ...
    pub name: String,
    /// The error's `message`.
    pub message: JsString,
}

impl JsError {
    /// An error with the given name and message.
    pub fn new(name: &str, message: impl Into<JsString>) -> Self {
        JsError {
            name: name.to_string(),
            message: message.into(),
        }
    }

    /// A plain `Error`.
    pub fn error(message: impl Into<JsString>) -> Self {
        JsError::new("Error", message)
    }

    /// A `TypeError`.
    pub fn type_error(message: impl Into<JsString>) -> Self {
        JsError::new("TypeError", message)
    }

    /// `String(error)`, as a template literal shows a caught error: `Name: message`, or just the name when the
    /// message is empty.
    pub fn to_js_string(&self) -> JsString {
        let mut out = JsString::from(self.name.as_str());
        if !self.message.is_empty() {
            out.push_str(": ");
            out.push_js(&self.message);
        }
        out
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_js_string())
    }
}

impl std::error::Error for JsError {}
