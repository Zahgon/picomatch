// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::thread;

use picomatch::cli;

/// Deeply nested patterns recurse in the regular expression parser and compiler. The parsers refuse
/// values nested past their `MAX_NESTING` with an error, and this stack holds that depth.
const STACK_BYTES: usize = 512 << 20;

fn serve() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprint!("{}", cli::USAGE);
        return ExitCode::from(2);
    }
    let mut input = Vec::new();
    if let Err(error) = io::stdin().read_to_end(&mut input) {
        eprintln!("picomatch: cannot read standard input: {error}");
        return ExitCode::from(1);
    }
    let (output, status) = cli::run(&input);
    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout.write_all(&output).and_then(|()| stdout.flush()) {
        eprintln!("picomatch: cannot write standard output: {error}");
        return ExitCode::from(1);
    }
    ExitCode::from(status)
}

fn main() -> ExitCode {
    let worker = thread::Builder::new().stack_size(STACK_BYTES).spawn(serve);
    match worker.map(thread::JoinHandle::join) {
        Ok(Ok(status)) => status,
        Ok(Err(panic)) => {
            let reason = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown cause");
            eprintln!("picomatch: internal error: {reason}");
            ExitCode::from(70)
        }
        Err(error) => {
            eprintln!("picomatch: cannot start: {error}");
            ExitCode::from(70)
        }
    }
}
