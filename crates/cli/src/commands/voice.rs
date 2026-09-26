//! `enscrive-docs voice <subcommand>` — voice administration.
//!
//! For now this surfaces `voice tune` only. Future verbs (list, show, create,
//! delete) can slot in alongside without restructuring main.rs.

use crate::global::GlobalArgs;
use clap::{Args, Subcommand};
use enscrive_docs_core::redact::redact_excerpt;
use enscrive_docs_core::{Config, EnscriveClient, UpdateVoiceApiRequest, VoiceConfigApi};
use std::io::Write;
use std::process::Command as OsCommand;

#[derive(Args, Clone, Debug)]
pub struct VoiceArgs {
    #[command(subcommand)]
    pub command: VoiceCommand,
}

#[derive(Subcommand, Clone, Debug)]
pub enum VoiceCommand {
    /// Open the voice config in $EDITOR, validate, and PUT back to Enscrive
    Tune(TuneArgs),
}

#[derive(Args, Clone, Debug)]
pub struct TuneArgs {
    /// Name of the voice to tune (as known to Enscrive — same as the `name`
    /// field on `[[voices]]` in enscrive-docs.toml)
    pub voice: String,
}

pub async fn run(global: GlobalArgs, args: VoiceArgs) -> Result<(), String> {
    match args.command {
        VoiceCommand::Tune(a) => tune(global, a).await,
    }
}

async fn tune(global: GlobalArgs, args: TuneArgs) -> Result<(), String> {
    let config_path = global.resolved_config_path();
    let cfg = Config::load(&config_path).map_err(|e| e.to_string())?;

    let api_key = cfg
        .resolved_api_key(global.api_key.as_deref())
        .map_err(|e| e.to_string())?;
    let endpoint = cfg.resolved_endpoint(global.endpoint.as_deref());
    let provider_key = cfg.resolved_provider_key(global.embedding_provider_key.as_deref());
    let live_credentials = [api_key.clone(), provider_key.clone().unwrap_or_default()];
    let client = EnscriveClient::with_provider_key(endpoint, api_key, provider_key);
    let credentials = live_credentials
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    // Resolve voice_id by name.
    let voices = client.list_voices().await.map_err(|e| e.to_string())?;
    let voice = voices
        .iter()
        .find(|v| v.name == args.voice)
        .ok_or_else(|| {
            format!(
                "voice \"{}\" not found in tenant. Run `enscrive-docs bootstrap` to create it, \
                 or check the name in enscrive-docs.toml.",
                args.voice
            )
        })?;

    let fresh = client.get_voice(&voice.id).await.map_err(|e| {
        format!(
            "get voice: {}",
            redact_excerpt(&e.to_string(), &credentials, 200)
        )
    })?;

    // Serialize current config to TOML for editing.
    let serialized =
        toml::to_string_pretty(&fresh.config).map_err(|e| format!("serialize config: {e}"))?;
    // The config is the user's own data and is PUT back after editing, so it
    // must NOT be redacted (a redacted value would be written back).
    let before = serialized;
    let header = format!(
        "# enscrive-docs voice tune — editing \"{}\" (id: {}, version: {})\n\
         # Save + exit to PUT back to Enscrive. Leave unchanged to abort.\n\n",
        redact_excerpt(&fresh.name, &credentials, 200),
        redact_excerpt(&fresh.id, &credentials, 200),
        fresh.version
    );
    let initial = format!("{header}{before}");

    let safe_name = redact_excerpt(&fresh.name, &credentials, 200);
    let edited = open_in_editor(&initial, &safe_name)?;

    // Strip the header comments we added so round-trip compares cleanly.
    let edited_stripped = edited
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') || !looks_like_our_header(l))
        .collect::<Vec<_>>()
        .join("\n");

    if edited_stripped.trim() == before.trim() {
        println!(
            "voice \"{}\" unchanged; aborting.",
            redact_excerpt(&fresh.name, &credentials, 200)
        );
        return Ok(());
    }

    let new_config = parse_edited_voice_config(&edited_stripped)?;

    let updated = client
        .update_voice(&fresh.id, &UpdateVoiceApiRequest { config: new_config })
        .await
        .map_err(|e| {
            format!(
                "PUT /v1/voices/{{id}}: {}",
                redact_excerpt(&e.to_string(), &credentials, 200)
            )
        })?;

    println!(
        "voice \"{}\" updated (version: {} -> {})",
        redact_excerpt(&updated.name, &credentials, 200),
        fresh.version,
        updated.version
    );
    Ok(())
}

fn parse_edited_voice_config(edited: &str) -> Result<VoiceConfigApi, String> {
    toml::from_str(edited).map_err(|error: toml::de::Error| {
        match error.span().and_then(|span| edited.get(..span.start)) {
            Some(prefix) => {
                let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
                let column = prefix
                    .rsplit_once('\n')
                    .map_or(prefix.chars().count(), |(_, last_line)| {
                        last_line.chars().count()
                    })
                    + 1;
                format!("edited voice config is not valid TOML (line {line}, column {column})")
            }
            None => "edited voice config is not valid TOML".to_string(),
        }
    })
}

/// Write `initial` to a temp file, invoke `$EDITOR` (fallback: vi), read back.
fn open_in_editor(initial: &str, voice_name: &str) -> Result<String, String> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let mut path = std::env::temp_dir();
    let pid = std::process::id();
    path.push(format!("enscrive-docs-voice-{voice_name}-{pid}.toml"));

    {
        let mut f = std::fs::File::create(&path)
            .map_err(|e| format!("create tempfile {}: {e}", path.display()))?;
        f.write_all(initial.as_bytes())
            .map_err(|e| format!("write tempfile: {e}"))?;
    }

    let status = OsCommand::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| format!("spawn editor ({editor}): {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err(format!("editor exited with status {status}; aborting"));
    }

    let content = std::fs::read_to_string(&path).map_err(|e| format!("read tempfile back: {e}"))?;
    let _ = std::fs::remove_file(&path);
    Ok(content)
}

fn looks_like_our_header(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("# enscrive-docs voice tune") || trimmed.starts_with("# Save + exit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_edited_toml_does_not_echo_offending_text() {
        let sentinel = "fake-provider-sentinel-42";
        let edited = format!("provider_token = \"{sentinel}");

        let error = parse_edited_voice_config(&edited).unwrap_err();

        assert!(error.starts_with("edited voice config is not valid TOML"));
        assert!(!error.contains(sentinel), "parser text leaked: {error}");
    }
}
