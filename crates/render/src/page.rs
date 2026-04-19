//! A single rendered documentation page in memory. Produced by the serve
//! pipeline by combining a markdown source file with its rendered HTML and
//! frontmatter, then served from an in-memory cache.

use crate::markdown::{Frontmatter, HeadingAnchor};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PageMeta {
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub order: i32,
    pub source_path: String,
    pub url_prefix: Option<String>,
    pub anchors: Vec<HeadingAnchor>,
}

/// One renderable doc page. `html` and `markdown` are kept side-by-side so
/// both `?format=md` and the default HTML view can be served from the same
/// cache entry.
#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub meta: PageMeta,
    pub html: String,
    pub markdown: String,
}

impl PageMeta {
    pub fn from_frontmatter(
        slug: String,
        source_path: String,
        url_prefix: Option<String>,
        frontmatter: &Frontmatter,
        anchors: Vec<HeadingAnchor>,
    ) -> Self {
        Self::build(slug, source_path, url_prefix, frontmatter, anchors, None)
    }

    /// Same as `from_frontmatter` but lets callers feed in the leading
    /// h1 extracted from the rendered markdown as a title fallback.
    /// Resolution: frontmatter title > leading h1 > slug-derived.
    pub fn build(
        slug: String,
        source_path: String,
        url_prefix: Option<String>,
        frontmatter: &Frontmatter,
        anchors: Vec<HeadingAnchor>,
        leading_h1: Option<String>,
    ) -> Self {
        let title = frontmatter
            .title
            .clone()
            .or(leading_h1)
            .unwrap_or_else(|| derive_title_from_slug(&slug));
        let description = frontmatter.description.clone();
        let order = frontmatter.order.unwrap_or(0);
        Self {
            slug,
            title,
            description,
            order,
            source_path,
            url_prefix,
            anchors,
        }
    }

    pub fn url(&self, base_path: &str) -> String {
        let prefix = self.url_prefix.as_deref().unwrap_or("");
        let base = base_path.trim_end_matches('/');
        format!(
            "{base}{prefix}/{slug}",
            base = base,
            prefix = if prefix.is_empty() {
                "".to_string()
            } else if prefix.starts_with('/') {
                prefix.to_string()
            } else {
                format!("/{prefix}")
            },
            slug = self.slug,
        )
    }
}

fn derive_title_from_slug(slug: &str) -> String {
    slug.split(&['-', '_', '/'][..])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_with_prefix_and_base_path() {
        let mut meta = PageMeta {
            slug: "guide".into(),
            title: "Guide".into(),
            description: None,
            order: 0,
            source_path: "guides/guide.md".into(),
            url_prefix: Some("/guides".into()),
            anchors: vec![],
        };
        assert_eq!(meta.url("/docs"), "/docs/guides/guide");
        meta.url_prefix = None;
        assert_eq!(meta.url(""), "/guide");
    }

    #[test]
    fn title_falls_back_to_slug() {
        let frontmatter = Frontmatter::default();
        let meta = PageMeta::from_frontmatter(
            "getting-started".into(),
            "getting-started.md".into(),
            None,
            &frontmatter,
            vec![],
        );
        assert_eq!(meta.title, "Getting Started");
    }
}
