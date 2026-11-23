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

//! Reading and writing a Notepad++ installation's configuration.
//!
//! Two files matter. `config.xml` holds the theme and the default font;
//! `shortcuts.xml` holds the shortcuts the user changed from the defaults. Both
//! are XML, and this crate has no XML parser — but the subset needed is a
//! handful of elements with quoted attributes, so the reader below is a scanner
//! rather than a parser. It never fails: a file it cannot make sense of imports
//! nothing, and a file that is not there is not an error.
//!
//! The import is deliberately partial. Notepad++ has commands this editor does
//! not, and settings with no counterpart here; those are counted and reported
//! rather than guessed at.
//!
//! The export goes the other way and is partial for the same reason: a command
//! Notepad++ has no number for cannot be named in its file, so it is counted
//! and left out. What is written is a file Notepad++ reads back as the same
//! bindings, which is the only property worth having.

use std::path::{Path, PathBuf};

use nn_core::xml::{attribute, elements, Element};
use nn_core::UdlSet;

use crate::keymap::{Chord, KeyName, Keymap};
use crate::menu::Action;
use crate::settings::{
    Settings, ThemeChoice, MAX_FONT_SIZE, MAX_TAB_WIDTH, MIN_FONT_SIZE, MIN_TAB_WIDTH,
};

/// What a Notepad++ installation had to say.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Import {
    /// `Some(true)` when Notepad++ was in light mode.
    pub light: Option<bool>,
    /// The default font size, already converted from points to pixels.
    pub font_size: Option<f32>,
    /// The global tab width.
    pub tab_width: Option<u16>,
    /// The shortcuts the user had changed, in file order.
    pub keymap: Vec<(Action, Chord)>,
    /// Shortcut entries naming a command this build does not have.
    pub skipped: usize,
    /// The user-defined languages the installation had.
    ///
    /// Not a setting, so `apply` leaves it alone: the caller decides where the
    /// languages are stored, and this module only reads them.
    pub langs: UdlSet,
}

impl Import {
    /// Reads the configuration at `path`.
    ///
    /// `path` may be the Notepad++ directory or either of the two files, so a
    /// user who pastes the path they had open in a file manager still lands on
    /// the right thing.
    pub fn read(path: &Path) -> Import {
        let dir = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let mut import = Import::default();
        if let Ok(text) = std::fs::read_to_string(dir.join("config.xml")) {
            import.read_config(&text);
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("shortcuts.xml")) {
            import.read_shortcuts(&text);
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("userDefineLang.xml")) {
            import.langs = UdlSet::parse(&text);
        }
        import
    }

    /// Reads the theme, font and tab width out of `config.xml`.
    fn read_config(&mut self, text: &str) {
        for element in elements(text, "GUIConfig") {
            match attribute(&element, "name") {
                Some("DarkMode") => {
                    if let Some(enable) = attribute(&element, "enable") {
                        self.light = Some(!enable.eq_ignore_ascii_case("yes"));
                    }
                }
                Some("TabSettings") => {
                    if let Some(width) = attribute(&element, "tabSize")
                        .and_then(|value| value.parse::<u16>().ok())
                    {
                        self.tab_width = Some(width.clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH));
                    }
                }
                _ => {}
            }
        }
        for element in elements(text, "WidgetStyle") {
            if attribute(&element, "name") != Some("Default Style") {
                continue;
            }
            if let Some(points) = attribute(&element, "fontSize")
                .and_then(|value| value.parse::<f32>().ok())
            {
                self.font_size = Some(points_to_pixels(points));
            }
        }
    }

    /// Reads the changed shortcuts out of `shortcuts.xml`.
    fn read_shortcuts(&mut self, text: &str) {
        for element in elements(text, "ScintKey") {
            let Some(chord) = chord_of(&element) else {
                continue;
            };
            let action = attribute(&element, "menuCmdID")
                .and_then(|id| id.parse::<u32>().ok())
                .and_then(command_of);
            match action {
                Some(action) => self.keymap.push((action, chord)),
                None => self.skipped += 1,
            }
        }
    }

    /// Writes the import into `settings`, returning how many settings changed.
    ///
    /// Only what the file actually carried is touched, so importing a
    /// `shortcuts.xml` on its own leaves the theme alone.
    pub fn apply(&self, settings: &mut Settings) -> usize {
        let mut changed = 0;
        if let Some(light) = self.light {
            // An imported configuration names a palette, not a policy: it says
            // "dark mode was on", which is a choice, not "follow the desktop".
            let choice = if light {
                ThemeChoice::Light
            } else {
                ThemeChoice::Dark
            };
            if settings.theme != choice {
                settings.theme = choice;
                changed += 1;
            }
        }
        if let Some(size) = self.font_size {
            if settings.font_size != size {
                settings.font_size = size;
                changed += 1;
            }
        }
        if let Some(width) = self.tab_width {
            if settings.tab_width != width {
                settings.tab_width = width;
                changed += 1;
            }
        }
        for (action, chord) in &self.keymap {
            if settings.keymap.lookup(chord) != Some(*action) {
                settings.keymap.set(*action, *chord);
                changed += 1;
            }
        }
        changed
    }

    /// A one-line description of what was found, for the status bar.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.light.is_some() {
            parts.push("theme".into());
        }
        if self.font_size.is_some() {
            parts.push("font".into());
        }
        if self.tab_width.is_some() {
            parts.push("tab width".into());
        }
        if !self.keymap.is_empty() {
            let count = self.keymap.len();
            parts.push(format!(
                "{count} shortcut{}",
                if count == 1 { "" } else { "s" }
            ));
        }
        if !self.langs.is_empty() {
            let count = self.langs.len();
            parts.push(format!(
                "{count} language{}",
                if count == 1 { "" } else { "s" }
            ));
        }
        if parts.is_empty() {
            return "Notepad++ config: nothing to import".into();
        }
        let mut out = format!("Imported {}", parts.join(", "));
        if self.skipped > 0 {
            out.push_str(&format!(" ({} skipped)", self.skipped));
        }
        out
    }
}

/// What an export produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// The `shortcuts.xml` text.
    pub text: String,
    /// How many bindings were written.
    pub written: usize,
    /// How many were left out because Notepad++ has no number for them.
    pub skipped: usize,
}

impl Export {
    /// A one-line description of what was written, for the status bar.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "Exported {} shortcut{}",
            self.written,
            if self.written == 1 { "" } else { "s" }
        );
        if self.skipped > 0 {
            out.push_str(&format!(" ({} skipped)", self.skipped));
        }
        out
    }
}

/// Renders `keymap` as a `shortcuts.xml` Notepad++ can read.
///
/// The file is the same shape Notepad++ writes: one `<ScintKey>` per binding
/// inside `<InternalCommands>`, with the three modifier attributes spelled
/// `yes`/`no` and the key as a decimal virtual-key code. `ScintID` is zero
/// throughout because every binding here is a menu command rather than a
/// Scintilla message.
///
/// Two things are left out, and both are counted rather than guessed at. A
/// command Notepad++ has no number for -- anything this editor added, such as
/// the split commands or the character panel -- cannot be named in the file. A
/// key with no virtual-key code, such as `=`, cannot be written as a number.
/// Writing a plausible-looking number for either would export a binding that
/// does something else when it came back.
///
/// The rows are sorted by command number so the file is stable: rebinding a
/// command moves its row rather than appending a second one, and two exports of
/// the same keymap are byte-identical.
pub fn render_shortcuts(keymap: &Keymap) -> Export {
    let mut rows: Vec<(u32, Chord)> = Vec::new();
    let mut skipped = 0;
    for (chord, action) in keymap.bindings() {
        match (command_id(*action), chord.key.to_vk()) {
            (Some(id), Some(_)) => rows.push((id, *chord)),
            _ => skipped += 1,
        }
    }
    rows.sort_by_key(|(id, _)| *id);

    let mut text = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
    text.push_str("<NotepadPlus>\n");
    text.push_str("    <InternalCommands>\n");
    for (id, chord) in &rows {
        // `to_vk` was checked above, so the unwrap cannot fire.
        let key = chord.key.to_vk().unwrap_or(0);
        text.push_str(&format!(
            "        <ScintKey ScintID=\"0\" menuCmdID=\"{id}\" Ctrl=\"{}\" Alt=\"{}\" Shift=\"{}\" Key=\"{key}\" />\n",
            yes_no(chord.ctrl),
            yes_no(chord.alt),
            yes_no(chord.shift),
        ));
    }
    text.push_str("    </InternalCommands>\n");
    // Notepad++ reads these three sections unconditionally. Writing them empty
    // keeps the file structurally what it expects, so an export does not look
    // like a truncated file to it.
    text.push_str("    <Macros>\n    </Macros>\n");
    text.push_str("    <UserDefinedCommands>\n    </UserDefinedCommands>\n");
    text.push_str("    <PluginCommands>\n    </PluginCommands>\n");
    text.push_str("</NotepadPlus>\n");

    Export {
        text,
        written: rows.len(),
        skipped,
    }
}

/// Writes `keymap` to `path` as a `shortcuts.xml`, creating the directory.
pub fn write_shortcuts(keymap: &Keymap, path: &Path) -> std::io::Result<Export> {
    let export = render_shortcuts(keymap);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, &export.text)?;
    Ok(export)
}

/// `yes` or `no`, the way Notepad++ spells a boolean attribute.
fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Where Notepad++ keeps its configuration on this platform, when it can be
/// guessed.
///
/// Only Windows has a standard location. Elsewhere the user types the path,
/// which is the honest answer: a Linux user's Notepad++ lives under a Wine
/// prefix, and guessing which one would be worse than asking.
pub fn default_dir() -> String {
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata)
                .join("Notepad++")
                .display()
                .to_string();
        }
    }
    String::new()
}

/// Converts a point size to the pixel size the editor works in.
///
/// Notepad++ writes the font size in points, which is what its own dialog
/// shows. A point is 1/72 inch and this editor's size is in pixels at the
/// conventional 96 DPI, so the factor is 4/3.
fn points_to_pixels(points: f32) -> f32 {
    (points * 4.0 / 3.0).round().clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// The chord a `ScintKey` element describes.
fn chord_of(element: &Element) -> Option<Chord> {
    let code = attribute(element, "Key")?.parse::<u32>().ok()?;
    Some(Chord {
        ctrl: yes(element, "Ctrl"),
        shift: yes(element, "Shift"),
        alt: yes(element, "Alt"),
        key: KeyName::from_vk(code)?,
    })
}

/// Whether a `yes`/`no` attribute is set.
fn yes(element: &Element, name: &str) -> bool {
    attribute(element, name).is_some_and(|value| value.eq_ignore_ascii_case("yes"))
}

/// The Notepad++ command numbers this editor can honour, paired with the
/// command each names.
///
/// The numbers mirror `menuCmdID.h` in the Notepad++ source. Only the ranges
/// this editor can honour are listed -- File, Edit, Search, Macro and Settings.
/// The View, Format and Language ranges are left out rather than guessed at: a
/// wrong number would silently bind a chord to the wrong command, which is
/// worse than not importing it.
///
/// One table rather than two `match` arms, because the import and the export
/// have to agree. Two arms would drift the first time a command was added to
/// one of them, and the symptom -- a shortcut that survives an export but not
/// the import back -- is invisible until someone tries it.
const COMMANDS: &[(u32, Action)] = &[
    // File
    (41001, Action::New),
    (41002, Action::Open),
    (41003, Action::CloseTab),
    (41005, Action::Save),
    (41007, Action::SaveAs),
    (41012, Action::Exit),
    // Edit
    (42001, Action::Undo),
    (42002, Action::Redo),
    (42003, Action::Cut),
    (42004, Action::Copy),
    (42005, Action::Paste),
    (42007, Action::SelectAll),
    // Search
    (43001, Action::Find),
    (43002, Action::FindNext),
    (43003, Action::FindPrev),
    (43004, Action::Replace),
    (43005, Action::GotoLine),
    (43006, Action::FindInFiles),
    // Macro
    (47001, Action::ToggleRecording),
    (47003, Action::PlayMacro),
    // Settings
    (49001, Action::Preferences),
];

/// The command a Notepad++ `menuCmdID` names, for the commands this editor has.
fn command_of(id: u32) -> Option<Action> {
    COMMANDS
        .iter()
        .find(|(number, _)| *number == id)
        .map(|(_, action)| *action)
}

/// The `menuCmdID` that names `action`, for the commands Notepad++ has.
fn command_id(action: Action) -> Option<u32> {
    COMMANDS
        .iter()
        .find(|(_, named)| *named == action)
        .map(|(number, _)| *number)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <GUIConfigs>
        <GUIConfig name="TabBar" dragAndDrop="yes" />
        <GUIConfig name="DarkMode" enable="yes" />
        <GUIConfig name="TabSettings" tabSize="8" replaceBySpace="no" />
    </GUIConfigs>
    <GlobalStyles>
        <WidgetStyle name="Default Style" fontName="Courier New" fontSize="12" />
        <WidgetStyle name="Current line background colour" fontSize="99" />
    </GlobalStyles>
</NotepadPlus>
"#;

    const SHORTCUTS: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <InternalCommands>
        <ScintillaKeys>
            <ScintKey ScintID="0" menuCmdID="41005" Ctrl="yes" Alt="no" Shift="no" Key="83" />
            <ScintKey ScintID="0" menuCmdID="43001" Ctrl="yes" Alt="no" Shift="no" Key="70" />
            <ScintKey ScintID="0" menuCmdID="99999" Ctrl="yes" Alt="no" Shift="no" Key="70" />
            <ScintKey ScintID="0" menuCmdID="41002" Ctrl="yes" Alt="yes" Shift="no" Key="79" />
        </ScintillaKeys>
    </InternalCommands>
</NotepadPlus>
"#;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("chord parses")
    }

    #[test]
    fn the_theme_comes_from_the_dark_mode_flag() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        assert_eq!(import.light, Some(false));
    }

    #[test]
    fn dark_mode_off_is_the_light_theme() {
        let mut import = Import::default();
        import.read_config(r#"<GUIConfig name="DarkMode" enable="no" />"#);
        assert_eq!(import.light, Some(true));
    }

    #[test]
    fn the_font_size_is_converted_from_points() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        assert_eq!(import.font_size, Some(16.0));
    }

    #[test]
    fn only_the_default_style_sets_the_font() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        // The second WidgetStyle says 99; it is not the default style.
        assert_eq!(import.font_size, Some(16.0));
    }

    #[test]
    fn a_font_size_outside_the_range_is_clamped() {
        let mut import = Import::default();
        import.read_config(r#"<WidgetStyle name="Default Style" fontSize="400" />"#);
        assert_eq!(import.font_size, Some(MAX_FONT_SIZE));
    }

    #[test]
    fn the_tab_width_comes_from_the_tab_settings() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        assert_eq!(import.tab_width, Some(8));
    }

    #[test]
    fn a_tab_width_outside_the_range_is_clamped() {
        let mut import = Import::default();
        import.read_config(r#"<GUIConfig name="TabSettings" tabSize="99" />"#);
        assert_eq!(import.tab_width, Some(MAX_TAB_WIDTH));
    }

    #[test]
    fn a_shortcut_becomes_a_chord() {
        let mut import = Import::default();
        import.read_shortcuts(SHORTCUTS);
        assert_eq!(import.keymap[0], (Action::Save, chord("Ctrl+S")));
        assert_eq!(import.keymap[1], (Action::Find, chord("Ctrl+F")));
    }

    #[test]
    fn alt_is_read_as_well_as_ctrl() {
        let mut import = Import::default();
        import.read_shortcuts(SHORTCUTS);
        assert_eq!(import.keymap[2], (Action::Open, chord("Ctrl+Alt+O")));
    }

    #[test]
    fn a_command_this_build_does_not_have_is_counted_not_bound() {
        let mut import = Import::default();
        import.read_shortcuts(SHORTCUTS);
        assert_eq!(import.skipped, 1);
        assert_eq!(import.keymap.len(), 3);
    }

    #[test]
    fn a_missing_file_imports_nothing() {
        let import = Import::read(Path::new("/nonexistent/notepad++"));
        assert_eq!(import, Import::default());
    }

    #[test]
    fn a_directory_holding_both_files_is_read_whole() {
        let dir = std::env::temp_dir().join("npp-import-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("config.xml"), CONFIG).expect("config");
        std::fs::write(dir.join("shortcuts.xml"), SHORTCUTS).expect("shortcuts");
        let import = Import::read(&dir);
        assert_eq!(import.light, Some(false));
        assert_eq!(import.keymap.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn either_file_can_be_named_directly() {
        let dir = std::env::temp_dir().join("npp-import-file-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("config.xml"), CONFIG).expect("config");
        let import = Import::read(&dir.join("config.xml"));
        assert_eq!(import.light, Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applying_writes_every_setting_the_file_carried() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        import.read_shortcuts(SHORTCUTS);
        let mut settings = Settings::default();
        let changed = import.apply(&mut settings);
        // Four, not five: two of the three shortcuts already held the chord the
        // file names. The theme counts because the default follows the desktop
        // and the file names a palette.
        assert_eq!(changed, 4);
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.font_size, 16.0);
        assert_eq!(settings.tab_width, 8);
        assert_eq!(settings.keymap.lookup(&chord("Ctrl+S")), Some(Action::Save));
        assert_eq!(
            settings.keymap.lookup(&chord("Ctrl+Alt+O")),
            Some(Action::Open)
        );
    }

    #[test]
    fn applying_twice_changes_nothing_the_second_time() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        import.read_shortcuts(SHORTCUTS);
        let mut settings = Settings::default();
        import.apply(&mut settings);
        assert_eq!(import.apply(&mut settings), 0);
    }

    #[test]
    fn a_shortcut_file_alone_leaves_the_theme_alone() {
        let mut import = Import::default();
        import.read_shortcuts(SHORTCUTS);
        let mut settings = Settings {
            theme: ThemeChoice::Light,
            ..Settings::default()
        };
        import.apply(&mut settings);
        assert_eq!(settings.theme, ThemeChoice::Light);
    }

    #[test]
    fn the_summary_names_what_was_found() {
        let mut import = Import::default();
        import.read_config(CONFIG);
        import.read_shortcuts(SHORTCUTS);
        assert_eq!(
            import.summary(),
            "Imported theme, font, tab width, 3 shortcuts (1 skipped)"
        );
    }

    #[test]
    fn an_empty_import_says_so() {
        assert_eq!(
            Import::default().summary(),
            "Notepad++ config: nothing to import"
        );
    }

    #[test]
    fn one_shortcut_is_not_plural() {
        let mut import = Import::default();
        import.read_shortcuts(
            r#"<ScintKey menuCmdID="41005" Ctrl="yes" Alt="no" Shift="no" Key="83" />"#,
        );
        assert_eq!(import.summary(), "Imported 1 shortcut");
    }

    const LANGS: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <UserLang name="Logfile" ext="log" udlVersion="2.1">
        <KeywordLists>
            <Keywords name="Comments">00# 01 02 03 04 05</Keywords>
            <Keywords name="Keywords1">ERROR WARN</Keywords>
        </KeywordLists>
    </UserLang>
</NotepadPlus>
"#;

    #[test]
    fn a_directory_holding_a_language_file_imports_it() {
        let dir = std::env::temp_dir().join("npp-import-langs-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("userDefineLang.xml"), LANGS).expect("langs");
        let import = Import::read(&dir);
        assert_eq!(import.langs.len(), 1);
        assert_eq!(import.langs.names(), vec!["Logfile"]);
        assert_eq!(import.langs.by_extension(Path::new("a.log")), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_summary_counts_the_languages() {
        let import = Import {
            langs: UdlSet::parse(LANGS),
            ..Import::default()
        };
        assert_eq!(import.summary(), "Imported 1 language");
    }

    #[test]
    fn a_language_file_alone_leaves_the_settings_alone() {
        let import = Import {
            langs: UdlSet::parse(LANGS),
            ..Import::default()
        };
        let mut settings = Settings::default();
        let before = settings.clone();
        assert_eq!(import.apply(&mut settings), 0);
        assert_eq!(settings, before);
    }

    #[test]
    fn a_key_outside_the_known_codes_is_skipped() {
        let mut import = Import::default();
        import.read_shortcuts(
            r#"<ScintKey menuCmdID="41005" Ctrl="yes" Alt="no" Shift="no" Key="255" />"#,
        );
        assert!(import.keymap.is_empty());
        assert_eq!(import.skipped, 0);
    }

    #[test]
    fn every_command_in_the_table_is_bindable() {
        for id in 41000..50000 {
            if let Some(action) = command_of(id) {
                assert!(
                    Action::BINDABLE.contains(&action),
                    "{id} maps to {} which the shortcut editor does not offer",
                    action.name()
                );
            }
        }
    }

    #[test]
    fn no_two_command_ids_name_the_same_command() {
        let mut seen: Vec<Action> = Vec::new();
        for id in 41000..50000 {
            if let Some(action) = command_of(id) {
                assert!(!seen.contains(&action), "{} twice", action.name());
                seen.push(action);
            }
        }
    }

    #[test]
    fn every_command_number_reads_back_as_the_command_it_names() {
        for (id, action) in COMMANDS {
            assert_eq!(command_of(*id), Some(*action));
            assert_eq!(command_id(*action), Some(*id));
        }
    }

    #[test]
    fn an_export_names_the_commands_notepad_plus_knows() {
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, chord("Ctrl+Shift+S"));
        let export = render_shortcuts(&keymap);
        assert!(export.text.contains(
            r#"<ScintKey ScintID="0" menuCmdID="41005" Ctrl="yes" Alt="no" Shift="yes" Key="83" />"#
        ));
        assert!(export.text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n"));
        assert!(export.text.contains("<InternalCommands>"));
        assert!(export.text.ends_with("</NotepadPlus>\n"));
    }

    #[test]
    fn an_export_counts_the_commands_notepad_plus_cannot_name() {
        // The split commands and the character panel are this editor's own;
        // Notepad++ has no number for them, so they cannot be written.
        let keymap = Keymap::default();
        let export = render_shortcuts(&keymap);
        assert_eq!(export.written + export.skipped, keymap.bindings().len());
        assert!(export.skipped > 0);
        assert!(!export.text.contains("SplitVertical"));
    }

    #[test]
    fn an_export_round_trips_through_the_import() {
        // The property that matters: what is written reads back as the same
        // bindings. Anything else is a file that looks right and is not.
        let mut keymap = Keymap::default();
        keymap.set(Action::Save, chord("Ctrl+Shift+S"));
        keymap.set(Action::Find, chord("Ctrl+Alt+F"));
        let export = render_shortcuts(&keymap);

        let mut import = Import::default();
        import.read_shortcuts(&export.text);
        assert_eq!(import.keymap.len(), export.written);
        for (action, chord) in &import.keymap {
            assert_eq!(keymap.lookup(chord), Some(*action), "for {}", action.name());
        }
    }

    #[test]
    fn an_export_is_sorted_by_command_number() {
        let keymap = Keymap::default();
        let export = render_shortcuts(&keymap);
        let ids: Vec<u32> = export
            .text
            .lines()
            .filter_map(|line| line.split("menuCmdID=\"").nth(1))
            .filter_map(|rest| rest.split('"').next())
            .filter_map(|id| id.parse().ok())
            .collect();
        assert_eq!(ids.len(), export.written);
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
    }

    #[test]
    fn two_exports_of_the_same_keymap_are_identical() {
        let keymap = Keymap::default();
        assert_eq!(render_shortcuts(&keymap), render_shortcuts(&keymap));
    }

    #[test]
    fn an_export_summary_says_what_was_written() {
        let export = Export {
            text: String::new(),
            written: 1,
            skipped: 0,
        };
        assert_eq!(export.summary(), "Exported 1 shortcut");
        let export = Export {
            text: String::new(),
            written: 4,
            skipped: 2,
        };
        assert_eq!(export.summary(), "Exported 4 shortcuts (2 skipped)");
    }

    #[test]
    fn an_export_writes_the_file_and_creates_its_directory() {
        let dir = std::env::temp_dir().join(format!(
            "npp-export-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = dir.join("nested").join("shortcuts.xml");
        let export = write_shortcuts(&Keymap::default(), &path).expect("writes");
        let text = std::fs::read_to_string(&path).expect("reads back");
        assert_eq!(text, export.text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
