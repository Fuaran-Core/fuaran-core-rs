//! The canonical number form — the `.NET Double.ToString("R", InvariantCulture)`
//! layout every Fuaran wire writes a number in. The twin of the reference
//! `formatFiniteDouble` (`Fuaran.Core.Wire`), so a number a Rust host writes is the
//! number every other host writes.

use crate::prelude::*;
use core::fmt::{self, Write};

/// The canonical number form: finite doubles in the canonical layout, the IEEE
/// specials as quoted sentinel strings, negative zero collapsed to `0`.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        "\"NaN\"".to_string()
    } else if n == f64::INFINITY {
        "\"Infinity\"".to_string()
    } else if n == f64::NEG_INFINITY {
        "\"-Infinity\"".to_string()
    } else {
        format_finite_double(n)
    }
}

/// Renders a finite `f64` in the canonical Fuaran wire form, matching
/// `.NET Double.ToString("R", InvariantCulture)`:
///
/// - shortest round-trip significant digits (Rust's shortest float formatting
///   shares the .NET / V8 / CPython / Go digit sequence — the shortest decimal that
///   round-trips to a given `f64` is unique, so every correct shortest algorithm
///   agrees);
/// - fixed-point layout iff the leading-digit exponent is in `[-4, 16]`;
/// - otherwise scientific with an uppercase `E`, an always-present exponent sign,
///   and a ≥2-digit zero-padded exponent;
/// - negative zero collapses to `"0"`.
///
/// The caller must ensure `f` is finite — `NaN` and `±Inf` are not representable on
/// the wire and are rejected upstream. This mirrors the reference
/// `formatFiniteDouble`; the shared corpus's `metric-float-*` fixtures pin the
/// divergence-zone values.
pub fn format_finite_double(f: f64) -> String {
    let mut out = String::new();
    write_finite_double(f, &mut out).expect("a String accepts every write");
    out
}

/// [`format_finite_double`] written through `out`, building nothing: the
/// shortest digits are rendered into a buffer on the stack and laid out from
/// there. The form the invocation's identity is written in, so a number in it
/// costs no allocation (DECISIONS.md D12).
pub(crate) fn write_finite_double<W: Write + ?Sized>(f: f64, out: &mut W) -> fmt::Result {
    if f == 0.0 {
        return out.write_str("0"); // collapses -0.0 to 0 (−0.0 == 0.0 in IEEE 754)
    }

    // Shortest round-trip digits via the scientific form `d[.ddd]e±XX`; the
    // longest a finite f64 takes is 24 bytes (`-1.2345678901234567e-308`).
    let mut buf = StackText::<32>::new();
    write!(buf, "{f:e}")?;
    let s = buf.as_str();

    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let e_idx = s.find('e').expect("{:e} always emits an 'e'");
    let mantissa = &s[..e_idx];
    let exp: i32 = s[e_idx + 1..].parse().expect("{:e} exponent is an integer");

    let (int_part, frac_part) = match mantissa.find('.') {
        Some(dot) => (&mantissa[..dot], &mantissa[dot + 1..]),
        None => (mantissa, ""),
    };
    // Significant digits; the first digit has place value 10^exp.
    let digits = Digits(int_part, frac_part);

    if neg {
        out.write_char('-')?;
    }
    if (-4..=16).contains(&exp) {
        fixed_point(&digits, exp, out)
    } else {
        scientific(&digits, exp, out)
    }
}

/// The significant digits, held as the two runs `{:e}` writes them in.
struct Digits<'a>(&'a str, &'a str);

impl Digits<'_> {
    fn len(&self) -> usize {
        self.0.len() + self.1.len()
    }

    /// The digits from position `from` to `to`, written.
    fn write<W: Write + ?Sized>(&self, from: usize, to: usize, out: &mut W) -> fmt::Result {
        let n = self.0.len();
        if from < n {
            out.write_str(&self.0[from..to.min(n)])?;
        }
        if to > n {
            out.write_str(&self.1[from.max(n) - n..to - n])?;
        }
        Ok(())
    }
}

fn zeros<W: Write + ?Sized>(n: usize, out: &mut W) -> fmt::Result {
    (0..n).try_for_each(|_| out.write_char('0'))
}

/// Lays significant digits out with a decimal point, where the first digit has
/// place value `10^exp` (`exp` in `[-4, 16]`).
fn fixed_point<W: Write + ?Sized>(digits: &Digits<'_>, exp: i32, out: &mut W) -> fmt::Result {
    let n = digits.len();
    if exp >= 0 {
        let int_len = (exp + 1) as usize;
        if n <= int_len {
            digits.write(0, n, out)?;
            zeros(int_len - n, out)
        } else {
            digits.write(0, int_len, out)?;
            out.write_char('.')?;
            digits.write(int_len, n, out)
        }
    } else {
        // exp < 0: `0.<(-exp-1) zeros><digits>`
        out.write_str("0.")?;
        zeros((-exp - 1) as usize, out)?;
        digits.write(0, n, out)
    }
}

/// Lays significant digits out as `d[.ddd]E±XX` with the .NET exponent conventions
/// (uppercase `E`, mandatory sign, ≥2 zero-padded exponent digits).
fn scientific<W: Write + ?Sized>(digits: &Digits<'_>, exp: i32, out: &mut W) -> fmt::Result {
    let n = digits.len();
    digits.write(0, 1, out)?;
    if n > 1 {
        out.write_char('.')?;
        digits.write(1, n, out)?;
    }
    let (sign, mag) = if exp < 0 { ('-', -exp) } else { ('+', exp) };
    write!(out, "E{sign}{mag:02}") // ≥2 digits, zero-padded
}

/// A text of at most `N` bytes, written on the stack; a write past `N` fails.
struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    fn new() -> Self {
        StackText {
            bytes: [0; N],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("only whole str slices are written")
    }
}

impl<const N: usize> Write for StackText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::format_finite_double;

    /// Pins the canonical number form against the corpus divergence-zone values
    /// (the `metric-float-*` fixtures) plus a spread of fixed-point / scientific
    /// boundary cases.
    #[test]
    fn format_finite_double_matches_corpus() {
        let cases: &[(f64, &str)] = &[
            // Corpus divergence-zone (metric-float-*).
            (1e21, "1E+21"),
            (1e-7, "1E-07"),
            (0.30000000000000004, "0.30000000000000004"),
            (1.2345678901234568e17, "1.2345678901234568E+17"),
            (5e-324, "5E-324"), // smallest positive subnormal
            (-0.0, "0"),        // negative zero collapses
            // Fixed-point range [-4, 16].
            (0.0, "0"),
            (1.0, "1"),
            (100.0, "100"),
            (123.5, "123.5"),
            (0.5, "0.5"),
            (0.0001, "0.0001"), // exp = -4, last fixed-point value
            (-42.25, "-42.25"),
            // Scientific just outside the range.
            (0.00001, "1E-05"), // exp = -5, first scientific value
        ];
        for &(input, want) in cases {
            assert_eq!(
                format_finite_double(input),
                want,
                "format_finite_double({input})"
            );
        }
    }
}
