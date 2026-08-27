// SPDX-License-Identifier: MIT
// Copyright (c) 2017-present, Jon Schlinkert.

//! JavaScript number conversions: `Number.prototype.toString()`, `parseInt` and `ToNumber`.

/// `Number::toString(x)` in radix 10: the shortest decimal that round-trips, in fixed notation for exponents from
/// -7 to 20 and in exponential notation (`1e+21`, `1.5e-7`) otherwise. Negative zero is `"0"`.
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, exponent) = shortest_digits(value.abs());
    let k = digits.len() as i32;
    // The decimal point sits after `n` digits: value = 0.digits * 10^n.
    let n = exponent + 1;
    let body = if k <= n && n <= 21 {
        let mut s = digits.clone();
        s.extend(std::iter::repeat_n('0', (n - k) as usize));
        s
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        format!("{int}.{frac}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let exp = if e >= 0 { format!("e+{e}") } else { format!("e-{}", -e) };
        if k == 1 {
            format!("{digits}{exp}")
        } else {
            let (first, rest) = digits.split_at(1);
            format!("{first}.{rest}{exp}")
        }
    };
    format!("{sign}{body}")
}

/// The shortest round-trip decimal digits of a positive finite number and the exponent of its first digit. When the
/// number lies exactly halfway between two shortest candidates, the even one is chosen, as the specification of
/// `Number::toString` requires.
fn shortest_digits(value: f64) -> (String, i32) {
    let (digits, exponent) = split_exponential(&format!("{value:e}"));
    match even_tie_digits(value, &digits, exponent) {
        Some(even) => (even, exponent),
        None => (digits, exponent),
    }
}

/// The significant digits (trailing zeros removed) and the exponent of a number formatted with `{:e}`.
fn split_exponential(formatted: &str) -> (String, i32) {
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((formatted, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    (digits.to_string(), exponent.parse().unwrap_or(0))
}

/// The even neighbour of `digits` when `value` is exactly the midpoint between `digits` and that neighbour and the
/// neighbour round-trips too. Ties need more decimal digits than a double has bits of spacing, so only long digit
/// strings are checked.
fn even_tie_digits(value: f64, digits: &str, exponent: i32) -> Option<String> {
    let last = digits.bytes().last()? - b'0';
    if digits.len() < 15 || last.is_multiple_of(2) {
        return None;
    }
    // Every finite double has a terminating decimal expansion of at most 767 significant digits.
    let (exact, exact_exponent) = split_exponential(&format!("{value:.1100e}"));
    if exact_exponent != exponent || exact.len() != digits.len() + 1 || !exact.ends_with('5') {
        return None;
    }
    let low = &exact[..digits.len()];
    let candidate = if low == digits {
        increment(low)?
    } else {
        low.to_string()
    };
    if candidate.len() != digits.len() || !(candidate.as_bytes()[candidate.len() - 1] - b'0').is_multiple_of(2) {
        return None;
    }
    let text = format!("{}.{}e{exponent}", &candidate[..1], &candidate[1..]);
    (text.parse::<f64>() == Ok(value)).then_some(candidate)
}

/// A decimal digit string plus one in its last place, or `None` when that carries into a new digit.
fn increment(digits: &str) -> Option<String> {
    let mut bytes = digits.as_bytes().to_vec();
    for byte in bytes.iter_mut().rev() {
        if *byte == b'9' {
            *byte = b'0';
        } else {
            *byte += 1;
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::number_to_string;

    #[test]
    fn formats_like_javascript() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.5, "1.5"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (5e-324, "5e-324"),
        ];
        for (value, text) in cases {
            assert_eq!(number_to_string(value), text);
        }
    }
}
