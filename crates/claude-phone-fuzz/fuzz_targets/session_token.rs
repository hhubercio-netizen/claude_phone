// TM-TEST.1 — fuzz `SessionToken::parse`.
//
// `SessionToken::parse` is the first thing called on the token bytes
// received in a PhoneHello / WrapperHello frame, before any constant-time
// comparison or session-table lookup. If the parser panics on a crafted
// 43-byte input, an attacker can DoS the gateway worker without holding
// a valid token. The validator is a tight loop over `is_base64url_byte`
// folded into a single bit (timing-oracle safe — see token.rs), so
// libFuzzer's mutator should converge on length-43 inputs quickly and
// then drill into the charset checks.

#![no_main]

use claude_phone_shared::SessionToken;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = SessionToken::parse(s);
    }
});
