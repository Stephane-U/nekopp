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

//! Neko++ — a GPU-rendered Notepad++ workalike.
//!
//! The binary is a thin shell around these modules: `gpu` owns the surface,
//! `renderer` owns the pipelines, `view` owns layout and the shaping cache, and
//! `quad` draws every non-glyph rectangle in one instanced call.

pub mod a11y;
pub mod api;
pub mod api_config;
pub mod about;
pub mod browser;
pub mod changelog;
pub mod charset;
pub mod chrome;
pub mod cli;
pub mod complete;
pub mod context;
pub mod desktop;
pub mod findbar;
pub mod finddialog;
pub mod folder;
pub mod fonts;
pub mod functionlist;
pub mod gpu;
pub mod i18n;
pub mod icon;
pub mod iconrender;
pub mod icons;
pub mod layout;
pub mod links;
pub mod menu;
pub mod minimap;
pub mod nppimport;
pub mod palette;
pub mod playground;
pub mod pluginlist;
pub mod portal;
pub mod peeker;
pub mod prefs;
pub mod print;
pub mod prompt;
pub mod quad;
pub mod renderer;
pub mod rest;
pub mod results;
// Moved to `nn-shared::session` in the hundred-and-thirty-eighth pass: the
// terminal shell reads and writes the same `session.txt`, so the format and the
// directory it lives in have one home. The re-export keeps every caller and
// every test path in this crate spelled the way it was.
pub use nn_shared::session;
pub mod settings;
pub mod shortcuts;
#[cfg(target_os = "linux")]
pub mod single;
pub mod snapshot;
#[cfg(target_os = "linux")]
pub mod startup;
pub mod statusbar;
pub mod styles;
pub mod tabbar;
pub mod text;
pub mod theme;
pub mod themes;
pub mod toolbar;
pub mod tooltip;
pub mod vector;
pub mod view;

pub use about::AboutBox;
pub use api::Call as ApiCall;
pub use browser::Browser;
pub use changelog::{ChangeHistory, Row as ChangeRow};
pub use charset::CharPanel;
pub use chrome::{Chrome, Mode};
pub use cli::Args;
pub use complete::{CompletionMode, CompletionPopup, Item, ParamHint};
pub use context::{ContextMenu, Entry};
pub use findbar::{Button, Chip, Field, FindBar, Hit};
pub use finddialog::{DialogHit, FindDialog, Tab as FindTab};
pub use folder::FolderPanel;
pub use functionlist::FunctionList;
pub use gpu::Gpu;
pub use i18n::{Catalog, Language};
pub use icon::window_icon;
pub use keymap::{Chord, KeyName, Keymap};
pub use layout::{Split, DIVIDER, MIN_PANE};
pub use menu::{Action, BrowserKind, MatchMode, MenuBar, PluginMenu, Sort, WindowSort};
pub use minimap::Minimap;
pub use nppimport::{Export, Import};
pub use palette::Palette;
// The keymap moved to `nn-shared` in the hundred-and-eleventh pass: a chord is
// a decision, and the terminal front-end has to show the same ones in the same
// menu rows. Re-exported so `crate::keymap::Chord` keeps saying what it says.
pub use nn_shared::keymap;
// Both of these live in `nn-core`: the MCP server reads the same configuration
// directory and runs the same plugins, and a second copy of either rule would be
// a second way for the two binaries to disagree about where a plugin is.
pub use nn_core::paths;
pub use nn_core::plugin;
pub use nn_core::plugin::{Bridge, DocumentHost, Inventory, Outcome, Plugins};
pub use peeker::DocumentPeeker;
pub use playground::Playground;
pub use pluginlist::{PluginList, Row as PluginRow};
pub use prefs::Preferences;
pub use print::{Control as PrintControl, PrintPreview};
pub use prompt::{PromptBar, PromptKind};
pub use quad::QuadRenderer;
pub use renderer::{PaneRef, Renderer};
pub use results::ResultsPanel;
pub use session::{Session, SessionTab};
pub use settings::{FileDialog, Settings, ThemeChoice};
pub use shortcuts::Shortcuts;
#[cfg(target_os = "linux")]
pub use single::{Client, Request, Server, Waiter};
pub use snapshot::{autosave_wait, soonest, Snapshot, SnapshotEntry};
pub use statusbar::StatusBar;
pub use styles::{Slot, StyleConfigurator};
pub use tabbar::{TabBar, TabColour, TabInfo, TabLayout};
pub use theme::{Rgb, Theme};
pub use themes::Palette as ThemePalette;
pub use tooltip::Tooltip;
pub use toolbar::Toolbar;
pub use view::EditorView;
