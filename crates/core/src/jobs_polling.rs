//! Shared polling helper for the `/v1` `202 Accepted` + `JobLaunchResponse`
//! contract, ported from `enscrive-cli/src/jobs_polling.rs`.
//!
//! `POST /v1/ingest` (and every other async-by-default `/v1` mutation) now
//! answers with `{ job_id, status, poll_url }` instead of doing the work
//! inline. The caller polls `GET /v1/jobs/{job_id}` with exponential
//! backoff (2s → 15s, matching the CLI) until the server reports a
//! terminal `.status`.
//!
//! [`JobPoller`] abstracts the HTTP GET so [`await_job_terminal`] can be
//! unit-tested against an in-memory fake instead of a live server.

use crate::error::{EnscriveError, Result};
use crate::types::ImportJobStatus;
use std::future::Future;

const INITIAL_DELAY_SECS: u64 = 2;
const MAX_DELAY_SECS: u64 = 15;

/// Minimal polling surface — abstracts `EnscriveClient::get_job_status` so
/// the loop can be exercised without a live HTTP server.
pub trait JobPoller {
    fn get_job_status(&self, job_id: &str) -> impl Future<Output = Result<ImportJobStatus>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Succeeded,
    Failed,
}

/// Classify a job `.status` string. `None` means non-terminal — caller
/// should keep polling.
pub fn classify_status(status: &str) -> Option<TerminalKind> {
    match status {
        "complete" | "completed" | "succeeded" => Some(TerminalKind::Succeeded),
        "failed" | "cancelled" => Some(TerminalKind::Failed),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct PollConfig {
    pub initial_delay: std::time::Duration,
    pub max_delay: std::time::Duration,
    pub timeout: std::time::Duration,
}

impl PollConfig {
    pub fn waited(timeout_secs: u64) -> Self {
        Self {
            initial_delay: std::time::Duration::from_secs(INITIAL_DELAY_SECS),
            max_delay: std::time::Duration::from_secs(MAX_DELAY_SECS),
            timeout: std::time::Duration::from_secs(timeout_secs),
        }
    }
}

/// Poll `job_id` until the server reports a terminal `.status` or the
/// deadline elapses.
///
/// * `Ok((TerminalKind::Succeeded, job))` — job reached a success status.
/// * `Err(EnscriveError::Other(_))` — job reached a failure status
///   (`failed` / `cancelled`), the deadline elapsed without a terminal
///   status, or every poll attempt failed. The error message names which
///   case occurred so callers never swallow it silently.
pub async fn await_job_terminal<P: JobPoller>(
    poller: &P,
    job_id: &str,
    cfg: PollConfig,
) -> Result<(TerminalKind, ImportJobStatus)> {
    let deadline = std::time::Instant::now() + cfg.timeout;
    let mut delay = cfg.initial_delay;
    let mut last_status = String::new();
    let mut had_success_response = false;

    loop {
        match poller.get_job_status(job_id).await {
            Ok(job) => {
                had_success_response = true;
                last_status = job.status.clone();
                if let Some(kind) = classify_status(&job.status) {
                    return match kind {
                        TerminalKind::Succeeded => Ok((kind, job)),
                        TerminalKind::Failed => {
                            let reason = job
                                .error_message
                                .clone()
                                .unwrap_or_else(|| "job terminated without error_message".into());
                            Err(EnscriveError::Other(format!(
                                "ingest job {job_id} {}: {reason}",
                                job.status
                            )))
                        }
                    };
                }

                if std::time::Instant::now() >= deadline {
                    return Err(EnscriveError::Other(format!(
                        "timed out after {}s polling ingest job {job_id} (last status: {})",
                        cfg.timeout.as_secs(),
                        job.status
                    )));
                }
            }
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(EnscriveError::Other(if had_success_response {
                        format!(
                            "poll failed after timeout for ingest job {job_id} \
                             (last status: {last_status}): {e}"
                        )
                    } else {
                        format!("poll failed after timeout for ingest job {job_id}: {e}")
                    }));
                }
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(cfg.max_delay);
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Drives the polling loop with a scripted sequence of responses. Once
    /// the scripted queue is exhausted the last entry repeats — real
    /// services hold a stable status once terminal, and repeat-on-exhaust
    /// lets the timeout test drive the loop to its deadline without
    /// padding the script.
    struct ScriptedPoller {
        responses: Mutex<std::collections::VecDeque<ImportJobStatus>>,
        calls: Mutex<u64>,
    }

    impl ScriptedPoller {
        fn new(seq: Vec<ImportJobStatus>) -> Self {
            Self {
                responses: Mutex::new(seq.into()),
                calls: Mutex::new(0),
            }
        }

        fn call_count(&self) -> u64 {
            *self.calls.lock().unwrap()
        }
    }

    impl JobPoller for ScriptedPoller {
        async fn get_job_status(&self, _job_id: &str) -> Result<ImportJobStatus> {
            *self.calls.lock().unwrap() += 1;
            let mut queue = self.responses.lock().unwrap();
            if queue.len() > 1 {
                Ok(queue.pop_front().unwrap())
            } else {
                // Repeat the last entry indefinitely.
                Ok(queue.front().unwrap().clone())
            }
        }
    }

    fn job(status: &str) -> ImportJobStatus {
        ImportJobStatus {
            id: "job-1".to_string(),
            status: status.to_string(),
            phase: String::new(),
            progress_percent: 0.0,
            total_documents: 0,
            documents_ingested: 0,
            documents_failed: 0,
            error_message: None,
            warnings: Vec::new(),
        }
    }

    fn fast_cfg() -> PollConfig {
        PollConfig {
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            timeout: std::time::Duration::from_secs(5),
        }
    }

    #[test]
    fn classify_status_terminal() {
        assert_eq!(classify_status("completed"), Some(TerminalKind::Succeeded));
        assert_eq!(classify_status("complete"), Some(TerminalKind::Succeeded));
        assert_eq!(classify_status("succeeded"), Some(TerminalKind::Succeeded));
        assert_eq!(classify_status("failed"), Some(TerminalKind::Failed));
        assert_eq!(classify_status("cancelled"), Some(TerminalKind::Failed));
    }

    #[test]
    fn classify_status_non_terminal() {
        assert_eq!(classify_status("pending"), None);
        assert_eq!(classify_status("running"), None);
        assert_eq!(classify_status(""), None);
    }

    #[tokio::test]
    async fn pending_then_running_then_complete() {
        let poller = ScriptedPoller::new(vec![
            job("pending"),
            job("running"),
            job("completed"),
        ]);
        let (kind, result) = await_job_terminal(&poller, "abc", fast_cfg())
            .await
            .expect("expected success");
        assert_eq!(kind, TerminalKind::Succeeded);
        assert_eq!(result.status, "completed");
        assert_eq!(poller.call_count(), 3);
    }

    #[tokio::test]
    async fn failed_status_surfaces_error_message() {
        let mut failed = job("failed");
        failed.error_message = Some("embedding rate-limit exceeded".to_string());
        let poller = ScriptedPoller::new(vec![job("pending"), failed]);
        let err = await_job_terminal(&poller, "xyz", fast_cfg())
            .await
            .expect_err("expected failure");
        let msg = err.to_string();
        assert!(msg.contains("xyz"), "got: {msg}");
        assert!(msg.contains("embedding rate-limit exceeded"), "got: {msg}");
    }

    #[tokio::test]
    async fn cancelled_classified_as_failed() {
        let poller = ScriptedPoller::new(vec![job("cancelled")]);
        let err = await_job_terminal(&poller, "c", fast_cfg())
            .await
            .expect_err("expected failure");
        assert!(err.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn times_out_when_no_terminal_status() {
        let poller = ScriptedPoller::new(vec![job("pending"), job("running")]);
        let cfg = PollConfig {
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            timeout: std::time::Duration::from_millis(5),
        };
        let err = await_job_terminal(&poller, "t", cfg)
            .await
            .expect_err("expected timeout");
        assert!(err.to_string().contains("timed out"), "got: {err}");
    }
}
