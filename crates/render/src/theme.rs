//! Layered theming. Two built-in variants ("neutral" default, "enscrive"
//! brand variant). Customization layers (config tokens, custom CSS,
//! template overrides) are composed at serve time.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVariant {
    #[default]
    Neutral,
    Enscrive,
}

impl ThemeVariant {
    pub fn from_str_loose(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "enscrive" => Self::Enscrive,
            _ => Self::Neutral,
        }
    }

    pub fn css_asset_path(&self) -> &'static str {
        match self {
            Self::Neutral => "themes/neutral/style.css",
            Self::Enscrive => "themes/enscrive/style.css",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub variant: ThemeVariant,
    pub accent_color: Option<String>,
    pub custom_css: Option<String>,
    pub logo_data_url: Option<String>,
}

impl Theme {
    pub fn new(variant: ThemeVariant) -> Self {
        Self {
            variant,
            accent_color: None,
            custom_css: None,
            logo_data_url: None,
        }
    }

    pub fn with_accent_color(mut self, color: impl Into<String>) -> Self {
        self.accent_color = Some(color.into());
        self
    }

    pub fn with_custom_css(mut self, css: impl Into<String>) -> Self {
        self.custom_css = Some(css.into());
        self
    }

    /// Inline CSS variable overrides driven by [theme] tokens. Rendered into
    /// a <style> block in the page head, after the embedded theme CSS, so
    /// they win cleanly without rewriting the base stylesheet.
    pub fn css_variables(&self) -> String {
        let mut out = String::new();
        if let Some(color) = &self.accent_color {
            out.push_str(":root{--accent:");
            out.push_str(color);
            out.push_str(";}");
        }
        out
    }
}
