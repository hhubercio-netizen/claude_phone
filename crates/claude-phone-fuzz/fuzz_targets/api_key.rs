// TM-TEST.1 — fuzz `ApiKey::parse`.
//
// `ApiKey::parse` runs on the api_key field of every WrapperHello, before
// any constant-time comparison against the wrapper's configured key. The
// symmetry-pin proptest already asserts ApiKey and SessionToken behave
// identically on every input, but libFuzzer hunts for coverage states
// that a property-based test (without a coverage harness) cannot reach.

#![no_main]

use claude_phone_shared::ApiKey;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = ApiKey::parse(s);
    }
});
