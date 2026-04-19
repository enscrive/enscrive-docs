use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "enscrive-docs", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Ingest,
    Serve,
    Watch,
    Search { query: String },
    View,
    Eval,
    Version,
    Config,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => println!("init: not implemented"),
        Command::Ingest => println!("ingest: not implemented"),
        Command::Serve => println!("serve: not implemented"),
        Command::Watch => println!("watch: not implemented"),
        Command::Search { query } => println!("search: not implemented (query: {query})"),
        Command::View => println!("view: not implemented"),
        Command::Eval => println!("eval: not implemented"),
        Command::Version => println!("version: not implemented"),
        Command::Config => println!("config: not implemented"),
    }
}
