use crate::commands::serve::{rebuild_pages, serve_with_state, setup_state, ServeArgs};
use crate::global::GlobalArgs;
use clap::Args;
use notify::{
    event::{ModifyKind, RemoveKind},
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Args, Clone, Debug)]
pub struct WatchArgs {
    /// Port to bind. Resolution: --port > $PORT > [serve] port > 3737.
    #[arg(long, env = "PORT")]
    pub port: Option<u16>,

    /// Bind address
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// URL prefix when serving behind a reverse-proxy subpath (e.g. "/docs")
    #[arg(long = "base-path")]
    pub base_path: Option<String>,

    /// Debounce window for file change events (milliseconds).
    #[arg(long, default_value_t = 250)]
    pub debounce_ms: u64,
}

pub async fn run(global: GlobalArgs, args: WatchArgs) -> Result<(), String> {
    let state = setup_state(&global, args.base_path.as_deref(), /* watch_mode */ true).await?;

    // Collect every collection's source root so the watcher can pick up
    // changes anywhere in the configured tree.
    let mut roots: Vec<PathBuf> = state
        .cfg
        .collections
        .iter()
        .map(|c| {
            if c.path.is_absolute() {
                c.path.clone()
            } else {
                state.config_dir.join(&c.path)
            }
        })
        .collect();
    roots.sort();
    roots.dedup();

    // notify::Watcher uses a sync mpsc channel — we hand it over to a
    // dedicated blocking thread that translates raw events into a
    // single-shot tokio channel for the async event-loop below.
    let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(raw_tx)
        .map_err(|e| format!("init watcher: {e}"))?;
    for root in &roots {
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| format!("watch {}: {e}", root.display()))?;
        println!("watching {}", root.display());
    }

    let (change_tx, mut change_rx) = tokio::sync::mpsc::channel::<()>(64);
    std::thread::spawn(move || {
        for event in raw_rx {
            let event = match event {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("watch error: {err}");
                    continue;
                }
            };
            if !is_relevant(&event) {
                continue;
            }
            // Channel send may fail if the receiver was dropped (server
            // shutdown). That's fine; just stop forwarding.
            if change_tx.blocking_send(()).is_err() {
                break;
            }
        }
    });

    // Background reloader: debounces file change pings, rebuilds pages,
    // then broadcasts a reload event to connected browsers. Holds the
    // watcher inside the closure so it stays alive for the program's
    // duration.
    let state_for_reload = state.clone();
    let debounce = Duration::from_millis(args.debounce_ms);
    tokio::spawn(async move {
        let _watcher = watcher; // keep alive
        loop {
            // Wait for the first event.
            if change_rx.recv().await.is_none() {
                break;
            }
            // Drain anything that arrived in the debounce window.
            let deadline = tokio::time::Instant::now() + debounce;
            loop {
                match tokio::time::timeout_at(deadline, change_rx.recv()).await {
                    Ok(Some(_)) => continue,
                    _ => break,
                }
            }
            match rebuild_pages(&state_for_reload) {
                Ok(()) => {
                    let count = state_for_reload.pages.load().len();
                    println!("reloaded — {count} page(s)");
                    if let Some(tx) = state_for_reload.event_tx.as_ref() {
                        let _ = tx.send("reload");
                    }
                }
                Err(e) => eprintln!("reload failed: {e}"),
            }
        }
    });

    // Translate WatchArgs back to a ServeArgs for the shared HTTP loop.
    let serve_args = ServeArgs {
        port: args.port,
        bind: args.bind.clone(),
        base_path: args.base_path.clone(),
    };
    println!("watch mode: edits to .md files trigger live reload");
    serve_with_state(state, &serve_args).await
}

/// Decide whether a notify::Event is worth a re-render. We only care
/// about content changes to .md files; ignore metadata-only updates,
/// directory events, and editor swap files.
fn is_relevant(event: &Event) -> bool {
    let interesting_kind = matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Create(_)
            | EventKind::Remove(RemoveKind::File)
            | EventKind::Remove(RemoveKind::Any)
    );
    if !interesting_kind {
        return false;
    }
    let mut suffixes = HashSet::new();
    for path in &event.paths {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Skip editor temp/swap files: vim .swp, emacs #foo#, JetBrains ___jb_*
            if name.starts_with('.')
                || name.starts_with('#')
                || name.contains("___jb_")
                || name.ends_with('~')
            {
                return false;
            }
        }
        if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
            suffixes.insert(ext.to_ascii_lowercase());
        }
    }
    suffixes.iter().any(|s| s == "md")
}
