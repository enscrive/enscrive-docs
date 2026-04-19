//! Core types, HTTP client, and config loader for the enscrive-docs CLI.
//!
//! This crate is published as a library so other Rust applications can call
//! the public Enscrive API and load enscrive-docs config files without
//! depending on the rendering layer or the CLI binary.

pub mod client;
pub mod config;
pub mod error;
pub mod types;

pub use client::{EnscriveClient, IngestProgress};
pub use config::{
    CollectionConfig, Config, EnscriveAuthConfig, SearchConfig, ServeConfig, SiteConfig,
    ThemeConfig, VersionConfig, VoiceConfig,
};
pub use error::{EnscriveError, Result};
pub use types::{
    CollectionDetail, CreateVoiceApiRequest, IngestDocument, IngestProgressEvent, IngestRequest,
    SearchFilter, SearchQuery, SearchResultItem, SearchResults, VoiceConfigApi, VoiceDetail,
};
