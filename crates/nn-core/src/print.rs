//! Page layout for printing.
//!
//! The editor is monospace, so a page is a grid: a fixed number of columns
//! across and a fixed number of rows down, both derived from the paper size,
//! the margins and the character cell. That makes the whole thing arithmetic
//! rather than shaping, which is why it lives in `nn-core` and is tested with
//! string literals instead of a font.
//!
//! Three decisions are worth stating.
//!
//! Wrapping is by word, falling back to by character for a word that is longer
//! than the line. Notepad++ prints through Scintilla, whose print wrap mode
//! defaults to word wrap, and a page of code broken mid-identifier is harder to
//! read than one with a ragged right edge.
//!
//! A wrapped continuation carries no line number. The number belongs to the
//! logical line, and repeating it on every segment would suggest the file has
//! more lines than it does.
//!
//! A form feed starts a new page. It is the one control character a text file
//! can carry that means "page break", and honouring it is what makes printing a
//! file that already has page breaks come out the way its author intended.

use crate::datetime;

/// A paper size, in points (1/72 inch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    /// The key used in the settings file.
    pub key: &'static str,
    /// The name shown in the dialog.
    pub name: &'static str,
    /// Width in portrait orientation.
    pub width: f32,
    /// Height in portrait orientation.
    pub height: f32,
}

impl Paper {
    /// ISO A4, the default outside North America.
    pub const A4: Paper = Paper {
        key: "a4",
        name: "A4",
        width: 595.0,
        height: 842.0,
    };
    /// US Letter.
    pub const LETTER: Paper = Paper {
        key: "letter",
        name: "Letter",
        width: 612.0,
        height: 792.0,
    };
    /// US Legal.
    pub const LEGAL: Paper = Paper {
        key: "legal",
        name: "Legal",
        width: 612.0,
        height: 1008.0,
    };
    /// ISO A5.
    pub const A5: Paper = Paper {
        key: "a5",
        name: "A5",
        width: 420.0,
        height: 595.0,
    };
    /// US Tabloid, for the rare wide listing.
    pub const TABLOID: Paper = Paper {
        key: "tabloid",
        name: "Tabloid",
        width: 792.0,
        height: 1224.0,
    };

    /// Every size the dialog offers, smallest first.
    pub const ALL: [Paper; 5] = [
        Paper::A5,
        Paper::A4,
        Paper::LETTER,
        Paper::LEGAL,
        Paper::TABLOID,
    ];

    /// The size with this key, if any.
    pub fn from_key(key: &str) -> Option<Paper> {
        Paper::ALL.into_iter().find(|paper| paper.key == key)
    }

    /// The width and height in the orientation asked for.
    pub fn size(&self, landscape: bool) -> (f32, f32) {
        if landscape {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// The blank border around the text, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Margins {
    /// Two centimetres all round, which is what a word processor offers as
    /// "normal" and what Notepad++ uses.
    pub const NORMAL: Margins = Margins {
        left: 56.7,
        right: 56.7,
        top: 56.7,
        bottom: 56.7,
    };
}

/// How a document is laid out on paper.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSetup {
    pub paper: Paper,
    pub landscape: bool,
    pub margins: Margins,
    /// The size the text is drawn at, in points.
    pub font_size: f32,
    /// The height of one line, in points. Larger than `font_size` because a
    /// page of code with no leading is unreadable.
    pub line_height: f32,
    /// The header template, split into left, centre and right by tabs.
    pub header: String,
    /// The footer template, split the same way.
    pub footer: String,
    /// Whether to print a line-number gutter.
    pub line_numbers: bool,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            landscape: false,
            margins: Margins::NORMAL,
            font_size: 10.0,
            line_height: 12.0,
            header: String::new(),
            // The page number alone, which is what Notepad++ puts there.
            footer: "$(PAGE)".to_string(),
            line_numbers: false,
        }
    }
}

impl PageSetup {
    /// The text area, in points, after the margins.
    pub fn area(&self) -> (f32, f32) {
        let (width, height) = self.paper.size(self.landscape);
        (
            (width - self.margins.left - self.margins.right).max(1.0),
            (height - self.margins.top - self.margins.bottom).max(1.0),
        )
    }

    /// How many lines the header and footer take out of the text area.
    ///
    /// One line each when the template is not empty, none when it is. A blank
    /// header should not cost a line of text.
    fn furniture_rows(&self) -> usize {
        usize::from(!self.header.trim().is_empty()) + usize::from(!self.footer.trim().is_empty())
    }

    /// The number of text rows on a page.
    pub fn rows(&self) -> usize {
        let (_, height) = self.area();
        let usable = height - self.furniture_rows() as f32 * self.line_height;
        ((usable / self.line_height).floor() as usize).max(1)
    }

    /// The number of columns on a page, given a character cell width.
    ///
    /// The gutter is taken out first: a line number is as wide as the widest
    /// number plus two spaces, and the text has to fit in what is left.
    pub fn columns(&self, char_width: f32, digits: usize) -> usize {
        let (width, _) = self.area();
        let gutter = if self.line_numbers {
            (digits + 2) as f32 * char_width
        } else {
            0.0
        };
        (((width - gutter) / char_width).floor() as usize).max(1)
    }
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLine {
    /// The line number to print in the gutter, if this row starts a line.
    pub number: Option<usize>,
    /// The text of the row, already expanded and wrapped.
    pub text: String,
    /// Whether this row continues the line above rather than starting one.
    pub continued: bool,
}

/// One page.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub lines: Vec<PageLine>,
    /// Left, centre and right header zones, already substituted.
    pub header: [String; 3],
    /// Left, centre and right footer zones, already substituted.
    pub footer: [String; 3],
}

/// A laid-out document.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub pages: Vec<Page>,
    /// The columns each page holds, for the caller to size its preview.
    pub columns: usize,
    /// The text rows each page holds.
    pub rows: usize,
}

impl Layout {
    /// The number of pages.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether the layout is empty.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

/// What the header and footer templates can refer to.
#[derive(Debug, Clone, Default)]
pub struct Fields {
    /// The file name, or the tab's title for an unsaved buffer.
    pub file_name: String,
    /// The full path, empty for an unsaved buffer.
    pub path: String,
    /// The date, as `YYYY-MM-DD`.
    pub date: String,
    /// The time, as `HH:MM:SS`.
    pub time: String,
}

impl Fields {
    /// Fills the date and time from the clock.
    pub fn now() -> Self {
        let stamp = datetime::local_stamp(datetime::now_secs());
        let (date, time) = match stamp.split_once(' ') {
            Some((date, time)) => (date.to_string(), time.to_string()),
            None => (stamp.clone(), String::new()),
        };
        Self {
            date,
            time,
            ..Self::default()
        }
    }
}

/// Substitutes the `$(NAME)` tokens in a template.
///
/// An unknown token is left as it stands rather than blanked: a header reading
/// `$(PAGE_OF)` is a visible mistake, while one reading nothing at all looks
/// like the feature is broken.
pub fn substitute(template: &str, fields: &Fields, page: usize, pages: usize) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else {
            // No closing paren: the rest is literal text.
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        let value = match name {
            "PAGE" => Some(page.to_string()),
            "NB_PAGES" => Some(pages.to_string()),
            "FILE_NAME" => Some(fields.file_name.clone()),
            "FULL_CURRENT_PATH" => Some(fields.path.clone()),
            "CURRENT_DATE" => Some(fields.date.clone()),
            "CURRENT_TIME" => Some(fields.time.clone()),
            _ => None,
        };
        match value {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + 2 + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Splits a header or footer template into its three zones.
///
/// One tab separates left from centre, a second centre from right. A template
/// with no tab is all left, which is what makes `$(PAGE)` land in the corner
/// rather than the middle.
pub fn zones(template: &str, fields: &Fields, page: usize, pages: usize) -> [String; 3] {
    let mut parts = template.splitn(3, '\t');
    let left = parts.next().unwrap_or_default();
    let centre = parts.next().unwrap_or_default();
    let right = parts.next().unwrap_or_default();
    [
        substitute(left, fields, page, pages),
        substitute(centre, fields, page, pages),
        substitute(right, fields, page, pages),
    ]
}

/// Expands tabs to the next multiple of `tab_width`.
///
/// A tab stop is counted in characters, not in points, because the font is
/// monospace: the two are the same thing up to a constant.
pub fn expand_tabs(text: &str, tab_width: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let width = tab_width.max(1);
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for ch in text.chars() {
        if ch == '\t' {
            let next = (column / width + 1) * width;
            for _ in column..next {
                out.push(' ');
            }
            column = next;
        } else {
            out.push(ch);
            column += 1;
        }
    }
    out
}

/// Breaks one logical line into rows of at most `columns` characters.
///
/// Word wrap, with a character break for a word that cannot fit on a line of
/// its own. The returned rows never include the trailing space that the break
/// consumed, so a wrapped line does not end in whitespace.
pub fn wrap_line(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    if text.chars().count() <= columns {
        return vec![text.to_string()];
    }
    let mut rows = Vec::new();
    let mut rest: Vec<char> = text.chars().collect();
    while rest.len() > columns {
        // The last space at or before the limit is the break, if there is one.
        let window = &rest[..columns];
        let break_at = window.iter().rposition(|ch| *ch == ' ');
        match break_at {
            Some(index) if index > 0 => {
                rows.push(rest[..index].iter().collect());
                // Skip the space itself and any run of them, so the next row
                // does not start with the indentation of the previous one.
                let mut next = index;
                while next < rest.len() && rest[next] == ' ' {
                    next += 1;
                }
                rest = rest[next..].to_vec();
            }
            _ => {
                rows.push(rest[..columns].iter().collect());
                rest = rest[columns..].to_vec();
            }
        }
    }
    rows.push(rest.iter().collect());
    rows
}

/// Lays a document out on pages.
///
/// `char_width` is the width of one character cell in points, which the caller
/// measures from the font it will draw with. The layout and the drawing
/// therefore agree about how many characters fit.
pub fn layout(text: &str, setup: &PageSetup, char_width: f32, fields: &Fields) -> Layout {
    let char_width = char_width.max(0.1);
    let logical: Vec<&str> = split_lines(text);
    let digits = logical.len().max(1).to_string().len();
    let columns = setup.columns(char_width, digits);
    let rows = setup.rows();

    // Every row of the document, before it is cut into pages. `None` is a
    // forced break: a blank line and a form feed both produce a row with no
    // text, so the break cannot be recognised from the row itself.
    let mut body: Vec<Option<PageLine>> = Vec::new();
    for (index, line) in logical.iter().enumerate() {
        if *line == "\u{c}" {
            body.push(None);
            continue;
        }
        let expanded = expand_tabs(line, 4);
        for (segment, row) in wrap_line(&expanded, columns).into_iter().enumerate() {
            body.push(Some(PageLine {
                number: if setup.line_numbers && segment == 0 {
                    Some(index + 1)
                } else {
                    None
                },
                text: row,
                continued: segment > 0,
            }));
        }
    }

    // Cut into pages.
    let mut pages: Vec<Page> = Vec::new();
    let mut current: Vec<PageLine> = Vec::new();
    for row in body {
        match row {
            None => {
                pages.push(Page {
                    lines: std::mem::take(&mut current),
                    header: blank_zones(),
                    footer: blank_zones(),
                });
            }
            Some(line) => {
                current.push(line);
                if current.len() == rows {
                    pages.push(Page {
                        lines: std::mem::take(&mut current),
                        header: blank_zones(),
                        footer: blank_zones(),
                    });
                }
            }
        }
    }
    if !current.is_empty() || pages.is_empty() {
        pages.push(Page {
            lines: current,
            header: blank_zones(),
            footer: blank_zones(),
        });
    }

    // The header and footer need the page count, so they are filled in once the
    // pages exist.
    let total = pages.len();
    for (index, page) in pages.iter_mut().enumerate() {
        page.header = zones(&setup.header, fields, index + 1, total);
        page.footer = zones(&setup.footer, fields, index + 1, total);
    }

    Layout {
        pages,
        columns,
        rows,
    }
}

/// Three empty zones, for a page whose furniture has not been filled in yet.
fn blank_zones() -> [String; 3] {
    [
        String::new(),
        String::new(),
        String::new(),
    ]
}

/// Splits text into logical lines, keeping a form feed as its own line.
///
/// `str::lines` would fold a form feed into the line it sits on, and the layout
/// needs to see it alone to know where the page ends.
fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                out.push(&text[start..index]);
                index += 1;
                start = index;
            }
            b'\r' => {
                out.push(&text[start..index]);
                index += 1;
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                start = index;
            }
            b'\x0c' => {
                if start < index {
                    out.push(&text[start..index]);
                }
                out.push("\u{c}");
                index += 1;
                // The newline that usually follows a form feed belongs to the
                // break, not to an empty line after it.
                if index < bytes.len() && bytes[index] == b'\r' {
                    index += 1;
                }
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> PageSetup {
        PageSetup::default()
    }

    fn fields() -> Fields {
        Fields {
            file_name: "notes.txt".to_string(),
            path: "/tmp/notes.txt".to_string(),
            date: "2024-05-06".to_string(),
            time: "07:08:09".to_string(),
        }
    }

    #[test]
    fn a4_is_the_default_and_is_taller_than_it_is_wide() {
        let setup = setup();
        assert_eq!(setup.paper, Paper::A4);
        let (width, height) = setup.paper.size(false);
        assert!(height > width);
    }

    #[test]
    fn landscape_swaps_the_sides() {
        let (width, height) = Paper::A4.size(true);
        assert!(width > height);
        assert_eq!(width, Paper::A4.height);
    }

    #[test]
    fn every_paper_key_reads_back() {
        for paper in Paper::ALL {
            assert_eq!(Paper::from_key(paper.key), Some(paper));
        }
        assert_eq!(Paper::from_key("a3"), None);
    }

    #[test]
    fn the_text_area_is_the_paper_less_the_margins() {
        let setup = setup();
        let (width, height) = setup.area();
        assert!((width - (595.0 - 56.7 * 2.0)).abs() < 0.01);
        assert!((height - (842.0 - 56.7 * 2.0)).abs() < 0.01);
    }

    #[test]
    fn a_blank_header_costs_no_rows() {
        let mut setup = setup();
        setup.header = String::new();
        setup.footer = String::new();
        let bare = setup.rows();
        setup.header = "title".to_string();
        assert_eq!(setup.rows(), bare - 1);
        setup.footer = "$(PAGE)".to_string();
        assert_eq!(setup.rows(), bare - 2);
    }

    #[test]
    fn a_whitespace_only_header_costs_no_rows() {
        let mut setup = setup();
        setup.header = "   ".to_string();
        setup.footer = String::new();
        let bare = setup.rows();
        setup.header = "  \t ".to_string();
        assert_eq!(setup.rows(), bare);
    }

    #[test]
    fn the_gutter_takes_columns_from_the_text() {
        let mut setup = setup();
        setup.line_numbers = false;
        let wide = setup.columns(6.0, 4);
        setup.line_numbers = true;
        let narrow = setup.columns(6.0, 4);
        // Six characters of gutter: four digits and two spaces.
        assert_eq!(wide - narrow, 6);
    }

    #[test]
    fn a_page_always_has_at_least_one_row_and_column() {
        let mut setup = setup();
        setup.margins = Margins {
            left: 500.0,
            right: 500.0,
            top: 800.0,
            bottom: 800.0,
        };
        assert_eq!(setup.rows(), 1);
        assert_eq!(setup.columns(10.0, 1), 1);
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(expand_tabs("\tx", 4), "    x");
        assert_eq!(expand_tabs("ab\tx", 4), "ab  x");
        assert_eq!(expand_tabs("abcd\tx", 4), "abcd    x");
        assert_eq!(expand_tabs("no tabs", 4), "no tabs");
    }

    #[test]
    fn a_short_line_is_one_row() {
        assert_eq!(wrap_line("hello", 10), vec!["hello"]);
    }

    #[test]
    fn a_long_line_wraps_at_a_space() {
        assert_eq!(wrap_line("hello world again", 12), vec!["hello world", "again"]);
    }

    #[test]
    fn a_wrapped_row_does_not_start_with_the_space_it_broke_at() {
        assert_eq!(wrap_line("aaaa bbbb", 5), vec!["aaaa", "bbbb"]);
    }

    #[test]
    fn a_word_longer_than_the_line_breaks_by_character() {
        assert_eq!(wrap_line("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn a_line_of_exactly_the_width_does_not_wrap() {
        assert_eq!(wrap_line("abcd", 4), vec!["abcd"]);
    }

    #[test]
    fn a_page_number_substitutes() {
        let text = substitute("page $(PAGE) of $(NB_PAGES)", &fields(), 2, 7);
        assert_eq!(text, "page 2 of 7");
    }

    #[test]
    fn the_file_tokens_substitute() {
        let text = substitute("$(FILE_NAME) in $(FULL_CURRENT_PATH)", &fields(), 1, 1);
        assert_eq!(text, "notes.txt in /tmp/notes.txt");
    }

    #[test]
    fn the_date_and_time_substitute() {
        let text = substitute("$(CURRENT_DATE) $(CURRENT_TIME)", &fields(), 1, 1);
        assert_eq!(text, "2024-05-06 07:08:09");
    }

    #[test]
    fn an_unknown_token_is_left_alone() {
        assert_eq!(substitute("$(NOPE)", &fields(), 1, 1), "$(NOPE)");
    }

    #[test]
    fn an_unclosed_token_is_literal_text() {
        assert_eq!(substitute("$(PAGE", &fields(), 1, 1), "$(PAGE");
    }

    #[test]
    fn a_template_with_no_tab_is_all_left() {
        let zones = zones("$(PAGE)", &fields(), 3, 9);
        assert_eq!(zones, ["3".to_string(), String::new(), String::new()]);
    }

    #[test]
    fn two_tabs_make_three_zones() {
        let zones = zones("l\tc\tr", &fields(), 1, 1);
        assert_eq!(zones, ["l".to_string(), "c".to_string(), "r".to_string()]);
    }

    #[test]
    fn one_tab_leaves_the_right_zone_empty() {
        let zones = zones("l\tc", &fields(), 1, 1);
        assert_eq!(zones, ["l".to_string(), "c".to_string(), String::new()]);
    }

    #[test]
    fn a_short_document_is_one_page() {
        let layout = layout("one\ntwo\nthree", &setup(), 6.0, &fields());
        assert_eq!(layout.len(), 1);
        assert_eq!(layout.pages[0].lines.len(), 3);
        assert_eq!(layout.pages[0].lines[0].text, "one");
    }

    #[test]
    fn an_empty_document_is_one_empty_page() {
        // A print of an empty buffer should still produce a sheet, the way a
        // word processor does, rather than nothing at all.
        let layout = layout("", &setup(), 6.0, &fields());
        assert_eq!(layout.len(), 1);
        assert!(layout.pages[0].lines.is_empty());
    }

    #[test]
    fn a_document_longer_than_a_page_spills_onto_the_next() {
        let mut setup = setup();
        setup.header = String::new();
        setup.footer = String::new();
        let rows = setup.rows();
        let text = (0..rows + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let layout = layout(&text, &setup, 6.0, &fields());
        assert_eq!(layout.len(), 2);
        assert_eq!(layout.pages[0].lines.len(), rows);
        assert_eq!(layout.pages[1].lines.len(), 5);
    }

    #[test]
    fn the_footer_carries_the_page_number_of_each_page() {
        let mut setup = setup();
        setup.footer = "$(PAGE)/$(NB_PAGES)".to_string();
        let rows = setup.rows();
        let text = (0..rows * 2).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        let layout = layout(&text, &setup, 6.0, &fields());
        assert_eq!(layout.len(), 2);
        assert_eq!(layout.pages[0].footer[0], "1/2");
        assert_eq!(layout.pages[1].footer[0], "2/2");
    }

    #[test]
    fn line_numbers_appear_only_on_the_first_row_of_a_line() {
        let mut setup = setup();
        setup.line_numbers = true;
        setup.header = String::new();
        setup.footer = String::new();
        let columns = setup.columns(6.0, 1);
        let long = "x".repeat(columns * 2);
        let layout = layout(&format!("first\n{long}"), &setup, 6.0, &fields());
        let lines = &layout.pages[0].lines;
        assert_eq!(lines[0].number, Some(1));
        assert_eq!(lines[1].number, Some(2));
        assert_eq!(lines[2].number, None);
        assert!(lines[2].continued);
    }

    #[test]
    fn line_numbers_are_off_by_default() {
        let layout = layout("one\ntwo", &setup(), 6.0, &fields());
        assert!(layout.pages[0].lines.iter().all(|line| line.number.is_none()));
    }

    #[test]
    fn a_form_feed_starts_a_new_page() {
        let layout = layout("one\n\u{c}\ntwo", &setup(), 6.0, &fields());
        assert_eq!(layout.len(), 2);
        assert_eq!(layout.pages[0].lines[0].text, "one");
        assert_eq!(layout.pages[1].lines[0].text, "two");
    }

    #[test]
    fn a_crlf_file_lays_out_the_same_as_an_lf_one() {
        let lf = layout("one\ntwo", &setup(), 6.0, &fields());
        let crlf = layout("one\r\ntwo", &setup(), 6.0, &fields());
        assert_eq!(lf.pages[0].lines, crlf.pages[0].lines);
    }

    #[test]
    fn a_trailing_newline_does_not_add_a_blank_page() {
        let layout = layout("one\n", &setup(), 6.0, &fields());
        assert_eq!(layout.len(), 1);
        assert_eq!(layout.pages[0].lines.len(), 1);
    }

    #[test]
    fn the_layout_reports_the_grid_it_used() {
        let mut setup = setup();
        setup.line_numbers = true;
        let layout = layout("one", &setup, 6.0, &fields());
        assert_eq!(layout.rows, setup.rows());
        assert_eq!(layout.columns, setup.columns(6.0, 1));
    }

    #[test]
    fn a_tab_is_expanded_before_wrapping() {
        let mut setup = setup();
        setup.header = String::new();
        setup.footer = String::new();
        let layout = layout("\tx", &setup, 6.0, &fields());
        assert_eq!(layout.pages[0].lines[0].text, "    x");
    }
}
