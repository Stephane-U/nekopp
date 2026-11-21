//! The feature reference, cut into sections.
//!
//! `docs/nekopp-features.md` is the source of truth for both the PDF and this
//! server, and it is embedded into the binary rather than read from disk: the
//! server is started by an MCP client from an arbitrary working directory, and
//! a documentation server that needs its repository beside it is a server that
//! breaks the first time it is installed. `include_str!` is also what keeps the
//! two copies from diverging -- there is only one copy.
//!
//! The document is flat on purpose. Every chapter is `##`, every subsection is
//! `###`, and a section's text is everything between its heading and the next
//! heading. That is enough to answer "what does this feature do" without a
//! markdown AST, and it means the anchors in the PDF and the slugs in the
//! `nekopp://features/<slug>` URIs are produced by the same rule.

use std::collections::{HashMap, HashSet};

/// The feature reference, as shipped in the repository.
pub const FEATURES_MARKDOWN: &str = include_str!("../../../docs/nekopp-features.md");

/// One heading and the markdown under it.
///
/// `body` never contains a heading: sections are cut at every `##` and `###`,
/// so a chapter's body is its own prose and its subsections are sections of
/// their own. That is what makes `get_section` return something a reader can
/// take in at once instead of twenty thousand words of menu tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading level: 2 for a chapter, 3 for a subsection.
    pub level: u8,
    /// The heading text without the leading hashes.
    pub heading: String,
    /// The URI fragment and resource name for this section.
    pub slug: String,
    /// The markdown between this heading and the next one, trimmed.
    pub body: String,
}

impl Section {
    /// The heading as it appears in the document, hashes included.
    ///
    /// Used when a section is handed back as markdown, so that what a client
    /// receives can be pasted into the document unchanged.
    pub fn markdown(&self) -> String {
        format!("{} {}\n\n{}\n", "#".repeat(self.level as usize), self.heading, self.body)
    }

    /// The first sentence of the body, for a resource description.
    ///
    /// A one-line summary in `resources/list` is worth the small amount of
    /// guessing: the alternative is a list of a hundred bare headings, and a
    /// client that shows descriptions would show nothing.
    pub fn summary(&self) -> String {
        let paragraph = self
            .body
            .lines()
            .skip_while(|line| line.trim().is_empty() || line.trim_start().starts_with(['#', '|', '-', '*']))
            .take_while(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let sentence = paragraph
            .split_once(". ")
            .map(|(head, _)| head)
            .unwrap_or(&paragraph);
        let sentence = sentence.trim().trim_end_matches('.');
        if sentence.is_empty() {
            // A section that is nothing but a table has no sentence to quote;
            // its heading is still a better description than an empty string.
            return format!("The {} section.", self.heading);
        }
        truncate(sentence, 160)
    }
}

/// The whole reference: a title and its sections, in document order.
#[derive(Debug, Clone)]
pub struct Document {
    /// The document's `# ` heading.
    pub title: String,
    /// Chapters and subsections, in the order they are printed.
    pub sections: Vec<Section>,
}

impl Document {
    /// Parse the embedded reference.
    ///
    /// Parsing costs a few hundred microseconds, so the server does it once at
    /// startup and keeps the result; this function exists so that callers do
    /// not have to know the markdown is a constant.
    pub fn embedded() -> Self {
        Self::parse(FEATURES_MARKDOWN)
    }

    /// Cut markdown at its `##` and `###` headings.
    ///
    /// Fenced code blocks are skipped while looking for headings: a shell
    /// sample with a `# comment` in it is not a chapter, and a document that
    /// is also a code sample would otherwise be cut in the wrong places.
    pub fn parse(markdown: &str) -> Self {
        let mut title = String::new();
        let mut sections = Vec::new();
        let mut used: HashMap<String, usize> = HashMap::new();
        let mut taken: HashSet<String> = HashSet::new();
        let mut open: Option<(u8, String)> = None;
        let mut body: Vec<&str> = Vec::new();
        let mut fenced = false;

        for line in markdown.lines() {
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                fenced = !fenced;
                if open.is_some() {
                    body.push(line);
                }
                continue;
            }

            if !fenced {
                if let Some((level, heading)) = heading_line(line) {
                    if level == 1 {
                        if title.is_empty() {
                            title = heading;
                        }
                        continue;
                    }
                    if let Some((level, heading)) = open.take() {
                        push_section(
                            &mut sections,
                            &mut used,
                            &mut taken,
                            level,
                            &heading,
                            &body.join("\n"),
                        );
                    }
                    body.clear();
                    open = Some((level, heading));
                    continue;
                }
            }

            if open.is_some() {
                body.push(line);
            }
        }

        if let Some((level, heading)) = open.take() {
            push_section(
                &mut sections,
                &mut used,
                &mut taken,
                level,
                &heading,
                &body.join("\n"),
            );
        }

        Self { title, sections }
    }

    /// Find a section by slug or by heading.
    ///
    /// Both spellings are accepted because an agent has the heading in front of
    /// it more often than the slug -- search results carry both -- and a tool
    /// that only accepted one of them would fail for no reason a model can see.
    /// Matching ignores case and collapses runs of whitespace.
    pub fn section(&self, key: &str) -> Option<&Section> {
        let wanted = normalise(key);
        if wanted.is_empty() {
            return None;
        }
        self.sections
            .iter()
            .find(|section| section.slug == wanted)
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|section| normalise(&section.heading) == wanted)
            })
    }

    /// The subsections of a chapter, in order.
    pub fn children(&self, slug: &str) -> Vec<&Section> {
        let mut children = Vec::new();
        let mut inside = false;
        for section in &self.sections {
            if section.slug == slug && section.level == 2 {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            if section.level <= 2 {
                break;
            }
            children.push(section);
        }
        children
    }
}

/// The stable name of a heading.
///
/// Deliberately the same rule as `slugify` in `docs/build-pdf.py`: the PDF
/// anchors and the resource URIs are two views of one document, and a section
/// that is `#encodings-and-line-endings` in the PDF has to be
/// `nekopp://features/encodings-and-line-endings` here.
pub fn slugify(heading: &str) -> String {
    let mut slug = String::new();
    for ch in heading.trim().chars() {
        if matches!(ch, '`' | '*' | '_') {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug.to_string()
    }
}

/// Lowercase, single-spaced, for comparing headings with what a client typed.
fn normalise(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Cut one line of text to at most `limit` characters, on a word boundary.
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut = String::new();
    for word in text.split_whitespace() {
        if cut.chars().count() + word.chars().count() + 1 > limit {
            break;
        }
        if !cut.is_empty() {
            cut.push(' ');
        }
        cut.push_str(word);
    }
    cut.push_str(" ...");
    cut
}

/// A `# `-style heading, if the line is one.
fn heading_line(line: &str) -> Option<(u8, String)> {
    if !line.starts_with('#') {
        return None;
    }
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let heading = rest.trim();
    if heading.is_empty() {
        return None;
    }
    Some((hashes as u8, heading.to_string()))
}

/// Append a finished section, giving duplicate headings distinct slugs.
///
/// "Plugins" is both a menu and a chapter, and a URI that resolves to two
/// sections resolves to neither. The second one becomes `plugins-2`, which is
/// also what `build-pdf.py` does to the HTML anchor.
fn push_section(
    sections: &mut Vec<Section>,
    counts: &mut HashMap<String, usize>,
    taken: &mut HashSet<String>,
    level: u8,
    heading: &str,
    body: &str,
) {
    let base = slugify(heading);
    let mut count = counts.get(&base).copied().unwrap_or(0);
    let mut slug = if count == 0 {
        base.clone()
    } else {
        format!("{base}-{count}")
    };
    while taken.contains(&slug) {
        count += 1;
        slug = format!("{base}-{count}");
    }
    counts.insert(base, count);
    taken.insert(slug.clone());

    sections.push(Section {
        level,
        heading: heading.to_string(),
        slug,
        body: body.trim().to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::embedded()
    }

    #[test]
    fn the_document_has_a_title_and_its_chapters() {
        let doc = doc();
        assert_eq!(doc.title, "Neko++ Feature Reference");
        // 27 chapters and 75 subsections: the number is the document's own
        // shape, so a section that goes missing is a test failure rather than a
        // quietly shorter table of contents.
        assert_eq!(doc.sections.len(), 102);
        assert_eq!(doc.sections.iter().filter(|s| s.level == 2).count(), 27);
        assert_eq!(doc.sections.iter().filter(|s| s.level == 3).count(), 75);
        assert!(doc.sections.iter().all(|s| !s.body.is_empty()));
    }

    #[test]
    fn slugs_are_unique_and_ascii() {
        let doc = doc();
        let mut seen = std::collections::HashSet::new();
        for section in &doc.sections {
            assert!(seen.insert(section.slug.clone()), "{}", section.slug);
            assert!(section.slug.is_ascii());
            assert!(!section.slug.starts_with('-') && !section.slug.ends_with('-'));
        }
    }

    #[test]
    fn a_repeated_heading_gets_a_numbered_slug() {
        let doc = doc();
        let plugins: Vec<_> = doc
            .sections
            .iter()
            .filter(|s| s.heading == "Plugins")
            .collect();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].slug, "plugins");
        assert_eq!(plugins[1].slug, "plugins-1");
        assert_eq!(plugins[0].level, 3);
        assert_eq!(plugins[1].level, 2);
    }

    #[test]
    fn a_section_stops_at_the_next_heading() {
        let doc = doc();
        let file = doc.section("file").expect("the File menu");
        assert!(file.body.starts_with("The document's lifecycle."));
        assert!(file.body.contains("- Print Now (`Ctrl+Alt+Shift+P`)"));
        assert!(!file.body.contains("### Edit"));
        assert!(!file.body.lines().any(|line| line.starts_with('#')));
    }

    #[test]
    fn sections_are_found_by_slug_and_by_heading() {
        let doc = doc();
        for key in [
            "encodings-and-line-endings",
            "Encodings and Line Endings",
            "  encodings   AND line endings ",
        ] {
            let section = doc.section(key).expect(key);
            assert_eq!(section.slug, "encodings-and-line-endings");
        }
        assert!(doc.section("no such section").is_none());
        assert!(doc.section("   ").is_none());
    }

    #[test]
    fn a_chapter_knows_its_subsections() {
        let doc = doc();
        let menus = doc.children("the-menus");
        assert_eq!(menus.len(), 13);
        assert_eq!(menus.first().unwrap().heading, "File");
        assert_eq!(menus.last().unwrap().heading, "Help");
        // A subsection has no children of its own.
        assert!(doc.children("file").is_empty());
    }

    #[test]
    fn the_slug_rule_matches_the_pdf_builder() {
        // These are the strings `docs/build-pdf.py` produces; if the two rules
        // ever disagree the PDF links and the resource URIs stop agreeing too.
        assert_eq!(slugify("The Menus"), "the-menus");
        assert_eq!(slugify("Unicode and Script Support"), "unicode-and-script-support");
        assert_eq!(slugify("  《Spaced》  "), "spaced");
        assert_eq!(slugify("Fix `a*b_c`"), "fix-abc");
        assert_eq!(slugify("---"), "section");
        assert_eq!(slugify("Settings/EOL"), "settings-eol");
    }

    #[test]
    fn a_heading_inside_a_code_fence_is_body_text() {
        let parsed = Document::parse(
            "# Title\n\n## Chapter\n\n```sh\n# not a chapter\n## not either\n```\n\nAfter.\n",
        );
        assert_eq!(parsed.title, "Title");
        assert_eq!(parsed.sections.len(), 1);
        assert!(parsed.sections[0].body.contains("## not either"));
        assert!(parsed.sections[0].body.contains("After."));
    }

    #[test]
    fn the_first_paragraph_is_the_summary() {
        let doc = doc();
        let file = doc.section("file").unwrap();
        assert_eq!(file.summary(), "The document's lifecycle");
        // A section that opens with a list still gets a summary from the
        // paragraph under it rather than an empty description.
        assert!(!doc.section("keyboard-shortcuts").unwrap().summary().is_empty());
    }

    #[test]
    fn a_section_renders_back_to_markdown() {
        let doc = doc();
        let section = doc.section("bookmarks").unwrap();
        let markdown = section.markdown();
        assert!(markdown.starts_with("## Bookmarks\n\n"));
        let again = Document::parse(&markdown);
        assert_eq!(again.sections.len(), 1);
        assert_eq!(again.sections[0].heading, "Bookmarks");
    }
}
