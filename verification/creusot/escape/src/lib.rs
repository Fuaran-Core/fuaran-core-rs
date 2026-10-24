//! The canonical escape (WIRE_FORMAT §2 rule 6), under Creusot.
//!
//! The reference's proof of the canonical form (`proofs/WireCanon.fst` in the Core
//! repository) models the escape as three functions over an abstract alphabet:
//! `esc_ch` (one character's escape), `escape` (every character's, in order) and
//! `quoted` (the escape between two quotes). The logic functions below state the
//! same three over UTF-8 bytes, clause for clause, and the spine's own
//! `escape_bytes` (`src/json/escape.rs`, the loop `json::escape_string` runs over
//! the UTF-8 encoding of its argument) is proved to return exactly `quoted` of its
//! input.
//!
//! The proof is about the crate's function, not a copy of it: [`spine_escape`] is
//! that file, compiled here as a module of this crate, where `cfg(creusot)` attaches
//! its specification and its loop's proof steps.
//!
//! Bytes rather than characters because rule 6 escapes only ASCII (`"`, `\` and
//! U+0000–U+001F), each a single UTF-8 byte that never continues a sequence, so the
//! escape of a string's bytes IS the UTF-8 encoding of the escape of its characters.
//! See `../../README.md` for what this proves, at what cost, and what it does not.

extern crate alloc;
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

/// The spine's escape, exactly as the crate compiles it (`src/json/escape.rs`). Its
/// `escape_bytes` carries `#[ensures(result@ == quoted(s@))]` under `cfg(creusot)`.
#[path = "../../../../src/json/escape.rs"]
#[allow(dead_code)]
mod spine_escape;
