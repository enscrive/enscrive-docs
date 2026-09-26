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
/// in an already-lowercased hostname, per the fleet ENS-6483 redaction
/// spec (~/work/sec-wave/w3-research/ENS-6483-redaction-spec.md), rule 2
/// and rule 3:
///
/// - a common provider/platform-key prefix, BOUNDARY-anchored — at the
///   start of the host, or immediately after any non-alphanumeric
///   character (not just `.`, which is what the previous per-label
///   `starts_with` check amounted to) — followed by at least
///   `MIN_KEY_SUFFIX_LEN` more `[a-z0-9_-]` characters. The trailing run
///   is the entropy signal: a bare prefix with nothing after it isn't
///   itself suspicious.
/// - this product's own key shape instead: `enscrive_` + exactly 8 hex
///   digits + `_`. That whole fixed form is self-contained (the 8 hex
///   digits are the entropy signal) and needs no extra trailing length.
/// - OR a run of `MIN_OPAQUE_RUN_LEN`+ consecutive `[a-z0-9_]` characters
///   anywhere in the host — `.` and `-` break the run (an ordinary
///   hyphenated hostname can otherwise easily run past 32 total alnum
///   characters).
///
/// Boundary anchoring (not "starts a DNS label") is what makes
/// `prefix-sk-<16+ chars>` count as a match — the hyphen before `sk-` is a
/// boundary too — while a host like `network-edge.example.com` or an ALB
/// name like `my-keycloak-loadbalancer-1234567890.us-east-1.elb.amazonaws.com`
/// stays named: the `rk-`/`sk-`-shaped substrings inside them
/// ("netwo**rk-**edge") follow an alphanumeric character, not a boundary,
/// and none of their hyphen/dot-separated segments reach the suffix or
/// run-length thresholds.
fn host_looks_credential_shaped(host_lower: &str) -> bool {
    // The fleet spec's generic vendor/platform prefixes (a plain prefix
    // string; each needs MIN_KEY_SUFFIX_LEN more suffix characters to
    // count — see below).
    const KEY_PREFIXES: &[&str] = &[
        "sk-ant-",
        "sk-proj-",
        "sk-",
        "rk-",
        "pk-",
        "xoxb-",
        "xoxp-",
        "ghp_",
        "gho_",
        "github_pat_",
        "aiza",
        "npg_",
    ];
    const MIN_KEY_SUFFIX_LEN: usize = 16;
    const MIN_OPAQUE_RUN_LEN: usize = 32;
    // This product's own key shape: `enscrive_<8 hex>_`, a fixed,
    // self-contained form handled separately from the generic prefixes.
    const ENSCRIVE_PREFIX: &str = "enscrive_";
    const ENSCRIVE_ID_LEN: usize = 8;

    let is_boundary = |c: char| !c.is_ascii_alphanumeric();
    let is_key_suffix_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';

    let mut prev_char: Option<char> = None;
    for (i, c) in host_lower.char_indices() {
        let at_boundary = match prev_char {
            None => true,
            Some(p) => is_boundary(p),
        };
        prev_char = Some(c);
        if !at_boundary {
            continue;
        }
        let rest = &host_lower[i..];

        if let Some(after) = rest.strip_prefix(ENSCRIVE_PREFIX) {
            let hex_len = after
                .chars()
                .take(ENSCRIVE_ID_LEN)
                .take_while(char::is_ascii_hexdigit)
                .count();
            if hex_len == ENSCRIVE_ID_LEN && after.as_bytes().get(ENSCRIVE_ID_LEN) == Some(&b'_') {
                return true;
            }
        }

        for prefix in KEY_PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                let suffix_len = after.chars().take_while(|&c| is_key_suffix_char(c)).count();
                if suffix_len >= MIN_KEY_SUFFIX_LEN {
                    return true;
                }
            }
        }
    }

    let mut run = 0usize;
    for c in host_lower.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            run += 1;
            if run >= MIN_OPAQUE_RUN_LEN {
                return true;
            }
        } else {
            // Includes '-': a hyphen breaks a run exactly like a `.`
            // label boundary would.
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
    /// doesn't echo the live key at all — a shape with enough trailing
    /// characters (rule 2's generic-prefix branch, kept under 32 total so
    /// rule 3 can't be what's actually catching it), a boundary-anchored
    /// shape following a hyphen rather than a label start (the ENS-6483 M
    /// fix — this escaped the old per-label `starts_with` check), and this
    /// product's own fixed `enscrive_<8 hex>_` shape.
    #[tokio::test]
    async fn redirect_error_redacts_known_key_shapes() {
        for shaped_host in [
            "sk-ant-api03-abcdefghijklmnop.x.example",
            "prefix-sk-abcdefghijklmnopqrst.example",
            "enscrive_1a2b3c4d_x.evil.example",
        ] {
            let redirect_response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{shaped_host}/steal\r\nContent-Length: 0\r\n\r\n"
            );
            let redirect_calls = Arc::new(AtomicUsize::new(0));
            let redirect_addr = spawn_mock(redirect_response, redirect_calls);

            let client = EnscriveClient::new(format!("http://{redirect_addr}"), "unrelated-key");
            let result = client.list_corpora().await;

            match result {
                Err(EnscriveError::Redirected { location_host, .. }) => {
                    assert_eq!(
                        location_host, "<redacted host>",
                        "expected shape-based redaction for {shaped_host}"
                    );
                }
                other => panic!(
                    "expected Err(EnscriveError::Redirected) for {shaped_host}, got {other:?}"
                ),
            }
        }
    }

    /// ENS-6483 R1 L1: a key-shape prefix with FEWER than 16 trailing
    /// `[a-z0-9_-]` characters is not, on its own, a strong enough signal —
    /// `sk-ant-api03-abc` only has 9 (`api03-abc`) before the `.`. Per
    /// spec this host is named unless it separately echoes a live
    /// credential, which the second half of this test proves by reusing
    /// the exact same host STRING as the client's real API key.
    #[tokio::test]
    async fn short_key_shape_is_named_unless_it_is_also_a_live_credential() {
        let short_shape_host = "sk-ant-api03-abc.x.example";

        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{short_shape_host}/steal\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);
        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "unrelated-key");
        match client.list_corpora().await {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(
                    location_host, short_shape_host,
                    "a too-short key shape must not be redacted by shape alone"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }

        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{short_shape_host}/steal\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);
        // Same host string, but now it IS the live API key (rule 1, not
        // rule 2 — this must still redact).
        let client = EnscriveClient::new(format!("http://{redirect_addr}"), short_shape_host);
        match client.list_corpora().await {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(
                    location_host, "<redacted host>",
                    "a live credential must redact even when its shape alone would not"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }

    /// ENS-6483 R1 L1: the opaque-run threshold is 32, not 24 — a run of
    /// exactly 31 stays named, 32 is redacted, with no key-shape prefix
    /// and no live-credential overlap so only rule 3 can be responsible
    /// either way.
    #[tokio::test]
    async fn opaque_run_boundary_is_32_not_24() {
        let run_31 = "a".repeat(31);
        let run_32 = "a".repeat(32);

        for (run, expect_redacted) in [(run_31, false), (run_32, true)] {
            let host = format!("{run}.example.com");
            let redirect_response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{host}/steal\r\nContent-Length: 0\r\n\r\n"
            );
            let redirect_calls = Arc::new(AtomicUsize::new(0));
            let redirect_addr = spawn_mock(redirect_response, redirect_calls);
            let client = EnscriveClient::new(format!("http://{redirect_addr}"), "unrelated-key");

            match client.list_corpora().await {
                Err(EnscriveError::Redirected { location_host, .. }) => {
                    if expect_redacted {
                        assert_eq!(
                            location_host, "<redacted host>",
                            "a run of {} chars should be redacted",
                            run.len()
                        );
                    } else {
                        assert_eq!(
                            location_host, host,
                            "a run of {} chars should stay named",
                            run.len()
                        );
                    }
                }
                other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
            }
        }
    }

    /// An ordinary hyphenated hostname — an ALB name or a plain
    /// "network-edge"-style host — must NOT be redacted. Before the
    /// hyphen-breaks-a-run fix, both tripped the heuristic: the ALB name's
    /// hyphenated segments run past 24 alnum characters when hyphens don't
    /// break the count (now 32, and still broken by '-'/'.' either way).
    /// "network-edge" never matched the prefix check even before this fix
    /// (its "rk-" substring is mid-word, not label-initial), but it stays
    /// a useful regression case for the boundary-anchoring behavior too.
    #[tokio::test]
    async fn redirect_error_does_not_redact_ordinary_hyphenated_hosts() {
        for ordinary_host in [
            "my-keycloak-loadbalancer-1234567890.us-east-1.elb.amazonaws.com",
            "network-edge.example.com",
        ] {
            let redirect_response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{ordinary_host}/steal\r\nContent-Length: 0\r\n\r\n"
            );
            let redirect_calls = Arc::new(AtomicUsize::new(0));
            let redirect_addr = spawn_mock(redirect_response, redirect_calls);

            let client = EnscriveClient::new(format!("http://{redirect_addr}"), "unrelated-key");
            let result = client.list_corpora().await;

            match result {
                Err(EnscriveError::Redirected { location_host, .. }) => {
                    assert_eq!(
                        location_host, ordinary_host,
                        "expected {ordinary_host} to be named, not redacted"
                    );
                }
                other => panic!(
                    "expected Err(EnscriveError::Redirected) for {ordinary_host}, got {other:?}"
                ),
            }
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

    /// ENS-6483 R1 L2(a): `ping()` has its own 3xx check, separate from
    /// `send_typed`'s — this proves it independently, the same way
    /// `redirect_is_refused_and_target_is_never_contacted` proves it for
    /// `send_typed` via `list_corpora()`. Without a dedicated test, a
    /// change that deleted `ping()`'s own `is_redirection()` branch would
    /// leave every other test passing while `ping()` silently reported a
    /// 3xx as a successful status.
    #[tokio::test]
    async fn ping_refuses_a_redirect_and_never_contacts_the_target() {
        let target_calls = Arc::new(AtomicUsize::new(0));
        let target_addr = spawn_mock(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_string(),
            target_calls.clone(),
        );

        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_addr}/v1/corpora\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_addr = spawn_mock(redirect_response, redirect_calls.clone());

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "test-secret-key");
        let result = client.ping().await;

        match result {
            Err(EnscriveError::Redirected { status, .. }) => {
                assert_eq!(status.as_u16(), 307);
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
            "ping()'s redirect target was contacted — X-API-Key may have been resent"
        );
    }

    /// ENS-6483 R1 L2(b): the embedding-provider/BYOK key must be
    /// redacted the same way the API key is — `with_provider_key` passes
    /// it into `credentials` alongside `api_key` (see `send_typed`), but
    /// nothing previously exercised that specific slot. Deliberately
    /// leaves the Location host free of the API key, so this can only
    /// pass because the provider key is checked too.
    #[tokio::test]
    async fn redirect_error_redacts_a_host_that_contains_the_provider_key() {
        let live_provider_key = "provider-byok-secret-99";
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{live_provider_key}.attacker.example/steal\r\nContent-Length: 0\r\n\r\n"
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);

        let client = EnscriveClient::with_provider_key(
            format!("http://{redirect_addr}"),
            "unrelated-api-key",
            Some(live_provider_key),
        );
        let result = client.list_corpora().await;

        match result {
            Err(EnscriveError::Redirected { location_host, .. }) => {
                assert_eq!(location_host, "<redacted host>");
                assert!(
                    !location_host.contains(live_provider_key),
                    "the live provider key leaked into the error message"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }

    /// ENS-6483 R1 L2(c): `EnscriveError::Redirected` carries only
    /// `status` and `location_host` (see error.rs) — a 3xx response body
    /// can never reach it structurally. This test still gives that
    /// invariant an explicit regression check: the mock redirect response
    /// carries a body with a sentinel value a credential-bearing payload
    /// might contain, and neither the error's Display nor its Debug
    /// output may ever contain it.
    #[tokio::test]
    async fn redirected_error_never_carries_the_3xx_body() {
        let sentinel = "leaked-3xx-body-sentinel-do-not-log-me";
        let body = format!("{{\"error\":\"see other\",\"detail\":\"{sentinel}\"}}");
        let redirect_response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/steal\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let redirect_calls = Arc::new(AtomicUsize::new(0));
        let redirect_addr = spawn_mock(redirect_response, redirect_calls);

        let client = EnscriveClient::new(format!("http://{redirect_addr}"), "test-secret-key");
        let result = client.list_corpora().await;

        match result {
            Err(err @ EnscriveError::Redirected { .. }) => {
                let displayed = err.to_string();
                let debugged = format!("{err:?}");
                assert!(
                    !displayed.contains(sentinel),
                    "3xx body sentinel leaked into the Display message: {displayed}"
                );
                assert!(
                    !debugged.contains(sentinel),
                    "3xx body sentinel leaked into the Debug message: {debugged}"
                );
            }
            other => panic!("expected Err(EnscriveError::Redirected), got {other:?}"),
        }
    }
}
