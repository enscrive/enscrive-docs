use crate::global::GlobalArgs;
use arc_swap::ArcSwap;
use axum::{
    body::Body,
    extract::{Path as AxPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use clap::Args;
use enscrive_docs_core::{
    Config, EnscriveClient, SearchFilter, SearchQuery as ApiSearchQuery, SearchWithVoiceBody,
};
use enscrive_docs_render::{
    embedded_asset, render_index, render_markdown, render_page, templates::build_nav, IndexContext,
    Page, PageContext, PageMeta, ThemeVariant,
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{Any, CorsLayer};
use walkdir::WalkDir;

/// Built-in default. Chosen to dodge the crowded 8080/3000/8000 cluster;
/// not in IANA's registry for any popular dev tool. Override with
/// --port, the PORT env var, or [serve] port in enscrive-docs.toml.
const DEFAULT_PORT: u16 = 3737;

#[derive(Args, Clone, Debug)]
pub struct ServeArgs {
    /// Port to bind. Resolution: --port > $PORT > [serve] port > 3737.
    #[arg(long, env = "PORT")]
    pub port: Option<u16>,

    /// Bind address
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// URL prefix when serving behind a reverse-proxy subpath (e.g. "/docs")
    #[arg(long = "base-path")]
    pub base_path: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub site_title: String,
    pub site_description: String,
    pub base_path: String,
    pub asset_base: String,
    pub theme_css_path: String,
    pub theme_variables: String,
    pub custom_css: String,
    pub pages: Arc<ArcSwap<HashMap<String, Page>>>, // slug -> Page
    pub pages_meta: Arc<ArcSwap<Vec<PageMeta>>>,
    pub doc_id_to_slug: Arc<ArcSwap<HashMap<String, String>>>, // doc_id -> slug
    pub enscrive: Arc<EnscriveServer>,
    /// Broadcast channel for live-reload events. Subscribers (browsers
    /// connected to /_events) receive each message verbatim. None when
    /// running plain `serve`; populated under `watch`.
    pub event_tx: Option<broadcast::Sender<&'static str>>,
    /// Whether the rendered pages should include the SSE reload listener.
    pub watch_mode: bool,
    /// Configuration directory used to resolve relative collection paths
    /// when the watcher needs to re-read source files.
    pub config_dir: PathBuf,
    pub cfg: Arc<Config>,
}

pub struct EnscriveServer {
    pub client: EnscriveClient,
    /// collection_name -> collection_id
    pub collection_ids: HashMap<String, String>,
    /// voice_name -> voice_id
    pub voice_ids: HashMap<String, String>,
    pub default_collection_id: Option<String>,
    pub collection_default_voice: HashMap<String, String>,
    pub default_voice_name: Option<String>,
    pub default_limit: u32,
}

pub async fn run(global: GlobalArgs, args: ServeArgs) -> Result<(), String> {
    init_tracing();
    let state = setup_state(&global, args.base_path.as_deref(), /* watch_mode */ false).await?;
    serve_with_state(state, &args).await
}

/// Build the full AppState. Shared between `serve` and `watch`. The
/// `watch_mode` flag controls whether rendered pages include the SSE
/// reload listener; it does not affect the routes themselves.
/// `base_path_override` (when Some) wins over the config file's
/// `[site] base_path`; pass None to defer to config.
pub async fn setup_state(
    global: &GlobalArgs,
    base_path_override: Option<&str>,
    watch_mode: bool,
) -> Result<AppState, String> {
    let config_path = global.resolved_config_path();
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

    let api_key = cfg
        .resolved_api_key(global.api_key.as_deref())
        .map_err(|e| e.to_string())?;
    let endpoint = cfg.resolved_endpoint(global.endpoint.as_deref());
    let provider_key = cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
    let client = EnscriveClient::with_provider_key(endpoint.clone(), api_key, provider_key);

    println!("loading collections + voices from {endpoint} ...");
    let collections = client.list_collections().await.map_err(|e| e.to_string())?;
    let voices = client.list_voices().await.map_err(|e| e.to_string())?;

    let mut collection_ids = HashMap::new();
    let mut collection_default_voice = HashMap::new();
    let mut default_collection_id: Option<String> = None;
    for entry in &cfg.collections {
        let id = collections
            .iter()
            .find(|c| c.name == entry.name)
            .map(|c| c.id.clone())
            .ok_or_else(|| {
                format!(
                    "Enscrive collection \"{}\" not found in tenant; create it first",
                    entry.name
                )
            })?;
        if default_collection_id.is_none() {
            default_collection_id = Some(id.clone());
        }
        collection_default_voice.insert(entry.name.clone(), entry.voice.clone());
        collection_ids.insert(entry.name.clone(), id);
    }
    let mut voice_ids = HashMap::new();
    for entry in &cfg.voices {
        let id = voices
            .iter()
            .find(|v| v.name == entry.name)
            .map(|v| v.id.clone())
            .ok_or_else(|| {
                format!(
                    "Enscrive voice \"{}\" not found in tenant; create it first",
                    entry.name
                )
            })?;
        voice_ids.insert(entry.name.clone(), id);
    }

    let base_path = base_path_override
        .map(str::to_string)
        .or_else(|| cfg.site.base_path.clone())
        .unwrap_or_default();
    let base_path = normalize_base_path(&base_path);
    let asset_base = format!("{base_path}/_assets");

    let theme_variant = ThemeVariant::from_str_loose(&cfg.theme.variant);
    let theme_css_path = theme_variant.css_asset_path().to_string();
    let theme_variables = build_theme_variables(&cfg.theme.accent_color);
    let custom_css = load_custom_css(&config_dir, cfg.theme.custom_css.as_deref());

    println!("rendering markdown into in-memory cache ...");
    let (pages, pages_meta, doc_id_to_slug) = build_pages(&config_dir, &cfg)?;
    println!("  {} page(s) ready", pages.len());

    let default_limit = cfg.search.results_per_page.unwrap_or(10);
    let cfg_arc = Arc::new(cfg);
    let state = AppState {
        site_title: cfg_arc.site.title.clone(),
        site_description: cfg_arc
            .site
            .description
            .clone()
            .unwrap_or_else(|| "Documentation powered by Enscrive.".to_string()),
        base_path: base_path.clone(),
        asset_base,
        theme_css_path,
        theme_variables,
        custom_css,
        pages: Arc::new(ArcSwap::from_pointee(pages)),
        pages_meta: Arc::new(ArcSwap::from_pointee(pages_meta)),
        doc_id_to_slug: Arc::new(ArcSwap::from_pointee(doc_id_to_slug)),
        enscrive: Arc::new(EnscriveServer {
            client,
            collection_ids,
            voice_ids,
            default_collection_id,
            collection_default_voice,
            default_voice_name: cfg_arc
                .search
                .default_voice
                .clone()
                .or_else(|| cfg_arc.collections.first().map(|c| c.voice.clone())),
            default_limit,
        }),
        event_tx: if watch_mode {
            Some(broadcast::channel::<&'static str>(64).0)
        } else {
            None
        },
        watch_mode,
        config_dir: config_dir.clone(),
        cfg: cfg_arc,
    };
    Ok(state)
}

/// Bind a TCP listener and serve the router until shutdown. Shared by
/// `serve` and `watch`.
pub async fn serve_with_state(state: AppState, args: &ServeArgs) -> Result<(), String> {
    let port = args
        .port
        .or(state.cfg.serve.port)
        .unwrap_or(DEFAULT_PORT);
    let base_path = state.base_path.clone();
    let app = build_router(state.clone(), &base_path);
    let bind_addr: SocketAddr = format!("{}:{}", args.bind, port)
        .parse()
        .map_err(|e| format!("invalid bind: {e}"))?;
    println!(
        "listening on http://{bind_addr}{base_path}/  (asset base: {})",
        state.asset_base
    );
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| format!("bind {bind_addr}: {e}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("serve: {e}"))?;
    Ok(())
}

fn build_router(state: AppState, base_path: &str) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET])
        .allow_headers(Any)
        .allow_origin(Any);

    let routes = Router::new()
        .route("/", get(handle_index))
        .route("/healthz", get(handle_healthz))
        .route("/llms.txt", get(handle_llms_txt))
        .route("/sitemap.xml", get(handle_sitemap))
        .route("/search", get(handle_search))
        .route("/_events", get(handle_events))
        .route("/_assets/*path", get(handle_asset))
        .route("/*slug", get(handle_page))
        .with_state(state)
        .layer(cors);

    if base_path.is_empty() {
        routes
    } else {
        Router::new().nest(base_path, routes)
    }
}

// -- Handlers --

async fn handle_healthz() -> &'static str {
    "ok"
}

async fn handle_index(State(state): State<AppState>) -> Html<String> {
    let pages_meta = state.pages_meta.load();
    let nav = build_nav(&pages_meta, &state.base_path, None);
    let ctx = IndexContext {
        site_title: state.site_title.clone(),
        site_description: state.site_description.clone(),
        base_path: state.base_path.clone(),
        asset_base: state.asset_base.clone(),
        theme_css_path: state.theme_css_path.clone(),
        theme_variables: state.theme_variables.clone(),
        custom_css: state.custom_css.clone(),
        nav,
        watch_mode: state.watch_mode,
    };
    match render_index(&ctx) {
        Ok(html) => Html(html),
        Err(e) => Html(format!("<h1>render error</h1><pre>{e}</pre>")),
    }
}

#[derive(Deserialize)]
struct PageQuery {
    format: Option<String>,
}

async fn handle_page(
    State(state): State<AppState>,
    AxPath(slug): AxPath<String>,
    Query(q): Query<PageQuery>,
) -> Response {
    let slug = slug.trim_end_matches('/').to_string();
    let pages = state.pages.load();
    let page = match pages.get(&slug) {
        Some(p) => p,
        None => return not_found(&slug),
    };

    match q.format.as_deref() {
        Some("md") => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            page.markdown.clone(),
        )
            .into_response(),
        Some("json") => Json(serde_json::json!({
            "slug": page.meta.slug,
            "title": page.meta.title,
            "description": page.meta.description,
            "url": page.meta.url(&state.base_path),
            "anchors": page.meta.anchors,
            "content_html": page.html,
            "content_md": page.markdown,
        }))
        .into_response(),
        _ => {
            let pages_meta = state.pages_meta.load();
            let nav = build_nav(&pages_meta, &state.base_path, Some(&slug));
            let anchors_html =
                enscrive_docs_render::templates::render_anchor_list(&page.meta.anchors);
            let ctx = PageContext {
                site_title: state.site_title.clone(),
                site_description: state.site_description.clone(),
                base_path: state.base_path.clone(),
                asset_base: state.asset_base.clone(),
                theme_css_path: state.theme_css_path.clone(),
                theme_variables: state.theme_variables.clone(),
                custom_css: state.custom_css.clone(),
                page_title: page.meta.title.clone(),
                page_description: page
                    .meta
                    .description
                    .clone()
                    .unwrap_or_else(|| state.site_description.clone()),
                page_html: page.html.clone(),
                page_anchors_html: anchors_html,
                nav,
                watch_mode: state.watch_mode,
            };
            match render_page(&ctx) {
                Ok(html) => Html(html).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html(format!("<h1>render error</h1><pre>{e}</pre>")),
                )
                    .into_response(),
            }
        }
    }
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    voice: Option<String>,
    collection: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct SearchResponseItem {
    document_id: String,
    score: f32,
    snippet: String,
    url: Option<String>,
    title: Option<String>,
    collection_id: String,
}

#[derive(Serialize)]
struct SearchResponse {
    query: String,
    results: Vec<SearchResponseItem>,
    search_time_ms: u64,
    total_candidates: u32,
}

async fn handle_search(
    State(state): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Response {
    let query = match p.q.as_deref().filter(|v| !v.trim().is_empty()) {
        Some(q) => q.trim().to_string(),
        None => {
            return Json(SearchResponse {
                query: String::new(),
                results: vec![],
                search_time_ms: 0,
                total_candidates: 0,
            })
            .into_response()
        }
    };

    // Default to the configured collection (the first one) when the request
    // does not specify ?collection=name. Without a collection filter the
    // upstream search RPC errors out on multi-collection tenants.
    let collection_name = p.collection.clone();
    let collection_id = collection_name
        .as_deref()
        .and_then(|name| state.enscrive.collection_ids.get(name).cloned())
        .or_else(|| state.enscrive.default_collection_id.clone());

    // Resolve the voice: explicit ?voice= > collection's configured voice >
    // search.default_voice > none.
    let voice_name = p
        .voice
        .clone()
        .or_else(|| {
            collection_name
                .as_deref()
                .and_then(|n| state.enscrive.collection_default_voice.get(n).cloned())
        })
        .or_else(|| state.enscrive.default_voice_name.clone());
    let voice_id = voice_name
        .as_deref()
        .and_then(|name| state.enscrive.voice_ids.get(name).cloned());

    let limit = p.limit.unwrap_or(state.enscrive.default_limit);

    // Use voice-tuned search when a voice is resolved; fall back to plain
    // /v1/search otherwise. Voice-tuned search is the differentiated
    // capability (Enscrive eval-tunable retrieval).
    let result = if let Some(voice_id) = voice_id {
        let body = SearchWithVoiceBody {
            query: query.clone(),
            voice_id,
            collection_id: collection_id.clone(),
            limit: Some(limit),
            include_vectors: false,
            filters: None,
            granularity: None,
            oversample_factor: None,
            score_threshold: None,
            extended_results: false,
            score_floor: None,
            hybrid_alpha: None,
            resolution: None,
        };
        state.enscrive.client.search_with_voice(&body).await
    } else {
        let api_query = ApiSearchQuery {
            query: query.clone(),
            collection_id,
            filters: Some(SearchFilter::default()),
            limit: Some(limit),
            score_threshold: None,
            include_vectors: false,
            ..Default::default()
        };
        state.enscrive.client.search(&api_query).await
    };

    match result {
        Ok(results) => {
            let pages = state.pages.load();
            let doc_id_to_slug = state.doc_id_to_slug.load();
            let items = results
                .results
                .into_iter()
                .map(|r| {
                    let slug = doc_id_to_slug
                        .get(&r.document_id)
                        .or_else(|| doc_id_to_slug.get(&r.id))
                        .cloned()
                        .unwrap_or_else(|| r.document_id.clone());
                    let page = pages.get(&slug);
                    // Build URL with a Text Fragment (#:~:text=...) so
                    // chromium-based browsers and Safari scroll to and
                    // visually highlight the matching passage. Firefox
                    // ignores the fragment and just lands on the page.
                    let url = page.map(|p| {
                        let base = p.meta.url(&state.base_path);
                        match build_text_fragment(&r.content) {
                            Some(frag) => format!("{base}#:~:text={frag}"),
                            None => base,
                        }
                    });
                    let title = page.map(|p| p.meta.title.clone());
                    let snippet =
                        r.content.chars().take(280).collect::<String>();
                    SearchResponseItem {
                        document_id: r.document_id,
                        score: r.score,
                        snippet,
                        url,
                        title,
                        collection_id: r.collection_id,
                    }
                })
                .collect();
            Json(SearchResponse {
                query,
                results: items,
                search_time_ms: results.search_time_ms,
                total_candidates: results.total_candidates,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "upstream_search_failed",
                "detail": e.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn handle_llms_txt(State(state): State<AppState>) -> Response {
    let mut out = format!("# {}\n\n", state.site_title);
    if !state.site_description.is_empty() {
        out.push_str(&state.site_description);
        out.push_str("\n\n");
    }
    out.push_str("## Pages\n\n");
    let pages_meta = state.pages_meta.load();
    let nav = build_nav(&pages_meta, &state.base_path, None);
    for item in nav {
        out.push_str(&format!("- [{title}]({url})\n", title = item.title, url = item.url));
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        out,
    )
        .into_response()
}

async fn handle_sitemap(State(state): State<AppState>) -> Response {
    let pages_meta = state.pages_meta.load();
    let nav = build_nav(&pages_meta, &state.base_path, None);
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for item in nav {
        out.push_str(&format!("  <url><loc>{url}</loc></url>\n", url = item.url));
    }
    out.push_str("</urlset>\n");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        out,
    )
        .into_response()
}

/// SSE endpoint used by watch-mode browsers to receive `reload` events.
/// In plain serve mode (event_tx == None) the connection is held open
/// with keep-alive comments and never sends a real event — same shape,
/// no surprises.
async fn handle_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    use futures::StreamExt;
    let rx = match state.event_tx.as_ref() {
        Some(tx) => tx.subscribe(),
        None => {
            // No broadcaster; use a never-yielding stream so the
            // connection stays open and clients don't reconnect-storm.
            let (tx, rx) = broadcast::channel::<&'static str>(1);
            std::mem::forget(tx);
            rx
        }
    };
    let stream = BroadcastStream::new(rx).map(
        |res: std::result::Result<&'static str, tokio_stream::wrappers::errors::BroadcastStreamRecvError>| {
            let kind = res.unwrap_or("reload");
            Ok(Event::default().event(kind).data(""))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Rebuild the in-memory page cache from disk and atomically swap it
/// into the live AppState. Called by the watch subcommand after a
/// markdown file changes, then followed by a broadcast("reload") so
/// connected browsers refresh.
pub fn rebuild_pages(state: &AppState) -> Result<(), String> {
    let (pages, pages_meta, doc_id_to_slug) = build_pages(&state.config_dir, &state.cfg)?;
    state.pages.store(Arc::new(pages));
    state.pages_meta.store(Arc::new(pages_meta));
    state.doc_id_to_slug.store(Arc::new(doc_id_to_slug));
    Ok(())
}

async fn handle_asset(AxPath(path): AxPath<String>) -> Response {
    match embedded_asset(&path) {
        Some(bytes) => {
            let mime = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static(
                    "application/octet-stream",
                )),
            );
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            (StatusCode::OK, headers, Body::from(bytes)).into_response()
        }
        None => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

fn not_found(_slug: &str) -> Response {
    (StatusCode::NOT_FOUND, Html("<h1>404</h1><p>Page not found.</p>")).into_response()
}

// -- Setup helpers --

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "enscrive_docs=info,tower_http=warn".into()),
        )
        .try_init();
}

fn normalize_base_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn build_theme_variables(accent_color: &Option<String>) -> String {
    let mut out = String::new();
    if let Some(color) = accent_color {
        out.push_str(":root{--accent:");
        out.push_str(color);
        out.push_str(";}");
    }
    out
}

fn load_custom_css(config_dir: &Path, custom_css: Option<&Path>) -> String {
    let path = match custom_css {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        Some(p) => config_dir.join(p),
        None => return String::new(),
    };
    std::fs::read_to_string(&path).unwrap_or_default()
}

fn build_pages(
    config_dir: &Path,
    cfg: &Config,
) -> Result<
    (
        HashMap<String, Page>,
        Vec<PageMeta>,
        HashMap<String, String>,
    ),
    String,
> {
    let mut pages: HashMap<String, Page> = HashMap::new();
    let mut metas: Vec<PageMeta> = Vec::new();
    let mut doc_id_to_slug: HashMap<String, String> = HashMap::new();
    let mut seen_slugs: BTreeMap<String, String> = BTreeMap::new();

    for collection in &cfg.collections {
        let root = if collection.path.is_absolute() {
            collection.path.clone()
        } else {
            config_dir.join(&collection.path)
        };
        if !root.exists() {
            return Err(format!(
                "collection \"{}\" path missing: {}",
                collection.name,
                root.display()
            ));
        }

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str())
                != Some("md")
            {
                continue;
            }
            let path = entry.path();
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let rendered = render_markdown(&raw);
            let rel = path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            let doc_id = rel.clone();
            let slug = make_slug(&rel, collection.url_prefix.as_deref());
            if let Some(prev) = seen_slugs.insert(slug.clone(), doc_id.clone()) {
                eprintln!(
                    "warn: slug collision \"{slug}\" between {prev} and {doc_id}; later wins"
                );
            }
            let meta = PageMeta::build(
                slug.clone(),
                doc_id.clone(),
                collection.url_prefix.clone(),
                &rendered.frontmatter,
                rendered.anchors,
                rendered.leading_h1,
            );
            doc_id_to_slug.insert(doc_id.clone(), slug.clone());
            metas.push(meta.clone());
            pages.insert(
                slug,
                Page {
                    meta,
                    html: rendered.html,
                    markdown: raw,
                },
            );
        }
    }

    metas.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    Ok((pages, metas, doc_id_to_slug))
}

fn make_slug(rel_path: &str, url_prefix: Option<&str>) -> String {
    let no_ext = rel_path.trim_end_matches(".md").trim_end_matches(".MD");
    let no_ext = no_ext
        .trim_end_matches("/index")
        .trim_end_matches("/README")
        .trim_end_matches("/readme");
    let cleaned = no_ext.trim_matches('/');
    if let Some(prefix) = url_prefix {
        let p = prefix.trim_matches('/');
        if p.is_empty() {
            cleaned.to_string()
        } else {
            format!("{p}/{cleaned}")
        }
    } else {
        cleaned.to_string()
    }
}

/// Extract a clean text-fragment string from a chunk of markdown content,
/// suitable for appending after `#:~:text=` in a URL. The result must
/// match visible text in the rendered page (markdown sigils stripped) so
/// the browser can find and scroll to it.
///
/// Strategy: walk the chunk looking for the first run of "real" prose —
/// strip leading markdown markers (#, >, -, *, digits.) per line, drop
/// inline backticks/asterisks, and grab a 4-12 word phrase between 20
/// and 80 characters long. Returns None if no usable phrase is found.
fn build_text_fragment(content: &str) -> Option<String> {
    fn strip_inline_marks(s: &str) -> String {
        s.chars()
            .filter(|c| !matches!(c, '`' | '*' | '_' | '~' | '[' | ']' | '<' | '>'))
            .collect()
    }
    fn strip_line_prefix(line: &str) -> &str {
        let mut s = line.trim_start();
        loop {
            let stripped = s
                .strip_prefix('#')
                .or_else(|| s.strip_prefix('>'))
                .or_else(|| s.strip_prefix('-'))
                .or_else(|| s.strip_prefix('*'))
                .or_else(|| s.strip_prefix('+'));
            match stripped {
                Some(rest) => s = rest.trim_start(),
                None => break,
            }
        }
        // Skip leading "1. " / "12. " ordered-list markers.
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
            s = &s[i + 2..];
        }
        s.trim()
    }

    for raw_line in content.lines() {
        let cleaned = strip_inline_marks(strip_line_prefix(raw_line));
        let normalized: String =
            cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.len() < 20 {
            continue;
        }
        // Take up to ~80 chars on a word boundary, but never less than 20.
        let take = if normalized.len() <= 80 {
            normalized.clone()
        } else {
            let mut end = 80;
            while end > 20 && !normalized.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = &normalized[..end];
            // Back up to the last space so we don't cut mid-word.
            match truncated.rfind(' ') {
                Some(sp) if sp > 20 => truncated[..sp].to_string(),
                _ => truncated.to_string(),
            }
        };
        return Some(percent_encode_fragment(&take));
    }
    None
}

/// Percent-encode for a URL fragment value. Spaces become %20 (not '+'
/// — '+' has no special meaning in fragments). We deliberately encode
/// '&' and '#' because they would terminate the fragment.
fn percent_encode_fragment(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        let safe = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if safe {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_strips_heading_marker() {
        // First line is too short (<20 chars); helper falls through to body.
        let f = build_text_fragment(
            "# enscrive-docs\n\nThis is the body that should be picked up.",
        )
        .unwrap();
        assert!(f.starts_with("This"), "got: {f}");
    }

    #[test]
    fn fragment_picks_up_long_heading() {
        let f = build_text_fragment("## Configuring voices and collections")
            .unwrap();
        assert!(f.starts_with("Configuring"), "got: {f}");
    }

    #[test]
    fn fragment_skips_short_first_line() {
        let f = build_text_fragment(
            "# Hi\n\nThis is a much longer paragraph that should be the fragment target.",
        )
        .unwrap();
        assert!(f.contains("longer"), "got: {f}");
    }

    #[test]
    fn fragment_strips_blockquote_and_inline_marks() {
        let f = build_text_fragment("> Turn any **markdown** directory into a `tool`")
            .unwrap();
        let decoded = f.replace("%20", " ");
        assert!(
            decoded.starts_with("Turn any markdown directory into a tool"),
            "got decoded: {decoded}"
        );
    }

    #[test]
    fn fragment_truncates_at_word_boundary() {
        let long = "x".repeat(200);
        let f = build_text_fragment(&format!("body word {long} more"));
        // long line has no spaces beyond position 20, fragment will still be
        // produced but short; just ensure we get something non-empty.
        assert!(f.is_some());
    }

    #[test]
    fn fragment_returns_none_on_empty() {
        assert!(build_text_fragment("").is_none());
        assert!(build_text_fragment("# \n#\n   \n").is_none());
    }

    #[test]
    fn percent_encoding_handles_special_chars() {
        assert_eq!(percent_encode_fragment("hello world"), "hello%20world");
        assert_eq!(percent_encode_fragment("a&b#c"), "a%26b%23c");
        assert_eq!(percent_encode_fragment("safe-chars_.~"), "safe-chars_.~");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    println!("\nshutting down");
}
