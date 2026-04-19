use crate::global::GlobalArgs;
use clap::Args;

#[derive(Args, Clone, Debug)]
pub struct SearchArgs {
    /// The query string
    pub query: String,

    /// Limit results to a configured collection
    #[arg(long)]
    pub collection: Option<String>,

    /// Voice name (defaults to [search] default_voice)
    #[arg(long)]
    pub voice: Option<String>,

    /// Maximum number of results
    #[arg(long, default_value_t = 10)]
    pub limit: u32,

    /// Output format: "human", "json", or "md"
    #[arg(long, default_value = "human")]
    pub format: String,
}

pub async fn run(_global: GlobalArgs, _args: SearchArgs) -> Result<(), String> {
    Err("search: not implemented yet".to_string())
}
