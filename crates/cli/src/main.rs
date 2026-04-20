mod commands;
mod global;

use clap::{Parser, Subcommand};
use global::GlobalArgs;

#[derive(Parser)]
#[command(
    name = "enscrive-docs",
    version,
    about = "Retrieval-native documentation backed by Enscrive neural search"
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold an enscrive-docs.toml in the current directory
    Init(commands::init::InitArgs),

    /// Idempotently create voices + collections from config, then first ingest
    Bootstrap(commands::bootstrap::BootstrapArgs),

    /// Push the configured markdown directories into Enscrive collections
    Ingest(commands::ingest::IngestArgs),

    /// Serve the docs as HTML + JSON search + /llms.txt
    Serve(commands::serve::ServeArgs),

    /// Serve + watch markdown files; auto-reload the browser on save
    Watch(commands::watch::WatchArgs),

    /// One-shot neural search against the configured collections
    Search(commands::search::SearchArgs),

    /// Voice administration (tune, …)
    Voice(commands::voice::VoiceArgs),

    /// Delete and recreate configured collections (destructive, --yes required)
    Reset(commands::reset::ResetArgs),

    /// Inspect the resolved configuration
    Config(commands::config::ConfigArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init(args) => commands::init::run(cli.global, args).await,
        Command::Bootstrap(args) => commands::bootstrap::run(cli.global, args).await,
        Command::Ingest(args) => commands::ingest::run(cli.global, args).await,
        Command::Serve(args) => commands::serve::run(cli.global, args).await,
        Command::Watch(args) => commands::watch::run(cli.global, args).await,
        Command::Search(args) => commands::search::run(cli.global, args).await,
        Command::Voice(args) => commands::voice::run(cli.global, args).await,
        Command::Reset(args) => commands::reset::run(cli.global, args).await,
        Command::Config(args) => commands::config::run(cli.global, args).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
