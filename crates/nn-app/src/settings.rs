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

//! The user's preferences, and the file they live in.
//!
//! The format is the same line-oriented one the session uses: a keyword, a
//! space, and the value. A settings file is small and read once at startup, so
//! a real parser would be a dependency bought for nothing — and this crate has
//! deliberately stayed free of them.
//!
//! Unknown keywords are skipped rather than rejected, so a file written by a
//! newer version still loads the settings this one understands.

use crate::keymap::Keymap;
use crate::paths;
use crate::styles::Slot;
use crate::tabbar::TabLayout;
use nn_core::backup::BackupMode;
use nn_core::run::RunList;
use crate::theme::Rgb;

/// The smallest and largest font size the preferences dialog offers.
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 40.0;
/// The smallest and largest window scale `ui_scale` accepts.
///
/// A half and four times the furniture: below the first the labels stop being
/// legible, and above the second the bars take more of the window than the
/// document does.
pub const MIN_UI_SCALE: f32 = 0.5;
pub const MAX_UI_SCALE: f32 = 4.0;
/// The tab widths the preferences dialog offers.
pub const MIN_TAB_WIDTH: u16 = 1;
pub const MAX_TAB_WIDTH: u16 = 16;
/// The autosave intervals the preferences dialog offers, in seconds.
///
/// Zero is off. The upper bound is five minutes: past that the interval is
/// longer than most editing sessions and the feature stops being a safety net.
pub const MIN_AUTOSAVE: u32 = 0;
pub const MAX_AUTOSAVE: u32 = 300;
/// How many verbose backups to keep per file.
pub const MIN_BACKUP_KEEP: usize = 1;
pub const MAX_BACKUP_KEEP: usize = 100;
/// The longest idle time that opens the playground by itself, in minutes.
///
/// An hour. Past that the machine has almost certainly been left alone and a
/// screensaver that starts an hour late is a screensaver that never starts.
pub const MIN_PLAYGROUND_MINUTES: u32 = 0;
pub const MAX_PLAYGROUND_MINUTES: u32 = 60;

/// Which file dialog Open and Save As use.
///
/// The portal is the default because it is the only dialog that can reach a
/// user's bookmarks, their network shares, and -- under Flatpak or snap -- any
/// file at all. The built-in browser stays available because the portal is a
/// separate service that is not always running, and because a dialog drawn by
/// the application is the only one that works over a bare X connection with no
/// session bus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileDialog {
    /// Ask `org.freedesktop.portal.FileChooser`, falling back to the built-in
    /// browser when no portal answers.
    #[default]
    Portal,
    /// Always use the built-in browser.
    Builtin,
}

impl FileDialog {
    /// Every choice, in the order the preferences dialog cycles them.
    pub const ALL: [FileDialog; 2] = [FileDialog::Portal, FileDialog::Builtin];

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self {
            FileDialog::Portal => "portal",
            FileDialog::Builtin => "builtin",
        }
    }

    /// The label shown in the preferences dialog.
    pub fn label(self) -> &'static str {
        match self {
            FileDialog::Portal => "System dialog",
            FileDialog::Builtin => "Built-in browser",
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == text)
    }

    /// The next choice, wrapping. What a switch row does when stepped.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

/// Which palette the editor draws with.
///
/// `System` is not a synonym for one of the others: a user who has told their
/// desktop to go dark at sunset expects the editor to follow, and a user who
/// picked a named theme here expects it to stay put whatever the desktop does.
/// A boolean cannot say both, and the settings file has to record which one was
/// meant.
///
/// The named themes are the catalogue in [`crate::themes`]. They are variants
/// rather than a `Named(&str)` payload so that a typo is a compile error rather
/// than a settings line that silently does nothing -- the same reason the
/// languages are a table with typed keys and not a directory of names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the desktop's colour-scheme preference, falling back to dark when
    /// the desktop has no opinion or no portal to ask.
    #[default]
    System,
    /// The built-in light palette, hand-tuned to Notepad++'s own colours.
    Light,
    /// The built-in dark palette, from the Visual Studio Code family.
    Dark,
    AyuDark,
    AyuLight,
    AyuMirage,
    CatppuccinFrappe,
    CatppuccinLatte,
    CatppuccinMacchiato,
    CatppuccinMocha,
    Darcula,
    Dracula,
    Everforest,
    EverforestLight,
    GitHubDark,
    GitHubDarkDimmed,
    GitHubLight,
    GruvboxDark,
    GruvboxLight,
    Horizon,
    Iceberg,
    Kanagawa,
    KanagawaDragon,
    KanagawaLotus,
    MaterialOcean,
    MaterialPalenight,
    Molokai,
    Monokai,
    MonokaiPro,
    Moonlight,
    NightOwl,
    Nightfox,
    Nord,
    OceanicNext,
    OneDark,
    OneLight,
    PandaSyntax,
    QuietLight,
    RosePine,
    RosePineDawn,
    RosePineMoon,
    ShadesOfPurple,
    SolarizedDark,
    SolarizedLight,
    Synthwave84,
    TokyoNight,
    TokyoNightDay,
    TokyoNightStorm,
    VitesseDark,
    Zenburn,
}

impl ThemeChoice {
    /// Every choice, in the order the preferences dialog cycles them.
    ///
    /// `System`, the two built-ins, then the catalogue in its own order. A
    /// cycling row is only navigable if the order is predictable, which is why
    /// the named themes are alphabetical and the two built-ins are not buried
    /// in the middle of them.
    pub const ALL: [ThemeChoice; 50] = [
        ThemeChoice::System,
        ThemeChoice::Light,
        ThemeChoice::Dark,
        ThemeChoice::AyuDark,
        ThemeChoice::AyuLight,
        ThemeChoice::AyuMirage,
        ThemeChoice::CatppuccinFrappe,
        ThemeChoice::CatppuccinLatte,
        ThemeChoice::CatppuccinMacchiato,
        ThemeChoice::CatppuccinMocha,
        ThemeChoice::Darcula,
        ThemeChoice::Dracula,
        ThemeChoice::Everforest,
        ThemeChoice::EverforestLight,
        ThemeChoice::GitHubDark,
        ThemeChoice::GitHubDarkDimmed,
        ThemeChoice::GitHubLight,
        ThemeChoice::GruvboxDark,
        ThemeChoice::GruvboxLight,
        ThemeChoice::Horizon,
        ThemeChoice::Iceberg,
        ThemeChoice::Kanagawa,
        ThemeChoice::KanagawaDragon,
        ThemeChoice::KanagawaLotus,
        ThemeChoice::MaterialOcean,
        ThemeChoice::MaterialPalenight,
        ThemeChoice::Molokai,
        ThemeChoice::Monokai,
        ThemeChoice::MonokaiPro,
        ThemeChoice::Moonlight,
        ThemeChoice::NightOwl,
        ThemeChoice::Nightfox,
        ThemeChoice::Nord,
        ThemeChoice::OceanicNext,
        ThemeChoice::OneDark,
        ThemeChoice::OneLight,
        ThemeChoice::PandaSyntax,
        ThemeChoice::QuietLight,
        ThemeChoice::RosePine,
        ThemeChoice::RosePineDawn,
        ThemeChoice::RosePineMoon,
        ThemeChoice::ShadesOfPurple,
        ThemeChoice::SolarizedDark,
        ThemeChoice::SolarizedLight,
        ThemeChoice::Synthwave84,
        ThemeChoice::TokyoNight,
        ThemeChoice::TokyoNightDay,
        ThemeChoice::TokyoNightStorm,
        ThemeChoice::VitesseDark,
        ThemeChoice::Zenburn,
    ];

    /// The catalogue entry for a named theme, or `None` for the three built-ins.
    ///
    /// The key is written as a literal beside the variant it belongs to rather
    /// than kept as an index into [`crate::themes::CATALOGUE`]: an index is a
    /// number that stays correct until someone inserts a palette in the middle
    /// of the table, and a key is a word the compiler checks against the
    /// `by_key` signature at least, and a test checks end to end.
    pub fn palette(self) -> Option<&'static crate::themes::Palette> {
        let key = match self {
            ThemeChoice::System | ThemeChoice::Light | ThemeChoice::Dark => return None,
            ThemeChoice::AyuDark => "ayu_dark",
            ThemeChoice::AyuLight => "ayu_light",
            ThemeChoice::AyuMirage => "ayu_mirage",
            ThemeChoice::CatppuccinFrappe => "catppuccin_frappe",
            ThemeChoice::CatppuccinLatte => "catppuccin_latte",
            ThemeChoice::CatppuccinMacchiato => "catppuccin_macchiato",
            ThemeChoice::CatppuccinMocha => "catppuccin_mocha",
            ThemeChoice::Darcula => "darcula",
            ThemeChoice::Dracula => "dracula",
            ThemeChoice::Everforest => "everforest",
            ThemeChoice::EverforestLight => "everforest_light",
            ThemeChoice::GitHubDark => "github_dark",
            ThemeChoice::GitHubDarkDimmed => "github_dark_dimmed",
            ThemeChoice::GitHubLight => "github_light",
            ThemeChoice::GruvboxDark => "gruvbox_dark",
            ThemeChoice::GruvboxLight => "gruvbox_light",
            ThemeChoice::Horizon => "horizon",
            ThemeChoice::Iceberg => "iceberg",
            ThemeChoice::Kanagawa => "kanagawa",
            ThemeChoice::KanagawaDragon => "kanagawa_dragon",
            ThemeChoice::KanagawaLotus => "kanagawa_lotus",
            ThemeChoice::MaterialOcean => "material_ocean",
            ThemeChoice::MaterialPalenight => "material_palenight",
            ThemeChoice::Molokai => "molokai",
            ThemeChoice::Monokai => "monokai",
            ThemeChoice::MonokaiPro => "monokai_pro",
            ThemeChoice::Moonlight => "moonlight",
            ThemeChoice::NightOwl => "night_owl",
            ThemeChoice::Nightfox => "nightfox",
            ThemeChoice::Nord => "nord",
            ThemeChoice::OceanicNext => "oceanic_next",
            ThemeChoice::OneDark => "one_dark",
            ThemeChoice::OneLight => "one_light",
            ThemeChoice::PandaSyntax => "panda_syntax",
            ThemeChoice::QuietLight => "quiet_light",
            ThemeChoice::RosePine => "rose_pine",
            ThemeChoice::RosePineDawn => "rose_pine_dawn",
            ThemeChoice::RosePineMoon => "rose_pine_moon",
            ThemeChoice::ShadesOfPurple => "shades_of_purple",
            ThemeChoice::SolarizedDark => "solarized_dark",
            ThemeChoice::SolarizedLight => "solarized_light",
            ThemeChoice::Synthwave84 => "synthwave_84",
            ThemeChoice::TokyoNight => "tokyo_night",
            ThemeChoice::TokyoNightDay => "tokyo_night_day",
            ThemeChoice::TokyoNightStorm => "tokyo_night_storm",
            ThemeChoice::VitesseDark => "vitesse_dark",
            ThemeChoice::Zenburn => "zenburn",
        };
        crate::themes::by_key(key)
    }

    /// The keyword written to the settings file.
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
            named => named.palette().map_or("dark", |palette| palette.key),
        }
    }

    /// The label shown in the preferences dialog and the Settings menu.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
            named => named.palette().map_or("Dark", |palette| palette.name),
        }
    }

    /// Reads a keyword back, or `None` when it is not one of ours.
    ///
    /// `light` and `dark` are the two keywords the file used before this was a
    /// three-way choice, and they still name the two built-in palettes, so an
    /// existing settings file keeps working and keeps meaning what it meant.
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == text)
    }

    /// The next choice, wrapping.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|choice| *choice == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    /// The previous choice, wrapping.
    pub fn previous(self) -> Self {
        let at = Self::ALL.iter().position(|choice| *choice == self).unwrap_or(0);
        Self::ALL[(at + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// The theme this choice resolves to.
    ///
    /// `system` is what the desktop said, or `None` when it said nothing -- no
    /// portal, no session bus, or a desktop with no preference. The fallback is
    /// dark, which is the palette this editor has always opened with, so a
    /// machine that cannot answer looks exactly as it did before.
    pub fn theme(self, system: Option<bool>) -> crate::theme::Theme {
        match self {
            ThemeChoice::Light => crate::theme::Theme::LIGHT,
            ThemeChoice::Dark => crate::theme::Theme::DARK,
            ThemeChoice::System => {
                if system.unwrap_or(false) {
                    crate::theme::Theme::LIGHT
                } else {
                    crate::theme::Theme::DARK
                }
            }
            named => named.palette().map_or(crate::theme::Theme::DARK, |palette| {
                crate::theme::Theme::from_palette(palette)
            }),
        }
    }

    /// Whether the theme this choice resolves to is a light one.
    ///
    /// Read off the resolved theme rather than the variant, so a named palette
    /// and a `System` answer are described the same way.
    pub fn is_light(self, system: Option<bool>) -> bool {
        self.theme(system).background.luminance() >= 128
    }
}

/// Everything the user can change that survives a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Which palette to draw with.
    pub theme: ThemeChoice,
    /// The editor font size in pixels.
    pub font_size: f32,
    /// How big the window furniture is drawn: the menu bar, the toolbar, the tab
    /// strip, the status bar and the panels.
    ///
    /// Zero, the default, means "follow the display": a 4K monitor whose desktop
    /// reports a scale of 2 gets furniture twice the size. A number is an
    /// override, for the screens where the desktop reports 1 on a dense panel and
    /// the labels come out too small to read. A setting rather than a menu command
    /// because it is a fact about the screen, and it should be there again next
    /// launch.
    pub ui_scale: f32,
    /// How many columns a tab occupies.
    pub tab_width: u16,
    /// Whether long lines wrap.
    pub wrap: bool,
    /// Whether the minimap is shown.
    pub minimap: bool,
    /// Whether the toolbar is shown.
    pub toolbar: bool,
    /// Whether the function list is shown.
    pub function_list: bool,
    /// Whether closing the find bar drops its highlights.
    ///
    /// On by default, which is what Notepad++ does: a search you have finished
    /// with should not leave the document speckled. Turning it off keeps the
    /// highlights after the bar closes, which is what you want when the point of
    /// the search was to see every occurrence at once.
    pub volatile_find: bool,
    /// Whether the search opens as Notepad++'s floating window instead of the
    /// bar docked along the bottom.
    ///
    /// Off by default, because the bar was here first and it reserves no space
    /// the text could have used. On, the same search is drawn as a sheet with
    /// tabs -- the presentation Notepad++ ships.
    pub find_floating: bool,
    /// Whether the playground can be opened, from the keyboard and by idling.
    ///
    /// On by default. It is a shortcut and a screensaver, not a mode the window
    /// enters on its own, so there is nothing to be surprised by.
    pub playground: bool,
    /// How many minutes of no input open the playground by itself.
    ///
    /// Zero -- never -- by default: a screen that turns into a room while
    /// someone is reading it is a screen that has taken the document away.
    pub playground_after: u32,
    /// Which chord runs which command.
    pub keymap: Keymap,
    /// Colours the user changed in the Style Configurator.
    ///
    /// Sparse: only the slots that differ from the theme's own palette are
    /// stored, so switching between light and dark keeps the user's edits on top
    /// of whichever base they picked. Sorted by slot so the file is stable.
    pub styles: Vec<(Slot, Rgb)>,
    /// The commands saved in the Run menu.
    pub runs: RunList,
    /// How the tab strip arranges its tabs.
    pub tab_layout: TabLayout,
    /// What to do with a file's previous contents when it is saved over.
    pub backup_mode: BackupMode,
    /// How many verbose backups to keep per file before the oldest is deleted.
    pub backup_keep: usize,
    /// The paper the print preview starts on, as a `Paper` key.
    pub print_paper: String,
    /// Whether the print preview starts in landscape.
    pub print_landscape: bool,
    /// Whether the print preview starts with a line-number gutter.
    pub print_line_numbers: bool,
    /// The print header template, with tabs written as `\t`.
    pub print_header: String,
    /// The print footer template, with tabs written as `\t`.
    pub print_footer: String,
    /// The localization file to load, as a name inside the `localization`
    /// directory. `None` is English.
    ///
    /// A file name rather than a path: the directory is ours, and a stored
    /// absolute path would break the moment the config moves.
    pub language: Option<String>,
    /// The plugins the user switched on, by the name each module gives itself.
    ///
    /// Empty by default, and that is the point: a file in the plugins folder is
    /// *installed*, not *running*. Dropping a `.wasm` in adds a row to the
    /// manager and nothing else, and the editor does not execute code the user
    /// has not asked for. A name here that matches no file is ignored, and is
    /// dropped the next time the manager writes the list back.
    pub enabled_plugins: Vec<String>,
    /// Which file dialog Open and Save As use.
    pub file_dialog: FileDialog,
    /// How often to snapshot unsaved buffers, in seconds. Zero is off.
    ///
    /// On by default, unlike `backup_mode`. The two differ in who they disturb:
    /// a backup writes into the user's own directory, which is why it is opt-in,
    /// while a snapshot writes only into ours and is the difference between a
    /// crash costing an hour of work and costing nothing.
    pub autosave_seconds: u32,
    /// The `strftime` pattern "Date Time (customized)" inserts.
    ///
    /// Remembered because the whole point of the customized variant is that the
    /// user has a shape they keep wanting; asking again every time would make it
    /// no better than the two fixed ones.
    pub datetime_format: String,
    /// The URL template "Search on Internet" opens, with `$(CURRENT_WORD)` in it.
    ///
    /// A template rather than a fixed engine because Notepad++ lets the user pick
    /// one, and the choice is a preference, not a per-search decision.
    pub search_engine: String,
    /// Whether a zoom change applies to both panes.
    ///
    /// Off by default, which is Notepad++'s default: two panes usually hold two
    /// different files, and making one bigger should not shrink the other.
    pub zoom_sync: bool,
    /// Whether the two panes scroll together vertically.
    pub sync_scroll_v: bool,
    /// Whether the two panes scroll horizontally together.
    pub sync_scroll_h: bool,
    /// The `stylers.xml` the theme was imported from, if one was.
    ///
    /// A path rather than the colours themselves: the file is the user's, and
    /// re-reading it on start means an edit to it takes effect without a second
    /// import. A path that no longer exists falls back to the built-in pair
    /// rather than failing to start.
    pub theme_path: Option<String>,
    /// Whether the REST listener starts with the window.
    ///
    /// Off by default, and the default is the feature: a listener that starts on
    /// its own is a service the user did not ask for and cannot see. `--rest`
    /// turns it on for one launch; this key turns it on for every launch, which
    /// is what a harness or a script on the same machine wants.
    pub rest_enabled: bool,
    /// The port the REST listener takes when `--rest` names none.
    pub rest_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            font_size: 15.0,
            ui_scale: 0.0,
            tab_width: 4,
            wrap: false,
            minimap: true,
            toolbar: true,
            function_list: false,
            volatile_find: true,
            find_floating: false,
            playground: true,
            playground_after: 0,
            keymap: Keymap::default(),
            styles: Vec::new(),
            runs: RunList::default(),
            tab_layout: TabLayout::Horizontal,
            backup_mode: BackupMode::None,
            backup_keep: 10,
            autosave_seconds: 7,
            datetime_format: "%Y-%m-%d %H:%M:%S".to_string(),
            search_engine: DEFAULT_SEARCH_ENGINE.to_string(),
            zoom_sync: false,
            sync_scroll_v: false,
            sync_scroll_h: false,
            theme_path: None,
            rest_enabled: false,
            rest_port: crate::rest::DEFAULT_PORT,
            file_dialog: FileDialog::Portal,
            print_paper: "a4".to_string(),
            print_landscape: false,
            print_line_numbers: false,
            print_header: String::new(),
            print_footer: "$(PAGE)".to_string(),
            language: None,
            enabled_plugins: Vec::new(),
        }
    }
}

impl Settings {
    /// The file the settings are stored in.
    pub fn path() -> Option<std::path::PathBuf> {
        paths::config_file("settings.txt")
    }

    /// Reads the settings, or the defaults when there are none.
    pub fn load() -> Settings {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Writes the settings.
    pub fn save(&self) {
        if let Some(path) = Self::path() {
            paths::write(&path, &self.render());
        }
    }

    /// Parses the on-disk format.
    pub fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((keyword, value)) = line.split_once(' ') else {
                continue;
            };
            let value = value.trim();
            match keyword {
                "theme" => {
                    settings.theme = ThemeChoice::from_key(value).unwrap_or_default()
                }
                "font_size" => {
                    if let Ok(size) = value.parse::<f32>() {
                        settings.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                    }
                }
                "ui_scale" => {
                    if let Ok(scale) = value.parse::<f32>() {
                        // Zero is the word "follow the display" spelled as a
                        // number; anything under it is a typo rather than a wish
                        // to shrink the chrome to nothing.
                        settings.ui_scale = if scale > 0.0 {
                            scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE)
                        } else {
                            0.0
                        };
                    }
                }
                "tab_width" => {
                    if let Ok(width) = value.parse::<u16>() {
                        settings.tab_width = width.clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
                    }
                }
                "wrap" => settings.wrap = value == "on",
                "minimap" => settings.minimap = value == "on",
                "toolbar" => settings.toolbar = value == "on",
                "function_list" => settings.function_list = value == "on",
                "volatile_find" => settings.volatile_find = value == "on",
                "find_floating" => settings.find_floating = value == "on",
                "playground" => settings.playground = value == "on",
                "playground_after" => {
                    if let Ok(minutes) = value.parse::<u32>() {
                        settings.playground_after = minutes.min(MAX_PLAYGROUND_MINUTES);
                    }
                }
                "style" => {
                    // `style <slot> #RRGGBB`.
                    if let Some((slot, color)) = value.split_once(' ') {
                        if let (Some(slot), Some(color)) =
                            (Slot::from_key(slot.trim()), parse_hex(color.trim()))
                        {
                            crate::styles::record(&mut settings.styles, slot, color);
                        }
                    }
                }
                "run" => settings.runs.parse_line(value),
                "tab_layout" => {
                    settings.tab_layout = TabLayout::from_key(value).unwrap_or_default()
                }
                "backup" => {
                    settings.backup_mode = BackupMode::from_key(value).unwrap_or(BackupMode::None);
                }
                "backup_keep" => {
                    if let Ok(keep) = value.parse::<usize>() {
                        settings.backup_keep = keep.clamp(MIN_BACKUP_KEEP, MAX_BACKUP_KEEP);
                    }
                }
                "print_paper" => settings.print_paper = value.to_string(),
                "print_landscape" => settings.print_landscape = value == "on",
                "print_line_numbers" => settings.print_line_numbers = value == "on",
                "print_header" => settings.print_header = unescape_tabs(value),
                "print_footer" => settings.print_footer = unescape_tabs(value),
                "language" => {
                    // `language none` is how the file spells "back to English".
                    settings.language = if value.is_empty() || value == "none" {
                        None
                    } else {
                        Some(value.to_string())
                    };
                }
                "plugin" => {
                    // One line per plugin, because a name is a phrase with
                    // spaces in it and a single line of them would need a
                    // separator no plugin name is allowed to contain. The rule
                    // for what a line means lives in the plugin host: the MCP
                    // server reads the same lines without this parser.
                    if let Some(name) = nn_core::plugin::enabled_line(line) {
                        if !settings.enabled_plugins.iter().any(|n| n == name) {
                            settings.enabled_plugins.push(name.to_string());
                        }
                    }
                }
                "file_dialog" => {
                    settings.file_dialog = FileDialog::from_key(value).unwrap_or_default()
                }
                "rest" => settings.rest_enabled = value == "on",
                "rest_port" => {
                    // Zero is not a port a client can reach, so it is refused
                    // rather than stored: a listener on port 0 picks one the
                    // user would then have to look up in the log.
                    if let Ok(port) = value.parse::<u16>() {
                        if port > 0 {
                            settings.rest_port = port;
                        }
                    }
                }
                "datetime_format" => settings.datetime_format = value.to_string(),
                "search_engine" => settings.search_engine = value.to_string(),
                "zoom_sync" => settings.zoom_sync = value == "on",
                "sync_scroll_v" => settings.sync_scroll_v = value == "on",
                "sync_scroll_h" => settings.sync_scroll_h = value == "on",
                "theme_path" => {
                    settings.theme_path = if value == "none" {
                        None
                    } else {
                        Some(value.to_string())
                    };
                }
                "autosave" => {
                    if let Ok(seconds) = value.parse::<u32>() {
                        settings.autosave_seconds = seconds.clamp(MIN_AUTOSAVE, MAX_AUTOSAVE);
                    }
                }
                "key" => {
                    // `key <action> <chord>`: the value holds both halves.
                    if let Some((action, chord)) = value.split_once(' ') {
                        settings.keymap.parse_line(action.trim(), chord.trim());
                    }
                }
                _ => {}
            }
        }
        settings
    }

    /// Renders the on-disk format.
    pub fn render(&self) -> String {
        let on_off = |value: bool| if value { "on" } else { "off" };
        let mut out = String::from("# Neko++ settings\n");
        out.push_str(&format!("theme {}\n", self.theme.key()));
        out.push_str(&format!("font_size {}\n", self.font_size));
        out.push_str(&format!("ui_scale {}\n", self.ui_scale));
        out.push_str(&format!("tab_width {}\n", self.tab_width));
        out.push_str(&format!("wrap {}\n", on_off(self.wrap)));
        out.push_str(&format!("minimap {}\n", on_off(self.minimap)));
        out.push_str(&format!("function_list {}\n", on_off(self.function_list)));
        out.push_str(&format!("volatile_find {}\n", on_off(self.volatile_find)));
        out.push_str(&format!("find_floating {}\n", on_off(self.find_floating)));
        out.push_str(&format!("playground {}\n", on_off(self.playground)));
        out.push_str(&format!("playground_after {}\n", self.playground_after));
        out.push_str(&format!("toolbar {}\n", on_off(self.toolbar)));
        // Sorted here rather than trusted from the caller: the file is a
        // canonical record, and a stable order makes it diffable.
        let mut styles = self.styles.clone();
        styles.sort_by_key(|(slot, _)| Slot::ALL.iter().position(|x| x == slot).unwrap_or(0));
        for (slot, color) in &styles {
            out.push_str(&format!(
                "style {} #{:02X}{:02X}{:02X}\n",
                slot.key(),
                color.0,
                color.1,
                color.2
            ));
        }
        out.push_str(&format!("tab_layout {}\n", self.tab_layout.key()));
        out.push_str(&format!("backup {}\n", self.backup_mode.key()));
        out.push_str(&format!("backup_keep {}\n", self.backup_keep));
        out.push_str(&format!("autosave {}\n", self.autosave_seconds));
        out.push_str(&format!("file_dialog {}\n", self.file_dialog.key()));
        out.push_str(&format!("print_paper {}\n", self.print_paper));
        out.push_str(&format!(
            "print_landscape {}\n",
            on_off(self.print_landscape)
        ));
        out.push_str(&format!(
            "print_line_numbers {}\n",
            on_off(self.print_line_numbers)
        ));
        out.push_str(&format!(
            "print_header {}\n",
            escape_tabs(&self.print_header)
        ));
        out.push_str(&format!(
            "print_footer {}\n",
            escape_tabs(&self.print_footer)
        ));
        out.push_str(&format!(
            "language {}\n",
            self.language.as_deref().unwrap_or("none")
        ));
        for name in &self.enabled_plugins {
            out.push_str(&format!("plugin {name}\n"));
        }
        out.push_str(&format!("datetime_format {}\n", self.datetime_format));
        out.push_str(&format!("search_engine {}\n", self.search_engine));
        out.push_str(&format!(
            "zoom_sync {}\n",
            if self.zoom_sync { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "sync_scroll_v {}\n",
            if self.sync_scroll_v { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "sync_scroll_h {}\n",
            if self.sync_scroll_h { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "theme_path {}\n",
            self.theme_path.as_deref().unwrap_or("none")
        ));
        out.push_str(&format!("rest {}\n", on_off(self.rest_enabled)));
        out.push_str(&format!("rest_port {}\n", self.rest_port));
        out.push_str(&self.runs.render());
        out.push_str(&self.keymap.render());
        out
    }
}

/// The search engine "Search on Internet" starts with.
///
/// DuckDuckGo rather than Google: it answers a plain GET with no cookie wall and
/// no consent interstitial, so the browser lands on results instead of a dialog.
pub const DEFAULT_SEARCH_ENGINE: &str = "https://duckduckgo.com/?q=$(CURRENT_WORD)";

/// Writes a tab as the two characters `\t`.
///
/// The settings file is line-based, so a literal tab in a header template would
/// be invisible and a trailing one would be trimmed away on the next read. The
/// escape keeps the three zones of a header distinguishable.
fn escape_tabs(text: &str) -> String {
    text.replace('\t', "\\t")
}

/// Reads `\t` back as a tab.
fn unescape_tabs(text: &str) -> String {
    text.replace("\\t", "\t")
}

/// Reads `#RRGGBB`, with or without the hash.
fn parse_hex(text: &str) -> Option<Rgb> {
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_round_trip() {
        let settings = Settings::default();
        assert_eq!(Settings::parse(&settings.render()), settings);
    }

    #[test]
    fn every_setting_round_trips() {
        let settings = Settings {
            theme: ThemeChoice::Light,
            font_size: 22.0,
            tab_width: 8,
            wrap: true,
            minimap: false,
            toolbar: false,
            function_list: true,
            volatile_find: false,
            find_floating: false,
            playground: false,
            playground_after: 7,
            backup_mode: BackupMode::Verbose,
            backup_keep: 4,
            autosave_seconds: 30,
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&settings.render()), settings);
    }

    #[test]
    fn the_rest_listener_is_off_until_it_is_asked_for() {
        assert!(!Settings::default().rest_enabled);
        assert_eq!(Settings::default().rest_port, crate::rest::DEFAULT_PORT);
        let settings = Settings {
            rest_enabled: true,
            rest_port: 9000,
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("rest on\n"));
        assert!(text.contains("rest_port 9000\n"));
        let parsed = Settings::parse(&text);
        assert!(parsed.rest_enabled);
        assert_eq!(parsed.rest_port, 9000);
    }

    #[test]
    fn a_rest_port_of_zero_is_not_stored() {
        // 0 means "any port" to the kernel, which is not something the user can
        // type into a client, so it is refused rather than remembered.
        let parsed = Settings::parse("rest_port 0\n");
        assert_eq!(parsed.rest_port, crate::rest::DEFAULT_PORT);
        let parsed = Settings::parse("rest_port not-a-port\n");
        assert_eq!(parsed.rest_port, crate::rest::DEFAULT_PORT);
    }

    #[test]
    fn the_switched_on_plugins_round_trip() {
        let settings = Settings {
            enabled_plugins: vec!["JSON".to_string(), "Half Width".to_string()],
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("plugin JSON\n"));
        assert!(text.contains("plugin Half Width\n"));
        assert_eq!(Settings::parse(&text).enabled_plugins, settings.enabled_plugins);
    }

    #[test]
    fn no_plugin_is_switched_on_by_default() {
        // Installing a plugin is dropping a file in; running it is a decision
        // the user makes in the manager.
        assert!(Settings::default().enabled_plugins.is_empty());
    }

    #[test]
    fn a_repeated_plugin_line_is_kept_once() {
        let settings = Settings::parse("plugin JSON\nplugin JSON\nplugin  XML  \n");
        assert_eq!(
            settings.enabled_plugins,
            vec!["JSON".to_string(), "XML".to_string()]
        );
        let blank = Settings::parse("plugin\nplugin   \n");
        assert!(blank.enabled_plugins.is_empty());
    }

    #[test]
    fn volatile_find_is_on_by_default() {
        assert!(Settings::default().volatile_find);
    }

    #[test]
    fn volatile_find_is_written_as_a_switch() {
        let mut settings = Settings::default();
        assert!(settings.render().contains("\nvolatile_find on\n"));
        settings.volatile_find = false;
        assert!(settings.render().contains("\nvolatile_find off\n"));
    }

    #[test]
    fn backup_and_autosave_are_written_as_keywords() {
        let settings = Settings {
            backup_mode: BackupMode::Simple,
            backup_keep: 3,
            autosave_seconds: 15,
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("\nbackup simple\n"), "{text}");
        assert!(text.contains("\nbackup_keep 3\n"), "{text}");
        assert!(text.contains("\nautosave 15\n"), "{text}");
    }

    #[test]
    fn the_file_dialog_is_the_portal_by_default() {
        assert_eq!(Settings::default().file_dialog, FileDialog::Portal);
        assert!(Settings::default()
            .render()
            .contains("\nfile_dialog portal\n"));
    }

    #[test]
    fn the_file_dialog_can_be_forced_to_the_built_in_browser() {
        let settings = Settings {
            file_dialog: FileDialog::Builtin,
            ..Settings::default()
        };
        assert!(settings.render().contains("\nfile_dialog builtin\n"));
        assert_eq!(Settings::parse(&settings.render()), settings);
    }

    #[test]
    fn an_unknown_file_dialog_falls_back_to_the_portal() {
        // The portal is the choice that degrades: when it is missing we fall
        // back to the built-in browser anyway, so an unreadable keyword cannot
        // leave the user with no dialog at all.
        assert_eq!(
            Settings::parse("file_dialog telepathy\n").file_dialog,
            FileDialog::Portal
        );
    }

    #[test]
    fn the_file_dialog_cycles_through_every_choice() {
        let mut mode = FileDialog::Portal;
        for _ in 0..FileDialog::ALL.len() {
            mode = mode.next();
        }
        assert_eq!(mode, FileDialog::Portal);
        assert_eq!(FileDialog::Portal.next(), FileDialog::Builtin);
    }

    #[test]
    fn an_unknown_backup_mode_falls_back_to_writing_nothing() {
        // A mode this version does not know must not become a mode that copies
        // files somewhere unexpected.
        assert_eq!(
            Settings::parse("backup sometimes\n").backup_mode,
            BackupMode::None
        );
    }

    #[test]
    fn an_autosave_interval_outside_the_range_is_clamped() {
        assert_eq!(Settings::parse("autosave 100000\n").autosave_seconds, MAX_AUTOSAVE);
        assert_eq!(Settings::parse("autosave 0\n").autosave_seconds, 0);
        assert_eq!(Settings::parse("autosave soon\n").autosave_seconds, 7);
    }

    #[test]
    fn a_backup_keep_outside_the_range_is_clamped() {
        assert_eq!(Settings::parse("backup_keep 0\n").backup_keep, MIN_BACKUP_KEEP);
        assert_eq!(Settings::parse("backup_keep 9999\n").backup_keep, MAX_BACKUP_KEEP);
        assert_eq!(Settings::parse("backup_keep many\n").backup_keep, 10);
    }

    #[test]
    fn autosave_is_on_by_default_and_backup_is_not() {
        let settings = Settings::default();
        assert_eq!(settings.autosave_seconds, 7);
        assert_eq!(settings.backup_mode, BackupMode::None);
    }

    #[test]
    fn the_print_setup_round_trips_through_the_file() {
        let settings = Settings {
            print_paper: "letter".to_string(),
            print_landscape: true,
            print_line_numbers: true,
            print_header: "$(FILE_NAME)".to_string(),
            print_footer: "$(PAGE)/$(NB_PAGES)".to_string(),
            ..Settings::default()
        };
        let text = settings.render();
        let read = Settings::parse(&text);
        assert_eq!(read.print_paper, "letter");
        assert!(read.print_landscape);
        assert!(read.print_line_numbers);
        assert_eq!(read.print_header, "$(FILE_NAME)");
        assert_eq!(read.print_footer, "$(PAGE)/$(NB_PAGES)");
    }

    #[test]
    fn a_tab_in_a_header_survives_the_file() {
        // The file is line-based, so a literal tab would be trimmed away and
        // the three zones of the header would collapse into one.
        let settings = Settings {
            print_header: "left\tcentre\tright".to_string(),
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("print_header left\\tcentre\\tright"), "{text}");
        assert_eq!(Settings::parse(&text).print_header, "left\tcentre\tright");
    }

    #[test]
    fn the_print_defaults_are_a4_portrait_with_a_page_number() {
        let settings = Settings::default();
        assert_eq!(settings.print_paper, "a4");
        assert!(!settings.print_landscape);
        assert!(!settings.print_line_numbers);
        assert!(settings.print_header.is_empty());
        assert_eq!(settings.print_footer, "$(PAGE)");
    }

    #[test]
    fn a_language_round_trips_through_the_file() {
        let settings = Settings {
            language: Some("french.xml".to_string()),
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("\nlanguage french.xml\n"), "{text}");
        assert_eq!(Settings::parse(&text).language.as_deref(), Some("french.xml"));
    }

    #[test]
    fn english_is_written_as_none_and_read_back_as_none() {
        let text = Settings::default().render();
        assert!(text.contains("\nlanguage none\n"), "{text}");
        assert_eq!(Settings::parse(&text).language, None);
        assert_eq!(Settings::parse("language none\n").language, None);
    }

    #[test]
    fn a_tab_layout_round_trips_through_the_file() {
        let settings = Settings {
            tab_layout: TabLayout::Vertical,
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("\ntab_layout vertical\n"), "{text}");
        assert_eq!(Settings::parse(&text).tab_layout, TabLayout::Vertical);
    }

    #[test]
    fn an_unknown_tab_layout_falls_back_to_a_single_row() {
        assert_eq!(
            Settings::parse("tab_layout diagonal\n").tab_layout,
            TabLayout::Horizontal
        );
    }

    #[test]
    fn a_style_override_round_trips() {
        let settings = Settings {
            styles: vec![
                (Slot::Keyword, Rgb::new(0x56, 0x9C, 0xD6)),
                (Slot::Background, Rgb::new(0x1E, 0x1E, 0x1E)),
            ],
            ..Settings::default()
        };
        let text = settings.render();
        assert!(text.contains("\nstyle keyword #569CD6\n"), "{text}");
        assert_eq!(Settings::parse(&text).styles, settings.styles);
    }

    #[test]
    fn a_style_line_without_a_hash_is_read() {
        let settings = Settings::parse("style caret FF0000\n");
        assert_eq!(settings.styles, vec![(Slot::Caret, Rgb::new(255, 0, 0))]);
    }

    #[test]
    fn a_malformed_style_line_is_skipped() {
        for line in [
            "style\n",
            "style nosuchslot #FFFFFF\n",
            "style keyword #FFFF\n",
            "style keyword #GGGGGG\n",
            "style keyword\n",
        ] {
            assert!(Settings::parse(line).styles.is_empty(), "{line:?}");
        }
    }

    #[test]
    fn a_repeated_style_line_keeps_the_last() {
        let settings = Settings::parse("style caret #111111\nstyle caret #222222\n");
        assert_eq!(settings.styles, vec![(Slot::Caret, Rgb::new(0x22, 0x22, 0x22))]);
    }

    #[test]
    fn style_lines_are_written_in_dialog_order() {
        let settings = Settings {
            styles: vec![
                (Slot::Caret, Rgb::new(1, 1, 1)),
                (Slot::Keyword, Rgb::new(2, 2, 2)),
            ],
            ..Settings::default()
        };
        let text = settings.render();
        let keyword = text.find("style keyword").unwrap();
        let caret = text.find("style caret").unwrap();
        assert!(keyword < caret, "the file should follow the dialog's order");
    }

    #[test]
    fn a_run_line_round_trips() {
        let mut settings = Settings::default();
        settings.runs.parse_line("cargo build --release");
        settings.runs.parse_line("!make -j8");
        let text = settings.render();
        assert!(text.contains("\nrun cargo build --release\n"), "{text}");
        assert!(text.contains("\nrun !make -j8\n"), "{text}");
        assert_eq!(Settings::parse(&text).runs, settings.runs);
    }

    #[test]
    fn a_run_line_keeps_its_spaces() {
        let settings = Settings::parse("run gcc -o $(NAME_PART) $(FULL_CURRENT_PATH)\n");
        assert_eq!(
            settings.runs.commands()[0].command,
            "gcc -o $(NAME_PART) $(FULL_CURRENT_PATH)"
        );
    }

    #[test]
    fn an_empty_run_line_is_skipped() {
        assert!(Settings::parse("run\n").runs.is_empty());
        assert!(Settings::parse("run    \n").runs.is_empty());
    }

    #[test]
    fn the_keymap_round_trips() {
        let settings = Settings::default();
        assert_eq!(Settings::parse(&settings.render()).keymap, settings.keymap);
    }

    #[test]
    fn the_file_lists_every_binding() {
        let text = Settings::default().render();
        assert!(text.starts_with("# Neko++ settings\n"));
        assert!(text.contains("\nkey save Ctrl+S\n"));
        assert!(text.contains("\nkey preferences Ctrl+,\n"));
        assert!(text.contains("\nkey unfold_all Ctrl+Alt+Shift+F\n"));
    }

    #[test]
    fn a_key_line_rebinds() {
        let settings = Settings::parse("key save F5\n");
        let chord = crate::keymap::Chord::parse("F5").unwrap();
        assert_eq!(settings.keymap.lookup(&chord), Some(crate::menu::Action::Save));
        let old = crate::keymap::Chord::parse("Ctrl+S").unwrap();
        assert_eq!(settings.keymap.lookup(&old), None);
    }

    #[test]
    fn a_key_line_for_an_unknown_command_is_skipped() {
        let settings = Settings::parse("key teleport F5\n");
        assert_eq!(settings.keymap, Settings::default().keymap);
    }

    #[test]
    fn a_key_line_without_a_chord_is_skipped() {
        let settings = Settings::parse("key save\n");
        assert_eq!(settings.keymap, Settings::default().keymap);
    }

    #[test]
    fn an_empty_file_gives_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let text = "# a comment\n\n   \ntheme light\n";
        assert_eq!(Settings::parse(text).theme, ThemeChoice::Light);
    }

    #[test]
    fn an_unknown_keyword_is_ignored() {
        let text = "theme light\nfuture_setting 3\n";
        let settings = Settings::parse(text);
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert_eq!(settings.font_size, Settings::default().font_size);
    }

    #[test]
    fn every_theme_keyword_round_trips() {
        for choice in ThemeChoice::ALL {
            let text = format!("theme {}\n", choice.key());
            assert_eq!(Settings::parse(&text).theme, choice);
        }
    }

    #[test]
    fn the_named_themes_are_the_catalogue_in_order() {
        // `ALL` puts the three built-ins first and then the catalogue, so a
        // theme added to one list and not the other is caught here rather than
        // by a user who cannot find it in Preferences.
        let named = &ThemeChoice::ALL[3..];
        assert_eq!(named.len(), crate::themes::CATALOGUE.len());
        for (choice, palette) in named.iter().zip(crate::themes::CATALOGUE.iter()) {
            assert_eq!(choice.key(), palette.key);
            assert_eq!(choice.label(), palette.name);
            assert_eq!(choice.palette(), Some(palette));
        }
    }

    #[test]
    fn every_choice_resolves_to_a_theme() {
        for choice in ThemeChoice::ALL {
            let theme = choice.theme(None);
            assert_eq!(theme.background, choice.theme(None).background);
            // A named theme is its palette's background; the three built-ins
            // are the two hand-written palettes.
            match choice.palette() {
                Some(palette) => assert_eq!(theme.background, palette.background),
                None => assert!(
                    theme.background == crate::theme::Theme::DARK.background
                        || theme.background == crate::theme::Theme::LIGHT.background
                ),
            }
            assert_eq!(
                choice.is_light(None),
                theme.background.luminance() >= 128,
                "{}",
                choice.label()
            );
        }
        // `System` is the only choice whose answer depends on the desktop.
        assert!(!ThemeChoice::System.is_light(Some(false)));
        assert!(ThemeChoice::System.is_light(Some(true)));
        assert!(ThemeChoice::Light.is_light(Some(false)));
        assert!(!ThemeChoice::Dark.is_light(Some(true)));
    }

    #[test]
    fn a_settings_file_from_before_the_third_choice_still_means_what_it_said() {
        // The file used to hold only `light` and `dark`. Both are still
        // keywords, so an upgrade does not silently move a user who chose one.
        assert_eq!(Settings::parse("theme light").theme, ThemeChoice::Light);
        assert_eq!(Settings::parse("theme dark").theme, ThemeChoice::Dark);
    }

    #[test]
    fn an_unknown_theme_falls_back_to_following_the_desktop() {
        assert_eq!(
            Settings::parse("theme sepia").theme,
            ThemeChoice::System
        );
    }

    #[test]
    fn the_default_theme_follows_the_desktop() {
        assert_eq!(Settings::default().theme, ThemeChoice::System);
    }

    #[test]
    fn a_system_choice_uses_what_the_desktop_said() {
        assert!(ThemeChoice::System.is_light(Some(true)));
        assert!(!ThemeChoice::System.is_light(Some(false)));
    }

    #[test]
    fn a_system_choice_with_no_answer_is_dark() {
        // No portal, no session bus, or a desktop with no preference: the
        // editor opens exactly as it did before this setting existed.
        assert!(!ThemeChoice::System.is_light(None));
    }

    #[test]
    fn an_explicit_choice_ignores_the_desktop() {
        assert!(ThemeChoice::Light.is_light(Some(false)));
        assert!(!ThemeChoice::Dark.is_light(Some(true)));
    }

    #[test]
    fn the_theme_row_steps_through_every_choice_and_wraps() {
        assert_eq!(ThemeChoice::System.next(), ThemeChoice::Light);
        assert_eq!(ThemeChoice::Light.next(), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::Dark.next(), ThemeChoice::AyuDark);
        assert_eq!(ThemeChoice::System.previous(), ThemeChoice::Zenburn);
        assert_eq!(ThemeChoice::Zenburn.previous(), ThemeChoice::VitesseDark);
        // One lap of `next` returns to the start, so the row can reach every
        // theme in the catalogue and cannot get stuck on the last one.
        let mut choice = ThemeChoice::System;
        for _ in 0..ThemeChoice::ALL.len() {
            choice = choice.next();
        }
        assert_eq!(choice, ThemeChoice::System);
    }

    #[test]
    fn a_malformed_value_keeps_the_default() {
        let text = "font_size wide\ntab_width -3\n";
        let settings = Settings::parse(text);
        assert_eq!(settings.font_size, Settings::default().font_size);
        assert_eq!(settings.tab_width, Settings::default().tab_width);
    }

    #[test]
    fn an_out_of_range_font_size_is_clamped() {
        assert_eq!(Settings::parse("font_size 400").font_size, MAX_FONT_SIZE);
        assert_eq!(Settings::parse("font_size 1").font_size, MIN_FONT_SIZE);
    }

    #[test]
    fn a_window_scale_is_clamped_and_zero_means_follow_the_display() {
        assert_eq!(Settings::parse("ui_scale 9").ui_scale, MAX_UI_SCALE);
        assert_eq!(Settings::parse("ui_scale 0.1").ui_scale, MIN_UI_SCALE);
        assert_eq!(Settings::parse("ui_scale 1.5").ui_scale, 1.5);
        // The default, and what a hand-edited nought turns back into: ask the
        // display rather than scale the furniture to nothing.
        assert_eq!(Settings::default().ui_scale, 0.0);
        assert_eq!(Settings::parse("ui_scale 0").ui_scale, 0.0);
        assert_eq!(Settings::parse("ui_scale -3").ui_scale, 0.0);
        assert_eq!(Settings::parse("ui_scale wide").ui_scale, 0.0);
    }

    #[test]
    fn a_window_scale_round_trips() {
        let settings = Settings {
            ui_scale: 2.0,
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&settings.render()).ui_scale, 2.0);
    }

    #[test]
    fn an_out_of_range_tab_width_is_clamped() {
        assert_eq!(Settings::parse("tab_width 99").tab_width, MAX_TAB_WIDTH);
        assert_eq!(Settings::parse("tab_width 0").tab_width, MIN_TAB_WIDTH);
    }

    #[test]
    fn a_line_without_a_value_is_skipped() {
        assert_eq!(Settings::parse("theme\n"), Settings::default());
    }

    #[test]
    fn a_fractional_font_size_survives() {
        assert_eq!(Settings::parse("font_size 15.5").font_size, 15.5);
    }
}
