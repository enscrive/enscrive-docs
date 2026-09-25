//! HTTP client for the public Enscrive API.
//!
//! Auth pattern (X-API-Key, optional X-Embedding-Provider-Key) and timeout
//! mirror enscrive-cli/src/client.rs to keep cross-CLI behavior consistent.

use crate::error::{EnscriveError, Result};
use crate::jobs_polling::{PollConfig, await_job_terminal};
use crate::types::{
    CorpusDetail, CreateCorpusRequest, CreateVoiceApiRequest, DeleteCorpusResponse,
    DeleteVoiceResponse, ImportJobStatus, IngestRequest, IngestSummary, JobLaunchResponse,
    SearchQuery, SearchResults, SearchWithVoiceBody, UpdateVoiceApiRequest, VoiceDetail,
};
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
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
            return Err(redirect_error(
                &response,
                &[
                    self.api_key.as_str(),
                    self.embedding_provider_key.as_deref().unwrap_or(""),
                ],
            ));
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
        self.send_typed::<CorpusDetail>(Method::GET, &format!("/v1/corpora/{id}"), NONE)
            .await
    }

    pub async fn create_corpus(&self, request: &CreateCorpusRequest) -> Result<CorpusDetail> {
        self.send_typed::<CorpusDetail>(Method::POST, "/v1/corpora", Some(request))
            .await
    }

    pub async fn delete_corpus(&self, id: &str) -> Result<DeleteCorpusResponse> {
        self.send_typed::<DeleteCorpusResponse>(Method::DELETE, &format!("/v1/corpora/{id}"), NONE)
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
        self.send_typed::<VoiceDetail>(Method::PUT, &format!("/v1/voices/{id}"), Some(request))
            .await
    }

    pub async fn delete_voice(&self, id: &str) -> Result<DeleteVoiceResponse> {
        self.send_typed::<DeleteVoiceResponse>(Method::DELETE, &format!("/v1/voices/{id}"), NONE)
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
            return Err(redirect_error(
                &response,
                &[
                    self.api_key.as_str(),
                    self.embedding_provider_key.as_deref().unwrap_or(""),
                ],
            ));
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
/// Call this BEFORE reading the response body. `credentials` are the live
/// credentials this client carries (`api_key`, and `embedding_provider_key`
/// if set) — if the `Location` host happens to contain one of them (e.g. a
/// misconfigured redirect target echoing it back), the host is redacted to
/// `<redacted host>` instead. The comparison is case-insensitive:
/// `url::Host::host_str()` always lowercases the host, so a case-sensitive
/// comparison would silently miss a mixed-case live credential. The host
/// is also redacted if it merely LOOKS credential-shaped (a known
/// provider/platform-key prefix, or a long run of key-alphabet
/// characters), even when none of `credentials` matches — defense in
/// depth for callers that don't have the exact live value handy.
fn redirect_error(response: &reqwest::Response, credentials: &[&str]) -> EnscriveError {
    let mut host =
        redirect_location_host(response).unwrap_or_else(|| "an unspecified host".to_string());
    let host_lower = host.to_ascii_lowercase();
    let echoes_credential = credentials
        .iter()
        .any(|c| !c.is_empty() && host_lower.contains(&c.to_ascii_lowercase()));
    if echoes_credential || host_looks_credential_shaped(&host_lower) {
        host = "<redacted host>".to_string();
    }
    EnscriveError::Redirected {
        status: response.status(),
        location_host: host,
    }
}

/// Best-effort, dependency-free detection of a credential-shaped substring
/// in an already-lowercased hostname: a common provider/platform-key
/// prefix (including this product's own `enscrive_` key prefix), or a run
/// of 24+ consecutive token-alphabet characters within one DNS label
/// (ordinary hostname labels don't run that long without a `.` or `-`
/// break; a bearer/API-key value typically does).
fn host_looks_credential_shaped(host_lower: &str) -> bool {
    const KEY_PREFIXES: &[&str] = &[
        "enscrive_",
        "sk-",
        "sk_",
        "sk-ant-",
        "pk-",
        "rk-",
        "aiza",
        "ya29.",
        "glpat-",
        "gho_",
        "ghp_",
        "ghu_",
        "ghs_",
        "xox",
    ];
    if KEY_PREFIXES.iter().any(|p| host_lower.contains(p)) {
        return true;
    }
    let mut run = 0usize;
    for c in host_lower.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            run += 1;
            if run >= 24 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Spawn a mock HTTP server on a background thread. It accepts
    /// connections (one per call unless `repeat` is true), counts them
    /// into `calls`, reads until it sees the end of the request headers,
    /// and writes back `raw_response` verbatim. Returns the address to
    /// connect to.
    fn spawn_mock(raw_response: String, calls: Arc<AtomicUsize>) -> String {
        spawn_mock_inner(raw_response, calls, false)
    }

    /// Same as `spawn_mock`, but keeps accepting connections instead of
    /// stopping after one — used by the positive-control test, which
    /// deliberately points TWO different clients at the same fixture.
    fn spawn_mock_repeating(raw_response: String, calls: Arc<AtomicUsize>) -> String {
        spawn_mock_inner(raw_response, calls, true)
    }

    fn spawn_mock_inner(raw_response: String, calls: Arc<AtomicUsize>, repeat: bool) -> String {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let addr = listener.local_addr().expect("mock listener address");
        std::thread::spawn(move || {
            loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                calls.fetch_add(1, Ordering::SeqCst);
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
                if !repeat {
                    break;
                }
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn redirect_is_refused_and_target_is_never_contacted() {
        let target_calls = Arc::new(AtomicUsize::new(0));
        let target_addr = spawn_mock(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_string(),
            target_calls.clone(),
        );

        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}/v1/corpora\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_addr = spawn_mock(redirect_response, redirect_calls.clone());

        // ENS-6483: EnscriveClient::new is the production constructor, and
        // list_corpora() drives the real send_typed()/redirect_error()
        // fail-closed path — not a synthetic status check — so deleting
        // that branch would fail this test.
        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "test-secret-key");
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { status, .. }) => {
                assert_eq!(status.as_u16(), 302);
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
        assert_eq!(
            redirect_calls.load(Ordering::SeqCst),
            1,
            "redirecting server was never hit"
        );
        assert_eq!(
            target_calls.load(Ordering::SeqCst),
            0,
            "redirect target was contacted — X-API-Key may have been resent"
        );
    }

    /// Proves the fixture in the test above is not just silently
    /// broken/unreachable: pointed at the SAME kind of redirect+target
    /// pair, a plain default-policy client (which DOES follow redirects)
    /// registers `target_calls == 1`. This is what makes `target_calls ==
    /// 0` in the negative control above a meaningful assertion rather than
    /// a vacuous one. Deliberately NOT `EnscriveClient` — its whole point
    /// is refusing to follow, so a different, ordinary following client is
    /// used here to validate the mock fixture itself.
    #[tokio::test]
    async fn following_client_against_the_same_fixture_proves_the_detector_works() {
        let target_calls = Arc::new(AtomicUsize::new(0));
        let target_addr = spawn_mock_repeating(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_string(),
            target_calls.clone(),
        );

        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}/v1/corpora\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_addr = spawn_mock(redirect_response, redirect_calls.clone());

        let following_client = reqwest::Client::new();
        let resp = following_client
            .get(format!("http://{redirect_addr}/v1/corpora"))
            .send()
            .await
            .expect("a following client should reach the target");

        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(redirect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            target_calls.load(Ordering::SeqCst),
            1,
            "the fixture's target must be reachable when a client actually follows"
        );
    }

    /// ENS-6483: if a redirect Location happens to echo the credential back
    /// (e.g. a misconfigured target), the error must redact the host
    /// rather than print it — never let the credential value itself reach
    /// a log or an error message.
    #[tokio::test]
    async fn redirect_error_redacts_a_host_that_contains_the_api_key() {
        let live_key = "test-secret-key-42";
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{live_key}.attacker.example/steal\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), live_key);
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(location_host, "<redacted host>");
                assert!(
                    !location_host.contains(live_key),
                    "the live key leaked into the error message"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }

    /// Same as above, but the live key's case differs from the host's
    /// (hosts are always lowercased by `url::Host`), proving the
    /// comparison is case-insensitive.
    #[tokio::test]
    async fn redirect_error_redacts_case_insensitively() {
        let live_key = "Test-Secret-KEY-42";
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}.attacker.example/steal\r\nContent-Length: 0\r\n\r\n",
            live_key.to_ascii_lowercase()
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), live_key);
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(
                    location_host, "<redacted host>",
                    "expected case-insensitive redaction"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }

    /// A host that merely LOOKS credential-shaped is redacted even when it
    /// doesn't echo the live key at all.
    #[tokio::test]
    async fn redirect_error_redacts_a_known_key_shape() {
        let redirect_response = "HTTP/1.1 302 Found\r\nLocation: http://sk-ant-api03-fakeAnthropicShapedValue.attacker.example/steal\r\nContent-Length: 0\r\n\r\n".to_string();
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "unrelated-key");
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(
                    location_host, "<redacted host>",
                    "expected shape-based redaction"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn successful_response_is_returned_normally() {
        let calls = Arc::new(AtomicUsize::new(0));
        let addr = spawn_mock(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]".to_string(),
            calls.clone(),
        );

        let client = EnscriveClient::new(format!("http://{addr}"), "test-secret-key");
        let result = client.list_corpora().await;

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
