//! Ranking sections against a query.
//!
//! The ranking exists so that a question about one feature comes back with that
//! feature first and the eleven places that mention it second. It is a
//! deliberately small model of relevance: a heading that is the query beats a
//! heading that contains the query, which beats a heading with one of the query's
//! words, which beats a mention in the body. Anything cleverer would be a search
//! engine, and the corpus is a hundred sections long.

use crate::doc::{Document, Section};

/// One section that matched, and the line to show for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Index into `Document::sections`.
    pub index: usize,
    /// Ranking score; higher is better.
    pub score: i64,
    /// A line of the section's body containing the query, for previewing.
    pub snippet: String,
}

/// Search the reference, best match first.
///
/// Ties are broken by document order, so the same query always returns the same
/// list: a tool that reshuffled equal matches would look like it had changed its
/// mind between two calls.
pub fn search(doc: &Document, query: &str, limit: usize) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut hits = Vec::new();
    for (index, section) in doc.sections.iter().enumerate() {
        let heading = section.heading.to_lowercase();
        let body = section.body.to_lowercase();
        let mut score = 0;
        let mut slug_score = 0;

        if heading == query {
            score += 500;
        } else if heading.contains(&query) {
            score += 250;
        }

        for term in &terms {
            let words = heading
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|word| !word.is_empty());
            if words.clone().any(|word| word.starts_with(term)) {
                score += 40;
            } else if heading.contains(term) {
                score += 20;
            }
            // A slug is derived from the heading, so it is weak evidence on its
            // own: `plugins-1` containing "plugin" says nothing the heading did
            // not already say.
            if section.slug.contains(term) {
                slug_score += 5;
            }
            // A whole word is worth three of a fragment: "bin" inside
            // "bindable" is not what a reader searching for "bin" meant.
            score += 3 * count_word(&body, term).min(12) as i64;
            score += (body.matches(term).count() - count_word(&body, term)).min(12) as i64;
        }

        // The whole query in the body is worth more than its words scattered
        // through it: "line numbers" on one line is about line numbers, whereas
        // "line" here and "numbers" there may be about nothing in particular.
        score += 10 * body.matches(&query).count().min(8) as i64;

        // A section whose slug happens to contain a word but whose text does
        // not is not a match, it is a coincidence.
        if score == 0 {
            continue;
        }
        hits.push(Hit {
            index,
            score: score + slug_score,
            snippet: snippet(section, &query, &terms),
        });
    }

    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    hits.truncate(limit);
    hits
}

/// How often `needle` appears in `haystack` as a whole word.
fn count_word(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(found) = haystack[start..].find(needle) {
        let index = start + found;
        let before = haystack[..index].chars().next_back();
        let after = haystack[index + needle.len()..].chars().next();
        let edge = |ch: Option<char>| ch.is_none_or(|ch| !ch.is_alphanumeric());
        if edge(before) && edge(after) {
            count += 1;
        }
        start = index + needle.len();
    }
    count
}

/// The line of the body that best shows why the section matched.
///
/// Preference order: the whole query, then a term as a word, then a term at
/// all. A snippet that shows "bindable" for the query "bin" is not wrong, but
/// it is not the line a reader wanted either.
fn snippet(section: &Section, query: &str, terms: &[&str]) -> String {
    let mut fallback = String::new();
    let mut loose: Option<&str> = None;
    for line in section.body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if fallback.is_empty() {
            fallback = trimmed.to_string();
        }
        let lowered = trimmed.to_lowercase();
        if lowered.contains(query) {
            return shorten(strip_markdown(trimmed), 200);
        }
        if loose.is_none() && terms.iter().any(|term| lowered.contains(term)) {
            loose = Some(trimmed);
        }
        if terms.iter().any(|term| count_word(&lowered, term) > 0) {
            return shorten(strip_markdown(trimmed), 200);
        }
    }
    shorten(strip_markdown(loose.unwrap_or(&fallback)), 200)
}

/// Drop the list bullet or quote marker from the front of a line.
fn strip_markdown(line: &str) -> &str {
    let line = line.trim_start_matches(['-', '*', '>', ' ']);
    line.trim_start()
}

/// Keep a snippet to one readable length, on a character boundary.
fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str(" ...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::embedded()
    }

    fn headings<'a>(doc: &'a Document, hits: &'a [Hit]) -> Vec<&'a str> {
        hits.iter()
            .map(|hit| doc.sections[hit.index].heading.as_str())
            .collect()
    }

    #[test]
    fn an_exact_heading_comes_first() {
        let doc = doc();
        let hits = search(&doc, "Bookmarks", 5);
        assert_eq!(headings(&doc, &hits)[0], "Bookmarks");
    }

    #[test]
    fn a_heading_match_beats_a_mention_in_the_body() {
        let doc = doc();
        let hits = search(&doc, "folding", 10);
        assert_eq!(doc.sections[hits[0].index].heading, "Folding");
        // Folding is mentioned all over the document, and every one of those
        // sections still has to come after the chapter about it.
        assert!(
            doc.sections[hits[1].index]
                .heading
                .to_lowercase()
                .contains("folding")
                || hits[1].score < hits[0].score
        );
    }

    #[test]
    fn scores_are_ordered_and_the_limit_is_honoured() {
        let doc = doc();
        let hits = search(&doc, "line", 7);
        assert_eq!(hits.len(), 7);
        for pair in hits.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
        // The same query twice is the same list: ties fall back to document
        // order rather than to whatever the sort happened to do.
        assert_eq!(search(&doc, "line", 7), hits);
    }

    #[test]
    fn a_query_with_no_matches_is_an_empty_list() {
        let doc = doc();
        assert!(search(&doc, "quantum chromodynamics", 5).is_empty());
        assert!(search(&doc, "   ", 5).is_empty());
        assert!(search(&doc, "fold", 0).is_empty());
    }

    #[test]
    fn a_multi_word_query_finds_the_section_that_names_it() {
        let doc = doc();
        let hits = search(&doc, "find and replace", 5);
        assert_eq!(headings(&doc, &hits)[0], "Find and Replace");
        // The chapter is a heading of its own, so the menu row that carries the
        // same words ranks below it rather than above.
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn the_snippet_shows_the_line_that_matched() {
        let doc = doc();
        let hits = search(&doc, "recycle bin", 20);
        let hit = hits
            .iter()
            .find(|hit| doc.sections[hit.index].heading == "File")
            .expect("the File menu mentions the recycle bin");
        assert!(
            hit.snippet.to_lowercase().contains("recycle bin"),
            "{}",
            hit.snippet
        );
        assert!(!hit.snippet.starts_with("- "));
        assert!(hit.snippet.chars().count() <= 204);
    }

    #[test]
    fn a_word_beats_a_fragment_of_a_longer_word() {
        let doc = doc();
        // "bin" appears inside "bindable" all over the keymap chapter; the
        // section that talks about a bin has to rank above them.
        let hits = search(&doc, "recycle bin", 5);
        let first = &doc.sections[hits[0].index];
        assert!(
            first.body.to_lowercase().contains("recycle bin"),
            "{} won for the wrong reason",
            first.heading
        );
    }

    #[test]
    fn every_section_can_be_found_by_its_own_heading() {
        // The inventory is only useful if a section can be reached from its
        // name, so this is checked against all hundred of them rather than one.
        let doc = doc();
        for (index, section) in doc.sections.iter().enumerate() {
            let hits = search(&doc, &section.heading, 3);
            assert!(
                hits.iter().any(|hit| hit.index == index),
                "{} not in its own results: {:?}",
                section.heading,
                headings(&doc, &hits)
            );
        }
    }
}
