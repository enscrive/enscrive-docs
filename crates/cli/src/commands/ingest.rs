use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::redact::redact_excerpt;
use enscrive_docs_core::{
    Config, CorpusConfig, EnscriveClient, IngestDocument, IngestRequest, VoiceConfig,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Args, Clone, Debug)]
pub struct IngestArgs {
    /// Limit ingest to one configured corpus by name
    #[arg(long)]
    pub corpus: Option<String>,

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

    if cfg.corpora.is_empty() {
        return Err(format!(
            "no [[corpora]] defined in {}",
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
        let provider_key = cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
        let live_credentials = vec![api_key.clone(), provider_key.clone().unwrap_or_default()];
        let client = EnscriveClient::with_provider_key(endpoint, api_key, provider_key);
        let corpora = client.list_corpora().await.map_err(|e| e.to_string())?;
        let voices = client.list_voices().await.map_err(|e| e.to_string())?;
        Some((client, corpora, voices, live_credentials))
    };

    let mut total_docs = 0usize;
    let mut total_corpora = 0usize;
    for entry in &cfg.corpora {
        if args
            .corpus
            .as_deref()
            .is_some_and(|only| entry.name != only)
        {
            continue;
        }
        let voice_cfg = cfg
            .voices
            .iter()
            .find(|v| v.name == entry.voice)
            .ok_or_else(|| {
                format!(
                    "corpus \"{}\" references voice \"{}\" which is not in [[voices]]",
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
            total_corpora += 1;
            continue;
        }

        let (client, corpora, voices, live_credentials) =
            remote.as_ref().expect("remote must exist when not dry-run");
        let credentials = live_credentials
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let corpus_id = corpora
            .iter()
            .find(|c| c.name == entry.name)
            .map(|c| c.id.clone())
            .ok_or_else(|| {
                format!(
                    "Enscrive corpus \"{}\" not found. Create it via the Enscrive UI or `enscrive corpus create` before ingesting.",
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
            "[{}] {} document(s) -> corpus {} (voice: {})",
            entry.name,
            docs.len(),
            redact_excerpt(&corpus_id, &credentials, 200),
            entry.voice
        );

        // `sync`/SSE are retired (async-by-default, ENS-628): the server
        // accepts-and-ignores them and always answers 202 + JobLaunchResponse,
        // so the client no longer asks for either — `client.ingest()` posts,
        // then polls `GET /v1/jobs/{job_id}` to terminal itself.
        let req = ingest_request(corpus_id, docs, voice_id);

        println!("  job launched, polling to terminal...");
        match client.ingest(&req).await {
            Ok(summary) => {
                println!(
                    "  ingested: {} ok / {} failed (job {}, status: {})",
                    summary.documents_ingested,
                    summary.documents_failed,
                    redact_excerpt(&summary.job_id, &credentials, 200),
                    redact_excerpt(&summary.status, &credentials, 200)
                );
                for line in format_warning_lines(&summary.warnings, &credentials) {
                    println!("{line}");
                }
                total_docs += summary.documents_ingested as usize;
                total_corpora += 1;
            }
            Err(e) => {
                let error = redact_excerpt(&e.to_string(), &credentials, 200);
                return Err(format!("ingest \"{}\" failed: {error}", entry.name));
            }
        }
    }

    println!("\ndone: {total_docs} document(s) across {total_corpora} corpus(a)");
    Ok(())
}

/// ENS-8269: the one place the ingest request is built. The client sends
/// `mode = "replace"` so a re-sent file is reconciled by document id: a
/// shortened file's removed tail is deleted (its rent stops), and an
/// unchanged file is neither re-embedded nor charged.
fn ingest_request(corpus_id: String, docs: Vec<IngestDocument>, voice_id: String) -> IngestRequest {
    IngestRequest {
        corpus_id,
        documents: docs,
        voice_id: Some(voice_id),
        dry_run: false,
        sync: None,
        no_batch: None,
        mode: None,
    }
}

fn format_warning_lines(warnings: &[String], credentials: &[&str]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| redact_excerpt(&format!("  ! warning: {warning}"), credentials, 200))
        .collect()
}

fn build_documents(
    config_dir: &Path,
    corpus: &CorpusConfig,
    _voice: &VoiceConfig,
) -> Result<Vec<IngestDocument>, String> {
    let root = if corpus.path.is_absolute() {
        corpus.path.clone()
    } else {
        config_dir.join(&corpus.path)
    };
    if !root.exists() {
        return Err(format!(
            "corpus \"{}\" path does not exist: {}",
            corpus.name,
            root.display()
        ));
    }

    let extension_filter = derive_extension_from_glob(&corpus.glob);
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
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let id = relative_doc_id(&root, path);
        let mut metadata = HashMap::new();
        metadata.insert("source_path".to_string(), id.clone());
        if let Some(prefix) = &corpus.url_prefix {
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

#[cfg(test)]
mod tests {
    use super::{build_documents, format_warning_lines, ingest_request};
    use enscrive_docs_core::{CorpusConfig, VoiceConfig};

    fn ens8269_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ens8269-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create test corpus dir");
        dir
    }

    fn ens8269_corpus(root: &std::path::Path) -> CorpusConfig {
        CorpusConfig {
            name: "docs".to_string(),
            voice: "neutral".to_string(),
            path: root.to_path_buf(),
            glob: "**/*.md".to_string(),
            url_prefix: None,
            embedding_model: None,
            dimensions: None,
            description: None,
        }
    }

    fn ens8269_voice() -> VoiceConfig {
        VoiceConfig {
            name: "neutral".to_string(),
            chunking_strategy: "paragraph".to_string(),
            parameters: std::collections::HashMap::new(),
            template_id: None,
            score_threshold: None,
            default_limit: None,
            description: None,
        }
    }

    fn ens8269_request(root: &std::path::Path) -> serde_json::Value {
        let docs = build_documents(root, &ens8269_corpus(root), &ens8269_voice())
            .expect("build documents");
        serde_json::to_value(ingest_request(
            "corpus-1".to_string(),
            docs,
            "voice-1".to_string(),
        ))
        .expect("request serializes")
    }

    #[test]
    fn warning_lines_redact_provider_key_shapes() {
        let provider_key = format!("{}{}", "sk-ant-api03-", "abcdefghijklmnop");
        let warning = format!("provider rejected {provider_key}");
        let lines = format_warning_lines(&[warning], &["api-key", &provider_key]);

        assert_eq!(
            lines,
            vec!["  ! warning: provider rejected [redacted]".to_string()]
        );
        assert!(!lines[0].contains(&provider_key));
    }

    // ENS-8269 (1): a shortened file is re-sent under replace, so the server
    // deletes its removed tail. Before the fix the request has no `mode`, and
    // the server applies `append`, which never deletes the tail.
    #[test]
    fn ens8269_shortened_file_is_resent_under_replace() {
        let root = ens8269_dir("shortened");
        std::fs::write(root.join("a.md"), "one\n\ntwo\n\nthree\n").expect("write a.md");
        let first = ens8269_request(&root);
        std::fs::write(root.join("a.md"), "one\n").expect("shorten a.md");
        let second = ens8269_request(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(second["mode"], "replace");
        assert_eq!(second["documents"].as_array().map(Vec::len), Some(1));
        assert_eq!(second["documents"][0]["id"], first["documents"][0]["id"]);
        assert_eq!(second["documents"][0]["content"], "one\n");
        assert_ne!(
            second["documents"][0]["fingerprint"],
            first["documents"][0]["fingerprint"]
        );
    }

    // ENS-8269 (2): the client half only: an unchanged file is re-sent under
    // replace with an equal id, content, metadata and fingerprint. Whether the
    // server then embeds nothing depends on its stored rows (developer
    // persist.rs :461-:470): rows written by an older chunker or model are
    // re-embedded once on the first replace send.
    #[test]
    fn ens8269_unchanged_file_is_resent_under_replace_byte_equal() {
        let root = ens8269_dir("unchanged");
        std::fs::write(root.join("a.md"), "one\n\ntwo\n").expect("write a.md");
        let first = ens8269_request(&root);
        let second = ens8269_request(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(first["mode"], "replace");
        assert_eq!(second["mode"], "replace");
        assert_eq!(first["documents"], second["documents"]);
    }

    // ENS-8269 (3): two files, one edited: both are sent under replace, and
    // the unedited one is equal to its first send, so only the edited one is
    // re-embedded.
    #[test]
    fn ens8269_two_files_one_edited_both_sent_under_replace() {
        let root = ens8269_dir("two");
        std::fs::write(root.join("a.md"), "alpha\n").expect("write a.md");
        std::fs::write(root.join("b.md"), "beta\n").expect("write b.md");
        let first = ens8269_request(&root);
        std::fs::write(root.join("b.md"), "beta edited\n").expect("edit b.md");
        let second = ens8269_request(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(second["mode"], "replace");
        assert_eq!(second["documents"].as_array().map(Vec::len), Some(2));
        assert_eq!(second["documents"][0], first["documents"][0]);
        assert_eq!(second["documents"][1]["id"], "b.md");
        assert_ne!(
            second["documents"][1]["fingerprint"],
            first["documents"][1]["fingerprint"]
        );
    }
}
