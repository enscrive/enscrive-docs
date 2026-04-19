use crate::global::GlobalArgs;
use clap::Args;

#[derive(Args, Clone, Debug)]
pub struct ServeArgs {
    /// Port to bind
    #[arg(long, default_value_t = 8080)]
    pub port: u16,

    /// Bind address
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// URL prefix when serving behind a reverse-proxy subpath (e.g. "/docs")
    #[arg(long = "base-path")]
    pub base_path: Option<String>,
}

pub async fn run(_global: GlobalArgs, _args: ServeArgs) -> Result<(), String> {
    Err("serve: not implemented yet".to_string())
}
