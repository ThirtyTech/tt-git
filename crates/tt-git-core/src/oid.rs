//! [`Oid`]: a git object id held as raw bytes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Longest id, in hex digits (SHA-256).
const MAX_HEX: usize = 64;

/// A git object id: SHA-1 (40 hex digits) or SHA-256 (64), stored as bytes
/// instead of a hex `String`, so it is `Copy` and needs no allocation.
///
/// Its text form (`Display`, `Debug`, serde) is the lowercase hex git
/// prints, so it reads, logs and serializes exactly like the `String` it
/// replaces. Comparing with a `str` compares that text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Oid {
    /// Two hex digits per byte, high nibble first; unused bytes are zero.
    bytes: [u8; MAX_HEX / 2],
    /// Number of hex digits.
    len: u8,
}

/// The hex text of an [`Oid`], on the stack.
#[derive(Clone, Copy)]
pub struct OidHex {
    buf: [u8; MAX_HEX],
    len: u8,
}

impl OidHex {
    pub fn as_str(&self) -> &str {
        // Only ASCII hex digits are ever written.
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or_default()
    }
}

impl std::ops::Deref for OidHex {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

/// Text that isn't an object id in canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOidError;

impl fmt::Display for ParseOidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a lowercase hex object id")
    }
}

impl std::error::Error for ParseOidError {}

impl Oid {
    /// Parses lowercase hex of 1 to 64 digits, the form git prints. Upper
    /// case is refused so that `Oid::from_hex(s)` equals an oid exactly when
    /// `s` equals its text.
    pub fn from_hex(s: &str) -> Option<Oid> {
        let hex = s.as_bytes();
        if hex.is_empty() || hex.len() > MAX_HEX {
            return None;
        }
        let mut bytes = [0u8; MAX_HEX / 2];
        for (i, &c) in hex.iter().enumerate() {
            let v = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => return None,
            };
            bytes[i / 2] |= if i % 2 == 0 { v << 4 } else { v };
        }
        Some(Oid {
            bytes,
            len: hex.len() as u8,
        })
    }

    /// The hex text, without allocating.
    pub fn hex(&self) -> OidHex {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut buf = [0u8; MAX_HEX];
        for (i, out) in buf.iter_mut().enumerate().take(self.len as usize) {
            let b = self.bytes[i / 2];
            let v = if i % 2 == 0 { b >> 4 } else { b & 0xf };
            *out = DIGITS[v as usize];
        }
        OidHex { buf, len: self.len }
    }

    /// Number of hex digits (40 for SHA-1).
    pub fn hex_len(&self) -> usize {
        self.len as usize
    }
}

impl FromStr for Oid {
    type Err = ParseOidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Oid::from_hex(s).ok_or(ParseOidError)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.hex().as_str(), f)
    }
}

/// Like the hex `String`'s: quoted.
impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.hex().as_str(), f)
    }
}

impl PartialEq<str> for Oid {
    fn eq(&self, other: &str) -> bool {
        self.hex().as_str() == other
    }
}

impl PartialEq<&str> for Oid {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
    }
}

impl PartialEq<String> for Oid {
    fn eq(&self, other: &String) -> bool {
        *self == **other
    }
}

impl PartialEq<Oid> for str {
    fn eq(&self, other: &Oid) -> bool {
        other == self
    }
}

impl PartialEq<Oid> for &str {
    fn eq(&self, other: &Oid) -> bool {
        other == *self
    }
}

impl PartialEq<Oid> for String {
    fn eq(&self, other: &Oid) -> bool {
        other == self.as_str()
    }
}

impl From<Oid> for String {
    fn from(oid: Oid) -> String {
        oid.to_string()
    }
}

impl Serialize for Oid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.hex().as_str())
    }
}

impl<'de> Deserialize<'de> for Oid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Oid::from_hex(&s).ok_or_else(|| serde::de::Error::custom(ParseOidError))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    #[test]
    fn round_trips_sha1_and_sha256() {
        for s in [SHA1, SHA256, "abc", "0"] {
            let oid: Oid = s.parse().unwrap();
            assert_eq!(oid.to_string(), s);
            assert_eq!(format!("{oid:?}"), format!("{s:?}"));
            assert_eq!(oid.hex_len(), s.len());
            assert_eq!(oid, s);
            assert_eq!(s, oid);
        }
    }

    #[test]
    fn length_is_part_of_identity() {
        assert_ne!(Oid::from_hex("abc").unwrap(), Oid::from_hex("abc0").unwrap());
        assert_ne!(Oid::from_hex(SHA1).unwrap(), "0123456789abcdef0123456789abcdef0123456");
    }

    #[test]
    fn refuses_non_canonical_text() {
        for s in ["", "xyz", "ABCDEF", "abc ", &"a".repeat(65)] {
            assert_eq!(Oid::from_hex(s), None, "{s:?}");
        }
    }

    #[test]
    fn serializes_as_the_hex_string() {
        let oid = Oid::from_hex(SHA1).unwrap();
        let json = serde_json::to_string(&oid).unwrap();
        assert_eq!(json, format!("\"{SHA1}\""));
        assert_eq!(serde_json::from_str::<Oid>(&json).unwrap(), oid);
    }

    #[test]
    fn is_small() {
        assert_eq!(std::mem::size_of::<Oid>(), 33);
    }
}
