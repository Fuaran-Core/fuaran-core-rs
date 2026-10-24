//! The canonical escape (WIRE_FORMAT §2 rule 6), over the UTF-8 bytes of a string.
//!
//! [`escape_bytes`] is the loop [`super::escape_string`] runs, and it is the function
//! the Creusot proof under `verification/creusot/escape/` checks: that crate compiles
//! THIS file as one of its own modules, so the proof is about these lines and no copy
//! of them. It proves that `escape_bytes(s)` is exactly `quoted(s)`, where `quoted`,
//! `escape` and `esc_ch` are logic functions written clause for clause from the
//! reference proof's (`proofs/WireCanon.fst` in the Core repository).
//!
//! The specification is attached under `cfg(creusot)`, which only the Creusot build
//! sets: in every other build of the crate each `cfg_attr` and `cfg` below is absent,
//! so the crate takes no dependency and the functions compile as written. Bytes
//! rather than characters because rule 6 escapes only ASCII (`"`, `\` and
//! U+0000–U+001F), each one UTF-8 byte that never continues a sequence, so the escape
//! of a string's bytes is the UTF-8 encoding of the escape of its characters. See
//! `verification/README.md` for what the proof covers and what it does not.

// The logic functions and the specification macros, in the Creusot build only (this
// file is then a module of the proof crate, whose root holds them).
#[cfg(creusot)]
use super::*;

use alloc::vec::Vec;

/// The ASCII byte of a nibble's lower-case hex digit.
#[cfg_attr(creusot, requires(n@ < 16))]
#[cfg_attr(creusot, ensures(result == hex_lo(n@)))]
fn hex_digit(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}

/// Quote and escape the UTF-8 bytes of a string the one canonical way: `"` and `\`
/// escaped, every byte below 0x20 as a lower-case `\u00xx` with no short form, and
/// every other byte — `/` and each byte of a non-ASCII character among them —
/// verbatim. Proved to return exactly the reference model's `quoted` of its input.
///
/// The dispatch is an `if` chain where a `match` would read more naturally: Creusot
/// 0.9.0 gives a `match` arm's default branch none of the earlier arms' negations, so
/// the last arm could not be shown to be the verbatim case.
#[cfg_attr(creusot, ensures(result@ == quoted(s@)))]
pub(crate) fn escape_bytes(s: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(s.len().saturating_add(2));
    out.push(b'"');
    #[cfg(creusot)]
    let _ = snapshot! { concat_laws::<u8>() };
    #[cfg(creusot)]
    proof_assert! { out@.ext_eq(Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, 0)))) };
    let mut i: usize = 0;
    #[cfg_attr(creusot, invariant(i@ <= s@.len()))]
    #[cfg_attr(creusot, invariant(out@ == Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, i@)))))]
    while i < s.len() {
        let b = s[i];
        // The step: the escape of one more byte is the escape so far, then that byte's
        // own (`escape_push_back`, over the prefix read so far).
        #[cfg(creusot)]
        let prefix = snapshot! { s@.subsequence(0, i@) };
        #[cfg(creusot)]
        let _ = snapshot! { concat_laws::<u8>() };
        #[cfg(creusot)]
        let _ = snapshot! { escape_push_back(*prefix, b) };
        #[cfg(creusot)]
        proof_assert! { s@.subsequence(0, i@ + 1).ext_eq(prefix.push_back(b)) };
        #[cfg(creusot)]
        proof_assert! { escape(s@.subsequence(0, i@ + 1)) == escape(*prefix).concat(esc_ch(b)) };
        if b == b'"' {
            #[cfg(creusot)]
            proof_assert! { esc_ch(b) == Seq::singleton(0x5Cu8).push_back(0x22u8) };
            out.push(b'\\');
            out.push(b'"');
        } else if b == b'\\' {
            #[cfg(creusot)]
            proof_assert! { esc_ch(b) == Seq::singleton(0x5Cu8).push_back(0x5Cu8) };
            out.push(b'\\');
            out.push(b'\\');
        } else if b < 0x20 {
            #[cfg(creusot)]
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
            #[cfg(creusot)]
            proof_assert! { esc_ch(b) == Seq::singleton(b) };
            out.push(b);
        }
        #[cfg(creusot)]
        proof_assert! { out@ == Seq::singleton(0x22u8).concat(escape(s@.subsequence(0, i@ + 1))) };
        i += 1;
    }
    #[cfg(creusot)]
    proof_assert! { s@.subsequence(0, s@.len()).ext_eq(s@) };
    out.push(b'"');
    out
}
