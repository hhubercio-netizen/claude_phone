use claude_phone_shared::SessionToken;

#[derive(Debug, Default, Clone)]
pub struct SessionState {
    pub token: Option<SessionToken>,
    pub public_url: Option<String>,
    pub peer_connected: bool,
}
