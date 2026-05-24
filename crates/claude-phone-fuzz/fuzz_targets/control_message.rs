// TM-TEST.1 — fuzz the wire-protocol JSON parser.
//
// `serde_json::from_str::<ControlMessage>` is the first piece of code that
// touches an externally-controlled byte stream on every WebSocket frame.
// A panic here is a DoS: any peer can crash the gateway worker by sending
// crafted JSON. We want libFuzzer's coverage-guided mutator hunting for
// inputs that violate the "parse returns Result, never panics" invariant.
//
// The target is intentionally narrow — just the parse path. We do not
// feed the resulting struct back into the gateway; that's the e2e
// pentest tests' job (TM-TEST.4).

#![no_main]

use claude_phone_shared::protocol::ControlMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // ControlMessage is parsed from a UTF-8 JSON text frame at runtime.
    // Reject non-UTF-8 here so libFuzzer spends its budget on JSON-shape
    // mutations rather than on encoding-validity decisions serde would
    // make in two lines anyway.
    if let Ok(s) = std::str::from_utf8(data) {
        // Result is intentionally discarded: success and failure are both
        // valid outcomes. Only a panic (or a deserialize loop that never
        // returns) is a bug.
        let _ = serde_json::from_str::<ControlMessage>(s);
    }
});
