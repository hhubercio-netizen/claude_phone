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

// TM-CODE.6 — GatewayConfig::validate bounds tests.

fn cfg_with(session_idle: u64, max_sessions: usize) -> GatewayConfig {
    let key = ApiKey::generate();
    let raw = key.as_str().to_string();
    let toml_doc = format!(
        r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["{raw}"]
        session_idle_timeout_secs = {session_idle}
        max_sessions = {max_sessions}
        "#
    );
    toml::from_str(&toml_doc).unwrap()
}

#[test]
fn validate_accepts_defaults() {
    // TM-CODE.6 — current production defaults must pass.
    let cfg = cfg_with(7 * 24 * 60 * 60, 100);
    cfg.validate().expect("defaults must validate");
}

#[test]
fn validate_accepts_documented_extremes() {
    // TM-CODE.6 — the documented bounds inclusive.
    cfg_with(60, 1).validate().unwrap();
    cfg_with(30 * 24 * 60 * 60, 10_000).validate().unwrap();
}

#[test]
fn validate_rejects_session_idle_too_low() {
    // TM-CODE.6
    let cfg = cfg_with(59, 100);
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("session_idle_timeout_secs"), "{err}");
    assert!(err.contains("TM-CODE.6"), "{err}");
}

#[test]
fn validate_rejects_session_idle_too_high() {
    // TM-CODE.6
    let cfg = cfg_with(30 * 24 * 60 * 60 + 1, 100);
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("session_idle_timeout_secs"), "{err}");
}

#[test]
fn validate_rejects_max_sessions_zero() {
    // TM-CODE.6
    let cfg = cfg_with(7 * 24 * 60 * 60, 0);
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("max_sessions"), "{err}");
}

#[test]
fn validate_rejects_max_sessions_too_high() {
    // TM-CODE.6
    let cfg = cfg_with(7 * 24 * 60 * 60, 10_001);
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("max_sessions"), "{err}");
}
