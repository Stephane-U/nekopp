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

//! What both front-ends decide, as opposed to how either one draws it.
//!
//! Neko++ has two front-ends: `nn-app`, which renders with a GPU into a window,
//! and `nn-tui`, which renders into a grid of characters. They are not two
//! editors. They are one editor drawn twice, and the rule that keeps it that
//! way is the one written in `docs/tui-plan.md`: any feature added to the
//! windowed version is converted to the terminal version where it is possible.
//!
//! A rule like that survives on structure, not on discipline. A second list of
//! actions, a second `match` that runs them, a second answer to what
//! "enabled" means -- each of those would drift at the first new command, and
//! the drift would be invisible until someone compared the two programs. So the
//! decisions live here, in one place, and each front-end keeps only the part it
//! is actually alone in: shaping glyphs and submitting draw calls on one side,
//! writing escape sequences into a cell grid on the other.
//!
//! Nothing in this crate opens a window, reads the terminal or measures a font.
//! Its one dependency is `nn-core`, and only because some commands carry an
//! engine-defined payload -- an `Encoding` to re-read a file's bytes as, a
//! `Hash` to compute -- and a payload spelled twice would be a second thing to
//! keep in step.
//!
//! The extraction is in progress, one pass at a time, with every pass required
//! to leave the windowed program's pictures byte-identical:
//!
//! * [`action`] -- the command vocabulary: every command the menus offer, the
//!   name it is written as in the settings file, and the lists the keymap and
//!   the text-command table agree with.
//! * [`menu`] -- the menus as a tree of rows: one builder per dropdown, each row
//!   carrying its label, its shortcut column and the command it runs.
//! * [`keymap`] -- the chords: the default bindings, the parser for a `key` line
//!   in the settings file, and the lookup both front-ends read before they do
//!   anything else with a keypress.
//! * [`theme`] -- the themes: the fifty choices with their labels and keys, the
//!   forty-nine colours each resolves to, and the sixteen of those a user may
//!   override by hand.
//! * [`prefs`] -- the seventeen rows of the preferences dialog, in order, with
//!   what each is called and whether it switches, steps or opens a dialog.
//! * [`settings`] -- the settings file: the struct, its defaults, the parser and
//!   the writer, plus the values a row can hold -- the tab strip's three
//!   arrangements, the two file dialogs, the window scale.
//! * [`color`] -- the colour type itself: three bytes and their brightness, with
//!   the conversion to linear values left to the front-end with a shader.
//! * [`i18n`] -- the translations: the bundled catalogues, the parser for a
//!   localization file, and the language list a dialog offers.
//!
//! The one dispatcher comes next; it is still in `nn-app` while the shape of what
//! the terminal needs is being learned from the terminal.

pub mod action;
pub mod catalogue;
pub mod color;
pub mod i18n;
pub mod keymap;
pub mod prefs;
pub mod settings;
pub mod menu;
pub mod theme;
