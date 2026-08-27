// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! The parts of the JavaScript runtime the library relies on: UTF-16 strings, values, number printing, `JSON` and
//! thrown errors.

pub mod error;
pub mod json;
pub mod number;
pub mod string;
pub mod value;

pub use error::JsError;
pub use string::JsString;
pub use value::{Object, Value};
