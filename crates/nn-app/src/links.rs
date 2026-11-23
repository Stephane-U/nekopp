//! Where the Help menu goes.
//!
//! The addresses live here rather than at the four `open_page` calls in
//! `main.rs`, because a link is the one thing in the program that a window
//! cannot show you is wrong: it either opens a browser or it does not, and the
//! only way to check a string is to read it. From a module, a test can.
//!
//! `HOME`, `ISSUES` and `MANUAL` are this project's pages. `community` is the
//! About box's LinkedIn address, taken from there rather than copied, so the two
//! cannot drift apart.

/// The project's home: the repository, whose README says how to build it.
pub const HOME: &str = "https://github.com/Stephane-U/nekopp";
/// Where a bug report or a question goes.
pub const ISSUES: &str = "https://github.com/Stephane-U/nekopp/issues";
/// The reference document, rendered by GitHub from the markdown the PDF is built
/// from.
pub const MANUAL: &str =
    "https://github.com/Stephane-U/nekopp/blob/main/docs/nekopp-features.md";

/// The community row: where the author can be reached.
pub fn community() -> &'static str {
    crate::about::link_url()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_https_on_this_project() {
        for url in [HOME, ISSUES, MANUAL] {
            assert!(
                url.starts_with("https://github.com/Stephane-U/nekopp"),
                "{url}"
            );
        }
        assert!(community().starts_with("https://"), "{}", community());
    }

    #[test]
    fn the_manual_is_the_reference_the_pdf_is_built_from() {
        assert!(
            MANUAL.ends_with("/blob/main/docs/nekopp-features.md"),
            "{MANUAL}"
        );
    }

    #[test]
    fn the_community_link_is_the_about_box_link() {
        assert_eq!(community(), crate::about::link_url());
    }
}
