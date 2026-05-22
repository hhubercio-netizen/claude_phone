use claude_phone_wrapper::qr::render_terminal;

#[test]
fn render_produces_non_empty_output() {
    let s = render_terminal("https://example.com/s/abc");
    assert!(!s.is_empty());
    assert!(s.lines().count() > 5, "QR output suspiciously short");
}

#[test]
fn different_inputs_produce_different_outputs() {
    let a = render_terminal("https://example.com/a");
    let b = render_terminal("https://example.com/b");
    assert_ne!(a, b);
}

#[test]
fn same_input_is_deterministic() {
    let a = render_terminal("https://example.com/same");
    let b = render_terminal("https://example.com/same");
    assert_eq!(a, b);
}
