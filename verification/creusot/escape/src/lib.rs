//! The canonical escape (WIRE_FORMAT §2 rule 6), under Creusot.
//!
//! The reference's proof of the canonical form (`proofs/WireCanon.fst` in the Core
//! repository) models the escape as three functions over an abstract alphabet:
//! `esc_ch` (one character's escape), `escape` (every character's, in order) and
//! `quoted` (the escape between two quotes). The logic functions below state the
//! same three over UTF-8 bytes, clause for clause, and [`escape_bytes`] — the
//! spine's `json::escape_string`, byte for byte, over the UTF-8 encoding of its
//! argument — is proved to return exactly `quoted` of its input.
//!
//! Bytes rather than characters because rule 6 escapes only ASCII (`"`, `\` and
//! U+0000–U+001F), each a single UTF-8 byte that never continues a sequence, so the
//! escape of a string's bytes IS the UTF-8 encoding of the escape of its characters.
//! See `../../README.md` for what this proves, at what cost, and what it does not.

extern crate creusot_std;

use creusot_std::prelude::*;

/// The lower-case hex digit of a nibble, as its ASCII byte (F*: `hexd_str`).
#[logic(open)]
pub fn hex_lo(n: Int) -> u8 {
    if n == 0 {
        0x30u8
    } else if n == 1 {
        0x31u8
    } else if n == 2 {
        0x32u8
    } else if n == 3 {
        0x33u8
    } else if n == 4 {
        0x34u8
    } else if n == 5 {
        0x35u8
    } else if n == 6 {
        0x36u8
    } else if n == 7 {
        0x37u8
    } else if n == 8 {
        0x38u8
    } else if n == 9 {
        0x39u8
    } else if n == 10 {
        0x61u8
    } else if n == 11 {
        0x62u8
    } else if n == 12 {
        0x63u8
    } else if n == 13 {
        0x64u8
    } else if n == 14 {
        0x65u8
    } else {
        0x66u8
    }
}

/// One byte's escape (F*: `esc_ch`). Rule 6 and only rule 6: `"` and `\` escaped,
/// a control character as `\u00` + its two lower-case hex nibbles, everything else
/// — `/` and every non-ASCII byte among it — verbatim. Each clause is the F* list
/// written front to back, as a singleton extended by `push_back`.
#[logic(open)]
pub fn esc_ch(b: u8) -> Seq<u8> {
    pearlite! {
        if b@ == 0x22 {
            Seq::singleton(0x5Cu8).push_back(0x22u8)
        } else if b@ == 0x5C {
            Seq::singleton(0x5Cu8).push_back(0x5Cu8)
        } else if b@ < 0x20 {
            Seq::singleton(0x5Cu8)
                .push_back(0x75u8)
                .push_back(0x30u8)
                .push_back(0x30u8)
                .push_back(hex_lo(b@ / 16))
                .push_back(hex_lo(b@ % 16))
        } else {
            Seq::singleton(b)
        }
    }
}

/// Every byte's escape, in order (F*: `escape`, clause for clause: the empty
/// sequence escapes to itself, and `c :: t` to `esc_ch c` followed by `escape t`).
#[logic(open)]
#[variant(s.len())]
pub fn escape(s: Seq<u8>) -> Seq<u8> {
    pearlite! {
        if s.len() == 0 {
            Seq::empty()
        } else {
            esc_ch(s[0]).concat(escape(s.tail()))
        }
    }
}

/// The step the writer's loop takes: escaping one more byte appends that byte's
/// escape. By induction on `s`, as the reference's `read_str_inverts_escape` is.
#[logic(open)]
#[variant(s.len())]
#[ensures(escape(s.push_back(b)) == escape(s).concat(esc_ch(b)))]
pub fn escape_push_back(s: Seq<u8>, b: u8) {
    if s.len() == 0 {
        proof_assert! { s.push_back(b)[0] == b };
        proof_assert! { s.push_back(b).tail().ext_eq(Seq::empty()) };
        proof_assert! { escape(s.push_back(b)) == esc_ch(b).concat(escape(Seq::empty())) };
    } else {
        escape_push_back(s.tail(), b);
        proof_assert! { s.push_back(b)[0] == s[0] };
        proof_assert! { s.push_back(b).tail().ext_eq(s.tail().push_back(b)) };
        proof_assert! { escape(s.push_back(b)) == esc_ch(s[0]).concat(escape(s.tail().push_back(b))) };
        proof_assert! { escape(s) == esc_ch(s[0]).concat(escape(s.tail())) };
    }
}

/// Three facts about `concat` and `push_back` the writer's bookkeeping rewrites
/// with, stated once for the solver: appending to the right operand appends to
/// the whole; a singleton on the right is a `push_back`; the empty sequence is a
/// right unit.
#[logic(open)]
#[ensures(forall<a: Seq<T>, c: Seq<T>, x: T> a.concat(c.push_back(x)) == a.concat(c).push_back(x))]
#[ensures(forall<a: Seq<T>, x: T> a.concat(Seq::singleton(x)) == a.push_back(x))]
#[ensures(forall<a: Seq<T>> a.concat(Seq::empty()) == a)]
pub fn concat_laws<T>() {
    proof_assert! { forall<a: Seq<T>, c: Seq<T>, x: T> a.concat(c.push_back(x)).ext_eq(a.concat(c).push_back(x)) };
    proof_assert! { forall<a: Seq<T>, x: T> a.concat(Seq::singleton(x)).ext_eq(a.push_back(x)) };
    proof_assert! { forall<a: Seq<T>> a.concat(Seq::empty()).ext_eq(a) };
}

/// The escape between two quotes (F*: `quoted`).
#[logic(open)]
pub fn quoted(s: Seq<u8>) -> Seq<u8> {
    pearlite! { Seq::singleton(0x22u8).concat(escape(s)).push_back(0x22u8) }
}

/// The ASCII byte of a nibble's lower-case hex digit.
#[requires(n@ < 16)]
#[ensures(result == hex_lo(n@))]
fn hex_digit(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}

/// Quote and escape the UTF-8 bytes of a string the one canonical way. This is
/// `json::escape_string`'s loop, byte for byte, and it returns exactly the
/// reference model's `quoted` of its input.
#[ensures(result@ == quoted(s@))]
pub fn escape_bytes(s: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.push(b'"');
    let _ = snapshot! { concat_laws::<u8>() };
    proof_assert! { out@.ext_eq(Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, 0)))) };
    let mut i: usize = 0;
    #[invariant(i@ <= s@.len())]
    #[invariant(out@ == Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, i@))))]
    while i < s.len() {
        let b = s[i];
        // The step: the escape of one more byte is the escape so far, then that
        // byte's own ([`escape_push_back`], over the prefix read so far).
        let prefix = snapshot! { s@.subsequence(0, i@) };
        let _ = snapshot! { concat_laws::<u8>() };
        let _ = snapshot! { escape_push_back(*prefix, b) };
        proof_assert! { s@.subsequence(0, i@ + 1).ext_eq(prefix.push_back(b)) };
        proof_assert! { escape(s@.subsequence(0, i@ + 1)) == escape(*prefix).concat(esc_ch(b)) };
        // An `if` chain where the spine's writer matches: Creusot 0.9.0 gives a
        // `match` arm's default branch none of the earlier arms' negations, so the
        // last arm could not be shown to be the verbatim case (see ../../README.md).
        if b == b'"' {
            proof_assert! { esc_ch(b) == Seq::singleton(0x5Cu8).push_back(0x22u8) };
            out.push(b'\\');
            out.push(b'"');
        } else if b == b'\\' {
            proof_assert! { esc_ch(b) == Seq::singleton(0x5Cu8).push_back(0x5Cu8) };
            out.push(b'\\');
            out.push(b'\\');
        } else if b < 0x20 {
            proof_assert! {
                esc_ch(b) == Seq::singleton(0x5Cu8)
                    .push_back(0x75u8)
                    .push_back(0x30u8)
                    .push_back(0x30u8)
                    .push_back(hex_lo(b@ / 16))
                    .push_back(hex_lo(b@ % 16))
            };
            out.push(b'\\');
            out.push(b'u');
            out.push(b'0');
            out.push(b'0');
            out.push(hex_digit(b / 16));
            out.push(hex_digit(b % 16));
        } else {
            proof_assert! { esc_ch(b) == Seq::singleton(b) };
            out.push(b);
        }
        proof_assert! { out@ == Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, i@ + 1))) };
        i += 1;
    }
    proof_assert! { s@.subsequence(0, s@.len()).ext_eq(s@) };
    out.push(b'"');
    out
}
