use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::redact::redact_excerpt;
use enscrive_docs_core::{
    Config, CorpusConfig, EnscriveClient, IngestDocument, IngestMode, IngestRequest, VoiceConfig,
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

    // ENS-8269 (R3261): plan every selected entry BEFORE any ingest request.
    // Under `mode = "replace"`, two entries that send the same document id to
    // the same corpus would delete each other's content, so a duplicate
    // (corpus, document id) refuses the whole run and nothing is sent.
    let mut planned: Vec<PlannedEntry<'_>> = Vec::new();
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
        let target = match remote.as_ref() {
            Some((_, corpora, _, _)) => corpora
                .iter()
                .find(|c| c.name == entry.name)
                .map(|c| c.id.clone())
                .ok_or_else(|| {
                    format!(
                        "Enscrive corpus \"{}\" not found. Create it via the Enscrive UI or `enscrive corpus create` before ingesting.",
                        entry.name
                    )
                })?,
            None => entry.name.clone(),
        };
        planned.push(PlannedEntry {
            entry,
            docs,
            target,
        });
    }
    refuse_duplicate_targets(&planned)?;

    let mut total_docs = 0usize;
    let mut total_corpora = 0usize;
    for PlannedEntry {
        entry,
        docs,
        target,
    } in planned
    {
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

        let (client, _, voices, live_credentials) =
            remote.as_ref().expect("remote must exist when not dry-run");
        let credentials = live_credentials
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let corpus_id = target;
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

/// ENS-8269 (R3261): one selected `[[corpora]]` entry, planned before any
/// ingest request. `target` is the resolved corpus id (the entry's name on
/// `--dry-run`, which resolves nothing remotely).
struct PlannedEntry<'a> {
    entry: &'a CorpusConfig,
    docs: Vec<IngestDocument>,
    target: String,
}

/// ENS-8269 (R3261): every (corpus, document id) the run would send, over ALL
/// selected entries, is unique, or the whole run is refused before any
/// request. Under replace, a second send of one document id deletes the
/// first send's content; the same root listed twice falls here too. A refusal
/// sends nothing, so it charges nothing.
fn refuse_duplicate_targets(planned: &[PlannedEntry<'_>]) -> Result<(), String> {
    let mut seen: HashMap<(&str, &str), &CorpusConfig> = HashMap::new();
    for plan in planned {
        for doc in &plan.docs {
            if let Some(first) = seen.insert((plan.target.as_str(), doc.id.as_str()), plan.entry) {
                return Err(format!(
                    "refusing to ingest: document \"{}\" would be sent to corpus \"{}\" from two configured roots ({} and {}); under replace the later send would delete the earlier one's content. Nothing was sent.",
                    doc.id,
                    plan.entry.name,
                    first.path.display(),
                    plan.entry.path.display()
                ));
            }
        }
    }
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
        mode: Some(IngestMode::Replace),
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

    // ENS-8269 (R3261): a config with two `[[corpora]]` entries, rooted at
    // `root-a` and `root-b`, each holding a different `intro.md`. `first` and
    // `second` name each entry's corpus.
    fn ens8269_two_root_config(
        dir: &std::path::Path,
        first: &str,
        second: &str,
    ) -> std::path::PathBuf {
        for root in ["root-a", "root-b"] {
            std::fs::create_dir_all(dir.join(root)).expect("create root");
            std::fs::write(dir.join(root).join("intro.md"), format!("{root} intro\n"))
                .expect("write intro.md");
        }
        let config = format!(
            "[site]\ntitle = \"ens8269\"\n\n\
             [[corpora]]\nname = \"{first}\"\nvoice = \"neutral\"\npath = \"root-a\"\n\n\
             [[corpora]]\nname = \"{second}\"\nvoice = \"neutral\"\npath = \"root-b\"\n\n\
             [[voices]]\nname = \"neutral\"\nchunking_strategy = \"paragraph\"\n"
        );
        let path = dir.join("enscrive-docs.toml");
        std::fs::write(&path, config).expect("write config");
        path
    }

    // `--dry-run` sends nothing and resolves nothing remotely, at every head.
    async fn ens8269_dry_run(config_path: &std::path::Path) -> Result<(), String> {
        let global = crate::global::GlobalArgs {
            config_path: Some(config_path.to_path_buf()),
            api_key: None,
            endpoint: None,
            embedding_provider_key: None,
            profile: None,
        };
        let args = super::IngestArgs {
            corpus: None,
            dry_run: true,
            force: false,
        };
        super::run(global, args).await
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

    // ENS-8269 (R3261): two configured roots that target ONE corpus and each
    // hold `intro.md` send the same document id. Under replace the later send
    // deletes the earlier one's content, so the run is refused before any
    // request, naming both roots and the id. Red before the fix: the run
    // returns Ok.
    #[tokio::test]
    async fn ens8269_two_roots_one_corpus_same_document_id_refuse_the_run() {
        let dir = ens8269_dir("collide");
        let config_path = ens8269_two_root_config(&dir, "docs", "docs");
        let result = ens8269_dry_run(&config_path).await;
        std::fs::remove_dir_all(&dir).ok();
        let err = result.expect_err("a duplicate (corpus, document id) must refuse the run");
        assert!(err.contains("intro.md"), "names the document id: {err}");
        assert!(err.contains("root-a"), "names the first root: {err}");
        assert!(err.contains("root-b"), "names the second root: {err}");
    }

    // Control: the same file name under two roots that target TWO corpora is
    // not a collision; the run goes on.
    #[tokio::test]
    async fn ens8269_two_roots_two_corpora_same_file_name_both_go_on() {
        let dir = ens8269_dir("twocorpora");
        let config_path = ens8269_two_root_config(&dir, "docs", "guides");
        let result = ens8269_dry_run(&config_path).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result, Ok(()));
    }

    // ENS-8269 (R3269): the LIVE path, against a local mock API. The two rows
    // above run `--dry-run`, where the planned target is the entry's NAME and
    // nothing is sent, so a refusal checked only under dry-run would leave
    // them green. These rows run `run` with dry-run OFF, resolve the corpus
    // ids from GET /v1/corpora, and count the ingest requests the mock gets.
    type Ens8269Log = std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>;

    struct Ens8269Mock {
        endpoint: String,
        requests: Ens8269Log,
    }

    impl Ens8269Mock {
        fn count(&self, method: &str, path: &str) -> usize {
            self.requests
                .lock()
                .expect("mock request log")
                .iter()
                .filter(|(m, p, _)| m == method && p == path)
                .count()
        }

        fn ingest_bodies(&self) -> Vec<serde_json::Value> {
            self.requests
                .lock()
                .expect("mock request log")
                .iter()
                .filter(|(m, p, _)| m == "POST" && p == "/v1/ingest")
                .map(|(_, _, body)| serde_json::from_str(body).expect("ingest body is JSON"))
                .collect()
        }
    }

    // A mock Enscrive API on 127.0.0.1, one request per connection
    // (`Connection: close`). GET /v1/corpora returns `corpora` as (name, id);
    // the voice `neutral` is `voice-1`; every POST /v1/ingest launches a job
    // that the first GET /v1/jobs/{id} reports `completed`.
    fn ens8269_mock(corpora: &[(&str, &str)]) -> Ens8269Mock {
        use std::io::{Read, Write};
        let corpora_json = serde_json::Value::Array(
            corpora
                .iter()
                .map(|(name, id)| serde_json::json!({ "id": id, "name": name }))
                .collect(),
        )
        .to_string();
        let voices_json = serde_json::json!([{
            "id": "voice-1",
            "name": "neutral",
            "config": { "chunking_strategy": "paragraph" }
        }])
        .to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let endpoint = format!("http://{}", listener.local_addr().expect("mock address"));
        let requests: Ens8269Log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut seen: Vec<u8> = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(at + 4);
                    }
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break None,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                };
                let Some(header_end) = header_end else {
                    continue;
                };
                let head = String::from_utf8_lossy(&seen[..header_end]).into_owned();
                let mut words = head.split_whitespace();
                let method = words.next().unwrap_or("").to_string();
                let path = words.next().unwrap_or("").to_string();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.trim().eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                while seen.len() < header_end + length {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let end = seen.len().min(header_end + length);
                let body = String::from_utf8_lossy(&seen[header_end..end]).into_owned();
                let posts = {
                    let mut log = log.lock().expect("mock request log");
                    log.push((method.clone(), path.clone(), body));
                    log.iter()
                        .filter(|(m, p, _)| m == "POST" && p == "/v1/ingest")
                        .count()
                };
                let (status, reply) = match (method.as_str(), path.as_str()) {
                    ("GET", "/v1/corpora") => ("200 OK", corpora_json.clone()),
                    ("GET", "/v1/voices") => ("200 OK", voices_json.clone()),
                    ("POST", "/v1/ingest") => (
                        "202 Accepted",
                        serde_json::json!({
                            "job_id": format!("job-{posts}"),
                            "status": "pending",
                            "poll_url": format!("/v1/jobs/job-{posts}")
                        })
                        .to_string(),
                    ),
                    ("GET", p) if p.starts_with("/v1/jobs/") => (
                        "200 OK",
                        serde_json::json!({
                            "id": p.trim_start_matches("/v1/jobs/"),
                            "status": "completed",
                            "documents_ingested": 1,
                            "documents_failed": 0
                        })
                        .to_string(),
                    ),
                    _ => ("404 Not Found", "{}".to_string()),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Ens8269Mock { endpoint, requests }
    }

    // One `[[corpora]]` entry per (name, root), in order; each root holds its
    // own `intro.md` ("<root> intro").
    fn ens8269_live_config(dir: &std::path::Path, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let mut config = String::from("[site]\ntitle = \"ens8269\"\n");
        for (name, root) in entries {
            std::fs::create_dir_all(dir.join(root)).expect("create root");
            std::fs::write(dir.join(root).join("intro.md"), format!("{root} intro\n"))
                .expect("write intro.md");
            config.push_str(&format!(
                "\n[[corpora]]\nname = \"{name}\"\nvoice = \"neutral\"\npath = \"{root}\"\n"
            ));
        }
        config.push_str("\n[[voices]]\nname = \"neutral\"\nchunking_strategy = \"paragraph\"\n");
        let path = dir.join("enscrive-docs.toml");
        std::fs::write(&path, config).expect("write config");
        path
    }

    async fn ens8269_live_run(
        config_path: &std::path::Path,
        mock: &Ens8269Mock,
    ) -> Result<(), String> {
        let global = crate::global::GlobalArgs {
            config_path: Some(config_path.to_path_buf()),
            api_key: Some("ens8269-test-key".to_string()),
            endpoint: Some(mock.endpoint.clone()),
            embedding_provider_key: None,
            profile: None,
        };
        let args = super::IngestArgs {
            corpus: None,
            dry_run: false,
            force: false,
        };
        super::run(global, args).await
    }

    // R3269 (a): an earlier entry that collides with nothing, then two roots
    // that target ONE corpus with the same document id. Live, the run is
    // refused before ANY ingest request, the earlier entry's included.
    #[tokio::test]
    async fn ens8269_live_collision_refuses_with_zero_ingest_requests() {
        let dir = ens8269_dir("live-collide");
        let config_path = ens8269_live_config(
            &dir,
            &[("guides", "root-c"), ("docs", "root-a"), ("docs", "root-b")],
        );
        let mock = ens8269_mock(&[("guides", "corpus-guides"), ("docs", "corpus-docs")]);
        let result = ens8269_live_run(&config_path, &mock).await;
        std::fs::remove_dir_all(&dir).ok();
        let err = result.expect_err("a duplicate (corpus, document id) must refuse the live run");
        assert!(err.contains("refusing to ingest"), "the refusal, not another error: {err}");
        assert!(mock.count("GET", "/v1/corpora") >= 1, "the live path resolved the corpus ids");
        assert_eq!(mock.count("POST", "/v1/ingest"), 0, "no ingest request is sent");
    }

    // R3269 (b), the control: the same file name under two roots that target
    // TWO corpora. Live, EXACTLY two ingest requests go out, in config order,
    // each naming its own resolved corpus id and carrying its own root's text.
    #[tokio::test]
    async fn ens8269_live_two_corpora_same_file_name_sends_exactly_two_requests() {
        let dir = ens8269_dir("live-two");
        let config_path = ens8269_live_config(&dir, &[("docs", "root-a"), ("guides", "root-b")]);
        let mock = ens8269_mock(&[("docs", "corpus-docs"), ("guides", "corpus-guides")]);
        let result = ens8269_live_run(&config_path, &mock).await;
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result, Ok(()));
        let bodies = mock.ingest_bodies();
        assert_eq!(bodies.len(), 2, "exactly two ingest requests");
        assert_eq!(bodies[0]["corpus_id"], "corpus-docs");
        assert_eq!(bodies[1]["corpus_id"], "corpus-guides");
        assert_eq!(bodies[0]["documents"].as_array().map(Vec::len), Some(1));
        assert_eq!(bodies[1]["documents"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            bodies[0]["documents"][0]["id"], "intro.md",
            "the first request carries the file's own document id"
        );
        assert_eq!(
            bodies[1]["documents"][0]["id"], "intro.md",
            "the second request carries the file's own document id"
        );
        assert!(
            bodies[0]["documents"][0]["content"]
                .as_str()
                .is_some_and(|c| c.contains("root-a intro")),
            "the first request carries root-a's text"
        );
        assert!(
            bodies[1]["documents"][0]["content"]
                .as_str()
                .is_some_and(|c| c.contains("root-b intro")),
            "the second request carries root-b's text"
        );
    }

    // R3269 (c): two corpus NAMES that resolve to ONE corpus id, the same file
    // name under each root. Only the live path can see this (on dry-run the
    // target is the name). Refused, with zero ingest requests.
    #[tokio::test]
    async fn ens8269_live_two_names_one_corpus_id_refuse_with_zero_ingest_requests() {
        let dir = ens8269_dir("live-alias");
        let config_path = ens8269_live_config(&dir, &[("docs", "root-a"), ("handbook", "root-b")]);
        let mock = ens8269_mock(&[("docs", "corpus-shared"), ("handbook", "corpus-shared")]);
        let result = ens8269_live_run(&config_path, &mock).await;
        std::fs::remove_dir_all(&dir).ok();
        let err = result.expect_err("two names resolving to one corpus id must refuse the live run");
        assert!(err.contains("refusing to ingest"), "the refusal, not another error: {err}");
        assert!(mock.count("GET", "/v1/corpora") >= 1, "the live path resolved the corpus ids");
        assert_eq!(mock.count("POST", "/v1/ingest"), 0, "no ingest request is sent");
    }

    // r4 (Sol 6.1 P1 :721): the two colliding entries are NOT adjacent: a
    // non-colliding entry sits between them (A, B, A). The check must hold
    // over the whole run, not only between neighbours. Refused, with zero
    // ingest requests, the earlier entries' included.
    #[tokio::test]
    async fn ens8269_live_nonadjacent_collision_refuses_with_zero_ingest_requests() {
        let dir = ens8269_dir("live-nonadjacent");
        let config_path = ens8269_live_config(
            &dir,
            &[("docs", "root-a"), ("guides", "root-c"), ("docs", "root-b")],
        );
        let mock = ens8269_mock(&[("docs", "corpus-docs"), ("guides", "corpus-guides")]);
        let result = ens8269_live_run(&config_path, &mock).await;
        std::fs::remove_dir_all(&dir).ok();
        let err = result.expect_err("a non-adjacent duplicate (corpus, document id) must refuse the live run");
        assert!(err.contains("refusing to ingest"), "the refusal, not another error: {err}");
        assert!(mock.count("GET", "/v1/corpora") >= 1, "the live path resolved the corpus ids");
        assert_eq!(mock.count("POST", "/v1/ingest"), 0, "no ingest request is sent");
    }

    // R3283 (1): one `[[corpora]]` entry per (name, root, files), in order;
    // each file holds "<root> <file>".
    fn ens8269_matrix_config(
        dir: &std::path::Path,
        entries: &[(&str, &str, &[&str])],
    ) -> std::path::PathBuf {
        let mut config = String::from("[site]\ntitle = \"ens8269\"\n");
        for (name, root, files) in entries {
            std::fs::create_dir_all(dir.join(root)).expect("create root");
            for file in *files {
                std::fs::write(dir.join(root).join(file), format!("{root} {file}\n"))
                    .expect("write file");
            }
            config.push_str(&format!(
                "\n[[corpora]]\nname = \"{name}\"\nvoice = \"neutral\"\npath = \"{root}\"\n"
            ));
        }
        config.push_str("\n[[voices]]\nname = \"neutral\"\nchunking_strategy = \"paragraph\"\n");
        let path = dir.join("enscrive-docs.toml");
        std::fs::write(&path, config).expect("write config");
        path
    }

    // R3283 (1): ONE live matrix over the guard's key (target, document id,
    // the whole run). Every case runs `run` live against its own mock; a case
    // that fails is NAMED in the one panic, so a mutant's red set reads by
    // case name.
    // Refusals, 2 x 2 x 2: order (adjacent; non-adjacent, a third corpus
    // between) x files per root (one; two) x target (the same name; two names
    // that resolve to one corpus id). Each expects the refusal and zero
    // POST /v1/ingest.
    // Controls that must SEND, corpus and document ids pinned: two corpora
    // with the same file name; ONE corpus with two different files.
    #[tokio::test]
    async fn ens8269_live_matrix_refuses_every_duplicate_key_and_sends_the_controls() {
        const ONE: &[&str] = &["intro.md"];
        const TWO: &[&str] = &["intro.md", "setup.md"];
        const GUIDE: &[&str] = &["guide.md"];
        const SETUP: &[&str] = &["setup.md"];
        let mut failed: Vec<String> = Vec::new();

        for (order, nonadjacent) in [("adjacent", false), ("nonadjacent", true)] {
            for (per_root, files) in [("one", ONE), ("two", TWO)] {
                for (target, second, corpora) in [
                    (
                        "same_name",
                        "docs",
                        &[("docs", "corpus-docs"), ("guides", "corpus-guides")][..],
                    ),
                    (
                        "two_names_one_id",
                        "handbook",
                        &[
                            ("docs", "corpus-shared"),
                            ("handbook", "corpus-shared"),
                            ("guides", "corpus-guides"),
                        ][..],
                    ),
                ] {
                    let case = format!("{order}/{per_root}/{target}");
                    let mut entries: Vec<(&str, &str, &[&str])> = vec![("docs", "root-a", files)];
                    if nonadjacent {
                        entries.push(("guides", "root-c", GUIDE));
                    }
                    entries.push((second, "root-b", files));
                    let dir = ens8269_dir(&format!("matrix-{order}-{per_root}-{target}"));
                    let config_path = ens8269_matrix_config(&dir, &entries);
                    let mock = ens8269_mock(corpora);
                    let result = ens8269_live_run(&config_path, &mock).await;
                    std::fs::remove_dir_all(&dir).ok();
                    let posts = mock.count("POST", "/v1/ingest");
                    let resolved = mock.count("GET", "/v1/corpora");
                    let refused = matches!(&result, Err(e) if e.contains("refusing to ingest"));
                    if !refused || posts != 0 || resolved == 0 {
                        failed.push(format!(
                            "{case}: result={result:?} ingest_posts={posts} corpora_gets={resolved}"
                        ));
                    }
                }
            }
        }

        for (case, entries, corpora, want) in [
            (
                "control/two_corpora_same_file_name",
                &[("docs", "root-a", ONE), ("guides", "root-b", ONE)][..],
                &[("docs", "corpus-docs"), ("guides", "corpus-guides")][..],
                &[
                    ("corpus-docs", "intro.md", "root-a intro.md"),
                    ("corpus-guides", "intro.md", "root-b intro.md"),
                ][..],
            ),
            (
                "control/one_corpus_two_different_files",
                &[("docs", "root-a", ONE), ("docs", "root-b", SETUP)][..],
                &[("docs", "corpus-docs")][..],
                &[
                    ("corpus-docs", "intro.md", "root-a intro.md"),
                    ("corpus-docs", "setup.md", "root-b setup.md"),
                ][..],
            ),
        ] {
            let dir = ens8269_dir(&case.replace('/', "-"));
            let config_path = ens8269_matrix_config(&dir, entries);
            let mock = ens8269_mock(corpora);
            let result = ens8269_live_run(&config_path, &mock).await;
            std::fs::remove_dir_all(&dir).ok();
            let got: Vec<(String, usize, String, String)> = mock
                .ingest_bodies()
                .iter()
                .map(|body| {
                    (
                        body["corpus_id"].as_str().unwrap_or("").to_string(),
                        body["documents"].as_array().map_or(0, Vec::len),
                        body["documents"][0]["id"].as_str().unwrap_or("").to_string(),
                        body["documents"][0]["content"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect();
            let sent_as_pinned = result.is_ok()
                && got.len() == want.len()
                && got.iter().zip(want.iter()).all(
                    |((corpus, docs, id, content), (want_corpus, want_id, want_text))| {
                        corpus == want_corpus
                            && *docs == 1
                            && id == want_id
                            && content.contains(want_text)
                    },
                );
            if !sent_as_pinned {
                failed.push(format!("{case}: result={result:?} ingest_requests={got:?}"));
            }
        }

        assert!(failed.is_empty(), "ENS-8269 R3283 matrix cases failed: {failed:#?}");
    }
}
