//! Durable routine check health and bounded recovery policy. Health contains safe categories,
//! never tool output or credentials; the roster encrypts it for every paired Device.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissedRunPolicy {
    Skip,
    #[default]
    Coalesce,
}

impl MissedRunPolicy {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "skip" => Ok(Self::Skip),
            "coalesce" => Ok(Self::Coalesce),
            _ => Err("Missed-run policy must be skip or coalesce.".into()),
        }
    }

    pub fn should_skip(self, due: i64, now: i64) -> bool {
        self == Self::Skip && now.saturating_sub(due) > 60
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Quiet,
    Ready,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckHealth {
    #[serde(default)]
    pub updated_at: f64,
    pub last_check_at: Option<f64>,
    pub last_success_at: Option<f64>,
    pub status: Option<CheckStatus>,
    #[serde(default)]
    pub connection_failures: u32,
    #[serde(default)]
    pub authentication_failures: u32,
    pub retry_at: Option<f64>,
    pub recovery_action: Option<String>,
    /// A successful quiet check does not clear a model provider's failure streak.
    #[serde(default)]
    pub model: ModelHealth,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelHealth {
    pub status: Option<CheckStatus>,
    pub connection_failures: u32,
    pub authentication_failures: u32,
    pub retry_at: Option<f64>,
    pub recovery_action: Option<String>,
}

impl ModelHealth {
    pub fn record_failure(&mut self, at: f64, failure: Failure) {
        let mut health = CheckHealth {
            connection_failures: self.connection_failures,
            authentication_failures: self.authentication_failures,
            ..Default::default()
        };
        health.record(at, false, Some(failure));
        self.status = health.status;
        self.connection_failures = health.connection_failures;
        self.authentication_failures = health.authentication_failures;
        self.retry_at = health.retry_at;
        self.recovery_action = health.recovery_action;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Connection,
    Authentication,
    Blocked,
    Script,
}

/// External tool errors use several transports. Only their category survives in health;
/// scripts may still inspect the original error in their own turn.
pub fn classify(error: &str) -> Failure {
    let error = error.to_lowercase();
    if [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication",
        "needs_auth",
        "sign in",
        "sign-in",
        "token expired",
        "invalid token",
        "not connected",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Authentication
    } else if [
        "connection",
        "connect error",
        "timed out",
        "timeout",
        "unavailable",
        "502",
        "503",
        "504",
        "dns",
        "network",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Connection
    } else if [
        "can change things",
        "only looks",
        "blocked",
        "permission",
        "read-only",
    ]
    .iter()
    .any(|word| error.contains(word))
    {
        Failure::Blocked
    } else {
        Failure::Script
    }
}

impl CheckHealth {
    pub fn record(&mut self, at: f64, found: bool, failure: Option<Failure>) {
        self.updated_at = at;
        self.last_check_at = Some(at);
        self.retry_at = None;
        self.recovery_action = None;
        match failure {
            None => {
                self.last_success_at = Some(at);
                self.status = Some(if found {
                    CheckStatus::Ready
                } else {
                    CheckStatus::Quiet
                });
                self.connection_failures = 0;
                self.authentication_failures = 0;
            }
            Some(Failure::Authentication) => {
                self.connection_failures = 0;
                self.authentication_failures = self.authentication_failures.saturating_add(1);
                self.status = Some(if self.authentication_failures >= 3 {
                    CheckStatus::Blocked
                } else {
                    CheckStatus::Failed
                });
                self.retry_at = Some(at + backoff(self.authentication_failures) as f64);
                self.recovery_action = Some("Reconnect the provider in Settings or sign in to the integration on the assigned Runner, then resume this routine.".into());
            }
            Some(Failure::Connection) => {
                self.authentication_failures = 0;
                self.connection_failures = self.connection_failures.saturating_add(1);
                self.status = Some(CheckStatus::Failed);
                self.retry_at = Some(at + backoff(self.connection_failures) as f64);
                self.recovery_action = Some("Check the connection on the assigned Runner. The routine retries automatically after its backoff.".into());
            }
            Some(failure) => {
                self.connection_failures = 0;
                self.authentication_failures = 0;
                self.status = Some(if failure == Failure::Blocked {
                    CheckStatus::Blocked
                } else {
                    CheckStatus::Failed
                });
                self.recovery_action =
                    Some("Ask the bot to fix its check; checks use read-only tools.".into());
            }
        }
    }

    pub fn resume(&mut self) {
        self.connection_failures = 0;
        self.authentication_failures = 0;
        self.retry_at = None;
        self.recovery_action = None;
        self.status = None;
        self.model = Default::default();
    }
}

/// Five minutes initially, doubles to a six-hour cap, persisted so restarts do not reset it.
fn backoff(failures: u32) -> i64 {
    (300_i64 << failures.saturating_sub(1).min(7)).min(6 * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_backoff_is_bounded_and_success_resets_it() {
        let mut health = CheckHealth::default();
        health.record(100.0, false, None);
        for (attempt, wait) in [(1, 300.0), (2, 600.0), (3, 1200.0), (4, 2400.0)] {
            health.record(200.0, false, Some(Failure::Connection));
            assert_eq!(health.connection_failures, attempt);
            assert_eq!(health.retry_at, Some(200.0 + wait));
            assert_eq!(health.last_success_at, Some(100.0));
        }
        for _ in 0..50 {
            health.record(200.0, false, Some(Failure::Connection));
        }
        assert_eq!(health.retry_at, Some(200.0 + 21_600.0));
        health.record(500.0, false, None);
        assert_eq!(health.status, Some(CheckStatus::Quiet));
        assert_eq!(health.last_success_at, Some(500.0));
        assert_eq!((health.connection_failures, health.retry_at), (0, None));
    }

    #[test]
    fn repeated_authentication_failure_blocks_with_recovery() {
        let mut health = CheckHealth::default();
        for attempt in 1..=3 {
            health.record(
                100.0,
                false,
                Some(classify("HTTP 401 Unauthorized: expired token")),
            );
            assert_eq!(health.authentication_failures, attempt);
        }
        assert_eq!(health.status, Some(CheckStatus::Blocked));
        assert!(health
            .recovery_action
            .as_deref()
            .unwrap()
            .contains("resume"));
        health.resume();
        assert_eq!(
            (
                health.authentication_failures,
                health.retry_at,
                health.status
            ),
            (0, None, None)
        );
        assert_eq!(
            classify("connect error: connection refused"),
            Failure::Connection
        );
        assert_eq!(
            classify("write can change things, and a check only looks"),
            Failure::Blocked
        );
        assert_eq!(
            classify("ReferenceError: name is undefined"),
            Failure::Script
        );
    }

    #[test]
    fn quiet_checks_do_not_reset_model_authentication_failures() {
        let mut health = CheckHealth::default();
        for attempt in 1..=3 {
            health.record(attempt as f64, false, None);
            health
                .model
                .record_failure(attempt as f64, Failure::Authentication);
        }
        assert_eq!(health.status, Some(CheckStatus::Quiet));
        assert_eq!(health.model.status, Some(CheckStatus::Blocked));
        assert_eq!(health.model.authentication_failures, 3);
    }
}
