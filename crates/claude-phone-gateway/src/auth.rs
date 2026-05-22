use claude_phone_shared::ApiKey;

/// Verifies that the incoming wrapper's API key is in the allowlist.
/// Constant-time comparison against each allowed key.
pub fn verify_api_key(provided: &ApiKey, allowed: &[ApiKey]) -> bool {
    allowed.iter().any(|a| a.constant_time_eq(provided))
}
