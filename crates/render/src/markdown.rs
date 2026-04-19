//! Markdown -> HTML pipeline. Mirrors the pattern used by the existing
//! Enscrive developer-portal docs renderer (pulldown-cmark + ammonia).

use gray_matter::engine::YAML;
use gray_matter::Matter;
use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};

/// Output of rendering a single markdown document.
pub struct RenderedMarkdown {
    pub html: String,
    pub frontmatter: Frontmatter,
    pub anchors: Vec<HeadingAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub draft: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeadingAnchor {
    pub level: u32,
    pub text: String,
    pub slug: String,
}

/// Parse the frontmatter (if any), render the markdown body to sanitized
/// HTML, and collect anchors for h2/h3 headings (used to build a TOC).
pub fn render_markdown(raw: &str) -> RenderedMarkdown {
    let matter = Matter::<YAML>::new();
    let parsed = matter.parse(raw);
    let frontmatter: Frontmatter = parsed
        .data
        .as_ref()
        .and_then(|d| d.deserialize().ok())
        .unwrap_or_default();
    let body = parsed.content;

    let anchors = collect_anchors(&body);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(&body, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);

    let sanitized = ammonia::Builder::default()
        .add_tags(["pre", "code"])
        .add_generic_attributes(["id", "class"])
        .clean(&html_out)
        .to_string();

    RenderedMarkdown {
        html: sanitized,
        frontmatter,
        anchors,
    }
}

/// Walk the parsed events looking for h2/h3 headings, slugify their text
/// (without trying to inject id="..." into the rendered HTML; that would
/// require a custom event-stream rewriter and is deferred).
fn collect_anchors(markdown: &str) -> Vec<HeadingAnchor> {
    let mut anchors = Vec::new();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    let parser = Parser::new_ext(markdown, options);

    let mut current_level: Option<u32> = None;
    let mut current_text = String::new();

    use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};
    fn level_to_u32(level: HeadingLevel) -> u32 {
        match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    }

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let lvl = level_to_u32(level);
                if matches!(lvl, 2 | 3) {
                    current_level = Some(lvl);
                    current_text.clear();
                } else {
                    current_level = None;
                }
            }
            Event::Text(t) | Event::Code(t) if current_level.is_some() => {
                current_text.push_str(&t);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_level.take() {
                    let text = current_text.trim().to_string();
                    if !text.is_empty() {
                        anchors.push(HeadingAnchor {
                            level,
                            slug: slug::slugify(&text),
                            text,
                        });
                    }
                    current_text.clear();
                }
            }
            _ => {}
        }
    }
    anchors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_basic_markdown() {
        let r = render_markdown("# Hello\n\nworld");
        assert!(r.html.contains("Hello"));
        assert!(r.html.contains("<p>world</p>"));
    }

    #[test]
    fn parses_frontmatter() {
        let r = render_markdown(
            "---\ntitle: Hi\norder: 5\n---\n# body\n",
        );
        assert_eq!(r.frontmatter.title.as_deref(), Some("Hi"));
        assert_eq!(r.frontmatter.order, Some(5));
        assert!(r.html.contains("body"));
    }

    #[test]
    fn collects_anchors() {
        let md = "# Title\n\n## Section A\n\nbody\n\n## Section B\n\nmore";
        let r = render_markdown(md);
        assert_eq!(r.anchors.len(), 2);
        assert_eq!(r.anchors[0].slug, "section-a");
        assert_eq!(r.anchors[1].slug, "section-b");
    }

    #[test]
    fn sanitizes_dangerous_html() {
        let r = render_markdown("<script>alert(1)</script>\n\nok");
        assert!(!r.html.contains("<script"));
        assert!(r.html.contains("ok"));
    }
}
