use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::{
    CollectionConfig, Config, EnscriveClient, IngestDocument, IngestRequest, VoiceConfig,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Args, Clone, Debug)]
pub struct IngestArgs {
    /// Limit ingest to one configured collection by name
    #[arg(long)]
    pub collection: Option<String>,

    /// Walk the file tree but do not POST to /v1/ingest
    #[arg(long)]
    pub dry_run: bool,

    /// Force re-ingest even when the server's fingerprint matches
    #[arg(long)]
    pub force: bool,
}

pub async fn run(global: GlobalArgs, args: IngestArgs) -> Result<(), String> {
    let config_path = global.resolved_config_path();
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

    if cfg.collections.is_empty() {
        return Err(format!(
            "no [[collections]] defined in {}",
            config_path.display()
        ));
    }

    // For --dry-run we skip the API entirely so the command is usable offline
    // for previewing what would be ingested.
    let remote = if args.dry_run {
        None
    } else {
        let api_key = cfg
            .resolved_api_key(global.api_key.as_deref())
            .map_err(|e| e.to_string())?;
        let endpoint = cfg.resolved_endpoint(global.endpoint.as_deref());
        let provider_key =
            cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
        let client = EnscriveClient::with_provider_key(endpoint, api_key, provider_key);
        let collections = client.list_collections().await.map_err(|e| e.to_string())?;
        let voices = client.list_voices().await.map_err(|e| e.to_string())?;
        Some((client, collections, voices))
    };

    let mut total_docs = 0usize;
    let mut total_collections = 0usize;
    for entry in &cfg.collections {
        if let Some(only) = args.collection.as_deref() {
            if entry.name != only {
                continue;
            }
        }
        let voice_cfg = cfg
            .voices
            .iter()
            .find(|v| v.name == entry.voice)
            .ok_or_else(|| {
                format!(
                    "collection \"{}\" references voice \"{}\" which is not in [[voices]]",
                    entry.name, entry.voice
                )
            })?;
        let docs = build_documents(&config_dir, entry, voice_cfg)?;
        if docs.is_empty() {
            println!(
                "[{}] no markdown files found at {} (glob: {})",
                entry.name,
                entry.path.display(),
                entry.glob
            );
            continue;
        }

        if args.dry_run {
            println!(
                "[{}] (dry-run) {} document(s) for voice \"{}\"",
                entry.name,
                docs.len(),
                entry.voice
            );
            for doc in &docs {
                println!(
                    "  {} ({} bytes, fp={})",
                    doc.id,
                    doc.content.len(),
                    &doc.fingerprint[..16.min(doc.fingerprint.len())]
                );
            }
            total_docs += docs.len();
            total_collections += 1;
            continue;
        }

        let (client, collections, voices) = remote
            .as_ref()
            .expect("remote must exist when not dry-run");
        let collection_id = collections
            .iter()
            .find(|c| c.name == entry.name)
            .map(|c| c.id.clone())
            .ok_or_else(|| {
                format!(
                    "Enscrive collection \"{}\" not found. Create it via the Enscrive UI or `enscrive collections create` before ingesting.",
                    entry.name
                )
            })?;
        let voice_id = voices
            .iter()
            .find(|v| v.name == entry.voice)
            .map(|v| v.id.clone())
            .ok_or_else(|| {
                format!(
                    "Enscrive voice \"{}\" not found. Create it before ingesting.",
                    entry.voice
                )
            })?;

        println!(
            "[{}] {} document(s) -> collection {} (voice: {})",
            entry.name,
            docs.len(),
            collection_id,
            entry.voice
        );

        let req = IngestRequest {
            collection_id,
            documents: docs,
            voice_id: Some(voice_id),
            dry_run: false,
            sync: Some(true),
            no_batch: None,
        };

        match client.ingest(&req).await {
            Ok(events) => {
                let succeeded = events
                    .iter()
                    .filter(|e| e.error_message.is_none())
                    .count();
                let failed = events.len() - succeeded;
                println!(
                    "  ingested: {} ok / {} failed ({} events total)",
                    succeeded,
                    failed,
                    events.len()
                );
                for ev in events.iter().filter(|e| e.error_message.is_some()) {
                    if let Some(err) = ev.error_message.as_deref() {
                        println!("  ! {}: {err}", ev.document_id);
                    }
                }
                total_docs += succeeded;
                total_collections += 1;
            }
            Err(e) => {
                return Err(format!("ingest \"{}\" failed: {e}", entry.name));
            }
        }
    }

    println!(
        "\ndone: {total_docs} document(s) across {total_collections} collection(s)"
    );
    Ok(())
}

fn build_documents(
    config_dir: &Path,
    collection: &CollectionConfig,
    _voice: &VoiceConfig,
) -> Result<Vec<IngestDocument>, String> {
    let root = if collection.path.is_absolute() {
        collection.path.clone()
    } else {
        config_dir.join(&collection.path)
    };
    if !root.exists() {
        return Err(format!(
            "collection \"{}\" path does not exist: {}",
            collection.name,
            root.display()
        ));
    }

    let extension_filter = derive_extension_from_glob(&collection.glob);
    let mut docs = Vec::new();

    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Some(ext) = extension_filter.as_deref() {
            match entry.path().extension().and_then(|e| e.to_str()) {
                Some(actual) if actual.eq_ignore_ascii_case(ext) => {}
                _ => continue,
            }
        }

        let path = entry.path();
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let id = relative_doc_id(&root, path);
        let mut metadata = HashMap::new();
        metadata.insert("source_path".to_string(), id.clone());
        if let Some(prefix) = &collection.url_prefix {
            metadata.insert("url_prefix".to_string(), prefix.clone());
        }
        let fingerprint = fingerprint_content(&content);
        docs.push(IngestDocument {
            id,
            content,
            metadata,
            fingerprint,
        });
    }

    docs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(docs)
}

/// Extract a single extension hint from a simple glob pattern like `**/*.md`.
/// Returns None for patterns we cannot trivially interpret (no extension filter
/// is applied in that case — the caller falls back to walking everything).
fn derive_extension_from_glob(glob: &str) -> Option<String> {
    let trimmed = glob.trim();
    if let Some(idx) = trimmed.rfind('.') {
        let ext = &trimmed[idx + 1..];
        if !ext.is_empty()
            && ext
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(ext.to_string());
        }
    }
    None
}

fn relative_doc_id(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

fn fingerprint_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
