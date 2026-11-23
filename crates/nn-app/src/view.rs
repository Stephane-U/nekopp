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

//! The editor view: layout, per-line shaping cache, and the draw list.
//!
//! The expensive part of drawing text is shaping — turning a string into
//! positioned glyphs. Doing that for every visible line every frame would cap the
//! editor at a few hundred frames per second on a good day and far less on a
//! large file. So each line is shaped once and cached, keyed by its text and
//! style. Scrolling then costs one shape per newly revealed line, and typing
//! costs one shape for the line that changed.
//!
//! Line numbers are cached the same way, keyed by the number itself rather than
//! by line index. Rebuilding one buffer holding every visible number would
//! reshape all of them on every scroll step, which measured at 9 ms per frame at
//! 4K — the difference between missing and making the 120 Hz target.
//!
//! An unwrapped line is shaped one screenful at a time, not whole. Shaping is
//! linear in characters, so a thirty-thousand character JSONL record cost a
//! second per frame when a minimap drag put forty of them on screen; only the
//! seventy-eight columns that fit can be drawn, so only those are shaped, plus a
//! margin that keeps a scroll from reshaping on every pixel.

use std::collections::HashMap;
use std::ops::Range;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glyphon::{TextArea, TextBounds};
use nn_core::{Document, Editor, Style};

use crate::text::shaping_for;

use crate::minimap::Minimap;
use crate::quad::QuadRenderer;
use crate::palette::Palette;
use crate::theme::Theme;

/// A shaped line, kept until its text or its styling changes.
struct CachedLine {
    buffer: Buffer,
    text: String,
    /// `(char offset, style)` at the start of each run of equal style.
    runs: Vec<(usize, Style)>,
    /// The column an unwrapped line's window starts at. Zero for a wrapped
    /// segment, which is already as short as the pane.
    window: usize,
}

/// Collapses a line's per-character styles into runs of equal style.
///
/// A line of code is a handful of runs, not one per character, so this keeps
/// both the cache key and the shaped span list small.
fn style_runs(styles: &[Style]) -> Vec<(usize, Style)> {
    let mut runs: Vec<(usize, Style)> = Vec::with_capacity(8);
    for (i, &style) in styles.iter().enumerate() {
        match runs.last() {
            Some(&(_, last)) if last == style => {}
            _ => runs.push((i, style)),
        }
    }
    runs
}

/// Cuts `text` into one slice per run, each carrying its own colour.
fn spans<'a>(
    text: &'a str,
    runs: &[(usize, Style)],
    palette: &Palette,
    family: Family<'static>,
) -> Vec<(&'a str, Attrs<'static>)> {
    let mut out = Vec::with_capacity(runs.len());
    let mut chars = text.char_indices().peekable();
    let mut char_idx = 0usize;

    for (i, &(start, style)) in runs.iter().enumerate() {
        while char_idx < start && chars.next().is_some() {
            char_idx += 1;
        }
        let begin = chars.peek().map_or(text.len(), |&(b, _)| b);
        let stop = runs.get(i + 1).map_or(usize::MAX, |r| r.0);
        while char_idx < stop && chars.next().is_some() {
            char_idx += 1;
        }
        let end = chars.peek().map_or(text.len(), |&(b, _)| b);
        if begin < end {
            out.push((
                &text[begin..end],
                Attrs::new()
                    .family(family)
                    .color(palette.color(style).to_cosmic()),
            ));
        }
    }
    out
}

/// The character range of one wrapped segment of `line`.
///
/// With wrapping off there is a single segment covering the whole line.
fn segment_bounds(doc: &Document, line: usize, segment: u16, wrap: usize) -> (usize, usize) {
    let range = doc.line_range(line);
    if wrap == 0 {
        return (range.start, range.end);
    }
    let start = (range.start + segment as usize * wrap).min(range.end);
    let end = (start + wrap).min(range.end);
    (start, end)
}

/// Extra columns shaped beyond the ones that fit, so a scroll of a few columns
/// does not reshape the line.
const WINDOW_MARGIN: usize = 64;

/// The window of an unwrapped line to shape, as `(first column, columns)`.
///
/// The window always covers the columns on screen: it starts at a multiple of
/// [`WINDOW_MARGIN`] and is that much wider than the pane, so the last column in
/// view is inside it whatever the horizontal scroll. Two scrolls less than a
/// margin apart share a window, which is what keeps a drag from reshaping the
/// whole pane on every motion event.
fn line_window(scroll_x: f32, char_width: f32, visible: f32) -> (usize, usize) {
    let char_width = char_width.max(1.0);
    let across = (visible.max(0.0) / char_width).ceil() as usize;
    let first = (scroll_x.max(0.0) / char_width) as usize;
    (first / WINDOW_MARGIN * WINDOW_MARGIN, across + WINDOW_MARGIN)
}

/// The characters of `range` to shape for `window`, as a character range.
///
/// A tab is not one column wide and a wide character is more than one, so a
/// window that starts in the middle of such a line cannot be placed by width
/// alone: the shaped text is laid out from its own first character, and the tab
/// stops after the cut would be counted from the wrong place. Such a line is
/// shaped from its start and cut at the window's right edge instead, which still
/// costs what is on screen rather than what the line is: nothing past the last
/// column in view is shaped either way.
fn window_bounds(doc: &Document, range: Range<usize>, window: (usize, usize)) -> (usize, usize) {
    let stop = (range.start + window.0 + window.1).min(range.end);
    let one_column = doc
        .slice(range.start..stop)
        .chars()
        .all(|c| c.is_ascii() && c != '\t');
    let front = if one_column { window.0 } else { 0 };
    ((range.start + front).min(range.end), stop)
}

/// True if `runs` describes `styles` exactly, without allocating.
fn runs_match(styles: &[Style], runs: &[(usize, Style)]) -> bool {
    let Some(&(0, _)) = runs.first() else {
        return styles.is_empty();
    };
    let mut at = 0;
    for (i, &(start, style)) in runs.iter().enumerate() {
        let end = runs.get(i + 1).map_or(styles.len(), |r| r.0);
        if start != at || end > styles.len() {
            return false;
        }
        if styles[start..end].iter().any(|&s| s != style) {
            return false;
        }
        at = end;
    }
    at == styles.len()
}

/// Shapes and caches one `Buffer` per visible row.
///
/// The key is `(line, segment)`: with wrapping on, one document line is several
/// rows and each is shaped on its own.
struct LineCache {
    lines: HashMap<(usize, u16), CachedLine>,
    metrics: Metrics,
    family: Family<'static>,
    /// How many columns a tab occupies.
    tab_width: u16,
    /// The document revision the cache was last reconciled against.
    revision: u64,
    /// The document the cache was last reconciled against.
    id: u64,
}

impl LineCache {
    fn new(metrics: Metrics, family: Family<'static>, tab_width: u16) -> Self {
        Self {
            lines: HashMap::new(),
            metrics,
            family,
            tab_width,
            revision: u64::MAX,
            id: u64::MAX,
        }
    }

    /// Shapes `line` if the cache is missing or stale.
    ///
    /// `revalidate` is false when the document has not changed since the last
    /// frame, in which case a cached line is by definition still correct and the
    /// comparison — and its allocations — can be skipped entirely.
    ///
    /// `window` is where the shaped span starts for an unwrapped line, and how
    /// many columns it covers; a wrapped segment ignores it.
    #[allow(clippy::too_many_arguments)]
    fn ensure(
        &mut self,
        doc: &Document,
        line: usize,
        segment: u16,
        wrap: usize,
        window: (usize, usize),
        font_system: &mut FontSystem,
        palette: &Palette,
        revalidate: bool,
    ) {
        let range = doc.line_range(line);
        let (start, end) = if wrap == 0 {
            window_bounds(doc, range.clone(), window)
        } else {
            segment_bounds(doc, line, segment, wrap)
        };
        // The column the shaped text starts at, which is where it is drawn.
        // Zero for a line that had to be shaped from its start, and for a
        // wrapped segment, which always is.
        let front = if wrap == 0 { start - range.start } else { 0 };
        let styles = &doc.styles()[start..end];

        if let Some(cached) = self.lines.get(&(line, segment)) {
            // A cached window has to be the window being drawn, or the picture
            // would be of another part of the line.
            if cached.window == front
                && (!revalidate
                    || (doc.slice(start..end) == cached.text && runs_match(styles, &cached.runs)))
            {
                return;
            }
        }

        let text = doc.slice(start..end);
        let runs = style_runs(styles);

        let mut buffer = Buffer::new_empty(self.metrics);
        buffer.set_size(None, None);
        buffer.set_tab_width(self.tab_width);
        let default = Attrs::new()
            .family(self.family)
            .color(palette.theme.foreground.to_cosmic());
        buffer.set_rich_text(
            spans(&text, &runs, palette, self.family),
            &default,
            shaping_for(&text),
            None,
        );
        buffer.shape_until_scroll(font_system, false);

        self.lines.insert(
            (line, segment),
            CachedLine {
                buffer,
                text,
                runs,
                window: front,
            },
        );
    }

    fn get(&self, line: usize, segment: u16) -> Option<&CachedLine> {
        self.lines.get(&(line, segment))
    }

    /// Drops every row outside `keep`.
    fn retain(&mut self, keep: Range<usize>) {
        self.lines.retain(|&(line, _), _| keep.contains(&line));
    }
}

/// Everything needed to draw the editor.
pub struct EditorView {
    cache: LineCache,
    numbers: HashMap<u32, Buffer>,
    metrics: Metrics,
    family: Family<'static>,
    theme: Theme,
    /// The colours the text is drawn in, which the theme alone cannot say.
    palette: Palette,
    width: f32,
    height: f32,
    /// Where this view's top-left corner sits in the window.
    ///
    /// Zero for a single pane. A split view gives the second pane a non-zero
    /// origin, and every coordinate this view emits is offset by it, so the
    /// drawing code never has to know which pane it is.
    origin_x: f32,
    origin_y: f32,
    line_height: f32,
    char_width: f32,
    gutter_width: f32,
    /// Width of the bookmark margin, between the line numbers and the folds.
    bookmark_width: f32,
    /// Width of the fold margin, between the bookmarks and the text.
    fold_width: f32,
    padding: f32,
    /// The height available to the text, set by the renderer.
    ///
    /// Explicit rather than derived from the window height minus the bars,
    /// because a split view gives each pane a height that is not the window's and
    /// the bars are shared between them.
    text_height: f32,
    /// Height reserved at the top for the tab strip.
    content_top: f32,
    /// Width reserved on the right for the function list.
    right_dock: f32,
    /// Every search match, as character ranges.
    matches: Vec<Range<usize>>,
    /// The match the caret is on.
    current_match: Option<Range<usize>>,
    /// The bracket pair the caret is next to, as `(open, close)`.
    brace: Option<(usize, usize)>,
    /// Every occurrence of the word under the caret.
    smart: Vec<Range<usize>>,
    /// `(row, document line, wrapped segment)` for every row in the visible
    /// window, filled by `prepare`. Folding and wrapping both make rows and lines
    /// diverge, so the three are carried together rather than recomputed per draw
    /// call.
    rows: Vec<(usize, usize, u16)>,
    /// Whether long lines wrap instead of scrolling sideways.
    wrap: bool,
    /// How many whole rows the last prepared frame had room for.
    ///
    /// Kept so the next one can tell a window that shrank from one that did not:
    /// a caret that was on screen and no longer fits is scrolled back into
    /// view, and a caret that was already scrolled away is left alone.
    last_rows: usize,
    /// How many columns a tab occupies.
    tab_width: u16,
    /// The document thumbnail on the right edge.
    minimap: Minimap,
    /// Draw a dot for every space and a bar for every tab.
    show_whitespace: bool,
    /// Draw a mark where each line ends.
    show_eol: bool,
    /// Draw a rule at every indent level.
    show_indent_guides: bool,
    /// Draw a box where a control character sits.
    ///
    /// A quad rather than a substituted glyph, for the same reason the whitespace
    /// marks are: the shaped text is cached and the caret's x is
    /// `column * char_width`, so replacing a character would move every column
    /// after it.
    show_non_printing: bool,
    /// Draw a box for control characters and for the Unicode line separators.
    ///
    /// Notepad++'s "Show Control Characters & Unicode EOL" is the non-printing
    /// marks plus U+2028, U+2029 and U+0085, which are line breaks to a text
    /// editor and invisible to a reader. A superset of `show_non_printing`, so
    /// the two flags are read together.
    show_control_chars: bool,
    /// Draw a chevron where a long line was wrapped.
    show_wrap_symbol: bool,
}

impl EditorView {
    /// Measures the font and builds an empty view.
    pub fn new(font_system: &mut FontSystem, theme: Theme) -> Self {
        Self::with_char_width(theme, measure_editor_char_width(font_system))
    }

    /// Builds an empty view for a width that was measured earlier.
    ///
    /// Separated from `new` because the measurement is the first shaping a
    /// process does and the first shaping is the slow one: the font database
    /// resolves the family and the shaper fills its caches. A caller that can
    /// measure while something else is being set up -- which is what a cold
    /// start is -- does it there and comes in through this.
    pub fn with_char_width(theme: Theme, char_width: f32) -> Self {
        let font_size = 15.0;
        let line_height = 20.0;
        let metrics = Metrics::new(font_size, line_height);
        let family = Family::Monospace;

        Self {
            cache: LineCache::new(metrics, family, 4),
            numbers: HashMap::new(),
            metrics,
            family,
            theme,
            palette: Palette::new(theme),
            width: 800.0,
            height: 600.0,
            line_height,
            char_width,
            gutter_width: 48.0,
            bookmark_width: 12.0,
            fold_width: 14.0,
            padding: 8.0,
            text_height: 0.0,
            content_top: 0.0,
            right_dock: 0.0,
            origin_x: 0.0,
            origin_y: 0.0,
            matches: Vec::new(),
            current_match: None,
            brace: None,
            smart: Vec::new(),
            rows: Vec::new(),
            wrap: false,
            last_rows: 0,
            tab_width: 4,
            minimap: Minimap::new(),
            show_whitespace: false,
            show_eol: false,
            show_indent_guides: false,
            show_non_printing: false,
            show_control_chars: false,
            show_wrap_symbol: false,
        }
    }

    /// The gap between the window edge and the content.
    pub fn padding(&self) -> f32 {
        self.padding
    }

    /// The advance width of one character.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }

    /// Reserves `height` pixels at the bottom for the find bar.
    pub fn set_text_height(&mut self, height: f32) {
        self.text_height = height;
    }

    /// Reserves `height` pixels at the top for the tab strip.
    pub fn set_content_top(&mut self, height: f32) {
        self.content_top = height;
    }

    /// Reserves `width` pixels on the right for the function list.
    pub fn set_right_dock(&mut self, width: f32) {
        self.right_dock = width;
    }

    /// The y pixel where the text area starts, below the tab strip.
    pub fn content_top(&self) -> f32 {
        self.content_top
    }

    /// The height available to the text, between the tab strip and the find bar.
    pub fn text_height(&self) -> f32 {
        self.text_height.max(1.0)
    }

    /// Converts a viewport y to a window y.
    fn y(&self, viewport_y: f32) -> f32 {
        self.top() + viewport_y
    }

    /// The window y where this pane's content begins.
    fn top(&self) -> f32 {
        self.origin_y + self.content_top
    }

    /// The point inside the text area under a point in the window.
    ///
    /// The inverse of what the renderer does: every line is drawn at
    /// [`top`](Self::top) plus the viewport's own y, and every character at
    /// [`text_left`](Self::text_left) plus its column. A pointer position has to
    /// come back the same way before the editor can turn it into a line and a
    /// column, or the caret lands wherever raw window pixels happen to fall --
    /// with the menu bar, the toolbar and the tab strip all counted as text.
    pub fn text_point(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.text_left(), y - self.top())
    }

    /// Moves this pane's top-left corner in the window.
    pub fn set_origin(&mut self, x: f32, y: f32) {
        self.origin_x = x;
        self.origin_y = y;
    }

    /// This pane's top-left corner in the window.
    pub fn origin(&self) -> (f32, f32) {
        (self.origin_x, self.origin_y)
    }

    /// The pane's width, without the origin.
    pub fn pane_width(&self) -> f32 {
        self.width
    }

    /// Sets the search matches to highlight.
    pub fn set_matches(&mut self, matches: Vec<Range<usize>>, current: Option<Range<usize>>) {
        self.matches = matches;
        self.current_match = current;
    }

    /// Sets the occurrences to highlight for smart highlighting.
    pub fn set_smart(&mut self, ranges: Vec<Range<usize>>) {
        self.smart = ranges;
    }

    /// Forgets every mark that names a position in the document.
    ///
    /// The search matches, the current match, the smart occurrences and the
    /// bracket pair are all byte ranges into the text the view was drawing. A
    /// view about to draw another document has to drop them: kept, they paint
    /// the old file's words over the new one, at offsets that mean something
    /// else there.
    pub fn forget_marks(&mut self) {
        self.matches.clear();
        self.current_match = None;
        self.smart.clear();
        self.brace = None;
    }

    /// Sets the bracket pair to highlight, if the caret is next to one.
    pub fn set_brace(&mut self, brace: Option<(usize, usize)>) {
        self.brace = brace;
    }

    /// The theme in use.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// The height of one line, in pixels.
    pub fn line_height(&self) -> f32 {
        self.line_height
    }

    /// Steps the font size by `steps` notches, or restores the default when
    /// `steps` is 0.
    ///
    /// Every cached shape is thrown away: the metrics changed, so a cached
    /// buffer would draw at the wrong size.
    pub fn zoom(&mut self, font_system: &mut FontSystem, steps: i32) {
        const BASE: f32 = 15.0;
        const STEP: f32 = 1.0;
        let size = if steps == 0 {
            BASE
        } else {
            (self.metrics.font_size + steps as f32 * STEP).clamp(8.0, 40.0)
        };
        if (size - self.metrics.font_size).abs() < f32::EPSILON {
            return;
        }
        self.metrics = Metrics::new(size, (size * 4.0 / 3.0).round());
        self.line_height = self.metrics.line_height;
        self.char_width = measure_char_width(font_system, self.metrics, self.family);
        self.cache = LineCache::new(self.metrics, self.family, self.tab_width);
        self.numbers.clear();
    }

    /// Sets the font size in pixels, clamped to what the dialog offers.
    ///
    /// Unlike `zoom`, this is absolute: the preferences dialog shows a number,
    /// and a number has to mean the same thing every time it is applied.
    pub fn set_font_size(&mut self, font_system: &mut FontSystem, size: f32) {
        let size = size.clamp(crate::settings::MIN_FONT_SIZE, crate::settings::MAX_FONT_SIZE);
        if (size - self.metrics.font_size).abs() < f32::EPSILON {
            return;
        }
        self.metrics = Metrics::new(size, (size * 4.0 / 3.0).round());
        self.line_height = self.metrics.line_height;
        self.char_width = measure_char_width(font_system, self.metrics, self.family);
        self.cache = LineCache::new(self.metrics, self.family, self.tab_width);
        self.numbers.clear();
    }

    /// The font size in pixels.
    pub fn font_size(&self) -> f32 {
        self.metrics.font_size
    }

    /// How many columns a tab occupies.
    pub fn tab_width(&self) -> u16 {
        self.tab_width
    }

    /// Sets how many columns a tab occupies.
    ///
    /// Every cached shape has the old width baked into its glyph positions, so
    /// the cache is dropped rather than revalidated.
    pub fn set_tab_width(&mut self, width: u16) {
        let width = width.clamp(
            crate::settings::MIN_TAB_WIDTH,
            crate::settings::MAX_TAB_WIDTH,
        );
        if width == self.tab_width {
            return;
        }
        self.tab_width = width;
        self.cache = LineCache::new(self.metrics, self.family, width);
        self.numbers.clear();
    }

    /// The caret's position in window pixels, as `(x, y)`.
    ///
    /// Used to anchor the completion popup under the caret. Wrapping is handled
    /// the same way `push_caret` handles it, so the popup lands under the glyph
    /// the caret is actually drawn at.
    pub fn caret_point(&self, editor: &mut Editor) -> (f32, f32) {
        let line = editor.caret_line();
        let column = editor.caret_column();
        let wrap = editor.doc.wrap_cols();
        let Some(first) = editor.doc.row_of_line(line) else {
            return (self.text_left(), self.top());
        };
        let row = first + column.checked_div(wrap).unwrap_or(0);
        let col = if wrap == 0 { column } else { column % wrap };
        let x = self.text_left() + col as f32 * self.char_width - editor.viewport.scroll_x;
        (x, self.y(editor.viewport.line_y(row)))
    }

    /// The x pixel where text begins, right of the gutter and fold margin.
    pub fn text_left(&self) -> f32 {
        self.origin_x + self.gutter_width + self.bookmark_width + self.fold_width
    }

    /// The x pixel where the text column ends, left of the minimap and the
    /// function list.
    pub fn text_right(&self) -> f32 {
        self.origin_x + self.width - self.minimap.width() - self.right_dock
    }

    /// Whether long lines wrap.
    pub fn wrap(&self) -> bool {
        self.wrap
    }

    /// Turns wrapping on or off.
    /// Whether spaces and tabs are drawn as visible marks.
    pub fn show_whitespace(&self) -> bool {
        self.show_whitespace
    }

    /// Turns the whitespace marks on or off.
    pub fn set_show_whitespace(&mut self, on: bool) {
        self.show_whitespace = on;
    }

    /// Whether line ends are marked.
    pub fn show_eol(&self) -> bool {
        self.show_eol
    }

    /// Turns the end-of-line marks on or off.
    pub fn set_show_eol(&mut self, on: bool) {
        self.show_eol = on;
    }

    /// Whether indent guides are drawn.
    pub fn show_indent_guides(&self) -> bool {
        self.show_indent_guides
    }

    /// Turns the indent guides on or off.
    pub fn set_show_indent_guides(&mut self, on: bool) {
        self.show_indent_guides = on;
    }

    /// Whether control characters are boxed.
    pub fn show_non_printing(&self) -> bool {
        self.show_non_printing
    }

    /// Turns the non-printing marks on or off.
    pub fn set_show_non_printing(&mut self, on: bool) {
        self.show_non_printing = on;
    }

    /// Whether control characters and Unicode line separators are boxed.
    pub fn show_control_chars(&self) -> bool {
        self.show_control_chars
    }

    /// Turns the control-character marks on or off.
    pub fn set_show_control_chars(&mut self, on: bool) {
        self.show_control_chars = on;
    }

    /// Whether a wrapped line's break is marked.
    pub fn show_wrap_symbol(&self) -> bool {
        self.show_wrap_symbol
    }

    /// Turns the wrap symbol on or off.
    pub fn set_show_wrap_symbol(&mut self, on: bool) {
        self.show_wrap_symbol = on;
    }

    pub fn set_wrap(&mut self, wrap: bool) {
        self.wrap = wrap;
    }

    /// Swaps the palette.
    ///
    /// Every cached shape carries its colours baked in, so the cache is dropped
    /// rather than revalidated: a theme flip is a rare, deliberate act and one
    /// frame of reshaping is cheaper than a per-line colour comparison.
    pub fn set_theme(&mut self, theme: Theme) {
        // The whole theme, not just the background: an imported theme can share
        // a background with the built-in one and differ in every syntax colour,
        // and comparing the background alone would silently ignore it.
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.palette = Palette::new(theme);
        self.cache = LineCache::new(self.metrics, self.family, self.tab_width);
        self.numbers.clear();
        self.minimap_mut().invalidate();
    }

    /// Swaps the text colours without touching the chrome.
    ///
    /// Used when a document's language changes: the theme is the same, but a
    /// user-defined language brings its own colours for the keyword and
    /// delimiter slots. The cache is dropped for the same reason as in
    /// [`EditorView::set_theme`].
    pub fn set_palette(&mut self, palette: Palette) {
        if self.palette == palette {
            return;
        }
        self.palette = palette;
        self.cache = LineCache::new(self.metrics, self.family, self.tab_width);
        self.numbers.clear();
        self.minimap_mut().invalidate();
    }

    /// The palette in use.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The wrap width in columns for the current font and window.
    fn wrap_cols(&self) -> usize {
        if !self.wrap {
            return 0;
        }
        let width = (self.text_right() - self.text_left()).max(self.char_width);
        (width / self.char_width).floor().max(1.0) as usize
    }

    /// The minimap column as `(top, height)` in window pixels.
    ///
    /// The height follows the document rather than the pane, so the column
    /// cannot be measured without knowing how many rows there are to show.
    pub fn minimap_rect(&self, total: usize) -> (f32, f32) {
        (self.top(), Minimap::column_height(total, self.text_height()))
    }

    /// The minimap, for hit testing and toggling.
    pub fn minimap(&self) -> &Minimap {
        &self.minimap
    }

    /// The minimap, mutably.
    pub fn minimap_mut(&mut self) -> &mut Minimap {
        &mut self.minimap
    }

    /// The x pixel where the bookmark margin begins.
    pub fn bookmark_left(&self) -> f32 {
        self.origin_x + self.gutter_width
    }

    /// The x pixel where the fold margin begins.
    pub fn fold_left(&self) -> f32 {
        self.origin_x + self.gutter_width + self.bookmark_width
    }

    /// The width of the fold margin.
    pub fn fold_width(&self) -> f32 {
        self.fold_width
    }

    /// The document line under window pixel `y`.
    pub fn line_at(&self, editor: &mut Editor, y: f32) -> usize {
        let total = editor.doc.visible_line_count();
        let row = editor.viewport.y_to_line(y - self.top(), total);
        editor.doc.line_of_row(row)
    }

    /// Resizes the view and the editor's scroll window.
    pub fn resize(&mut self, editor: &mut Editor, width: f32, height: f32) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        self.update_gutter(editor.doc.len_lines());
        editor.viewport.set_size(
            self.text_right() - self.text_left(),
            self.text_height(),
        );
        editor
            .viewport
            .set_metrics(self.line_height, self.char_width);
    }

    /// Applies the wrap width the window now allows, keeping the top line.
    ///
    /// Rows are a function of the wrap width: a resize or a zoom renumbers every
    /// one of them, and a `first_line` that stayed put as a number would jump to
    /// another part of the document. The row is remapped through the line it was
    /// showing, so the text under the top edge does not move. Returns whether
    /// anything was reflowed.
    fn sync_wrap(&mut self, editor: &mut Editor) -> bool {
        let wrap = self.wrap_cols();
        if wrap == editor.doc.wrap_cols() {
            return false;
        }
        let top = editor.doc.line_of_row(editor.viewport.first_line);
        editor.doc.set_wrap(wrap);
        editor.viewport.first_line = editor.doc.row_of_line(top).unwrap_or(0);
        true
    }

    /// Keeps the caret on screen when the window changes shape under it.
    ///
    /// The rule is deliberately narrow. The anchor of the view is its top row,
    /// not the caret: the wheel scrolls the pane without moving the caret, so a
    /// resize that chased the caret would throw away the place the reader had
    /// scrolled to. The caret is pulled back only when it was on screen in the
    /// last frame and the window now has room for fewer rows -- a shrink, a zoom
    /// step, or a dock that took a slice of the pane -- or when wrapping was
    /// reflowed under it.
    fn keep_the_caret_in_view(&mut self, editor: &mut Editor, reflowed: bool) {
        let rows = editor.viewport.full_line_count();
        let top = editor.viewport.first_line;
        let was_visible = editor
            .caret_row()
            .is_some_and(|row| row >= top && row < top + self.last_rows);
        let shrank = rows < self.last_rows;
        self.last_rows = rows;
        if (shrank || reflowed) && was_visible {
            editor.ensure_caret_visible();
        }
    }

    /// Recomputes the gutter width for a document of `lines` lines.
    fn update_gutter(&mut self, lines: usize) {
        let digits = lines.max(1).to_string().len().max(3);
        self.gutter_width = digits as f32 * self.char_width + 2.0 * self.padding;
    }

    /// Shapes the visible lines, refreshes the gutter, and queues the rectangles.
    pub fn prepare(
        &mut self,
        editor: &mut Editor,
        font_system: &mut FontSystem,
        quads: &mut QuadRenderer,
    ) {
        self.update_gutter(editor.doc.len_lines());
        // The wrap width is a function of the font and the window, so it is
        // recomputed here rather than stored: a zoom or a resize changes it.
        let reflowed = self.sync_wrap(editor);
        editor.viewport.set_size(
            self.text_right() - self.text_left(),
            self.text_height(),
        );
        self.keep_the_caret_in_view(editor, reflowed);

        let total = editor.doc.visible_line_count();
        let visible = editor.viewport.visible_lines(total);
        self.rows.clear();
        for row in visible.clone() {
            let line = editor.doc.line_of_row(row);
            let segment = editor.doc.segment_of_row(row);
            self.rows.push((row, line, segment));
        }

        let caret_line = editor.caret_line();
        let wrap = editor.doc.wrap_cols();

        // Backgrounds, back to front.
        quads.push(
            self.origin_x,
            self.origin_y,
            self.width,
            self.height,
            self.theme.background.to_linear(),
        );
        quads.push(
            self.origin_x,
            self.top(),
            self.gutter_width,
            self.text_height(),
            self.theme.gutter_background.to_linear(),
        );
        quads.push(
            self.bookmark_left(),
            self.top(),
            self.bookmark_width,
            self.text_height(),
            self.theme.gutter_background.to_linear(),
        );
        quads.push(
            self.fold_left(),
            self.top(),
            self.fold_width,
            self.text_height(),
            self.theme.fold_background.to_linear(),
        );

        self.minimap.push_quads(
            &mut editor.doc,
            &editor.viewport,
            self.top(),
            self.text_height(),
            self.origin_x + self.width - self.right_dock,
            &self.palette,
            quads,
        );

        for &(row, line, _) in &self.rows {
            if line == caret_line {
                quads.push(
                    self.text_left(),
                    self.y(editor.viewport.line_y(row)),
                    self.text_right() - self.text_left(),
                    self.line_height,
                    self.theme.current_line.to_linear(),
                );
            }
        }

        self.push_bookmarks(editor, quads);
        self.push_braces(editor, quads);
        self.push_smart(editor, quads);
        self.push_style_marks(editor, quads);
        self.push_matches(editor, quads);
        self.push_selections(editor, quads);
        self.push_folds(editor, quads);
        self.push_marks(editor, quads);

        // One caret per selection, so a column block shows a caret on every line
        // it covers.
        let caret_color = self.theme.caret.to_linear();
        for sel in editor.selections.iter() {
            let line = editor.doc.char_to_line(sel.caret);
            let column = sel.caret - editor.doc.line_to_char(line);
            let Some(first) = editor.doc.row_of_line(line) else {
                continue;
            };
            let row = first + column.checked_div(wrap).unwrap_or(0);
            if !visible.contains(&row) {
                continue;
            }
            let col = if wrap == 0 { column } else { column % wrap };
            let x = self.text_left() + col as f32 * self.char_width - editor.viewport.scroll_x;
            quads.push(
                x,
                self.y(editor.viewport.line_y(row)),
                2.0,
                self.line_height,
                caret_color,
            );
        }

        // The cache is keyed by document line, so it is trimmed to the span the
        // visible rows cover rather than to the row range itself.
        let lo = self.rows.first().map_or(0, |r| r.1).saturating_sub(8);
        let hi = self.rows.last().map_or(0, |r| r.1) + 8;
        self.cache.retain(lo..hi);
        // Both halves matter: the id changes when the tab does, the revision
        // when the text does. Keying on the revision alone showed the previous
        // tab's text after a switch, because two files loaded from disk are
        // both at revision 0.
        let revalidate =
            editor.doc.id() != self.cache.id || editor.doc.revision() != self.cache.revision;
        self.cache.id = editor.doc.id();
        self.cache.revision = editor.doc.revision();
        // One window for every row of this frame: an unwrapped line is shaped a
        // screenful at a time, so a record of thirty thousand characters costs
        // what a line of code does.
        let window = line_window(
            editor.viewport.scroll_x,
            self.char_width,
            self.text_right() - self.text_left(),
        );
        for i in 0..self.rows.len() {
            let (_, line, segment) = self.rows[i];
            self.cache.ensure(
                &editor.doc,
                line,
                segment,
                wrap,
                window,
                font_system,
                &self.palette,
                revalidate,
            );
        }

        for i in 0..self.rows.len() {
            let (_, line, segment) = self.rows[i];
            if segment == 0 {
                self.ensure_number(line as u32 + 1, font_system);
            }
        }
        self.retain_numbers();
    }

    /// Queues a dot in the bookmark margin for every bookmarked visible line.
    ///
    /// The dot is a small filled square rather than a circle: the quad renderer
    /// draws rectangles, and at six pixels across the difference is invisible.
    fn push_bookmarks(&self, editor: &Editor, quads: &mut QuadRenderer) {
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let color = self.theme.bookmark.to_linear();
        let size = 6.0;
        let cx = self.bookmark_left() + self.bookmark_width * 0.5;

        for &(row, line, segment) in &self.rows {
            if segment != 0 || !doc.is_bookmarked(line) {
                continue;
            }
            let y = self.y(vp.line_y(row)) + (self.line_height - size) * 0.5;
            quads.push(cx - size * 0.5, y, size, size, color);
        }
    }

    /// Queues the `+`/`-` boxes of the fold margin.
    /// Queues the whitespace, end-of-line, and indent-guide marks.
    ///
    /// All three are quads rather than substituted glyphs. The shaped text is
    /// cached and the caret's x is `column * char_width`, so drawing a middle dot
    /// in place of a space would shift every column after it and put the caret in
    /// the wrong place. A quad costs eight floats and no reshaping.
    fn push_marks(&self, editor: &Editor, quads: &mut QuadRenderer) {
        if !(self.show_whitespace
            || self.show_eol
            || self.show_indent_guides
            || self.show_non_printing
            || self.show_control_chars
            || self.show_wrap_symbol)
        {
            return;
        }
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let wrap = doc.wrap_cols();
        let mark = self.theme.whitespace.to_linear();
        let guide = self.theme.indent_guide.to_linear();
        let cw = self.char_width;
        let tab = self.tab_width.max(1) as usize;
        let mid = self.line_height * 0.5;
        // Marks are placed column by column, so an unwrapped line is walked only
        // as far as the window that was shaped for it. Nothing is missed: the
        // columns past the window are off screen.
        let window = line_window(vp.scroll_x, cw, self.text_right() - self.text_left());
        for &(row, line, segment) in &self.rows {
            let (start, end) = segment_bounds(doc, line, segment, wrap);
            let (first, last) = if wrap == 0 {
                window_bounds(doc, start..end, window)
            } else {
                (start, end)
            };
            let clipped = last < end;
            let y = self.y(vp.line_y(row));
            let left = self.text_left() - vp.scroll_x;
            let mut col = first - start;
            let mut indent = 0usize;
            let mut seen_text = first > start;
            for c in doc.slice(first..last).chars() {
                match c {
                    ' ' => {
                        if self.show_whitespace {
                            quads.push(
                                left + col as f32 * cw + cw * 0.5 - 1.0,
                                y + mid - 1.0,
                                2.0,
                                2.0,
                                mark,
                            );
                        }
                        if !seen_text {
                            indent += 1;
                        }
                        col += 1;
                    }
                    '\t' => {
                        let next = (col / tab + 1) * tab;
                        if self.show_whitespace {
                            let width = (next - col) as f32 * cw;
                            quads.push(
                                left + col as f32 * cw + 2.0,
                                y + mid - 1.0,
                                (width - 4.0).max(2.0),
                                2.0,
                                mark,
                            );
                        }
                        if !seen_text {
                            indent = next;
                        }
                        col = next;
                    }
                    c if self.boxes(c) => {
                        // A box the height of the line's x-height, centred on the
                        // same baseline the whitespace dots use, so a run of
                        // control characters reads as a row of boxes rather than
                        // as a dashed line.
                        quads.push(
                            left + col as f32 * cw + 1.0,
                            y + mid - 4.0,
                            (cw - 2.0).max(2.0),
                            8.0,
                            mark,
                        );
                        seen_text = true;
                        col += 1;
                    }
                    _ => {
                        seen_text = true;
                        col += 1;
                    }
                }
            }
            if self.show_indent_guides && indent >= tab {
                let mut at = tab;
                while at <= indent {
                    quads.push(left + at as f32 * cw, y, 1.0, self.line_height, guide);
                    at += tab;
                }
            }
            // Only the last segment of a wrapped line carries the line's end.
            let segments = if wrap == 0 {
                1
            } else {
                doc.line_len(line).div_ceil(wrap).max(1) as u16
            };
            // A wrapped line's break gets a chevron in the right margin, which is
            // where Notepad++ puts it: the point is to tell a soft break from a
            // hard one without reading the gutter.
            if self.show_wrap_symbol && wrap != 0 && segment + 1 < segments {
                let x = left + wrap as f32 * cw - 6.0;
                quads.push(x, y + mid - 1.0, 4.0, 2.0, mark);
                quads.push(x + 2.0, y + mid - 3.0, 2.0, 2.0, mark);
                quads.push(x + 2.0, y + mid + 1.0, 2.0, 2.0, mark);
            }
            // The end-of-line mark sits one column past the last character, so it
            // only exists while the walk reached the line's end.
            if self.show_eol && segment + 1 == segments && !clipped {
                quads.push(
                    left + col as f32 * cw + 2.0,
                    y + mid - 3.0,
                    2.0,
                    6.0,
                    mark,
                );
            }
        }
    }

    /// Whether `c` gets a box under the current Show Symbol settings.
    ///
    /// Tab is excluded because it already has a bar of its own, and the two line
    /// breaks are excluded because they are the line's structure rather than its
    /// content. The Unicode separators are only boxed by the wider of the two
    /// settings, which is exactly the difference Notepad++ draws between them.
    fn boxes(&self, c: char) -> bool {
        if c == '\t' || c == '\n' || c == '\r' {
            return false;
        }
        if self.show_control_chars && matches!(c, '\u{2028}' | '\u{2029}' | '\u{85}') {
            return true;
        }
        self.show_non_printing && (c.is_control() || c == '\u{7f}')
    }

    fn push_folds(&self, editor: &Editor, quads: &mut QuadRenderer) {
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let color = self.theme.fold_marker.to_linear();
        let size = 9.0;
        let stroke = 1.0;
        let cx = self.fold_left() + self.fold_width * 0.5;

        for &(row, line, segment) in &self.rows {
            if segment != 0 || !doc.is_foldable(line) {
                continue;
            }
            let y = self.y(vp.line_y(row)) + (self.line_height - size) * 0.5;
            let left = cx - size * 0.5;
            let right = cx + size * 0.5 - stroke;
            let bottom = y + size - stroke;
            quads.push(left, y, size, stroke, color);
            quads.push(left, bottom, size, stroke, color);
            quads.push(left, y, stroke, size, color);
            quads.push(right, y, stroke, size, color);
            quads.push(cx - size * 0.25, y + size * 0.5 - stroke * 0.5, size * 0.5, stroke, color);
            if doc.is_folded(line) {
                quads.push(cx - stroke * 0.5, y + size * 0.25, stroke, size * 0.5, color);
            }
        }
    }

    /// Queues a rectangle behind each half of the matched bracket pair.
    ///
    /// Drawn before the search matches so a find highlight wins when the two
    /// overlap, which is what you want while stepping through results.
    fn push_braces(&self, editor: &Editor, quads: &mut QuadRenderer) {
        let Some((open, close)) = self.brace else {
            return;
        };
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let color = self.theme.match_current.to_linear();
        let wrap = doc.wrap_cols();
        for &(row, line, segment) in &self.rows {
            let (seg_start, seg_end) = segment_bounds(doc, line, segment, wrap);
            for pos in [open, close] {
                if pos < seg_start || pos >= seg_end {
                    continue;
                }
                let x = self.text_left() + (pos - seg_start) as f32 * self.char_width - vp.scroll_x;
                quads.push(x, self.y(vp.line_y(row)), self.char_width, self.line_height, color);
            }
        }
    }

    /// Queues one rectangle per visible line touched by a search match.
    /// Queues a rectangle behind every occurrence of the caret's word.
    ///
    /// Drawn before the search matches so a search highlight wins on the same
    /// character: the search is what the user asked for, the smart highlight is
    /// ambient.
    fn push_smart(&self, editor: &Editor, quads: &mut QuadRenderer) {
        if self.smart.is_empty() {
            return;
        }
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let color = self.theme.smart_highlight.to_linear();
        let wrap = doc.wrap_cols();
        for &(row, line, segment) in &self.rows {
            let (seg_start, seg_end) = segment_bounds(doc, line, segment, wrap);
            for m in &self.smart {
                let start = m.start.max(seg_start);
                let end = m.end.min(seg_end);
                if start >= end {
                    continue;
                }
                let x0 = self.text_left() + (start - seg_start) as f32 * self.char_width
                    - vp.scroll_x;
                let x1 = (self.text_left() + (end - seg_start) as f32 * self.char_width
                    - vp.scroll_x)
                    .min(self.text_right());
                quads.push(x0, self.y(vp.line_y(row)), x1 - x0, self.line_height, color);
            }
        }
    }

    /// Queues one rectangle per visible styled mark, in its style's colour.
    ///
    /// Drawn before the search matches so a match stays visible on top of a mark:
    /// a mark is a note the user made, a match is the answer to a question they
    /// just asked, and the answer should win.
    fn push_style_marks(&self, editor: &Editor, quads: &mut QuadRenderer) {
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let wrap = doc.wrap_cols();
        for (style, color) in self.theme.mark_styles.iter().enumerate() {
            let marks = editor.marks(style);
            if marks.is_empty() {
                continue;
            }
            let color = color.to_linear();
            for &(row, line, segment) in &self.rows {
                let (seg_start, seg_end) = segment_bounds(doc, line, segment, wrap);
                for mark in marks {
                    let start = mark.start.max(seg_start);
                    let end = mark.end.min(seg_end);
                    if start >= end {
                        continue;
                    }
                    let x0 = self.text_left() + (start - seg_start) as f32 * self.char_width
                        - vp.scroll_x;
                    let x1 = (self.text_left() + (end - seg_start) as f32 * self.char_width
                        - vp.scroll_x)
                        .min(self.text_right());
                    quads.push(x0, self.y(vp.line_y(row)), x1 - x0, self.line_height, color);
                }
            }
        }
    }

    fn push_matches(&self, editor: &Editor, quads: &mut QuadRenderer) {
        if self.matches.is_empty() {
            return;
        }
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let all = self.theme.match_highlight.to_linear();
        let current = self.theme.match_current.to_linear();

        let wrap = doc.wrap_cols();
        for &(row, line, segment) in &self.rows {
            let (seg_start, seg_end) = segment_bounds(doc, line, segment, wrap);
            for m in &self.matches {
                let start = m.start.max(seg_start);
                let end = m.end.min(seg_end);
                if start >= end {
                    continue;
                }
                let x0 = self.text_left() + (start - seg_start) as f32 * self.char_width
                    - vp.scroll_x;
                let x1 = (self.text_left() + (end - seg_start) as f32 * self.char_width
                    - vp.scroll_x)
                    .min(self.text_right());
                let color = if self.current_match.as_ref() == Some(m) {
                    current
                } else {
                    all
                };
                quads.push(x0, self.y(vp.line_y(row)), x1 - x0, self.line_height, color);
            }
        }
    }

    /// Queues one rectangle per visible line touched by a selection.
    fn push_selections(&self, editor: &Editor, quads: &mut QuadRenderer) {
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let color = self.theme.selection.to_linear();

        let wrap = doc.wrap_cols();
        for sel in editor.selections.iter() {
            if sel.is_empty() {
                continue;
            }
            for &(row, line, segment) in &self.rows {
                let line_range = doc.line_range(line);
                let (seg_start, seg_end) = segment_bounds(doc, line, segment, wrap);
                let start = sel.start().max(seg_start);
                let end = sel.end().min(seg_end);
                // A selection that runs past the end of the line covers its
                // terminator, which is drawn as a band to the right edge. Only
                // the last segment of the line can reach the terminator.
                let covers_break = sel.end() > line_range.end && seg_end == line_range.end;

                if start > end || (start == end && !covers_break) {
                    continue;
                }

                let x0 = self.text_left() + (start - seg_start) as f32 * self.char_width
                    - vp.scroll_x;
                let x1 = if covers_break {
                    self.text_right()
                } else {
                    self.text_left() + (end - seg_start) as f32 * self.char_width - vp.scroll_x
                };
                quads.push(x0, self.y(vp.line_y(row)), x1 - x0, self.line_height, color);
            }
        }
    }

    /// Shapes the line number `n` if it is not already cached.
    fn ensure_number(&mut self, n: u32, font_system: &mut FontSystem) {
        if self.numbers.contains_key(&n) {
            return;
        }
        let mut buffer = Buffer::new_empty(self.metrics);
        buffer.set_size(None, None);
        buffer.set_tab_width(self.tab_width);
        buffer.set_text(
            &n.to_string(),
            &Attrs::new().family(self.family),
            Shaping::Basic,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        self.numbers.insert(n, buffer);
    }

    /// Drops cached numbers that are no longer on screen.
    fn retain_numbers(&mut self) {
        let lo = self.rows.first().map_or(0, |r| r.1) as u32 + 1;
        let hi = self.rows.last().map_or(0, |r| r.1) as u32 + 1;
        self.numbers.retain(|n, _| *n >= lo && *n <= hi);
    }

    /// The text areas to hand to glyphon: line numbers and text, interleaved.
    pub fn text_areas<'a>(&'a self, editor: &Editor) -> Vec<TextArea<'a>> {
        let doc = &editor.doc;
        let vp = &editor.viewport;
        let caret_line = doc.char_to_line(editor.caret());
        let mut areas = Vec::with_capacity(self.rows.len() * 2);

        let gutter_bounds = TextBounds {
            left: self.origin_x as i32,
            top: self.top() as i32,
            right: (self.origin_x + self.gutter_width) as i32,
            bottom: (self.top() + self.text_height()) as i32,
        };
        // Clipped at `text_right`, not at the pane's edge: the minimap and the
        // function list are drawn as quads before the text, so a long line that
        // ran to the pane's edge would be painted over them.
        let text_bounds = TextBounds {
            left: self.text_left() as i32,
            top: self.top() as i32,
            right: self.text_right() as i32,
            bottom: (self.top() + self.text_height()) as i32,
        };

        for &(row, line, segment) in &self.rows {
            if segment != 0 {
                continue;
            }
            let n = line as u32 + 1;
            if let Some(buffer) = self.numbers.get(&n) {
                let digits = n.to_string().len() as f32;
                areas.push(TextArea {
                    buffer,
                    left: self.origin_x + self.gutter_width
                        - self.padding
                        - digits * self.char_width,
                    top: self.y(vp.line_y(row)),
                    scale: 1.0,
                    bounds: gutter_bounds,
                    default_color: if line == caret_line {
                        self.theme.gutter_active.to_cosmic()
                    } else {
                        self.theme.gutter_foreground.to_cosmic()
                    },
                    custom_glyphs: &[],
                });
            }
        }

        for &(row, line, segment) in &self.rows {
            let Some(cached) = self.cache.get(line, segment) else {
                continue;
            };
            areas.push(TextArea {
                buffer: &cached.buffer,
                // An unwrapped line is shaped from the column its window starts
                // at, so the buffer's first glyph belongs that far into the line.
                left: self.text_left() - vp.scroll_x + cached.window as f32 * self.char_width,
                top: self.y(vp.line_y(row)),
                scale: 1.0,
                bounds: text_bounds,
                default_color: self.theme.foreground.to_cosmic(),
                custom_glyphs: &[],
            });
        }

        areas
    }
}

/// Measures the advance width of the font an editor draws with.
///
/// Public so the measurement can be made on whichever thread built the font
/// system; see [`EditorView::with_char_width`].
pub fn measure_editor_char_width(font_system: &mut FontSystem) -> f32 {
    measure_char_width(font_system, Metrics::new(15.0, 20.0), Family::Monospace)
}

/// Measures the advance width of the monospace font.
fn measure_char_width(
    font_system: &mut FontSystem,
    metrics: Metrics,
    family: Family<'static>,
) -> f32 {
    const SAMPLE: &str = "0123456789";
    let mut buffer = Buffer::new_empty(metrics);
    buffer.set_size(None, None);
    buffer.set_text(SAMPLE, &Attrs::new().family(family), Shaping::Basic, None);
    buffer.shape_until_scroll(font_system, false);
    let width = buffer
        .layout_runs()
        .next()
        .map(|run| run.line_w)
        .unwrap_or(0.0);
    if width > 0.0 {
        width / SAMPLE.len() as f32
    } else {
        metrics.font_size * 0.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    /// A document of `lines` lines, each roughly `width` characters wide.
    ///
    /// The numbers are part of the text so a line can be recognised in a
    /// failure, and the padding makes lines long enough to wrap.
    fn document(lines: usize, width: usize) -> Document {
        let mut text = String::new();
        for line in 0..lines {
            text.push_str(&format!("{line:04} "));
            text.push_str(&"x".repeat(width.saturating_sub(5)));
            text.push('\n');
        }
        Document::from_text(&text)
    }

    /// A view sized like a window, as if one frame had already been drawn.
    fn view(editor: &mut Editor, width: f32, height: f32) -> EditorView {
        let mut view = EditorView::with_char_width(Theme::default(), 8.0);
        view.set_text_height(height);
        view.resize(editor, width, height);
        view.last_rows = editor.viewport.full_line_count();
        view
    }

    /// One resize, the way `prepare` sees it.
    fn resize(view: &mut EditorView, editor: &mut Editor, height: f32) -> bool {
        view.set_text_height(height);
        view.resize(editor, 900.0, height);
        let reflowed = view.sync_wrap(editor);
        view.keep_the_caret_in_view(editor, reflowed);
        reflowed
    }

    #[test]
    fn resizing_leaves_a_view_that_was_scrolled_away_from_the_caret_alone() {
        // The wheel scrolls without moving the caret, so this is the ordinary
        // state of a reader three hundred lines into a file whose caret is
        // still on line one. Resizing used to chase the caret and throw that
        // place away.
        let mut editor = Editor::new(document(500, 20));
        let mut view = view(&mut editor, 900.0, 420.0);
        assert_eq!(editor.viewport.full_line_count(), 21);
        editor.viewport.first_line = 100;
        editor.set_caret(0);
        assert_eq!(editor.caret_row(), Some(0));

        resize(&mut view, &mut editor, 700.0);
        assert_eq!(editor.viewport.first_line, 100, "growing moved the view");

        resize(&mut view, &mut editor, 200.0);
        assert_eq!(editor.viewport.first_line, 100, "shrinking moved the view");
    }

    #[test]
    fn a_window_that_shrinks_keeps_the_caret_it_was_showing() {
        let mut editor = Editor::new(document(500, 20));
        let mut view = view(&mut editor, 900.0, 420.0);
        editor.viewport.first_line = 100;
        editor.set_caret(editor.doc.line_to_char(110));
        assert_eq!(editor.caret_row(), Some(110));

        resize(&mut view, &mut editor, 200.0);

        let total = editor.doc.visible_line_count();
        let visible = editor.viewport.visible_lines(total);
        assert!(
            visible.contains(&110),
            "the caret is off screen at {visible:?}"
        );
    }

    #[test]
    fn a_window_that_grows_does_not_scroll_to_a_visible_caret() {
        let mut editor = Editor::new(document(500, 20));
        let mut view = view(&mut editor, 900.0, 420.0);
        editor.viewport.first_line = 100;
        editor.set_caret(editor.doc.line_to_char(105));

        resize(&mut view, &mut editor, 700.0);

        assert_eq!(editor.viewport.first_line, 100);
    }

    #[test]
    fn wrapping_keeps_the_line_under_the_top_edge() {
        // Rows are a function of the wrap width, so a resize renumbers every one
        // of them: without a remap the top row would show another line.
        let mut editor = Editor::new(document(300, 200));
        let mut view = view(&mut editor, 900.0, 420.0);
        editor.viewport.first_line = 40;
        assert_eq!(editor.doc.wrap_cols(), 0);
        assert_eq!(editor.doc.line_of_row(40), 40);

        view.set_wrap(true);
        let reflowed = view.sync_wrap(&mut editor);

        assert!(reflowed, "the wrap width did not change");
        assert!(editor.doc.wrap_cols() > 0);
        assert_eq!(
            editor.doc.line_of_row(editor.viewport.first_line),
            40,
            "the top of the view moved to another line"
        );
    }

    #[test]
    fn a_wider_window_that_unwraps_keeps_the_line_too() {
        let mut editor = Editor::new(document(300, 200));
        let mut view = view(&mut editor, 900.0, 420.0);
        view.set_wrap(true);
        view.sync_wrap(&mut editor);
        editor.viewport.first_line = editor.doc.row_of_line(40).unwrap();

        view.set_wrap(false);
        let reflowed = view.sync_wrap(&mut editor);

        assert!(reflowed);
        assert_eq!(editor.doc.wrap_cols(), 0);
        assert_eq!(editor.doc.line_of_row(editor.viewport.first_line), 40);
    }

    #[test]
    fn the_window_covers_every_column_that_fits() {
        // Whatever the horizontal scroll, the columns the pane shows must be
        // inside the window that gets shaped, or a line would draw blank at its
        // right edge.
        let (char_width, across) = (8.0, 800.0);
        for step in 0..200 {
            let scroll_x = step as f32 * 31.0;
            let (first, columns) = line_window(scroll_x, char_width, across);
            let left = (scroll_x / char_width) as usize;
            assert_eq!(first % WINDOW_MARGIN, 0, "window is not aligned");
            assert!(first <= left, "window starts past the left edge");
            assert!(
                first + columns >= left + 100,
                "window ends before the right edge at scroll {scroll_x}"
            );
        }
    }

    #[test]
    fn a_scroll_shorter_than_the_margin_keeps_the_window() {
        // The point of the margin: dragging a few columns must not reshape every
        // line in the pane.
        let first = line_window(0.0, 8.0, 800.0).0;
        for column in 0..WINDOW_MARGIN {
            let scroll_x = column as f32 * 8.0;
            assert_eq!(line_window(scroll_x, 8.0, 800.0).0, first);
        }
        assert!(line_window(WINDOW_MARGIN as f32 * 8.0, 8.0, 800.0).0 > first);
    }

    #[test]
    fn the_window_stops_at_the_end_of_the_line() {
        // A line shorter than its window is shaped whole; a longer one is cut
        // where the window ends.
        let line = Document::from_text(&format!("{}\n", "x".repeat(30_000)));
        assert_eq!(window_bounds(&line, 0..30_000, (640, 164)), (640, 804));

        let short = Document::from_text(&format!("{}\n", "x".repeat(700)));
        assert_eq!(window_bounds(&short, 0..700, (640, 164)), (640, 700));

        let tiny = Document::from_text(&format!("{}\n", "x".repeat(100)));
        assert_eq!(window_bounds(&tiny, 0..100, (640, 164)), (100, 100));
    }

    #[test]
    fn a_tabbed_line_is_shaped_from_its_start() {
        // A tab is not one column wide, so a window starting inside a tabbed
        // line would be drawn where the text is not. Those lines keep their start
        // and are cut at the window's right edge alone -- comparing rendered
        // frames caught this, 79,168 pixels of a tabbed file displaced by a
        // window that started at column 640.
        let plain = "x".repeat(2_000);
        let tabbed = Document::from_text(&format!("\t\t{plain}\n"));
        assert_eq!(window_bounds(&tabbed, 0..2_002, (640, 164)), (0, 804));

        let narrow = Document::from_text(&format!("{plain}x\n"));
        assert_eq!(window_bounds(&narrow, 0..2_001, (640, 164)), (640, 804));

        let wide = Document::from_text(&format!("{}\n", "é".repeat(2_000)));
        assert_eq!(window_bounds(&wide, 0..2_001, (640, 164)), (0, 804));
    }

    #[test]
    fn a_long_line_is_shaped_only_as_far_as_its_window() {
        // The cost of a frame used to grow with the longest line in the file:
        // shaping a record of thirty thousand characters took a whole second a
        // frame. Only the window is shaped now.
        let mut font_system = FontSystem::new();
        let palette = Palette::new(Theme::DARK);
        let mut cache = LineCache::new(Metrics::new(15.0, 20.0), Family::Monospace, 4);
        let doc = Document::from_text(&format!("{}\n", "x".repeat(30_000)));

        cache.ensure(&doc, 0, 0, 0, (640, 164), &mut font_system, &palette, false);

        let cached = cache.get(0, 0).expect("the line was not shaped");
        assert_eq!(cached.window, 640);
        assert_eq!(cached.text.chars().count(), 164);
    }
}
