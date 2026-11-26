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

//! The window's share of the translations: the list of labels to check.
//!
//! The catalogue itself lives in [`nn_shared::i18n`], because the words it
//! translates are the ones both front-ends draw. What is left here is the one
//! question only a window can ask -- does every bundled language cover every
//! label this program puts on screen -- since the widgets below are the only
//! thing that knows what the window draws.

pub use nn_shared::i18n::*;

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// Every label the window draws outside the menu bar.
    ///
    /// Each module is asked for its own list rather than a copy kept here: a
    /// widget that grows a label adds it to its own module's list or it is not
    /// translated, and the test below fails before it can ship in English.
    fn chrome_labels() -> Vec<&'static str> {
        let mut labels: Vec<&'static str> = Vec::new();
        labels.extend(crate::prefs::Preferences::labels());
        labels.extend(crate::statusbar::StatusBar::labels());
        labels.extend(crate::prompt::PromptBar::labels());
        labels.extend(crate::findbar::FindBar::labels());
        labels.extend(crate::finddialog::FindDialog::labels());
        labels.extend(crate::about::AboutBox::labels());
        labels.extend(crate::pluginlist::PluginList::labels());
        labels.extend(crate::toolbar::labels());
        labels.sort_unstable();
        labels.dedup();
        labels
    }

    /// No bundled language may leave a word of the window in English.
    ///
    /// The menu bar has the same test in `menu`, next to the rows it is about.
    /// This one is about everything else: the Preferences panel, the status bar,
    /// the dialogs. A language that covers the menus and not the panel says
    /// "Fichier" at the top of the window and "Preferences" in the middle of it.
    #[test]
    fn every_bundled_language_translates_every_window_label() {
        let labels = chrome_labels();
        assert!(labels.len() > 80, "{} labels", labels.len());
        for (file, source) in BUNDLED {
            let catalog = Catalog::parse(None, source).expect(file);
            let missing: Vec<&str> = labels
                .iter()
                .copied()
                .filter(|label| !is_literal(label))
                .filter(|label| !catalog.translates(label))
                .collect();
            assert!(
                missing.is_empty(),
                "{file} leaves {} in English: {missing:?}",
                missing.len()
            );
        }
    }
}
