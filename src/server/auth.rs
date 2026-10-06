//! Listener-local authentication diagnostics. Neither headers nor tokens are logged.
use axum::http::{HeaderMap, header::AUTHORIZATION};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

const WARNING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Missing,
    Invalid,
    IncorrectToken,
}
impl Failure {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Missing => {
                "Authentication failed: missing Authorization header; send Authorization: Bearer <token>."
            }
            Self::Invalid => {
                "Authentication failed: invalid Authorization header; send exactly one header in the form Authorization: Bearer <token>."
            }
            Self::IncorrectToken => "Authentication failed: incorrect bearer token value.",
        }
    }
    fn category(self) -> &'static str {
        match self {
            Self::Missing => "auth_header_missing",
            Self::Invalid => "auth_header_invalid",
            Self::IncorrectToken => "auth_token_incorrect",
        }
    }
}

#[derive(Default)]
struct Window {
    last_warning: Option<Instant>,
    suppressed: u64,
}
impl Window {
    fn observe(&mut self, now: Instant) -> Option<u64> {
        if self
            .last_warning
            .is_some_and(|last| now.duration_since(last) < WARNING_INTERVAL)
        {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.last_warning = Some(now);
        Some(std::mem::take(&mut self.suppressed))
    }
}

pub(super) struct Gate {
    expected: String,
    listener: String,
    windows: Mutex<[Window; 3]>,
}
impl Gate {
    pub(super) fn new(token: String, listener: String) -> Self {
        Self {
            expected: format!("Bearer {token}"),
            listener,
            windows: Mutex::new(std::array::from_fn(|_| Window::default())),
        }
    }
    pub(super) fn check(&self, headers: &HeaderMap) -> Result<(), Failure> {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let actual = values.next().ok_or(Failure::Missing)?.as_bytes();
        if values.next().is_some() {
            return Err(Failure::Invalid);
        }
        // Preserve the existing exact, constant-time authentication comparison.
        if bool::from(self.expected.as_bytes().ct_eq(actual)) {
            return Ok(());
        }
        let token = actual.strip_prefix(b"Bearer ").ok_or(Failure::Invalid)?;
        if token.is_empty()
            || !token
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(b))
        {
            return Err(Failure::Invalid);
        }
        Err(Failure::IncorrectToken)
    }
    pub(super) fn warn(&self, failure: Failure) {
        let suppressed = self.windows.lock().unwrap_or_else(|e| e.into_inner())[failure as usize]
            .observe(Instant::now());
        if let Some(suppressed) = suppressed {
            tracing::warn!(
                listener = %self.listener,
                category = failure.category(),
                suppressed_since_previous_warning = suppressed,
                rate_limit_seconds = WARNING_INTERVAL.as_secs(),
                "{} Further warnings of this category are suppressed for 30 seconds; the next warning reports the suppressed count. Every rejected request receives an explicit HTTP 401 error.",
                failure.message()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_headers_without_exposing_credentials() {
        let gate = Gate::new("secret-value".into(), "listener".into());
        let mut headers = HeaderMap::new();
        assert_eq!(gate.check(&headers), Err(Failure::Missing));
        headers.insert("Authentication", "Bearer secret-value".parse().unwrap());
        assert_eq!(gate.check(&headers), Err(Failure::Missing));
        for value in [
            "",
            "Basic private-value",
            "Bearer",
            "Bearer ",
            "Bearer two words",
            "Bearer a,b",
        ] {
            headers.insert(AUTHORIZATION, value.parse().unwrap());
            assert_eq!(gate.check(&headers), Err(Failure::Invalid));
        }
        headers.insert(AUTHORIZATION, "Bearer wrong-value".parse().unwrap());
        assert_eq!(gate.check(&headers), Err(Failure::IncorrectToken));
        headers.insert(AUTHORIZATION, "Bearer secret-value".parse().unwrap());
        assert_eq!(gate.check(&headers), Ok(()));
        headers.append(AUTHORIZATION, "Bearer secret-value".parse().unwrap());
        assert_eq!(gate.check(&headers), Err(Failure::Invalid));
        for failure in [Failure::Missing, Failure::Invalid, Failure::IncorrectToken] {
            assert!(!failure.message().contains("secret-value"));
            assert!(!failure.message().contains("wrong-value"));
        }
    }
    #[test]
    fn warning_limits_are_per_listener_and_category_with_explicit_counts() {
        let gate = Gate::new("secret".into(), "first".into());
        let second = Gate::new("secret".into(), "second".into());
        let now = Instant::now();
        let mut windows = gate.windows.lock().unwrap();
        assert_eq!(windows[0].observe(now), Some(0));
        for _ in 0..10 {
            assert_eq!(windows[0].observe(now), None);
        }
        assert_eq!(windows[1].observe(now), Some(0));
        assert_eq!(second.windows.lock().unwrap()[0].observe(now), Some(0));
        assert_eq!(windows[0].observe(now + WARNING_INTERVAL), Some(10));
        assert_eq!(windows[0].observe(now + WARNING_INTERVAL * 2), Some(0));
    }
}
