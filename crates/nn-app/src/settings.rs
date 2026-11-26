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

//! The settings, which now live in `nn-shared`.
//!
//! One settings file, one parser, one writer, two front-ends. What stayed behind
//! in this crate is nothing: the settings are values a line of text can carry,
//! and the drawing they influence lives in the widgets that read them.
//!
//! The module is kept as a name so that every `crate::settings::` in this crate
//! goes on working.

pub use nn_shared::settings::{
    FileDialog, Settings, TabLayout, ThemeChoice, UiScale, DEFAULT_PORT, DEFAULT_SEARCH_ENGINE,
    MAX_AUTOSAVE, MAX_BACKUP_KEEP, MAX_FONT_SIZE, MAX_PLAYGROUND_MINUTES, MAX_TAB_WIDTH,
    MAX_UI_SCALE, MIN_AUTOSAVE, MIN_BACKUP_KEEP, MIN_FONT_SIZE, MIN_PLAYGROUND_MINUTES,
    MIN_TAB_WIDTH, MIN_UI_SCALE,
};
