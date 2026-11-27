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

//! Core data model for Neko++: documents, editing, scrolling, encodings.
//!
//! This crate has no GPU, windowing, or I/O-loop dependencies. It is the part of
//! the editor that must be correct, so it is the part that is unit tested.

pub mod backup;
pub mod complete;
pub mod datetime;
pub mod document;
pub mod editor;
pub mod encoding;
pub mod eol;
pub mod findinfiles;
pub mod codepage;
pub mod fold;
pub mod hash;
pub mod highlighter;
pub mod lang;
pub mod lexer;
pub mod ops;
pub mod lines;
pub mod paths;
pub mod playground;
pub mod pdf;
pub mod plugin;
pub mod print;
pub mod project;
pub mod recorder;
pub mod run;
pub mod search;
pub mod selection;
pub mod symbols;
pub mod udl;
pub mod undo;
pub mod viewport;
pub mod wasm;
pub mod workspace;
pub mod xml;

pub use backup::{backup_before_save, prune_backups, BackupMode};
pub use complete::{
    arg_range, call_at, collect as collect_words, collect_many as collect_words_many,
    functions as scan_functions, matching as matching_words, params_of, path_prefix, prefix_range,
    Call, Function, MAX_CANDIDATES,
};
pub use datetime::{clock_stamp, file_stamp, local_stamp, now_secs, utc_stamp};
pub use document::{
    fold_number, is_fold_header, is_fold_white, DiskChange, Document, FileStamp, FoldLevel, Marker,
    Style, BOOKMARK, FOLD_LEVEL_BASE, FOLD_LEVEL_HEADERFLAG, FOLD_LEVEL_NUMBERMASK,
    FOLD_LEVEL_WHITEFLAG, LARGE_FILE_CHARS, STYLE_DEFAULT,
};
pub use editor::{Editor, Motion, Typed};
pub use codepage::CodePage;
pub use encoding::Encoding;
pub use eol::EolMode;
pub use findinfiles::{split_query, FindInFiles, Hit};
pub use highlighter::Highlighter;
pub use lang::{
    by_key as lang_by_key, by_name as lang_by_name, for_extension as lang_for_extension, LangSpec,
    LANGS,
};
pub use lexer::{style, CLikeLexer, ConfigLexer, LexState, Lexer, PlainLexer};
pub use lines::{Case, SortKey, SortOptions, SortOrder};
pub use ops::{Command as OpCommand, Error as OpError, Options as OpOptions, Param as OpParam};
pub use plugin::{Bridge, DocumentHost, Inventory, Outcome, Plugins};
pub use project::{Project, Projects, MAX_PROJECTS};
pub use recorder::{Macro, Step, MAX_REPEAT};
pub use run::{
    expand as expand_run, missing_program, program_of, RunCommand, RunContext, RunList,
    MACROS as RUN_MACROS,
};
pub use search::{Search, SearchError, SearchMode, SearchOptions};
pub use selection::{map_position, Selection, Selections};
pub use symbols::{scan as scan_symbols, Kind as SymbolKind, Symbol};
pub use udl::{Construct, Rgb8, Udl, UdlLexer, UdlSet};
pub use undo::{Edit, Step as UndoStep, UndoStack};
pub use viewport::Viewport;
pub use wasm::{Host, Instance, Module, Plugin, Value};
pub use workspace::{Entry, Tree, MAX_DEPTH, MAX_ENTRIES};
