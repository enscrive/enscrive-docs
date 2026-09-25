//! HTTP client for the public Enscrive API.
//!
//! Auth pattern (X-API-Key, optional X-Embedding-Provider-Key) and timeout
//! mirror enscrive-cli/src/client.rs to keep cross-CLI behavior consistent.

use crate::error::{EnscriveError, Result};
use crate::jobs_polling::{await_job_terminal, PollConfig};
use crate::types::{
    CorpusDetail, CreateCorpusRequest, CreateVoiceApiRequest, DeleteCorpusResponse,
    DeleteVoiceResponse, ImportJobStatus, IngestRequest, IngestSummary, JobLaunchResponse,
    SearchQuery, SearchResults, SearchWithVoiceBody, UpdateVoiceApiRequest, VoiceDetail,
};
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default wall-clock budget for polling a background ingest job to
/// terminal (30 minutes) — matches the CLI's convention for job-shaped
/// mutations (`enscrive-cli`'s `PollConfig::waited` call sites for corpus
/// populate / restore).
const DEFAULT_INGEST_POLL_TIMEOUT_SECS: u64 = 1800;

pub struct EnscriveClient {
    http: Client,
    base_url: String,
    api_key: String,
    embedding_provider_key: Option<String>,
}

impl EnscriveClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::with_provider_key(base_url, api_key, None::<String>)
    }

    pub fn with_provider_key(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        embedding_provider_key: Option<impl Into<String>>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            // ENS-6483: never follow redirects — X-API-Key/X-Embedding-Provider-Key
            // must not be resent to a different host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("build http client");
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            embedding_provider_key: embedding_provider_key
                .map(Into::into)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        let request = request.header("X-API-Key", &self.api_key);
        if let Some(provider_key) = &self.embedding_provider_key {
            return request.header("X-Embedding-Provider-Key", provider_key);
        }
        request
    }

    async fn send_typed<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let mut request = self.auth(self.http.request(method, self.url(path)));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        if response.status().is_redirection() {
            return Err(redirect_error(&response));
        }
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(EnscriveError::Http { status, body: text });
        }
        if text.trim().is_empty() {
            return serde_json::from_str("null").map_err(EnscriveError::from);
        }
        serde_json::from_str(&text).map_err(EnscriveError::from)
    }

    // -- Corpora --

    pub async fn list_corpora(&self) -> Result<Vec<CorpusDetail>> {
        self.send_typed::<Vec<CorpusDetail>>(Method::GET, "/v1/corpora", NONE)
            .await
    }

    pub async fn get_corpus(&self, id: &str) -> Result<CorpusDetail> {
        self.send_typed::<CorpusDetail>(
            Method::GET,
            &format!("/v1/corpora/{id}"),
            NONE,
        )
        .await
    }

    pub async fn create_corpus(
        &self,
        request: &CreateCorpusRequest,
    ) -> Result<CorpusDetail> {
        self.send_typed::<CorpusDetail>(Method::POST, "/v1/corpora", Some(request))
            .await
    }

    pub async fn delete_corpus(&self, id: &str) -> Result<DeleteCorpusResponse> {
        self.send_typed::<DeleteCorpusResponse>(
            Method::DELETE,
            &format!("/v1/corpora/{id}"),
            NONE,
        )
        .await
    }

    // -- Voices --

    pub async fn list_voices(&self) -> Result<Vec<VoiceDetail>> {
        self.send_typed::<Vec<VoiceDetail>>(Method::GET, "/v1/voices", NONE)
            .await
    }

    pub async fn get_voice(&self, id: &str) -> Result<VoiceDetail> {
        self.send_typed::<VoiceDetail>(Method::GET, &format!("/v1/voices/{id}"), NONE)
            .await
    }

    pub async fn create_voice(&self, request: &CreateVoiceApiRequest) -> Result<VoiceDetail> {
        self.send_typed::<VoiceDetail>(Method::POST, "/v1/voices", Some(request))
            .await
    }

    /// PUT /v1/voices/{id} — full-replace update of the voice config.
    pub async fn update_voice(
        &self,
        id: &str,
        request: &UpdateVoiceApiRequest,
    ) -> Result<VoiceDetail> {
        self.send_typed::<VoiceDetail>(
            Method::PUT,
            &format!("/v1/voices/{id}"),
            Some(request),
        )
        .await
    }

    pub async fn delete_voice(&self, id: &str) -> Result<DeleteVoiceResponse> {
        self.send_typed::<DeleteVoiceResponse>(
            Method::DELETE,
            &format!("/v1/voices/{id}"),
            NONE,
        )
        .await
    }

    // -- Ingest --

    /// `POST /v1/ingest` under the async-by-default `/v1` contract
    /// (ENS-628, `enscrive-developer/crates/server/src/api/v1/ingest.rs`):
    /// the server ALWAYS answers `202 Accepted` + `JobLaunchResponse` for
    /// every payload — there is no synchronous or SSE response shape any
    /// more, for any input. This client polls `GET /v1/jobs/{job_id}` with
    /// the CLI's exponential backoff (2s → 15s, [`crate::jobs_polling`])
    /// until the job reaches a terminal status, and surfaces a job failure
    /// or poll timeout as an `Err` rather than swallowing it — a silent
    /// failure here previously left the docs bootstrap ingest looking like
    /// a no-op.
    pub async fn ingest(&self, request: &IngestRequest) -> Result<IngestSummary> {
        let launch: JobLaunchResponse = self
            .send_typed(Method::POST, "/v1/ingest", Some(request))
            .await?;
        let (_kind, job) = await_job_terminal(
            self,
            &launch.job_id,
            PollConfig::waited(DEFAULT_INGEST_POLL_TIMEOUT_SECS),
        )
        .await?;
        Ok(IngestSummary {
            job_id: launch.job_id,
            status: job.status,
            documents_ingested: job.documents_ingested,
            documents_failed: job.documents_failed,
            error_message: job.error_message,
            warnings: job.warnings,
        })
    }

    /// `GET /v1/jobs/{job_id}` — poll a single job's current status.
    pub async fn get_job_status(&self, job_id: &str) -> Result<ImportJobStatus> {
        self.send_typed::<ImportJobStatus>(Method::GET, &format!("/v1/jobs/{job_id}"), NONE)
            .await
    }

    // -- Search --

    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResults> {
        self.send_typed::<SearchResults>(Method::POST, "/v1/search", Some(query))
            .await
    }

    /// Voice-tuned search (POST /v1/voices/search). Uses the voice's
    /// chunking+retrieval config rather than raw corpus defaults.
    pub async fn search_with_voice(&self, body: &SearchWithVoiceBody) -> Result<SearchResults> {
        self.send_typed::<SearchResults>(Method::POST, "/v1/voices/search", Some(body))
            .await
    }

    // -- Health --

    pub async fn ping(&self) -> Result<StatusCode> {
        let response = self
            .auth(self.http.get(self.url("/v1/corpora")))
            .send()
            .await?;
        if response.status().is_redirection() {
            return Err(redirect_error(&response));
        }
        Ok(response.status())
    }
}

const NONE: Option<&serde_json::Value> = None;

/// Best-effort host extracted from a 3xx response's `Location` header, for
/// error messages only. Handles both absolute and relative `Location`
/// values. Never returns the query string or full URL.
fn redirect_location_host(response: &reqwest::Response) -> Option<String> {
    let raw = response
        .headers()
        .get(reqwest::header::LOCATION)?
        .to_str()
        .ok()?;
    let url = reqwest::Url::parse(raw)
        .or_else(|_| response.url().join(raw))
        .ok()?;
    url.host_str().map(str::to_string)
}

/// Build the typed error for a 3xx response this client refuses to follow.
/// Call this BEFORE reading the response body.
fn redirect_error(response: &reqwest::Response) -> EnscriveError {
    EnscriveError::Redirected {
        status: response.status(),
        location_host: redirect_location_host(response)
            .unwrap_or_else(|| "an unspecified host".to_string()),
    }
}

impl crate::jobs_polling::JobPoller for EnscriveClient {
    fn get_job_status(
        &self,
        job_id: &str,
    ) -> impl std::future::Future<Output = Result<ImportJobStatus>> + Send {
        EnscriveClient::get_job_status(self, job_id)
    }
}

#[cfg(test)]
mod redirect_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener as StdTcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Spawn a one-shot mock HTTP server on a background thread. It accepts
    /// exactly one connection, reads until it sees the end of the request
    /// headers, records into `hit` that a connection was made, and writes
    /// back `raw_response` verbatim. Returns the address to connect to.
    fn spawn_mock(raw_response: String, hit: Arc<AtomicBool>) -> String {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let addr = listener.local_addr().expect("mock listener address");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                hit.store(true, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let mut seen = Vec::new();
                // Read until we've seen the end of the request headers (or
                // the peer closes / buffer fills); we don't need a real
                // HTTP parser for this test.
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                let _ = stream.write_all(raw_response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn redirect_is_refused_and_target_is_never_contacted() {
        let target_hit = Arc::new(AtomicBool::new(false));
        let target_addr = spawn_mock(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_string(),
            target_hit.clone(),
        );

        let redirect_hit = Arc::new(AtomicBool::new(false));
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}/v1/corpora\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_addr = spawn_mock(redirect_response, redirect_hit.clone());

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "test-secret-key");
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { status, .. }) => {
                assert_eq!(status.as_u16(), 302);
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
        assert!(redirect_hit.load(Ordering::SeqCst), "redirecting server was never hit");
        assert!(
            !target_hit.load(Ordering::SeqCst),
            "redirect target was contacted — X-API-Key may have been resent"
        );
    }

    #[tokio::test]
    async fn successful_response_is_returned_normally() {
        let hit = Arc::new(AtomicBool::new(false));
        let addr = spawn_mock(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]".to_string(),
            hit.clone(),
        );

        let client = EnscriveClient::new(format!("http://{addr}"), "test-secret-key");
        let result = client.list_corpora().await;

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(hit.load(Ordering::SeqCst));
    }
}
