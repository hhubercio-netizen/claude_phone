use claude_phone_gateway::config::GatewayConfig;

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
    assert_eq!(cfg.session_idle_timeout_secs, 300);
}
