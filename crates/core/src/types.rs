//! Subset of the public Enscrive API types we depend on.
//!
//! Source of truth: enscrive-developer/crates/types-api/src/lib.rs.
//! Field names and JSON shape match upstream. We define our own copies
//! here so this crate does not pull in the entire Enscrive workspace.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// -- Collections --

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollectionDetail {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub document_count: u64,
    #[serde(default)]
    pub embedding_count: u64,
    #[serde(default)]
    pub dimensions: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_voice_id: Option<String>,
    #[serde(default)]
    pub pending_count: u32,
    #[serde(default)]
    pub dirty: bool,
}

// -- Voices --

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoiceConfigApi {
    pub chunking_strategy: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default)]
    pub score_threshold: Option<f32>,
    #[serde(default)]
    pub default_limit: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoiceDetail {
    pub id: String,
    pub name: String,
    pub config: VoiceConfigApi,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreateVoiceApiRequest {
    pub name: String,
    pub config: VoiceConfigApi,
}

/// Body for `PUT /v1/voices/{id}` (full-replace semantics).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateVoiceApiRequest {
    pub config: VoiceConfigApi,
}

/// Response from `DELETE /v1/voices/{id}`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteVoiceResponse {
    pub deleted: bool,
    pub voice_id: String,
}

// -- Create / Delete collection --

/// Body for `POST /v1/collections`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreateCollectionRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub embedding_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
}

/// Response from `DELETE /v1/collections/{id}`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteCollectionResponse {
    pub deleted: bool,
    pub collection_id: String,
}

// -- Ingest --

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IngestDocument {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IngestRequest {
    pub collection_id: String,
    pub documents: Vec<IngestDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_batch: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IngestProgressEvent {
    pub document_id: String,
    pub status: String,
    #[serde(default)]
    pub chunks_created: Option<u32>,
    #[serde(default)]
    pub embeddings_stored: Option<u32>,
    #[serde(default)]
    pub tokens_used: Option<u32>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub progress_percent: f32,
    #[serde(default)]
    pub chunks_unchanged: Option<u32>,
}

// -- Search --

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SearchQuery {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<SearchFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
    pub include_vectors: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oversample_factor: Option<u32>,
    #[serde(default)]
    pub extended_results: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_floor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid_alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SearchFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchResultItem {
    pub id: String,
    pub document_id: String,
    pub collection_id: String,
    pub score: f32,
    pub content: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub chunk_index: Option<u32>,
    #[serde(default)]
    pub below_threshold: bool,
}

/// Request body for POST /v1/voices/search — voice-tuned search.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchWithVoiceBody {
    pub query: String,
    pub voice_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    pub include_vectors: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<SearchFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oversample_factor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
    #[serde(default)]
    pub extended_results: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_floor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid_alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SearchResults {
    pub results: Vec<SearchResultItem>,
    #[serde(default)]
    pub query_vector: Option<Vec<f32>>,
    #[serde(default)]
    pub search_time_ms: u64,
    #[serde(default)]
    pub embed_time_ms: u64,
    #[serde(default)]
    pub total_candidates: u32,
    #[serde(default)]
    pub applied_granularity: Option<String>,
    #[serde(default)]
    pub applied_dimensions: Option<u32>,
    #[serde(default)]
    pub threshold_applied: f32,
    #[serde(default)]
    pub results_above_threshold: i32,
}
