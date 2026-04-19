//! Static assets baked into the binary at build time via rust-embed.
//! Templates live in `assets/templates/`, theme CSS in `assets/themes/`,
//! the search palette JS in `assets/js/`, and any other static files in
//! `assets/static/`.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../assets"]
pub struct EmbeddedAssets;

/// Convenience: read an asset's bytes by path. Returns None if not present.
pub fn embedded_asset(path: &str) -> Option<Vec<u8>> {
    EmbeddedAssets::get(path).map(|f| f.data.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_neutral_theme_css() {
        let css = embedded_asset("themes/neutral/style.css")
            .expect("neutral theme CSS must be embedded");
        assert!(!css.is_empty());
    }

    #[test]
    fn ships_search_palette_js() {
        let js = embedded_asset("js/search.js").expect("search.js must be embedded");
        assert!(!js.is_empty());
    }
}
