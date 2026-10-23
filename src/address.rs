//! The content address: `sha256:` and the lower-case hex SHA-256 of a value's
//! canonical bytes — the digest convention every Core vector and record uses.

use crate::json::{JVal, parse, render_canonical};
use crate::prelude::*;
use crate::sha256::sha256_hex;

/// The prefix that names the digest algorithm.
pub const PREFIX: &str = "sha256:";

/// Why a value or a text has no content address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressFault {
    /// The value holds a non-finite number, which has no canonical form.
    NonFinite,
    /// The text is not the canonical form of the value it reads as, so its digest
    /// would not recompute from that value.
    NotCanonical,
}

/// The address of exactly these bytes, which the caller vouches are canonical.
pub fn of_bytes(canonical: &str) -> String {
    let mut out = String::from(PREFIX);
    out.push_str(&sha256_hex(canonical));
    out
}

/// The address of a value: the digest of its canonical bytes.
pub fn of_value(v: &JVal) -> Result<String, AddressFault> {
    if !finite(v) {
        return Err(AddressFault::NonFinite);
    }
    Ok(of_bytes(&render_canonical(v)))
}

/// The address of a text that must already be canonical: refused when it is not,
/// rather than silently re-rendered, because a re-rendered digest names bytes the
/// caller never held.
pub fn of_canonical_text(text: &str) -> Result<String, AddressFault> {
    match parse(text) {
        Ok(v) if render_canonical(&v) == text => Ok(of_bytes(text)),
        _ => Err(AddressFault::NotCanonical),
    }
}

fn finite(v: &JVal) -> bool {
    match v {
        JVal::Num(n) => n.is_finite(),
        JVal::Arr(items) => items.iter().all(finite),
        JVal::Obj(members) => members.iter().all(|(_, v)| finite(v)),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_the_digest_of_the_canonical_bytes() {
        assert_eq!(
            of_canonical_text("{}").unwrap(),
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert_eq!(of_canonical_text("{ }"), Err(AddressFault::NotCanonical));
        assert_eq!(of_value(&JVal::Num(f64::NAN)), Err(AddressFault::NonFinite));
    }
}
