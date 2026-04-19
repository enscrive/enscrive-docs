use crate::global::GlobalArgs;
use clap::Args;
use std::path::Path;

#[derive(Args, Clone, Debug)]
pub struct InitArgs {
    /// Theme variant to scaffold ("neutral" or "enscrive")
    #[arg(long, default_value = "neutral")]
    pub theme: String,

    /// Overwrite an existing enscrive-docs.toml
    #[arg(long)]
    pub force: bool,

    /// Skip the API-key probe of the local profiles.toml
    #[arg(long)]
    pub no_profile_detect: bool,
}

pub async fn run(global: GlobalArgs, args: InitArgs) -> Result<(), String> {
    let target = global.resolved_config_path();

    if target.exists() && !args.force {
        return Err(format!(
            "{} already exists; pass --force to overwrite",
            target.display()
        ));
    }

    let detected_profile = if args.no_profile_detect {
        None
    } else {
        global
            .profile
            .clone()
            .or_else(detected_default_profile)
    };

    let scaffold = scaffold_toml(&args.theme, detected_profile.as_deref());

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&target, scaffold)
        .map_err(|e| format!("write {}: {e}", target.display()))?;

    println!("scaffolded {}", target.display());
    if let Some(profile) = detected_profile {
        println!("detected enscrive-cli profile: {profile} (used for API key reuse)");
    } else {
        println!(
            "set ENSCRIVE_API_KEY or fill in [enscrive] api_key before running `enscrive-docs ingest`"
        );
    }
    println!("next: edit collections + voices, then run `enscrive-docs ingest`");
    Ok(())
}

/// Best-effort detection of an existing enscrive-cli profile name.
/// Returns the first profile name found in `~/.config/enscrive/profiles.toml`,
/// or None if the file does not exist or has no profiles.
fn detected_default_profile() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let path = Path::new(&home).join(".config/enscrive/profiles.toml");
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    value
        .get("profiles")?
        .as_table()?
        .keys()
        .next()
        .cloned()
}

fn scaffold_toml(theme_variant: &str, profile: Option<&str>) -> String {
    let profile_line = match profile {
        Some(name) => format!("profile = \"{name}\""),
        None => "# profile = \"default\"  # uncomment after creating ~/.config/enscrive/profiles.toml".to_string(),
    };
    format!(
        r##"# enscrive-docs.toml
#
# This file describes how enscrive-docs ingests your markdown into Enscrive
# and how it serves the resulting documentation. See https://docs.enscrive.io
# for the full configuration reference.

[enscrive]
{profile_line}
# api_key = "..."                  # or set ENSCRIVE_API_KEY in the environment
# endpoint = "https://api.enscrive.io"

[site]
title = "My App Docs"
description = "Documentation for my application."
# base_url = "https://app.example.com/docs"
# base_path = "/docs"              # set when serving behind a reverse-proxy subpath
# default_version = "latest"

[theme]
variant = "{theme_variant}"        # "neutral" (default) or "enscrive"
# accent_color = "#0ea5e9"
# logo_path = "./assets/logo.svg"
# custom_css = "./custom.css"
# template_dir = "./templates"

# Each [[collections]] entry maps a directory of markdown files to an
# Enscrive collection (created in advance via the Enscrive UI or CLI).
[[collections]]
name = "guides"
voice = "guides-voice"
path = "./docs"
glob = "**/*.md"
# url_prefix = "/guides"

# Each [[voices]] entry must already exist in your Enscrive tenant.
# enscrive-docs verifies them on `ingest`. Future versions will optionally
# create missing voices from this config.
#
# A note on score_threshold: the default of 0.0 surfaces all matches with
# their natural relevance scores. Raise it (0.2-0.5) once you have enough
# content to filter aggressively. Setting it too high before you have
# many docs ingested will produce empty search results.
[[voices]]
name = "guides-voice"
chunking_strategy = "baseline"
# parameters = {{ min_tokens = "256", max_tokens = "512" }}
score_threshold = 0.0
default_limit = 10

[search]
default_voice = "guides-voice"
results_per_page = 10
include_snippets = true
"##
    )
}
