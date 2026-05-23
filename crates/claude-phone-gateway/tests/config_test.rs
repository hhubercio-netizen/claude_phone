use claude_phone_gateway::config::GatewayConfig;
use claude_phone_shared::ApiKey;

#[test]
fn parses_minimal_toml() {
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
    "#;
    let cfg: GatewayConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.bind_addr, "127.0.0.1:8080".parse().unwrap());
    assert_eq!(cfg.api_keys.len(), 1);
}

#[test]
fn defaults_session_timeout() {
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
    "#;
    let cfg: GatewayConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.session_idle_timeout_secs, 7 * 24 * 60 * 60);
}

#[test]
fn rejects_invalid_api_key_at_load() {
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["not-a-valid-43-char-base64url-key"]
    "#;
    let r: Result<GatewayConfig, _> = toml::from_str(toml);
    assert!(
        r.is_err(),
        "malformed api_key must be rejected during deserialization"
    );
}

#[test]
fn debug_redacts_api_keys() {
    let key = ApiKey::generate();
    let raw = key.as_str().to_string();
    let toml_doc = format!(
        r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["{raw}"]
        "#
    );
    let cfg: GatewayConfig = toml::from_str(&toml_doc).unwrap();
    let dbg = format!("{cfg:?}");
    assert!(
        !dbg.contains(&raw),
        "Debug output must not contain the raw api_key: {dbg}"
    );
    assert!(
        dbg.contains("ApiKey(***)"),
        "Debug must show redacted marker: {dbg}"
    );
}
