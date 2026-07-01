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
        Ok(response.status())
    }
}

const NONE: Option<&serde_json::Value> = None;

impl crate::jobs_polling::JobPoller for EnscriveClient {
    fn get_job_status(
        &self,
        job_id: &str,
    ) -> impl std::future::Future<Output = Result<ImportJobStatus>> + Send {
        EnscriveClient::get_job_status(self, job_id)
    }
}
