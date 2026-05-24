use claude_phone_gateway::config::{Environment, GatewayConfig};
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

// TM-WS.9 — production fail-loud on missing public_origin.

#[test]
fn production_without_public_origin_refuses_to_validate() {
    // A production-tagged config that omits public_origin must be rejected
    // at startup; otherwise TM-WS.1/.2/.3 are silently disabled in prod.
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
        environment = "production"
    "#;
    let cfg: GatewayConfig = toml::from_str(toml).unwrap();
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("public_origin"), "{err}");
    assert!(err.contains("TM-WS.9"), "{err}");
}

#[test]
fn production_with_public_origin_validates() {
    // A correctly-configured production gateway must pass validation so
    // we never regress the happy path. Pairs with the negative test above.
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
        environment = "production"
        public_origin = "https://claude-phone.pl"
    "#;
    let cfg: GatewayConfig = toml::from_str(toml).unwrap();
    cfg.validate()
        .expect("production with public_origin must validate");
}

#[test]
fn development_without_public_origin_validates() {
    // Dev / test configs commonly omit public_origin; TM-WS.9 must NOT
    // fire outside production or we break every local dev workflow.
    let toml = r#"
        bind_addr = "127.0.0.1:8080"
        static_dir = "/var/www/claude-phone"
        api_keys = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
    "#;
    let cfg: GatewayConfig = toml::from_str(toml).unwrap();
    cfg.validate()
        .expect("default Development environment permits public_origin = None");
}

#[test]
fn environment_default_is_development() {
    // Red-team guard: a future refactor that "harmlessly" flips the
    // Default to Production would silently force every dev / test config
    // through TM-WS.9 and break local workflows. Pin the default here.
    assert_eq!(Environment::default(), Environment::Development);
}

// TM-SECRET.1 — gateway.toml permission mode enforcement at load.
//
// The mask `mode & 0o027 != 0` is the load-time gate. These tests are the
// forward-looking proof: they fail if a future refactor relaxes the mask
// (e.g. to `0o007` would permit group-write, to `0` would skip the check
// entirely), regresses the chmod-hint string, or drops the cfg(unix) guard
// and breaks the Windows build. Each negative test asserts that the bail
// message names the catalog row so log-grep on a misconfigured host points
// the operator at the right spec section.
#[cfg(unix)]
mod tm_secret_1 {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    use claude_phone_gateway::config::GatewayConfig;
    use claude_phone_shared::ApiKey;
    use tempfile::NamedTempFile;

    fn write_cfg_with_mode(mode: u32) -> NamedTempFile {
        // Real ApiKey so we exercise the load path past the mode check —
        // a stub key would also reject during toml parse and mask whether
        // the mode gate fired first.
        let key = ApiKey::generate();
        let raw = key.as_str().to_string();
        let body = format!(
            r#"
            bind_addr = "127.0.0.1:8080"
            static_dir = "/var/www/claude-phone"
            api_keys = ["{raw}"]
            "#
        );
        let f = NamedTempFile::new().expect("tempfile");
        std::fs::write(f.path(), body).expect("write toml");
        std::fs::set_permissions(f.path(), Permissions::from_mode(mode)).expect("chmod");
        f
    }

    #[test]
    fn rejects_world_readable_0644() {
        // 0644 leaks every api_key to any local user on a multi-tenant host.
        let f = write_cfg_with_mode(0o644);
        let err = GatewayConfig::load(f.path()).unwrap_err().to_string();
        assert!(err.contains("TM-SECRET.1"), "{err}");
        assert!(err.contains("permissive mode"), "{err}");
        assert!(err.contains("chmod 640"), "hint must guide fix: {err}");
    }

    #[test]
    fn rejects_group_writable_0660() {
        // 0660 lets any group member overwrite api_keys (inject their own).
        let f = write_cfg_with_mode(0o660);
        let err = GatewayConfig::load(f.path()).unwrap_err().to_string();
        assert!(err.contains("TM-SECRET.1"), "{err}");
    }

    #[test]
    fn rejects_world_executable_0641() {
        // 0641 is the canonical "looks tight but isn't" mode — owner+group
        // read, world-exec. World-exec on a regular file is meaningless but
        // the bit is set, which is what 0o027 catches. Pinning this prevents
        // a future "well, world-exec is harmless on data files" relaxation.
        let f = write_cfg_with_mode(0o641);
        let err = GatewayConfig::load(f.path()).unwrap_err().to_string();
        assert!(err.contains("TM-SECRET.1"), "{err}");
    }

    #[test]
    fn rejects_world_readable_0604() {
        // 0604 = owner-rw, world-r only (no group). Still leaks the secret
        // to any local user — covered by 0o027 because the world-read bit
        // (0o004) is set.
        let f = write_cfg_with_mode(0o604);
        let err = GatewayConfig::load(f.path()).unwrap_err().to_string();
        assert!(err.contains("TM-SECRET.1"), "{err}");
    }

    #[test]
    fn accepts_target_mode_0640() {
        // The documented production mode (root:claude-phone, 0640). The
        // happy-path: loading must succeed end-to-end including validate().
        let f = write_cfg_with_mode(0o640);
        let cfg =
            GatewayConfig::load(f.path()).expect("0640 is the documented prod mode and must load");
        assert_eq!(cfg.api_keys.len(), 1);
    }

    #[test]
    fn accepts_strictest_0600() {
        // 0600 (owner-only) is strictly tighter than 0640 — a single-user
        // host that runs the gateway as its own user should still work.
        let f = write_cfg_with_mode(0o600);
        GatewayConfig::load(f.path()).expect("0600 must load");
    }

    #[test]
    fn accepts_readonly_0440() {
        // 0440 (owner+group read, no write anywhere) is the immutable-ish
        // production setup an SRE might choose. Must continue to load.
        let f = write_cfg_with_mode(0o440);
        GatewayConfig::load(f.path()).expect("0440 must load");
    }
}
