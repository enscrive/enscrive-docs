//! Core types, HTTP client, and config loader for the enscrive-docs CLI.
//!
//! This crate is published as a library so other Rust applications can call
//! the public Enscrive API and load enscrive-docs config files without
//! depending on the rendering layer or the CLI binary.

pub mod client;
pub mod config;
pub mod error;
pub mod jobs_polling;
pub mod types;

pub use client::EnscriveClient;
pub use config::{
    CorpusConfig, Config, EnscriveAuthConfig, ReturnConfig, SearchConfig, ServeConfig,
    SiteConfig, ThemeConfig, VersionConfig, VoiceConfig,
};
pub use error::{EnscriveError, Result};
pub use types::{
    CorpusDetail, CreateCorpusRequest, CreateVoiceApiRequest, DeleteCorpusResponse,
    DeleteVoiceResponse, ImportJobStatus, IngestDocument, IngestRequest, IngestSummary,
    JobLaunchResponse, SearchFilter, SearchQuery, SearchResultItem, SearchResults,
    SearchWithVoiceBody, UpdateVoiceApiRequest, VoiceConfigApi, VoiceDetail,
};
