//! HTTP client for the public Enscrive API.
//!
//! Auth pattern (X-API-Key, optional X-Embedding-Provider-Key) and timeout
//! mirror enscrive-cli/src/client.rs to keep cross-CLI behavior consistent.

use crate::error::{EnscriveError, Result};
use crate::types::{
    CollectionDetail, CreateCollectionRequest, CreateVoiceApiRequest, DeleteCollectionResponse,
    DeleteVoiceResponse, IngestProgressEvent, IngestRequest, SearchQuery, SearchResults,
    SearchWithVoiceBody, UpdateVoiceApiRequest, VoiceDetail,
};
use futures_util::StreamExt;
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;
use tokio::sync::mpsc;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct EnscriveClient {
    http: Client,
    base_url: String,
    api_key: String,
    embedding_provider_key: Option<String>,
}

/// Streamed ingest progress, surfaced from the SSE response of POST /v1/ingest.
#[derive(Debug, Clone)]
pub enum IngestProgress {
    Event(IngestProgressEvent),
    Done,
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

    // -- Collections --

    pub async fn list_collections(&self) -> Result<Vec<CollectionDetail>> {
        self.send_typed::<Vec<CollectionDetail>>(Method::GET, "/v1/collections", NONE)
            .await
    }

    pub async fn get_collection(&self, id: &str) -> Result<CollectionDetail> {
        self.send_typed::<CollectionDetail>(
            Method::GET,
            &format!("/v1/collections/{id}"),
            NONE,
        )
        .await
    }

    pub async fn create_collection(
        &self,
        request: &CreateCollectionRequest,
    ) -> Result<CollectionDetail> {
        self.send_typed::<CollectionDetail>(Method::POST, "/v1/collections", Some(request))
            .await
    }

    pub async fn delete_collection(&self, id: &str) -> Result<DeleteCollectionResponse> {
        self.send_typed::<DeleteCollectionResponse>(
            Method::DELETE,
            &format!("/v1/collections/{id}"),
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

    /// Buffered (non-streaming) ingest. Returns all progress events as a vec.
    pub async fn ingest(&self, request: &IngestRequest) -> Result<Vec<IngestProgressEvent>> {
        let response = self
            .auth(self.http.post(self.url("/v1/ingest")))
            .json(request)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(EnscriveError::Http { status, body: text });
        }
        serde_json::from_str(&text).map_err(EnscriveError::from)
    }

    /// Streaming ingest via SSE. Returns a receiver that yields events as
    /// they arrive from the server.
    pub async fn ingest_stream(
        &self,
        request: &IngestRequest,
    ) -> Result<mpsc::Receiver<IngestProgress>> {
        let response = self
            .auth(self.http.post(self.url("/v1/ingest")))
            .header("Accept", "text/event-stream")
            .json(request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(EnscriveError::Http { status, body });
        }

        let (tx, rx) = mpsc::channel::<IngestProgress>(64);
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(b) => b,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(idx) = buffer.find("\n\n") {
                    let frame = buffer[..idx].to_string();
                    buffer.drain(..idx + 2);
                    for line in frame.lines() {
                        if let Some(payload) = line.strip_prefix("data:") {
                            let payload = payload.trim();
                            if payload.is_empty() {
                                continue;
                            }
                            if let Ok(event) =
                                serde_json::from_str::<IngestProgressEvent>(payload)
                            {
                                let _ = tx.send(IngestProgress::Event(event)).await;
                            }
                        }
                    }
                }
            }
            let _ = tx.send(IngestProgress::Done).await;
        });
        Ok(rx)
    }

    // -- Search --

    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResults> {
        self.send_typed::<SearchResults>(Method::POST, "/v1/search", Some(query))
            .await
    }

    /// Voice-tuned search (POST /v1/voices/search). Uses the voice's
    /// chunking+retrieval config rather than raw collection defaults.
    pub async fn search_with_voice(&self, body: &SearchWithVoiceBody) -> Result<SearchResults> {
        self.send_typed::<SearchResults>(Method::POST, "/v1/voices/search", Some(body))
            .await
    }

    // -- Health --

    pub async fn ping(&self) -> Result<StatusCode> {
        let response = self
            .auth(self.http.get(self.url("/v1/collections")))
            .send()
            .await?;
        Ok(response.status())
    }
}

const NONE: Option<&serde_json::Value> = None;
