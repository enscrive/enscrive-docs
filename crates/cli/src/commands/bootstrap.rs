//! `enscrive-docs bootstrap` — idempotent first-run setup.
//!
//! Creates any voices and collections from `enscrive-docs.toml` that do not
//! yet exist in the configured tenant, then runs a first ingest. Designed as
//! the one-shot "get a `/docs` site live" command for first-time setup.

use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::{
    Config, CreateCollectionRequest, CreateVoiceApiRequest, EnscriveClient, VoiceConfigApi,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Args, Clone, Debug)]
pub struct BootstrapArgs {
    /// Skip the first ingest after voices + collections are provisioned
    #[arg(long)]
    pub skip_ingest: bool,
}

pub async fn run(global: GlobalArgs, args: BootstrapArgs) -> Result<(), String> {
    let config_path = global.resolved_config_path();
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

    if cfg.voices.is_empty() {
        return Err(format!(
            "no [[voices]] defined in {}",
            config_path.display()
        ));
    }
    if cfg.collections.is_empty() {
        return Err(format!(
            "no [[collections]] defined in {}",
            config_path.display()
        ));
    }

    let api_key = cfg
        .resolved_api_key(global.api_key.as_deref())
        .map_err(|e| e.to_string())?;
    let endpoint = cfg.resolved_endpoint(global.endpoint.as_deref());
    let provider_key = cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
    let client = EnscriveClient::with_provider_key(endpoint, api_key, provider_key);

    // ---- Voices ----
    println!("bootstrap: reconciling voices");
    let existing_voices = client.list_voices().await.map_err(|e| e.to_string())?;
    let voice_by_name: HashMap<String, String> = existing_voices
        .iter()
        .map(|v| (v.name.clone(), v.id.clone()))
        .collect();
    for voice_cfg in &cfg.voices {
        if let Some(id) = voice_by_name.get(&voice_cfg.name) {
            println!("  [skip] voice \"{}\" already exists ({})", voice_cfg.name, id);
            continue;
        }
        let api_cfg = VoiceConfigApi {
            chunking_strategy: voice_cfg.chunking_strategy.clone(),
            parameters: voice_cfg.parameters.clone(),
            template_id: voice_cfg.template_id.clone(),
            score_threshold: voice_cfg.score_threshold,
            default_limit: voice_cfg.default_limit,
            description: voice_cfg.description.clone(),
            tags: Vec::new(),
        };
        let created = client
            .create_voice(&CreateVoiceApiRequest {
                name: voice_cfg.name.clone(),
                config: api_cfg,
            })
            .await
            .map_err(|e| format!("create voice \"{}\": {e}", voice_cfg.name))?;
        println!("  [create] voice \"{}\" -> {}", created.name, created.id);
    }

    // ---- Collections ----
    println!("bootstrap: reconciling collections");
    let existing_collections = client.list_collections().await.map_err(|e| e.to_string())?;
    let collection_by_name: HashMap<String, String> = existing_collections
        .iter()
        .map(|c| (c.name.clone(), c.id.clone()))
        .collect();
    for coll_cfg in &cfg.collections {
        if let Some(id) = collection_by_name.get(&coll_cfg.name) {
            println!(
                "  [skip] collection \"{}\" already exists ({})",
                coll_cfg.name, id
            );
            continue;
        }
        let embedding_model = coll_cfg.embedding_model.as_deref().ok_or_else(|| {
            format!(
                "collection \"{}\" does not exist and has no embedding_model in config; \
                 add `embedding_model = \"…\"` under [[collections]] so bootstrap can create it",
                coll_cfg.name
            )
        })?;
        let created = client
            .create_collection(&CreateCollectionRequest {
                name: coll_cfg.name.clone(),
                description: coll_cfg.description.clone(),
                embedding_model: embedding_model.to_string(),
                dimensions: coll_cfg.dimensions,
            })
            .await
            .map_err(|e| format!("create collection \"{}\": {e}", coll_cfg.name))?;
        println!(
            "  [create] collection \"{}\" -> {} (model: {})",
            created.name, created.id, created.model
        );
    }

    // ---- First ingest ----
    if args.skip_ingest {
        println!(
            "bootstrap: done. Run `enscrive-docs ingest` when ready to push {} collection(s).",
            cfg.collections.len()
        );
        return Ok(());
    }

    println!("bootstrap: first ingest");
    let _ = config_dir;
    crate::commands::ingest::run(
        global,
        crate::commands::ingest::IngestArgs {
            collection: None,
            dry_run: false,
            force: false,
        },
    )
    .await?;

    println!("bootstrap: done.");
    Ok(())
}
