// Neko++ -- a GPU-rendered text editor.
// Copyright (C) 2026 Stéphane Urdy
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, version 3 of the License.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.

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

/// The licence: the GNU project's own page for the GPLv3, which is the text in
/// `LICENSE` and the one an offline copy cannot show.
pub const LICENCE: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

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
        assert!(LICENCE.starts_with("https://"), "{LICENCE}");
    }

    #[test]
    fn the_licence_row_opens_the_gpl() {
        // The source tree's `LICENSE` is the same document, but a window cannot
        // open a file at a path it does not know, and a link that pointed at a
        // Notepad++ page would be a copy of somebody else's terms.
        assert_eq!(LICENCE, "https://www.gnu.org/licenses/gpl-3.0.html");
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
