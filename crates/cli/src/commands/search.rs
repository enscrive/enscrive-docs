use crate::global::GlobalArgs;
use clap::{Args, ValueEnum};
use enscrive_docs_core::{
    Config, EnscriveClient, SearchQuery as ApiSearchQuery, SearchResultItem, SearchResults,
    SearchWithVoiceBody,
};
use std::path::{Path, PathBuf};

#[derive(Args, Clone, Debug)]
pub struct SearchArgs {
    /// The query string
    pub query: String,

    /// Limit results to a configured collection (defaults to the first one)
    #[arg(long)]
    pub collection: Option<String>,

    /// Voice name (defaults to the collection's configured voice, then [search] default_voice)
    #[arg(long)]
    pub voice: Option<String>,

    /// Maximum number of results
    #[arg(long, default_value_t = 10)]
    pub limit: u32,

    /// Output format
    #[arg(long, value_enum, default_value_t = SearchFormat::Human)]
    pub format: SearchFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SearchFormat {
    /// Human-readable plain text (default)
    Human,
    /// Structured JSON envelope
    Json,
    /// Markdown — one section per result
    Md,
}

pub async fn run(global: GlobalArgs, args: SearchArgs) -> Result<(), String> {
    let config_path = global.resolved_config_path();
    let _config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

    let api_key = cfg
        .resolved_api_key(global.api_key.as_deref())
        .map_err(|e| e.to_string())?;
    let endpoint = cfg.resolved_endpoint(global.endpoint.as_deref());
    let provider_key = cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
    let client = EnscriveClient::with_provider_key(endpoint, api_key, provider_key);

    // Resolve collection -> id and voice -> id with the same defaults the
    // serve handler uses, so behavior is consistent across the CLI and
    // HTTP search surfaces.
    let collections = client.list_collections().await.map_err(|e| e.to_string())?;
    let voices = client.list_voices().await.map_err(|e| e.to_string())?;

    let collection_name = args
        .collection
        .clone()
        .or_else(|| cfg.collections.first().map(|c| c.name.clone()));
    let collection_id = collection_name
        .as_deref()
        .and_then(|name| collections.iter().find(|c| c.name == name).map(|c| c.id.clone()));
    if let Some(name) = collection_name.as_deref() {
        if collection_id.is_none() {
            return Err(format!(
                "collection \"{name}\" is not present in the Enscrive tenant; create it before searching"
            ));
        }
    }

    let voice_name = args
        .voice
        .clone()
        .or_else(|| {
            collection_name
                .as_deref()
                .and_then(|cn| cfg.collections.iter().find(|c| c.name == cn))
                .map(|c| c.voice.clone())
        })
        .or_else(|| cfg.search.default_voice.clone());
    let voice_id = voice_name
        .as_deref()
        .and_then(|name| voices.iter().find(|v| v.name == name).map(|v| v.id.clone()));

    let results = if let Some(voice_id) = voice_id {
        client
            .search_with_voice(&SearchWithVoiceBody {
                query: args.query.clone(),
                voice_id,
                collection_id: collection_id.clone(),
                limit: Some(args.limit),
                include_vectors: false,
                filters: None,
                granularity: None,
                oversample_factor: None,
                score_threshold: None,
                extended_results: false,
                score_floor: None,
                hybrid_alpha: None,
                resolution: None,
            })
            .await
    } else {
        client
            .search(&ApiSearchQuery {
                query: args.query.clone(),
                collection_id,
                limit: Some(args.limit),
                include_vectors: false,
                ..Default::default()
            })
            .await
    }
    .map_err(|e| e.to_string())?;

    match args.format {
        SearchFormat::Json => print_json(&args.query, &results),
        SearchFormat::Md => print_markdown(&args.query, &results),
        SearchFormat::Human => print_human(&args.query, &results),
    }
    Ok(())
}

fn print_human(query: &str, results: &SearchResults) {
    println!(
        "{} result(s) for \"{}\"  ({}ms search, {}ms embed, {} candidates)",
        results.results.len(),
        query,
        results.search_time_ms,
        results.embed_time_ms,
        results.total_candidates
    );
    if results.results.is_empty() {
        return;
    }
    for (i, r) in results.results.iter().enumerate() {
        println!();
        println!(
            "  [{i}] score={score:.3}  document={doc}  collection={coll}",
            i = i + 1,
            score = r.score,
            doc = r.document_id,
            coll = short_id(&r.collection_id)
        );
        let snippet = compact_snippet(&r.content, 240);
        println!("      {snippet}");
    }
}

fn print_markdown(query: &str, results: &SearchResults) {
    println!("# Results for \"{query}\"\n");
    println!(
        "_{n} match(es), {ms}ms search_\n",
        n = results.results.len(),
        ms = results.search_time_ms
    );
    for (i, r) in results.results.iter().enumerate() {
        println!(
            "## {i}. {doc}  (score {score:.3})",
            i = i + 1,
            doc = r.document_id,
            score = r.score
        );
        println!();
        println!("> {}", compact_snippet(&r.content, 320).replace('\n', " "));
        println!();
    }
}

fn print_json(query: &str, results: &SearchResults) {
    let envelope = serde_json::json!({
        "query": query,
        "search_time_ms": results.search_time_ms,
        "embed_time_ms": results.embed_time_ms,
        "total_candidates": results.total_candidates,
        "results": results.results.iter().map(item_to_json).collect::<Vec<_>>(),
    });
    match serde_json::to_string_pretty(&envelope) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error serializing results: {e}"),
    }
}

fn item_to_json(r: &SearchResultItem) -> serde_json::Value {
    serde_json::json!({
        "id": r.id,
        "document_id": r.document_id,
        "collection_id": r.collection_id,
        "score": r.score,
        "content": r.content,
        "snippet": compact_snippet(&r.content, 280),
        "chunk_index": r.chunk_index,
    })
}

fn compact_snippet(content: &str, max: usize) -> String {
    let normalized: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max {
        return normalized;
    }
    let mut out: String = normalized.chars().take(max).collect();
    out.push_str(" …");
    out
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}
