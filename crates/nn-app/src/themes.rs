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

//! The theme catalogue, which now lives in `nn-shared`.
//!
//! Forty-seven palettes of ten colours each is data, not drawing, and a terminal
//! drawing Tokyo Night needs the same sixteens the window does. The module is
//! kept as a name so that every `crate::themes::` in this crate goes on working:
//! the move changed where the table is written, not who reads it.

pub use nn_shared::catalogue::*;
