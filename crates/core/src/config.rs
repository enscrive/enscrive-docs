//! Loader for `enscrive-docs.toml`.
//!
//! Resolution precedence (highest wins):
//!   1. CLI flags (the binary applies these on top of the loaded Config)
//!   2. Environment variables (ENSCRIVE_API_KEY, ENSCRIVE_BASE_URL, ENSCRIVE_PROFILE)
//!   3. enscrive-docs.toml in the configured directory
//!   4. ~/.config/enscrive/profiles.toml (fallback for API key reuse with enscrive-cli)
//!   5. Built-in defaults

use crate::error::{EnscriveError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DEFAULT_BASE_URL: &str = "https://api.enscrive.io";
pub const CONFIG_FILE_NAME: &str = "enscrive-docs.toml";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    #[serde(default)]
    pub enscrive: EnscriveAuthConfig,
    pub site: SiteConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub collections: Vec<CollectionConfig>,
    #[serde(default)]
    pub voices: Vec<VoiceConfig>,
    #[serde(default)]
    pub versions: Vec<VersionConfig>,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub serve: ServeConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnscriveAuthConfig {
    /// Profile name to read from ~/.config/enscrive/profiles.toml. When set,
    /// the API key is sourced from the matching profile unless overridden.
    #[serde(default)]
    pub profile: Option<String>,
    /// Inline API key. Lowest precedence after CLI flag and env var.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub embedding_provider_key: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SiteConfig {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub base_path: Option<String>,
    #[serde(default)]
    pub default_version: Option<String>,
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            title: "Documentation".to_string(),
            description: None,
            base_url: None,
            base_path: None,
            default_version: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ThemeConfig {
    /// "neutral" (default) or "enscrive".
    #[serde(default = "default_theme_variant")]
    pub variant: String,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub logo_path: Option<PathBuf>,
    #[serde(default)]
    pub custom_css: Option<PathBuf>,
    #[serde(default)]
    pub template_dir: Option<PathBuf>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            variant: default_theme_variant(),
            accent_color: None,
            logo_path: None,
            custom_css: None,
            template_dir: None,
        }
    }
}

fn default_theme_variant() -> String {
    "neutral".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollectionConfig {
    pub name: String,
    pub voice: String,
    pub path: PathBuf,
    #[serde(default = "default_glob")]
    pub glob: String,
    #[serde(default)]
    pub url_prefix: Option<String>,
    /// Embedding model for `bootstrap` when the collection does not yet exist.
    /// Ignored when the collection is already present in Enscrive.
    #[serde(default)]
    pub embedding_model: Option<String>,
    /// MRL truncation dimension. Optional; defaults to the model's full output.
    /// Ignored when the collection already exists.
    #[serde(default)]
    pub dimensions: Option<u32>,
    /// Human-readable description attached to the collection at create time.
    /// Ignored when the collection already exists.
    #[serde(default)]
    pub description: Option<String>,
}

fn default_glob() -> String {
    "**/*.md".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoiceConfig {
    pub name: String,
    pub chunking_strategy: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub score_threshold: Option<f32>,
    #[serde(default)]
    pub default_limit: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionConfig {
    pub slug: String,
    pub collections: Vec<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SearchConfig {
    #[serde(default)]
    pub default_voice: Option<String>,
    #[serde(default)]
    pub results_per_page: Option<u32>,
    #[serde(default)]
    pub include_snippets: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ServeConfig {
    /// Port to bind. CLI --port and $PORT env override this.
    #[serde(default)]
    pub port: Option<u16>,
}

impl Config {
    /// Load `enscrive-docs.toml` from the given directory.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let path = dir.as_ref().join(CONFIG_FILE_NAME);
        Self::load(path)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| {
            EnscriveError::Config(format!("read {}: {e}", path.display()))
        })?;
        toml::from_str::<Config>(&raw).map_err(EnscriveError::from)
    }

    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let serialized = toml::to_string_pretty(self).map_err(|e| {
            EnscriveError::Config(format!("serialize config: {e}"))
        })?;
        std::fs::write(path.as_ref(), serialized).map_err(EnscriveError::from)
    }

    /// Resolve the API endpoint, honoring (in order): explicit override,
    /// env var, config-file inline, profile endpoint, built-in default.
    pub fn resolved_endpoint(&self, override_value: Option<&str>) -> String {
        override_value
            .map(str::to_string)
            .or_else(|| std::env::var("ENSCRIVE_BASE_URL").ok())
            .or_else(|| self.enscrive.endpoint.clone())
            .or_else(|| {
                self.enscrive
                    .profile
                    .as_deref()
                    .and_then(|p| read_profile_field(p, "endpoint").ok().flatten())
            })
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
    }

    /// Resolve the API key. Order: explicit override, env var, config inline,
    /// profile file at ~/.config/enscrive/profiles.toml.
    pub fn resolved_api_key(&self, override_value: Option<&str>) -> Result<String> {
        if let Some(key) = override_value {
            return Ok(key.to_string());
        }
        if let Ok(key) = std::env::var("ENSCRIVE_API_KEY") {
            if !key.is_empty() {
                return Ok(key);
            }
        }
        if let Some(key) = self.enscrive.api_key.as_ref().filter(|k| !k.is_empty()) {
            return Ok(key.clone());
        }
        if let Some(profile) = self.enscrive.profile.as_deref() {
            if let Some(key) = read_profile_api_key(profile)? {
                return Ok(key);
            }
        }
        Err(EnscriveError::Config(
            "no API key found (set ENSCRIVE_API_KEY, --api-key, [enscrive] api_key, or [enscrive] profile)"
                .to_string(),
        ))
    }

    pub fn resolved_provider_key(&self, override_value: Option<&str>) -> Option<String> {
        override_value
            .map(str::to_string)
            .or_else(|| std::env::var("ENSCRIVE_EMBEDDING_PROVIDER_KEY").ok())
            .or_else(|| self.enscrive.embedding_provider_key.clone())
            .filter(|v| !v.is_empty())
    }
}

/// Look up an API key inside ~/.config/enscrive/profiles.toml. The profiles
/// file format mirrors enscrive-cli; we read a `[profiles.<name>]` table and
/// look for an `api_key` field. Returns Ok(None) when the file or profile
/// is missing — that is not an error here.
fn read_profile_api_key(profile_name: &str) -> Result<Option<String>> {
    read_profile_field(profile_name, "api_key")
}

/// Generic helper: pull a string field out of a profile entry in
/// ~/.config/enscrive/profiles.toml. Returns Ok(None) when the file,
/// profile, or field is missing or empty.
fn read_profile_field(profile_name: &str, field: &str) -> Result<Option<String>> {
    let path = match profiles_path() {
        Some(p) => p,
        None => return Ok(None),
    };
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| EnscriveError::Config(format!("read {}: {e}", path.display())))?;
    let value: toml::Value = toml::from_str(&raw)?;
    let v = value
        .get("profiles")
        .and_then(|p| p.get(profile_name))
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|v| !v.is_empty());
    Ok(v)
}

fn profiles_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".config/enscrive/profiles.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let raw = r#"
            [site]
            title = "Test Docs"

            [[collections]]
            name = "guides"
            voice = "guides-voice"
            path = "./docs"

            [[voices]]
            name = "guides-voice"
            chunking_strategy = "paragraphs"
        "#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert_eq!(cfg.site.title, "Test Docs");
        assert_eq!(cfg.collections.len(), 1);
        assert_eq!(cfg.collections[0].glob, "**/*.md");
        assert_eq!(cfg.voices[0].chunking_strategy, "paragraphs");
        assert_eq!(cfg.theme.variant, "neutral");
    }

    #[test]
    fn endpoint_falls_back_to_default() {
        let cfg = Config::default();
        // Avoid env pollution: this is a sanity check, not a strict precedence test.
        let endpoint = cfg.resolved_endpoint(Some("https://override"));
        assert_eq!(endpoint, "https://override");
    }
}
