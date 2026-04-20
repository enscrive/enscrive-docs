//! `enscrive-docs reset` — delete-and-rebuild a documentation collection.
//!
//! `/v1` has no bulk "delete all documents" endpoint, so reset is implemented
//! as a full collection delete followed by a recreate (via the same bootstrap
//! path). This is destructive; `--yes` is required.

use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::{Config, EnscriveClient};

#[derive(Args, Clone, Debug)]
pub struct ResetArgs {
    /// Limit to a single collection by name. Default: reset every collection
    /// in the config.
    #[arg(long)]
    pub collection: Option<String>,

    /// Skip the re-ingest step after the collection is recreated
    #[arg(long)]
    pub skip_ingest: bool,

    /// Confirm destructive action. Required.
    #[arg(long)]
    pub yes: bool,
}

pub async fn run(global: GlobalArgs, args: ResetArgs) -> Result<(), String> {
    if !args.yes {
        return Err(
            "`reset` deletes and recreates collections (embeddings are lost). \
             Re-run with --yes to proceed."
                .to_string(),
        );
    }

    let config_path = global.resolved_config_path();
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

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

    let existing = client.list_collections().await.map_err(|e| e.to_string())?;

    let mut targeted = 0usize;
    for coll_cfg in &cfg.collections {
        if let Some(only) = args.collection.as_deref()
            && coll_cfg.name != only
        {
            continue;
        }
        targeted += 1;
        let Some(existing_entry) = existing.iter().find(|c| c.name == coll_cfg.name) else {
            println!(
                "[{}] not present in tenant; nothing to delete",
                coll_cfg.name
            );
            continue;
        };
        println!(
            "[{}] deleting collection {}…",
            coll_cfg.name, existing_entry.id
        );
        let resp = client
            .delete_collection(&existing_entry.id)
            .await
            .map_err(|e| format!("delete collection \"{}\": {e}", coll_cfg.name))?;
        if !resp.deleted {
            return Err(format!(
                "delete collection \"{}\" returned deleted=false",
                coll_cfg.name
            ));
        }
        println!("[{}] deleted", coll_cfg.name);
    }

    if let Some(only) = args.collection.as_deref()
        && targeted == 0
    {
        return Err(format!(
            "--collection \"{only}\" not found in [[collections]]"
        ));
    }

    // Recreate + (optionally) re-ingest via bootstrap. Idempotent either way.
    println!("reset: re-running bootstrap to recreate collections");
    crate::commands::bootstrap::run(
        global,
        crate::commands::bootstrap::BootstrapArgs {
            skip_ingest: args.skip_ingest,
        },
    )
    .await?;

    Ok(())
}
