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

//! Colours.
//!
//! Everything is authored as sRGB bytes because that is how designers and colour
//! pickers think. The GPU wants linear values, so conversion happens once, here,
//! at the boundary.

// The colour type itself moved to `nn-shared` in the hundred-and-fifteenth pass:
// three bytes are the same three bytes in a terminal, and the settings file that
// writes them as hex has to be read by both front-ends.
pub use nn_shared::color::Rgb;

/// What the GPU needs a colour turned into.
///
/// A trait rather than inherent methods because the type is no longer this
/// crate's to extend, and it is a trait here rather than in `nn-shared` because
/// none of these three answers means anything without a shader: linear values
/// for a vertex buffer, `cosmic_text`'s packed colour, and `wgpu`'s clear value.
pub trait ToGpu {
    /// The colour as linear RGBA, ready for a shader.
    fn to_linear(self) -> [f32; 4];
    /// The colour as linear RGBA with an explicit alpha.
    fn to_linear_alpha(self, alpha: f32) -> [f32; 4];
    /// The colour in the form glyphon expects.
    fn to_cosmic(self) -> cosmic_text::Color;
    /// The colour in the form `wgpu` expects for a clear value.
    fn to_wgpu(self) -> wgpu::Color;
}

impl ToGpu for Rgb {
    fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.0),
            srgb_to_linear(self.1),
            srgb_to_linear(self.2),
            1.0,
        ]
    }

    fn to_linear_alpha(self, alpha: f32) -> [f32; 4] {
        let [r, g, b, _] = self.to_linear();
        [r, g, b, alpha]
    }

    fn to_cosmic(self) -> cosmic_text::Color {
        cosmic_text::Color::rgb(self.0, self.1, self.2)
    }

    fn to_wgpu(self) -> wgpu::Color {
        let [r, g, b, a] = self.to_linear();
        wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: a as f64,
        }
    }
}

/// Blends `b` over `a` by `t`, in sRGB space.
///
/// The minimap wants muted versions of the syntax colours; blending towards the
/// column background is cheaper and steadier than a second palette.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(
        channel(a.0, b.0),
        channel(a.1, b.1),
        channel(a.2, b.2),
    )
}

/// Converts one sRGB channel to linear.
fn srgb_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// The colours the editor draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Behind the text.
    pub background: Rgb,
    /// Default text colour.
    pub foreground: Rgb,
    /// Behind the line numbers.
    pub gutter_background: Rgb,
    /// Line numbers.
    pub gutter_foreground: Rgb,
    /// The line number of the caret's line.
    pub gutter_active: Rgb,
    /// Behind the caret's line.
    pub current_line: Rgb,
    /// Background of the fold margin, right of the line numbers.
    pub fold_background: Rgb,
    /// The `+`/`-` box drawn on a foldable line.
    pub fold_marker: Rgb,
    /// The dot marking a bookmarked line.
    pub bookmark: Rgb,
    /// Background of the menu bar and its dropdowns.
    pub menu_background: Rgb,
    /// Background of the toolbar.
    pub toolbar_background: Rgb,
    /// The hairline under the toolbar and its group separators.
    pub toolbar_border: Rgb,
    /// Background of a hovered toolbar button.
    pub toolbar_hover: Rgb,
    /// A toolbar button's label.
    pub toolbar_text: Rgb,
    /// The body of a toolbar icon.
    ///
    /// Separate from `toolbar_text` because an icon is a drawing, not a glyph:
    /// it needs a little more contrast than a label to read at the same size,
    /// and it is the one place the two-tone scheme below is anchored.
    pub icon_ink: Rgb,
    /// The part of a toolbar icon that carries its meaning.
    ///
    /// Notepad++ draws its icons in two colours -- a grey body and a blue
    /// accent -- and the accent is what tells New from Close at a glance. It is
    /// a fixed hue rather than a theme colour so the icons stay recognisable
    /// across palettes, lightened on a dark background so it does not vibrate.
    pub icon_accent: Rgb,
    /// Menu label colour.
    pub menu_text: Rgb,
    /// A menu row that cannot be picked.
    ///
    /// Dimmer than `menu_text` but still readable: a greyed row is information
    /// -- it says the command exists and why it does not apply -- so it has to
    /// be legible rather than nearly invisible.
    pub menu_text_disabled: Rgb,
    /// Background of a highlighted menu row or title.
    pub menu_hover: Rgb,
    /// Dropdown frame and separator colour.
    pub menu_border: Rgb,
    /// The bar between two panes.
    ///
    /// Its own colour rather than the menu border's: the divider sits directly
    /// against the right pane's gutter, and a bar the same shade as the gutter
    /// reads as part of it rather than as something to grab.
    pub divider: Rgb,
    /// The shortcut column of a dropdown row.
    pub menu_shortcut: Rgb,
    /// A clickable address.
    ///
    /// Its own colour rather than [`Theme::icon_accent`], which happens to be
    /// the same blue today: the accent is chosen to read against a toolbar and
    /// this one has to read against the menu background, and a theme that
    /// retunes one should not silently retune the other.
    pub link: Rgb,
    /// Behind selected text.
    pub selection: Rgb,
    /// The caret itself.
    pub caret: Rgb,
    /// One colour per syntax style, indexed by the constants in
    /// [`nn_core::style`].
    pub syntax: [Rgb; 9],
    /// Behind the find bar.
    pub bar_background: Rgb,
    /// The hairline above the find bar.
    pub bar_border: Rgb,
    /// Labels and the match counter.
    pub bar_label: Rgb,
    /// Behind an enabled toggle.
    pub toggle_on: Rgb,
    /// The text of an enabled toggle.
    pub toggle_on_text: Rgb,
    /// Behind every search match.
    pub match_highlight: Rgb,
    /// Behind the match the caret is on.
    pub match_current: Rgb,
    /// Behind the five styled mark sets, in Search menu order.
    ///
    /// Five distinguishable hues rather than five shades of one: the whole point
    /// of the styles is telling two kinds of hit apart, and two shades of the same
    /// colour are not distinguishable at a glance. They are all dim enough to read
    /// text through, because a mark that hid its own text would be useless.
    pub mark_styles: [Rgb; 5],
    /// Behind every other occurrence of the word under the caret.
    ///
    /// Deliberately cooler and dimmer than `match_highlight`: smart highlighting
    /// is ambient, and it must not read as a search result.
    pub smart_highlight: Rgb,
    /// Behind the tab strip.
    pub tab_bar_background: Rgb,
    /// Behind an inactive tab.
    pub tab_inactive: Rgb,
    /// The bar along the leading edge of the active tab.
    ///
    /// Notepad++ paints this orange in both palettes — `#FAAA3C` when the pane
    /// has focus, `#FAD296` when it does not — and it is the single strongest
    /// cue for which document you are looking at. It is not derived from the
    /// accent colour for that reason: a theme that recoloured it would lose the
    /// one thing that makes a Notepad++ tab bar recognisable.
    pub tab_active_rule: Rgb,
    /// The title of the active tab.
    pub tab_active_text: Rgb,
    /// The title of an inactive tab.
    pub tab_inactive_text: Rgb,
    /// Behind an inactive tab the pointer is over.
    pub tab_hover: Rgb,
    /// The close cross and the pin on a tab.
    pub tab_button: Rgb,
    /// The pencil that marks a document with unsaved edits.
    pub tab_unsaved: Rgb,
    /// Behind a hover tooltip.
    ///
    /// Its own token rather than the menu's, because a tooltip is an annotation
    /// floating over the page rather than another menu: on the light palette it
    /// is deliberately dark, which is what makes it read as a label stuck on top
    /// of the window instead of as a panel that belongs to it.
    pub tooltip_background: Rgb,
    /// The text of a hover tooltip.
    pub tooltip_text: Rgb,
    /// Behind the minimap column.
    pub minimap_background: Rgb,
    /// The box marking the visible window on the minimap.
    pub minimap_viewport: Rgb,
    /// The dots and bars that stand in for spaces and tabs.
    pub whitespace: Rgb,
    /// The vertical rules marking each indent level.
    pub indent_guide: Rgb,
}

impl Theme {
    /// A dark theme in the Visual Studio Code family.
    pub const DARK: Theme = Theme {
        background: Rgb::new(30, 30, 30),
        foreground: Rgb::new(212, 212, 212),
        gutter_background: Rgb::new(37, 37, 38),
        gutter_foreground: Rgb::new(133, 133, 133),
        gutter_active: Rgb::new(198, 198, 198),
        current_line: Rgb::new(40, 40, 40),
        fold_background: Rgb::new(37, 37, 38),
        fold_marker: Rgb::new(120, 120, 120),
        bookmark: Rgb::new(90, 160, 230),
        menu_background: Rgb::new(45, 45, 48),
        toolbar_background: Rgb::new(37, 37, 40),
        toolbar_border: Rgb::new(60, 60, 64),
        toolbar_hover: Rgb::new(62, 62, 68),
        toolbar_text: Rgb::new(200, 200, 204),
        icon_ink: Rgb::new(206, 206, 210),
        icon_accent: Rgb::new(96, 165, 232),
        menu_text: Rgb::new(212, 212, 212),
        menu_text_disabled: Rgb::new(118, 118, 124),
        menu_hover: Rgb::new(62, 62, 68),
        menu_border: Rgb::new(70, 70, 74),
        divider: Rgb::new(110, 110, 118),
        menu_shortcut: Rgb::new(140, 140, 145),
        link: Rgb::new(96, 165, 232),
        selection: Rgb::new(38, 79, 120),
        caret: Rgb::new(174, 175, 173),
        syntax: [
            Rgb::new(212, 212, 212), // default
            Rgb::new(86, 156, 214),  // keyword
            Rgb::new(78, 201, 176),  // type
            Rgb::new(206, 145, 120), // string
            Rgb::new(106, 153, 85),  // comment
            Rgb::new(181, 206, 168), // number
            Rgb::new(220, 220, 170), // function
            Rgb::new(212, 212, 212), // operator
            Rgb::new(197, 134, 192), // preprocessor
        ],
        bar_background: Rgb::new(37, 37, 38),
        bar_border: Rgb::new(60, 60, 60),
        bar_label: Rgb::new(150, 150, 150),
        toggle_on: Rgb::new(14, 99, 156),
        toggle_on_text: Rgb::new(255, 255, 255),
        match_highlight: Rgb::new(70, 60, 20),
        match_current: Rgb::new(120, 90, 20),
        mark_styles: [
            Rgb::new(74, 52, 30),
            Rgb::new(30, 66, 44),
            Rgb::new(44, 40, 78),
            Rgb::new(78, 34, 44),
            Rgb::new(30, 62, 74),
        ],
        smart_highlight: Rgb::new(38, 56, 74),
        tab_bar_background: Rgb::new(24, 24, 24),
        tab_inactive: Rgb::new(45, 45, 45),
        tab_active_rule: Rgb::new(250, 170, 60),
        tab_active_text: Rgb::new(230, 230, 230),
        tab_inactive_text: Rgb::new(150, 150, 150),
        tab_hover: Rgb::new(58, 58, 58),
        tab_button: Rgb::new(190, 190, 194),
        tab_unsaved: Rgb::new(232, 176, 84),
        tooltip_background: Rgb::new(58, 58, 62),
        tooltip_text: Rgb::new(236, 236, 240),
        minimap_background: Rgb::new(26, 26, 26),
        minimap_viewport: Rgb::new(200, 200, 200),
        whitespace: Rgb::new(80, 80, 84),
        indent_guide: Rgb::new(52, 52, 56),
    };

    /// A light theme, for daylight and for projectors.
    ///
    /// The syntax colours are the same hues as the dark theme, darkened until
    /// they carry enough contrast against white. Reusing the hues keeps the
    /// document recognisable when the theme flips.
    pub const LIGHT: Theme = Theme {
        background: Rgb::new(255, 255, 255),
        foreground: Rgb::new(30, 30, 30),
        gutter_background: Rgb::new(240, 240, 240),
        gutter_foreground: Rgb::new(130, 130, 130),
        gutter_active: Rgb::new(40, 40, 40),
        current_line: Rgb::new(245, 245, 245),
        fold_background: Rgb::new(240, 240, 240),
        fold_marker: Rgb::new(120, 120, 120),
        bookmark: Rgb::new(30, 110, 200),
        menu_background: Rgb::new(245, 245, 245),
        toolbar_background: Rgb::new(238, 238, 238),
        toolbar_border: Rgb::new(205, 205, 205),
        toolbar_hover: Rgb::new(220, 230, 245),
        toolbar_text: Rgb::new(50, 50, 50),
        icon_ink: Rgb::new(64, 64, 68),
        icon_accent: Rgb::new(28, 108, 196),
        menu_text: Rgb::new(30, 30, 30),
        menu_text_disabled: Rgb::new(150, 150, 156),
        menu_hover: Rgb::new(220, 230, 245),
        menu_border: Rgb::new(200, 200, 200),
        divider: Rgb::new(150, 150, 156),
        menu_shortcut: Rgb::new(120, 120, 120),
        link: Rgb::new(28, 108, 196),
        selection: Rgb::new(180, 210, 245),
        caret: Rgb::new(30, 30, 30),
        syntax: [
            Rgb::new(30, 30, 30),    // default
            Rgb::new(0, 0, 200),     // keyword
            Rgb::new(38, 127, 110),  // type
            Rgb::new(163, 21, 21),   // string
            Rgb::new(0, 128, 0),     // comment
            Rgb::new(9, 134, 88),    // number
            Rgb::new(121, 94, 38),   // function
            Rgb::new(60, 60, 60),    // operator
            Rgb::new(128, 0, 128),   // preprocessor
        ],
        bar_background: Rgb::new(240, 240, 240),
        bar_border: Rgb::new(200, 200, 200),
        bar_label: Rgb::new(100, 100, 100),
        toggle_on: Rgb::new(0, 120, 215),
        toggle_on_text: Rgb::new(255, 255, 255),
        match_highlight: Rgb::new(255, 240, 160),
        match_current: Rgb::new(255, 200, 80),
        mark_styles: [
            Rgb::new(255, 226, 178),
            Rgb::new(198, 240, 198),
            Rgb::new(214, 208, 255),
            Rgb::new(255, 200, 214),
            Rgb::new(190, 232, 240),
        ],
        smart_highlight: Rgb::new(214, 232, 248),
        tab_bar_background: Rgb::new(228, 228, 228),
        tab_inactive: Rgb::new(240, 240, 240),
        tab_active_rule: Rgb::new(250, 170, 60),
        tab_active_text: Rgb::new(20, 20, 20),
        tab_inactive_text: Rgb::new(110, 110, 110),
        tab_hover: Rgb::new(232, 232, 232),
        tab_button: Rgb::new(70, 70, 74),
        tab_unsaved: Rgb::new(196, 96, 40),
        tooltip_background: Rgb::new(48, 48, 52),
        tooltip_text: Rgb::new(245, 245, 245),
        minimap_background: Rgb::new(248, 248, 248),
        minimap_viewport: Rgb::new(120, 120, 120),
        whitespace: Rgb::new(175, 175, 175),
        indent_guide: Rgb::new(224, 224, 224),
    };

    /// Builds a theme from a palette.
    ///
    /// Sixty-odd colours are more than anyone can pick well, and a constant per
    /// theme would be sixty lines of near-identical hex that drift apart at the
    /// first edit. So a theme in the catalogue is authored as the colours a
    /// designer actually chooses -- background, text, gutter, selection, caret,
    /// one accent and the nine syntax styles -- and the chrome is derived here.
    ///
    /// The derivation is deliberately boring: every neutral is the foreground
    /// mixed `t` of the way into the background, so a light theme darkens as
    /// `t` grows and a dark theme lightens, from one expression. The cost is
    /// that a palette cannot have, say, a green menu bar; the gain is that
    /// adding a theme is a fourteen-line table and not a directory.
    ///
    /// Three colours are not derived. The orange rule on the active tab and the
    /// pencil on an unsaved one are Notepad++'s own, and the tab bar is the one
    /// place a user recognises the editor by its colours; they stay whatever the
    /// palette says.
    pub fn from_palette(palette: &crate::themes::Palette) -> Theme {
        let background = palette.background;
        let foreground = palette.foreground;
        // A neutral: `t` of the way from the background towards the text.
        let wash = |t: f32| mix(background, foreground, t);
        // A hue, muted onto the background. The mark colours and the search
        // highlights are the only places a theme shows its own hue in the
        // chrome, and both have to stay readable through.
        let tint = |hue: Rgb, t: f32| mix(background, hue, t);
        let (tooltip_background, tooltip_text) = if palette.light {
            (wash(0.82), Rgb::new(255, 255, 255))
        } else {
            (wash(0.18), wash(0.94))
        };
        let highlight = if palette.light { 0.34 } else { 0.28 };
        Theme {
            background,
            foreground,
            gutter_background: palette.gutter,
            gutter_foreground: wash(0.55),
            gutter_active: wash(0.9),
            current_line: wash(0.05),
            fold_background: palette.gutter,
            fold_marker: wash(0.5),
            bookmark: palette.accent,
            menu_background: wash(0.07),
            toolbar_background: wash(0.04),
            toolbar_border: wash(0.14),
            toolbar_hover: wash(0.16),
            toolbar_text: wash(0.84),
            icon_ink: wash(0.88),
            icon_accent: palette.accent,
            menu_text: foreground,
            menu_text_disabled: wash(0.42),
            menu_hover: wash(0.16),
            menu_border: wash(0.16),
            divider: wash(0.25),
            menu_shortcut: wash(0.52),
            link: palette.accent,
            selection: palette.selection,
            caret: palette.caret,
            syntax: palette.syntax,
            bar_background: wash(0.06),
            bar_border: wash(0.14),
            bar_label: wash(0.58),
            toggle_on: palette.accent,
            toggle_on_text: if palette.light {
                Rgb::new(255, 255, 255)
            } else {
                mix(background, Rgb::new(0, 0, 0), 0.75)
            },
            match_highlight: tint(Rgb::new(220, 180, 60), highlight),
            match_current: tint(Rgb::new(220, 180, 60), highlight + 0.18),
            mark_styles: [
                tint(Rgb::new(204, 130, 60), 0.35),
                tint(Rgb::new(60, 160, 90), 0.35),
                tint(Rgb::new(120, 110, 200), 0.35),
                tint(Rgb::new(200, 80, 90), 0.35),
                tint(Rgb::new(60, 150, 170), 0.35),
            ],
            smart_highlight: wash(0.1),
            tab_bar_background: mix(background, Rgb::new(0, 0, 0), 0.1),
            tab_inactive: wash(0.07),
            tab_active_rule: Rgb::new(250, 170, 60),
            tab_active_text: foreground,
            tab_inactive_text: wash(0.58),
            tab_hover: wash(0.12),
            tab_button: wash(0.8),
            tab_unsaved: if palette.light {
                Rgb::new(196, 96, 40)
            } else {
                Rgb::new(232, 176, 84)
            },
            tooltip_background,
            tooltip_text,
            minimap_background: mix(background, Rgb::new(0, 0, 0), 0.03),
            minimap_viewport: wash(0.7),
            whitespace: wash(0.3),
            indent_guide: wash(0.12),
        }
    }
}

/// Reads a `#RRGGBB` or `RRGGBB` colour.
fn parse_hex(text: &str) -> Option<Rgb> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 || !text.is_ascii() {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some(Rgb::new(byte(0)?, byte(2)?, byte(4)?))
}

/// `c` moved `amount` of the way towards white (positive) or black (negative).
fn shade(c: Rgb, amount: f32) -> Rgb {
    if amount >= 0.0 {
        mix(c, Rgb::new(255, 255, 255), amount)
    } else {
        mix(c, Rgb::new(0, 0, 0), -amount)
    }
}

/// Builds a theme from a Notepad++ `stylers.xml`.
///
/// Notepad++'s theme file names the colours of its own widgets and of each
/// lexer's token classes. Only the parts this editor draws are read: the global
/// styles, and the `c` lexer's nine token classes, which are the nine styles
/// every built-in language here emits. A theme that colours `python` differently
/// from `c` is not honoured, because this editor has one palette per document
/// rather than one per language.
///
/// Anything the file does not name keeps the value from the base theme, which is
/// chosen by the background's brightness. A theme file is usually complete, but
/// a hand-edited one is not, and a missing colour should leave a readable editor
/// rather than a black rectangle.
///
/// The chrome -- menus, tabs, toolbar -- has no counterpart in the file, so it is
/// derived from the two colours that do: the background and the foreground. That
/// is what makes an imported theme look like one theme rather than a text area
/// dropped into someone else's window.
pub fn from_stylers(xml: &str) -> Option<Theme> {
    // The global styles first, because the background decides which base theme
    // the rest of the file is filled in against.
    let mut widgets: Vec<(&str, Option<Rgb>, Option<Rgb>)> = Vec::new();
    for element in nn_core::xml::elements(xml, "WidgetStyle") {
        let Some(name) = nn_core::xml::attribute(&element, "name") else {
            continue;
        };
        let fg = nn_core::xml::attribute(&element, "fgColor").and_then(parse_hex);
        let bg = nn_core::xml::attribute(&element, "bgColor").and_then(parse_hex);
        widgets.push((name, fg, bg));
    }
    let find = |name: &str| widgets.iter().find(|(n, _, _)| *n == name).copied();

    let (_, global_fg, global_bg) = find("GlobalStyles")?;
    let background = global_bg?;
    let foreground = global_fg?;

    let mut theme = if background.luminance() < 128 {
        Theme::DARK
    } else {
        Theme::LIGHT
    };
    theme.background = background;
    theme.foreground = foreground;

    // The widgets the file names, in the order they are read.
    if let Some((_, fg, bg)) = find("Current line background colour") {
        theme.current_line = bg.or(fg).unwrap_or(theme.current_line);
    }
    if let Some((_, fg, bg)) = find("Selected text colour") {
        theme.selection = bg.or(fg).unwrap_or(theme.selection);
    }
    if let Some((_, fg, _)) = find("Caret colour") {
        theme.caret = fg.unwrap_or(theme.caret);
    }
    if let Some((_, fg, bg)) = find("Line number margin") {
        theme.gutter_background = bg.unwrap_or(theme.gutter_background);
        theme.gutter_foreground = fg.unwrap_or(theme.gutter_foreground);
    }
    if let Some((_, fg, bg)) = find("Fold").or_else(|| find("Fold margin")) {
        theme.fold_background = bg.unwrap_or(theme.fold_background);
        theme.fold_marker = fg.unwrap_or(theme.fold_marker);
    }
    if let Some((_, fg, _)) = find("Bookmark margin") {
        theme.bookmark = fg.unwrap_or(theme.bookmark);
    }
    if let Some((_, fg, _)) = find("Indent guideline style") {
        theme.indent_guide = fg.unwrap_or(theme.indent_guide);
    }
    if let Some((_, fg, _)) = find("White space symbol") {
        theme.whitespace = fg.unwrap_or(theme.whitespace);
    }
    if let Some((_, _, bg)) = find("Smart HighLighting") {
        theme.smart_highlight = bg.unwrap_or(theme.smart_highlight);
    }
    if let Some((_, _, bg)) = find("Tags match highlighting") {
        theme.match_highlight = bg.unwrap_or(theme.match_highlight);
    }
    for (index, name) in [
        "Mark Style 1",
        "Mark Style 2",
        "Mark Style 3",
        "Mark Style 4",
        "Mark Style 5",
    ]
    .iter()
    .enumerate()
    {
        if let Some((_, _, bg)) = find(name) {
            theme.mark_styles[index] = bg.unwrap_or(theme.mark_styles[index]);
        }
    }

    // The token classes, from the `c` lexer. Notepad++ names them in capitals;
    // the mapping to this editor's nine styles is one-to-one.
    let lexer = nn_core::xml::elements(xml, "LexerType")
        .into_iter()
        .find(|element| nn_core::xml::attribute(element, "name") == Some("c"));
    if let Some(lexer) = lexer {
        for element in nn_core::xml::elements(lexer.body, "WordsStyle") {
            let Some(name) = nn_core::xml::attribute(&element, "name") else {
                continue;
            };
            let Some(fg) = nn_core::xml::attribute(&element, "fgColor").and_then(parse_hex) else {
                continue;
            };
            let slot = match name {
                "DEFAULT" => 0,
                "INSTRUCTION WORD" => 1,
                "TYPE WORD" => 2,
                "STRING" => 3,
                "COMMENT" => 4,
                "NUMBER" => 5,
                "FUNCTION" => 6,
                "OPERATOR" => 7,
                "PREPROCESSOR" => 8,
                _ => continue,
            };
            theme.syntax[slot] = fg;
        }
    }

    // The chrome, derived from the two colours the file does name.
    let raised = if background.luminance() < 128 {
        shade(background, 0.06)
    } else {
        shade(background, -0.04)
    };
    let sunken = if background.luminance() < 128 {
        shade(background, -0.03)
    } else {
        shade(background, -0.02)
    };
    let rule = mix(background, foreground, 0.18);
    let dim = mix(foreground, background, 0.35);
    let accent = Rgb::new(14, 99, 156);
    theme.menu_background = raised;
    theme.menu_text = foreground;
    theme.menu_text_disabled = mix(foreground, background, 0.45);
    theme.menu_hover = mix(raised, foreground, 0.12);
    theme.menu_border = rule;
    theme.menu_shortcut = dim;
    theme.toolbar_background = raised;
    theme.toolbar_border = rule;
    theme.toolbar_hover = theme.menu_hover;
    theme.toolbar_text = foreground;
    theme.divider = rule;
    theme.bar_background = raised;
    theme.bar_border = rule;
    theme.bar_label = dim;
    theme.toggle_on = accent;
    theme.toggle_on_text = Rgb::new(255, 255, 255);
    theme.match_current = shade(theme.match_highlight, 0.12);
    theme.tab_bar_background = sunken;
    theme.tab_inactive = raised;
    // The orange stays: it is the tab bar's identity, not a theme colour.
    theme.tab_active_text = foreground;
    theme.tab_inactive_text = dim;
    theme.tab_hover = mix(raised, foreground, 0.08);
    theme.tab_button = mix(foreground, background, 0.25);
    theme.tab_unsaved = mix(accent, Rgb::new(250, 170, 60), 0.5);
    theme.minimap_background = sunken;
    theme.minimap_viewport = mix(foreground, background, 0.5);
    theme.gutter_active = foreground;
    Some(theme)
}

impl Theme {
    /// The colour for a syntax style.
    ///
    /// Unknown styles fall back to the default text colour, so a lexer that
    /// invents a style index cannot make text invisible.
    pub fn style_color(&self, style: nn_core::Style) -> Rgb {
        self.syntax
            .get(style as usize)
            .copied()
            .unwrap_or(self.foreground)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::DARK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_and_white_are_exact() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mid_grey_is_darker_in_linear() {
        // 128 sRGB is about 0.216 linear.
        assert!((srgb_to_linear(128) - 0.2158).abs() < 0.001);
    }

    #[test]
    fn linear_conversion_is_monotonic() {
        let mut prev = -1.0;
        for c in 0..=255u8 {
            let v = srgb_to_linear(c);
            assert!(v > prev);
            prev = v;
        }
    }

    /// A `stylers.xml` with the elements the importer reads.
    const STYLERS: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <GlobalStyles>
        <WidgetStyle name="GlobalStyles" fgColor="C0C0C0" bgColor="101820" />
        <WidgetStyle name="Current line background colour" bgColor="1A2430" />
        <WidgetStyle name="Selected text colour" bgColor="264F78" />
        <WidgetStyle name="Caret colour" fgColor="FFCC00" />
        <WidgetStyle name="Line number margin" fgColor="808080" bgColor="0C1218" />
        <WidgetStyle name="Mark Style 1" bgColor="402010" />
    </GlobalStyles>
    <LexerStyles>
        <LexerType name="c" desc="C" ext="">
            <WordsStyle name="DEFAULT" fgColor="C0C0C0" bgColor="101820" />
            <WordsStyle name="COMMENT" fgColor="5F8F5F" bgColor="101820" />
            <WordsStyle name="KEYWORD" fgColor="000000" bgColor="101820" />
            <WordsStyle name="INSTRUCTION WORD" fgColor="569CD6" bgColor="101820" />
            <WordsStyle name="STRING" fgColor="CE9178" bgColor="101820" />
        </LexerType>
        <LexerType name="python" desc="Python" ext="py">
            <WordsStyle name="INSTRUCTION WORD" fgColor="FF0000" bgColor="101820" />
        </LexerType>
    </LexerStyles>
</NotepadPlus>"#;

    #[test]
    fn a_theme_file_supplies_the_colours_it_names() {
        let theme = from_stylers(STYLERS).expect("the file has a GlobalStyles element");
        assert_eq!(theme.background, Rgb::new(0x10, 0x18, 0x20));
        assert_eq!(theme.foreground, Rgb::new(0xC0, 0xC0, 0xC0));
        assert_eq!(theme.current_line, Rgb::new(0x1A, 0x24, 0x30));
        assert_eq!(theme.selection, Rgb::new(0x26, 0x4F, 0x78));
        assert_eq!(theme.caret, Rgb::new(0xFF, 0xCC, 0x00));
        assert_eq!(theme.gutter_background, Rgb::new(0x0C, 0x12, 0x18));
        assert_eq!(theme.mark_styles[0], Rgb::new(0x40, 0x20, 0x10));
    }

    #[test]
    fn the_token_classes_come_from_the_c_lexer_only() {
        // The file colours `python` differently from `c`. This editor has one
        // palette per document, so the `c` lexer is the one that is read and the
        // `python` block is ignored rather than merged.
        let theme = from_stylers(STYLERS).unwrap();
        assert_eq!(theme.syntax[1], Rgb::new(0x56, 0x9C, 0xD6));
        assert_eq!(theme.syntax[3], Rgb::new(0xCE, 0x91, 0x78));
        assert_eq!(theme.syntax[4], Rgb::new(0x5F, 0x8F, 0x5F));
        assert_ne!(theme.syntax[1], Rgb::new(0xFF, 0, 0));
    }

    #[test]
    fn a_colour_the_file_does_not_name_keeps_the_base_theme() {
        let theme = from_stylers(STYLERS).unwrap();
        // `NUMBER` is not in the file, so it keeps the dark theme's value.
        assert_eq!(theme.syntax[5], Theme::DARK.syntax[5]);
        // A dark background picks the dark base.
        assert_eq!(theme.syntax[2], Theme::DARK.syntax[2]);
    }

    #[test]
    fn a_light_background_picks_the_light_base() {
        let text = STYLERS.replace("bgColor=\"101820\"", "bgColor=\"FFFFFF\"");
        let theme = from_stylers(&text).unwrap();
        assert_eq!(theme.syntax[5], Theme::LIGHT.syntax[5]);
    }

    #[test]
    fn a_file_without_global_styles_is_refused() {
        // Half a theme is worse than none: the user would have no way to tell
        // which colours came from the file and which from the built-in theme.
        assert_eq!(from_stylers("<NotepadPlus></NotepadPlus>"), None);
        assert_eq!(from_stylers(""), None);
    }

    #[test]
    fn the_chrome_is_derived_from_the_two_colours_the_file_names() {
        let theme = from_stylers(STYLERS).unwrap();
        // The menu is a shade off the background rather than the background
        // itself, so a dropdown reads as a surface above the text.
        assert_ne!(theme.menu_background, theme.background);
        assert_eq!(theme.menu_text, theme.foreground);
        assert_eq!(theme.toolbar_text, theme.foreground);
        // A disabled row is dimmer than an enabled one but still legible.
        assert_ne!(theme.menu_text_disabled, theme.menu_text);
        assert!(theme.menu_text_disabled.luminance() > 40);
    }

    #[test]
    fn a_hex_colour_is_read_with_or_without_its_hash() {
        assert_eq!(parse_hex("#FF8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_hex("ff8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(parse_hex("FF80"), None);
        assert_eq!(parse_hex("GGGGGG"), None);
        assert_eq!(parse_hex(""), None);
    }
}
