use claude_phone_shared::ApiKey;
use claude_phone_wrapper::config::WrapperConfig;

fn write_toml(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.path().join(name);
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn loads_minimal_config_with_defaults() {
    let key = ApiKey::generate();
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(
        &dir,
        "config.toml",
        &format!(
            r#"
gateway_url = "wss://gw.example.com/api/wrapper"
api_key = "{}"
"#,
            key.as_str()
        ),
    );
    let cfg = WrapperConfig::load(&path).expect("loads");
    assert_eq!(cfg.gateway_url, "wss://gw.example.com/api/wrapper");
    // Defaults applied:
    assert_eq!(cfg.public_url_base, "https://claude-phone.pl");
    assert_eq!(cfg.rpc_bind, "127.0.0.1:0");
}

#[test]
fn overrides_apply() {
    let key = ApiKey::generate();
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(
        &dir,
        "config.toml",
        &format!(
            r#"
gateway_url = "wss://gw.example.com/api/wrapper"
api_key = "{}"
public_url_base = "https://staging.example.com"
rpc_bind = "127.0.0.1:7777"
"#,
            key.as_str()
        ),
    );
    let cfg = WrapperConfig::load(&path).expect("loads");
    assert_eq!(cfg.public_url_base, "https://staging.example.com");
    assert_eq!(cfg.rpc_bind, "127.0.0.1:7777");
}

#[test]
fn rejects_invalid_api_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(
        &dir,
        "config.toml",
        r#"
gateway_url = "wss://gw.example.com/api/wrapper"
api_key = "obviously-not-43-chars"
"#,
    );
    let r = WrapperConfig::load(&path);
    assert!(r.is_err(), "load must reject malformed api_key");
}

#[test]
fn parsed_api_key_roundtrip() {
    let key = ApiKey::generate();
    let cfg = WrapperConfig {
        gateway_url: "wss://gw.example.com/api/wrapper".into(),
        api_key: key.as_str().to_string(),
        public_url_base: "https://example.com".into(),
        rpc_bind: "127.0.0.1:0".into(),
    };
    let parsed = cfg.parsed_api_key().expect("valid api_key");
    assert_eq!(parsed.as_str(), key.as_str());
}

#[test]
fn rejects_missing_required_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_toml(&dir, "config.toml", r#"public_url_base = "x""#);
    let r = WrapperConfig::load(&path);
    assert!(r.is_err());
}
