use crate::global::GlobalArgs;
use clap::Args;
use enscrive_docs_core::Config;

#[derive(Args, Clone, Debug)]
pub struct ConfigArgs {
    /// Validate only; do not print
    #[arg(long)]
    pub validate: bool,
}

pub async fn run(global: GlobalArgs, args: ConfigArgs) -> Result<(), String> {
    let path = global.resolved_config_path();
    let cfg = Config::load(&path).map_err(|e| e.to_string())?;
    if args.validate {
        println!("ok: {}", path.display());
        return Ok(());
    }
    let pretty = toml::to_string_pretty(&cfg)
        .map_err(|e| format!("serialize: {e}"))?;
    println!("# resolved from {}\n{pretty}", path.display());
    Ok(())
}
