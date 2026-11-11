//! OAuth management transport. Authorization URLs and state stay in backend memory.
use crate::{
    http::exchange,
    loopback_callback::LoopbackCallbackServer,
    oauth::{AccountSession, AuthState, FailureClass, PollOutcome, Provider, StartOutcome},
    redact::Redactor,
};
use std::time::Duration;

pub struct OAuthClient {
    port: u16,
    management_key: String,
    pub session: AccountSession,
    callback_port: u16,
    callback_server: Option<LoopbackCallbackServer>,
}

// Do not implement Debug or Serialize: the management key and upstream state are sensitive.
impl OAuthClient {
    pub fn new(port: u16, management_key: String, provider: Provider) -> Self {
        let callback_port = LoopbackCallbackServer::default_port_for_provider(provider);
        Self {
            port,
            management_key,
            session: AccountSession::new(provider),
            callback_port,
            callback_server: None,
        }
    }

    #[doc(hidden)]
    pub fn with_callback_port(mut self, port: u16) -> Self {
        self.callback_port = port;
        self
    }

    pub fn callback_port(&self) -> u16 {
        self.callback_port
    }

    fn request(&self, method: &str, path: &str) -> Result<crate::http::HttpResponse, FailureClass> {
        let bearer = format!("Bearer {}", self.management_key);
        let redactor = Redactor::new([self.management_key.clone()]);
        exchange(self.port, method, path, &[("Authorization", &bearer)], Duration::from_secs(20), &redactor)
            .map_err(|_| FailureClass::Transport)
    }

    /// Caller must acknowledge the upstream terms before starting. The returned URL is
    /// only for opening in the system browser and must never be logged or sent to the UI.
    pub fn begin(&mut self) -> Result<String, FailureClass> {
        if !self.session.terms_acknowledged { return Err(FailureClass::TermsRequired); }
        if self.session.state != AuthState::LoggedOut { return Err(FailureClass::FlowStopped); }

        // Bind the loopback callback port first to ensure the port is available
        let (listener, bound_port) = match LoopbackCallbackServer::bind(self.callback_port) {
            Ok(pair) => pair,
            Err(class) => {
                let _ = self.session.begin(StartOutcome::Failed(class));
                return Err(class);
            }
        };

        let path = format!("/v8/management/oauth/auth-url?provider={}", self.session.provider.upstream_name());
        let response = match self.request("GET", &path) {
            Ok(response) => response,
            Err(class) => {
                let _ = self.session.begin(StartOutcome::Failed(class));
                return Err(class);
            }
        };
        if response.status != 200 {
            let class = FailureClass::UpstreamRejected;
            let _ = self.session.begin(StartOutcome::Failed(class));
            return Err(class);
        }
        let value: serde_json::Value = match serde_json::from_str(&response.body) {
            Ok(value) => value,
            Err(_) => {
                let class = FailureClass::UpstreamRejected;
                let _ = self.session.begin(StartOutcome::Failed(class));
                return Err(class);
            }
        };
        let url = value.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let state = value.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let expected_host = match self.session.provider {
            Provider::ChatGptCodex => "auth.openai.com",
            Provider::Antigravity => "accounts.google.com",
        };
        let safe_url = url.strip_prefix("https://").and_then(|s| s.split(['/', '?', '#']).next()) == Some(expected_host)
            && !url.bytes().any(|b| b.is_ascii_control() || b == b' ');
        let safe_state = !state.is_empty() && state.len() <= 256 && state.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !safe_url || !safe_state {
            let class = FailureClass::UpstreamRejected;
            let _ = self.session.begin(StartOutcome::Failed(class));
            return Err(class);
        }

        self.callback_port = bound_port;
        let server = LoopbackCallbackServer::start(
            listener,
            self.session.provider,
            state.to_string(),
            self.port,
        );
        self.callback_server = Some(server);

        self.session.begin(StartOutcome::Started { upstream_state: state.into(), url_host: expected_host.into() })?;
        Ok(url.to_string())
    }

    pub fn poll(&mut self) -> Result<AuthState, FailureClass> {
        self.poll_inner(false)
    }

    /// Call only after a real user explicitly starts a browser login while present.
    /// An upstream success still requires separate explicit confirmation.
    pub fn poll_user_present(&mut self) -> Result<AuthState, FailureClass> {
        self.poll_inner(true)
    }

    fn poll_inner(&mut self, user_present_flow: bool) -> Result<AuthState, FailureClass> {
        if matches!(self.session.state, AuthState::AwaitingConfirmation | AuthState::Authenticated | AuthState::Expired) {
            return Ok(self.session.state);
        }
        if self.session.state != AuthState::Authorizing { return Err(FailureClass::FlowStopped); }
        let state = self.session.upstream_state().ok_or(FailureClass::FlowStopped)?;
        let path = format!("/v8/management/oauth/status?state={state}");
        let response = match self.request("GET", &path) {
            Ok(value) => value,
            Err(class) => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                self.session.observe_poll(PollOutcome::Failed(class));
                return Err(class);
            }
        };
        if response.status != 200 {
            if let Some(mut server) = self.callback_server.take() {
                server.stop();
            }
            self.session.observe_poll(PollOutcome::Failed(FailureClass::UpstreamRejected));
            return Err(FailureClass::UpstreamRejected);
        }
        let value: serde_json::Value = match serde_json::from_str(&response.body) {
            Ok(value) => value,
            Err(_) => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                self.session.observe_poll(PollOutcome::Failed(FailureClass::UpstreamRejected));
                return Err(FailureClass::UpstreamRejected);
            }
        };
        let outcome = match value.get("status").and_then(|v| v.as_str()) {
            Some("wait") => PollOutcome::Wait,
            Some("ok") if user_present_flow => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                self.session.observe_success_for_confirmation()?;
                return Ok(self.session.state);
            }
            Some("ok") => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                PollOutcome::Success
            }
            Some("expired") => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                PollOutcome::Expired
            }
            _ => {
                if let Some(mut server) = self.callback_server.take() {
                    server.stop();
                }
                PollOutcome::Failed(FailureClass::UpstreamRejected)
            }
        };
        Ok(self.session.observe_poll(outcome))
    }

    pub fn cancel(&mut self) -> Result<(), FailureClass> {
        if let Some(mut server) = self.callback_server.take() {
            server.stop();
        }
        match self.session.state {
            AuthState::Authorizing => {
                let state = self.session.upstream_state().ok_or(FailureClass::FlowStopped)?;
                let path = format!("/v8/management/oauth/session?state={state}");
                let response = self.request("DELETE", &path)?;
                let value: serde_json::Value = serde_json::from_str(&response.body).map_err(|_| FailureClass::UpstreamRejected)?;
                if response.status != 200 || value.get("cancelled").and_then(|v| v.as_bool()) != Some(true) {
                    return Err(FailureClass::UpstreamRejected);
                }
            }
            AuthState::Error | AuthState::Expired | AuthState::AwaitingConfirmation => {}
            AuthState::LoggedOut | AuthState::Authenticated => {
                return Err(FailureClass::FlowStopped);
            }
        }
        self.session.cancel();
        Ok(())
    }
}

impl Drop for OAuthClient {
    fn drop(&mut self) {
        if let Some(mut server) = self.callback_server.take() {
            server.stop();
        }
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn polling_an_already_confirmed_session_preserves_state_without_network() {
        let mut client = OAuthClient::new(1, "fixture".into(), Provider::ChatGptCodex);
        client.session.acknowledge_terms();
        client.session.begin(StartOutcome::Started { upstream_state: "fixture".into(), url_host: "auth.openai.com".into() }).unwrap();
        client.session.observe_success_for_confirmation().unwrap();
        assert_eq!(client.poll_user_present().unwrap(), AuthState::AwaitingConfirmation);
        client.session.mark_user_present_authenticated().unwrap();
        assert_eq!(client.poll_user_present().unwrap(), AuthState::Authenticated);
        assert!(client.session.live);
    }

    #[test]
    fn cancel_clears_terminal_and_waiting_states_without_network() {
        for state in [AuthState::Expired, AuthState::Error, AuthState::AwaitingConfirmation] {
            let mut client = OAuthClient::new(1, "fixture".into(), Provider::ChatGptCodex);
            client.session.state = state;
            assert!(client.cancel().is_ok(), "cancel failed for state {state:?}");
            assert_eq!(client.session.state, AuthState::LoggedOut);
            assert_eq!(client.session.failure, Some(FailureClass::Cancelled));
        }
    }

    #[test]
    fn cancel_refuses_logged_out_or_authenticated() {
        let mut client = OAuthClient::new(1, "fixture".into(), Provider::ChatGptCodex);
        assert_eq!(client.session.state, AuthState::LoggedOut);
        assert_eq!(client.cancel(), Err(FailureClass::FlowStopped));

        client.session.state = AuthState::Authenticated;
        assert_eq!(client.cancel(), Err(FailureClass::FlowStopped));
    }
}
