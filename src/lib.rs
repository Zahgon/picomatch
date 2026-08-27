// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! Blazing fast and accurate glob matching: a Rust port of picomatch.
//!
//! Globs compile to JavaScript regular expression source, exactly as in the original, and [`regexp`] runs that source
//! with JavaScript's semantics. Strings are UTF-16, as they are in the original, because lengths, indices and the
//! `?` wildcard all count code units.

pub mod cli;
pub mod constants;
pub mod extglob;
pub mod js;
pub mod options;
pub mod parse;
pub mod picomatch;
pub mod regexp;
pub mod scan;
pub mod utils;
