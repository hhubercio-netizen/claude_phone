//! Secret token types used across the wrapper, gateway, and pair helper.
//!
//! Both `SessionToken` and `ApiKey` are 256-bit secrets encoded as
//! base64url without padding (43 characters). They are emitted from
//! the same `define_secret_token!` macro so that the security
//! properties (manual `Debug` redaction, `Zeroize` on drop, constant-time
//! equality via `subtle`, JSON deserialization that re-validates) cannot
//! drift between the two types.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::{Choice, ConstantTimeEq};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The provided string is not a valid 43-character base64url token.
    ///
    /// The variant is intentionally opaque: the rejected input is not
    /// included in the error and there is no separate variant for
    /// "wrong length" vs "wrong characters", so the error path leaks
    /// no information about how close the rejected value was to valid.
    #[error("invalid token")]
    Invalid,
}

/// Number of random bytes inside the secret. base64url-no-pad encoding
/// expands 32 bytes to ceil(32 * 4 / 3) = 43 characters.
const SECRET_BYTES: usize = 32;
const SECRET_STR_LEN: usize = (SECRET_BYTES * 4).div_ceil(3);

fn is_base64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

macro_rules! define_secret_token {
    ($name:ident, $debug_label:literal) => {
        /// A 256-bit secret encoded as 43-character base64url without padding.
        ///
        /// `Debug` and `Display` are intentionally never wired to print the
        /// underlying value. Callers must opt in by calling `.as_str()` —
        /// making leakage points easy to grep for in code review.
        #[derive(Clone)]
        pub struct $name(Zeroizing<String>);

        impl $name {
            /// Number of raw bytes in the underlying secret (32).
            pub const BYTES: usize = SECRET_BYTES;
            /// Length of the encoded string (43).
            pub const LEN: usize = SECRET_STR_LEN;

            /// Generate a fresh random secret from the OS CSPRNG.
            pub fn generate() -> Self {
                let mut bytes = [0u8; SECRET_BYTES];
                rand::thread_rng().fill_bytes(&mut bytes);
                Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes)))
            }

            /// Parse a string into the secret type, validating length and
            /// charset without short-circuiting on the first invalid byte.
            pub fn parse(s: &str) -> Result<Self, TokenError> {
                let bytes = s.as_bytes();
                // Fold all checks into a single bit; do not return early on
                // the first invalid byte (timing oracle defense).
                let length_ok = (bytes.len() == SECRET_STR_LEN) as u8;
                let mut chars_ok: u8 = 1;
                for &b in bytes.iter() {
                    chars_ok &= is_base64url_byte(b) as u8;
                }
                if length_ok & chars_ok == 1 {
                    Ok(Self(Zeroizing::new(s.to_string())))
                } else {
                    Err(TokenError::Invalid)
                }
            }

            /// Borrow the underlying string. The only opt-in path to read
            /// the secret value as a string.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Constant-time equality, returning a plain bool for ergonomic
            /// callers. Backed by `subtle::ConstantTimeEq` which carries a
            /// compiler barrier so LLVM cannot collapse the comparison into
            /// a branch.
            pub fn ct_eq(&self, other: &Self) -> bool {
                bool::from(<Self as ConstantTimeEq>::ct_eq(self, other))
            }
        }

        impl ConstantTimeEq for $name {
            fn ct_eq(&self, other: &Self) -> Choice {
                self.0.as_bytes().ct_eq(other.0.as_bytes())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!($debug_label, "(***)"))
            }
        }

        /// `try_from`-style validation entry. Kept as a convenience for
        /// callers who already hold an owned String; the input is dropped
        /// at the end of the call. Prefer `parse(&str)` to avoid handing
        /// us a `String` whose tail bytes we cannot zero ourselves.
        impl TryFrom<String> for $name {
            type Error = TokenError;
            fn try_from(mut s: String) -> Result<Self, Self::Error> {
                let out = Self::parse(&s);
                // Best-effort overwrite of the caller's input before drop
                // so an owned secret String the caller handed us does not
                // outlive this call in unsanitized form on the heap.
                s.zeroize();
                out
            }
        }

        /// Manual `Serialize` so the value passes through `serialize_str`
        /// by reference — no intermediate owned `String` is allocated, so
        /// no plain `String` of the secret is dropped without zeroization
        /// during JSON encoding.
        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        /// Manual `Deserialize` with a visitor that prefers the borrowed
        /// `visit_str` path (no allocation). When a format must hand us
        /// an owned `String` (e.g. JSON with escapes), we zeroize that
        /// intermediate buffer ourselves before it is dropped.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct SecretVisitor;
                impl<'de> serde::de::Visitor<'de> for SecretVisitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        write!(f, "a 43-character base64url secret")
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                        $name::parse(v).map_err(E::custom)
                    }
                    fn visit_string<E: serde::de::Error>(
                        self,
                        mut v: String,
                    ) -> Result<Self::Value, E> {
                        let out = $name::parse(&v);
                        v.zeroize();
                        out.map_err(E::custom)
                    }
                }
                deserializer.deserialize_str(SecretVisitor)
            }
        }
    };
}

define_secret_token!(SessionToken, "SessionToken");
define_secret_token!(ApiKey, "ApiKey");
