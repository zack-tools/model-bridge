//! Account state machine.
//!
//! `Authenticated` from a mock fixture is not a live login. The pinned-core
//! checker never calls the user-present transition. A warning ends that flow
//! until the operator explicitly dismisses it.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    ChatGptCodex,
    Antigravity,
}

impl Provider {
    pub fn upstream_name(self) -> &'static str {
        match self {
            Self::ChatGptCodex => "codex",
            Self::Antigravity => "antigravity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthState {
    LoggedOut,
    Authorizing,
    AwaitingConfirmation,
    Authenticated,
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    TermsRequired,
    AccountWarning,
    UpstreamRejected,
    Transport,
    Cancelled,
    RefusedUnwitnessedSuccess,
    FlowStopped,
    PortConflict,
}

#[derive(Clone, PartialEq, Eq)]
pub enum StartOutcome {
    Started { upstream_state: String, url_host: String },
    Failed(FailureClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    Wait,
    Success,
    Expired,
    Failed(FailureClass),
}

#[derive(Clone, Serialize)]
pub struct AccountSession {
    pub provider: Provider,
    pub state: AuthState,
    pub failure: Option<FailureClass>,
    pub terms_acknowledged: bool,
    pub failure_surfaced: bool,
    pub live: bool,
    pub user_present_marks: u32,
    pub upstream_state_held: bool,
    #[serde(skip)]
    upstream_state: Option<String>,
}

impl AccountSession {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            state: AuthState::LoggedOut,
            failure: None,
            terms_acknowledged: false,
            failure_surfaced: false,
            live: false,
            user_present_marks: 0,
            upstream_state_held: false,
            upstream_state: None,
        }
    }

    pub fn acknowledge_terms(&mut self) {
        self.terms_acknowledged = true;
    }

    pub fn upstream_state(&self) -> Option<&str> {
        self.upstream_state.as_deref()
    }

    pub fn begin(&mut self, outcome: StartOutcome) -> Result<(), FailureClass> {
        if self.state != AuthState::LoggedOut && self.state != AuthState::Error {
            return Err(FailureClass::FlowStopped);
        }
        if self.state == AuthState::Error && !self.failure_surfaced {
            self.failure = Some(FailureClass::FlowStopped);
            return Err(FailureClass::FlowStopped);
        }
        if self.state == AuthState::Error && self.failure_surfaced {
            self.failure = Some(FailureClass::FlowStopped);
            return Err(FailureClass::FlowStopped);
        }
        if !self.terms_acknowledged {
            self.failure = Some(FailureClass::TermsRequired);
            return Err(FailureClass::TermsRequired);
        }
        match outcome {
            StartOutcome::Started {
                upstream_state,
                url_host: _,
            } => {
                self.upstream_state = Some(upstream_state);
                self.upstream_state_held = true;
                self.state = AuthState::Authorizing;
                self.failure = None;
                self.live = false;
                Ok(())
            }
            StartOutcome::Failed(class) => {
                self.state = AuthState::Error;
                self.failure = Some(class);
                self.failure_surfaced = false;
                self.live = false;
                self.upstream_state = None;
                self.upstream_state_held = false;
                Err(class)
            }
        }
    }

    /// Operator has seen the warning. This does not resume the same flow.
    pub fn dismiss_surfaced_error(&mut self) -> Result<(), FailureClass> {
        if self.state != AuthState::Error || !self.failure_surfaced {
            return Err(FailureClass::FlowStopped);
        }
        self.state = AuthState::LoggedOut;
        self.failure = None;
        self.failure_surfaced = false;
        self.upstream_state = None;
        self.upstream_state_held = false;
        self.live = false;
        Ok(())
    }

    pub fn mark_failure_surfaced(&mut self) {
        if self.state == AuthState::Error {
            self.failure_surfaced = true;
        }
    }

    pub fn observe_poll(&mut self, outcome: PollOutcome) -> AuthState {
        match outcome {
            PollOutcome::Wait => self.state,
            PollOutcome::Expired => {
                self.state = AuthState::Expired;
                self.live = false;
                self.state
            }
            PollOutcome::Failed(class) => {
                self.state = AuthState::Error;
                self.failure = Some(class);
                self.failure_surfaced = false;
                self.live = false;
                self.state
            }
            PollOutcome::Success => {
                self.state = AuthState::Error;
                self.failure = Some(FailureClass::RefusedUnwitnessedSuccess);
                self.failure_surfaced = false;
                self.live = false;
                self.state
            }
        }
    }

    pub fn mark_mock_authenticated(&mut self) -> Result<(), FailureClass> {
        if self.state != AuthState::Authorizing {
            return Err(FailureClass::FlowStopped);
        }
        self.state = AuthState::Authenticated;
        self.live = false;
        self.failure = None;
        Ok(())
    }

    pub fn mark_user_present_authenticated(&mut self) -> Result<(), FailureClass> {
        if self.state != AuthState::AwaitingConfirmation {
            return Err(FailureClass::FlowStopped);
        }
        self.state = AuthState::Authenticated;
        self.live = true;
        self.user_present_marks += 1;
        self.failure = None;
        Ok(())
    }

    /// Only the transport may record a successful upstream status. This is not
    /// a verified live login until the user explicitly confirms their presence.
    pub(crate) fn observe_success_for_confirmation(&mut self) -> Result<(), FailureClass> {
        if self.state != AuthState::Authorizing {
            return Err(FailureClass::FlowStopped);
        }
        self.state = AuthState::AwaitingConfirmation;
        self.live = false;
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.state = AuthState::LoggedOut;
        self.failure = Some(FailureClass::Cancelled);
        self.live = false;
        self.upstream_state = None;
        self.upstream_state_held = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_stops_the_flow_and_success_without_a_witness_is_refused() {
        let mut session = AccountSession::new(Provider::ChatGptCodex);
        assert_eq!(
            session.begin(StartOutcome::Started {
                upstream_state: "hidden".into(),
                url_host: "auth.example".into()
            }),
            Err(FailureClass::TermsRequired)
        );
        session.acknowledge_terms();
        session
            .begin(StartOutcome::Failed(FailureClass::AccountWarning))
            .unwrap_err();
        assert_eq!(session.state, AuthState::Error);
        assert!(session
            .begin(StartOutcome::Started {
                upstream_state: "again".into(),
                url_host: "auth.example".into()
            })
            .is_err());
        session.mark_failure_surfaced();
        assert!(session.dismiss_surfaced_error().is_ok());
        assert_eq!(session.state, AuthState::LoggedOut);

        session
            .begin(StartOutcome::Started {
                upstream_state: "hidden-state".into(),
                url_host: "auth.example".into()
            })
            .unwrap();
        assert_eq!(
            session.observe_poll(PollOutcome::Success),
            AuthState::Error
        );
        assert!(!session.live);
        assert_eq!(session.user_present_marks, 0);
        let exported = serde_json::to_string(&session).unwrap();
        assert!(!exported.contains("hidden-state"));
    }

    #[test]
    fn mock_authenticated_is_not_live() {
        let mut session = AccountSession::new(Provider::Antigravity);
        session.acknowledge_terms();
        session
            .begin(StartOutcome::Started {
                upstream_state: "s".into(),
                url_host: "accounts.example".into()
            })
            .unwrap();
        session.mark_mock_authenticated().unwrap();
        assert_eq!(session.state, AuthState::Authenticated);
        assert!(!session.live);
        session.cancel();
        assert_eq!(session.state, AuthState::LoggedOut);
    }

    #[test]
    fn confirmation_requires_observed_upstream_success() {
        let mut session = AccountSession::new(Provider::ChatGptCodex);
        session.acknowledge_terms();
        session.begin(StartOutcome::Started { upstream_state: "s".into(), url_host: "auth.openai.com".into() }).unwrap();
        assert_eq!(session.mark_user_present_authenticated(), Err(FailureClass::FlowStopped));
        session.observe_success_for_confirmation().unwrap();
        assert!(!session.live);
        assert_eq!(session.state, AuthState::AwaitingConfirmation);
        session.mark_user_present_authenticated().unwrap();
        assert!(session.live);
        assert_eq!(session.user_present_marks, 1);
    }
}
