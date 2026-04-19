use crate::global::GlobalArgs;
use clap::Args;

#[derive(Args, Clone, Debug)]
pub struct IngestArgs {
    /// Limit to a single configured collection by name
    #[arg(long)]
    pub collection: Option<String>,

    /// Build the request without sending it to Enscrive
    #[arg(long)]
    pub dry_run: bool,

    /// Re-ingest even when content fingerprints are unchanged
    #[arg(long)]
    pub force: bool,
}

pub async fn run(_global: GlobalArgs, _args: IngestArgs) -> Result<(), String> {
    Err("ingest: not implemented yet".to_string())
}
