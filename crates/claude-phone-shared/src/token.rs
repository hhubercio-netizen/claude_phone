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
use zeroize::Zeroizing;

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
        #[derive(Clone, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
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

        impl TryFrom<String> for $name {
            type Error = TokenError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::parse(&s)
            }
        }

        impl From<$name> for String {
            fn from(t: $name) -> String {
                t.as_str().to_string()
            }
        }
    };
}

define_secret_token!(SessionToken, "SessionToken");
define_secret_token!(ApiKey, "ApiKey");
