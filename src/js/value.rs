// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! JavaScript values as JSON brings them in: requests and options keep JavaScript's types, so that the library's
//! truthiness tests and strict comparisons see what they see in JavaScript.

use super::number::number_to_string;
use super::string::JsString;

/// A JavaScript value.
#[derive(Clone, Debug, Default)]
pub enum Value {
    /// `undefined`.
    #[default]
    Undefined,
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    String(JsString),
    /// An array.
    Array(Vec<Value>),
    /// A plain object.
    Object(Object),
}

/// A plain object: properties in insertion order, enumerated in JavaScript order (array index keys first, in
/// ascending numeric order, then the other keys in insertion order).
#[derive(Clone, Debug, Default)]
pub struct Object {
    entries: Vec<(JsString, Value)>,
}

impl Object {
    /// An empty object.
    pub fn new() -> Self {
        Object::default()
    }

    /// The value of an own property.
    pub fn get(&self, key: &JsString) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The value of an own property named by Rust text.
    pub fn get_str(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k.eq_str(key)).map(|(_, v)| v)
    }

    /// Whether the object owns a property.
    pub fn has_own(&self, key: &str) -> bool {
        self.get_str(key).is_some()
    }

    /// Sets a property; a new key goes after the existing ones, an existing key keeps its place.
    pub fn set(&mut self, key: JsString, value: Value) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Sets a property named by Rust text.
    pub fn set_str(&mut self, key: &str, value: Value) {
        self.set(JsString::from(key), value);
    }

    /// The properties in JavaScript enumeration order.
    pub fn iter(&self) -> impl Iterator<Item = (&JsString, &Value)> {
        let mut indexed: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, (k, _))| k.array_index().map(|n| (n, i)))
            .collect();
        indexed.sort_unstable();
        let named = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (k, _))| k.array_index().is_none())
            .map(|(i, _)| i);
        let order: Vec<usize> = indexed.into_iter().map(|(_, i)| i).chain(named).collect();
        order.into_iter().map(move |i| (&self.entries[i].0, &self.entries[i].1))
    }
}

impl FromIterator<(JsString, Value)> for Object {
    fn from_iter<I: IntoIterator<Item = (JsString, Value)>>(iter: I) -> Self {
        let mut object = Object::new();
        for (key, value) in iter {
            object.set(key, value);
        }
        object
    }
}

impl Value {
    /// A string value from Rust text.
    pub fn str(text: &str) -> Value {
        Value::String(JsString::from(text))
    }

    /// Whether the value is `undefined`.
    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    /// The array, when the value is one.
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The object, when the value is a plain object.
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The string, when the value is one.
    pub fn as_string(&self) -> Option<&JsString> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The number, when the value is one.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(n.is_nan() || *n == 0.0),
            Value::String(s) => !s.is_empty(),
            Value::Array(_) | Value::Object(_) => true,
        }
    }

    /// Whether the value is exactly `true`.
    pub fn is_true(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    /// `String(value)`, as a template literal or string concatenation converts it.
    pub fn to_js_string(&self) -> JsString {
        match self {
            Value::Undefined => JsString::from("undefined"),
            Value::Null => JsString::from("null"),
            Value::Bool(b) => JsString::from(if *b { "true" } else { "false" }),
            Value::Number(n) => JsString::from(number_to_string(*n)),
            Value::String(s) => s.clone(),
            Value::Array(items) => {
                let parts: Vec<JsString> = items
                    .iter()
                    .map(|item| match item {
                        Value::Undefined | Value::Null => JsString::new(),
                        other => other.to_js_string(),
                    })
                    .collect();
                JsString::join(&parts, ",")
            }
            Value::Object(_) => JsString::from("[object Object]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_javascript_order() {
        let object: Object = ["b", "10", "9", "a"]
            .into_iter()
            .map(|k| (JsString::from(k), Value::Null))
            .collect();
        let keys: Vec<String> = object.iter().map(|(k, _)| k.to_string_lossy()).collect();
        assert_eq!(keys, ["9", "10", "b", "a"]);
    }

    #[test]
    fn converts_to_string() {
        let array = Value::Array(vec![Value::Number(1.0), Value::Null, Value::str("x")]);
        assert_eq!(array.to_js_string(), JsString::from("1,,x"));
        assert_eq!(Value::Number(-0.0).to_js_string(), JsString::from("0"));
    }
}
