mod commands;
mod global;

use clap::{Parser, Subcommand};
use global::GlobalArgs;

#[derive(Parser)]
#[command(
    name = "enscrive-docs",
    version,
    about = "Retrieval-native documentation backed by Enscrive"
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

    /// Push the configured markdown directories into Enscrive collections
    Ingest(commands::ingest::IngestArgs),

    /// Serve the docs as HTML + JSON search + /llms.txt
    Serve(commands::serve::ServeArgs),

    /// One-shot semantic search against the configured collections
    Search(commands::search::SearchArgs),

    /// Inspect the resolved configuration
    Config(commands::config::ConfigArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init(args) => commands::init::run(cli.global, args).await,
        Command::Ingest(args) => commands::ingest::run(cli.global, args).await,
        Command::Serve(args) => commands::serve::run(cli.global, args).await,
        Command::Search(args) => commands::search::run(cli.global, args).await,
        Command::Config(args) => commands::config::run(cli.global, args).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
