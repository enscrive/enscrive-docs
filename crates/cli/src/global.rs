use clap::Args;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct GlobalArgs {
    /// Path to enscrive-docs.toml. Defaults to the current directory.
    #[arg(long = "config", short = 'c', global = true)]
    pub config_path: Option<PathBuf>,

    /// API key (or set ENSCRIVE_API_KEY)
    #[arg(long = "api-key", env = "ENSCRIVE_API_KEY", global = true)]
    pub api_key: Option<String>,

    /// Base URL of the Enscrive API (default https://api.enscrive.io)
    #[arg(long = "endpoint", env = "ENSCRIVE_BASE_URL", global = true)]
    pub endpoint: Option<String>,

    /// Optional BYOK embedding provider key (X-Embedding-Provider-Key header)
    #[arg(
        long = "embedding-provider-key",
        env = "ENSCRIVE_EMBEDDING_PROVIDER_KEY",
        global = true
    )]
    pub embedding_provider_key: Option<String>,

    /// Named profile from ~/.config/enscrive/profiles.toml (overrides config file)
    #[arg(long = "profile", env = "ENSCRIVE_PROFILE", global = true)]
    pub profile: Option<String>,
}

impl GlobalArgs {
    pub fn resolved_config_path(&self) -> PathBuf {
        match &self.config_path {
            Some(p) => p.clone(),
            None => std::env::current_dir()
                .map(|d| d.join(enscrive_docs_core::config::CONFIG_FILE_NAME))
                .unwrap_or_else(|_| {
                    PathBuf::from(enscrive_docs_core::config::CONFIG_FILE_NAME)
                }),
        }
    }
}
