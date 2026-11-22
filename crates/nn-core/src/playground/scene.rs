//! The page the kitten walks on.
//!
//! Playground mode is a document, not a diorama. The window is filled with a page
//! of code at the editor's own line height -- a gutter of line numbers, then lines
//! of the chosen flavour -- and the kitten is two and a quarter lines tall, walking
//! across it. Everything that is not the kitten is made of characters: a table, a
//! plant, a lamp and a shelf are ASCII drawn into the page, the bottom rows are a
//! rug of punctuation, and the tokens the kitten bats are words lifted off the
//! text. There is nothing to install and nothing to render as a surface, which is
//! what keeps the mode a page rather than a place.
//!
//! Coordinates: one unit is one line of text. `x` runs right from the left edge,
//! `y` runs up the page from the bottom edge, and `z` comes out of the page towards
//! the reader, which is where the kitten's boxes and a lifted token live. Row 0 is
//! the top of the page, so a cell at row `r` has its centre at
//! `y = rows - r - 0.5`.

use super::corpus::Flavour;
use super::{Card, CardKind, Rng};

/// How wide one glyph card is, in line heights.
///
/// Monospace cells are about 0.55 of the line height in every font this editor
/// ships with; the app rasterises the atlas with the same ratio, so a card and a
/// cell are the same shape.
pub const GLYPH_WIDTH: f32 = 0.55;

/// The columns the line numbers take, including the gap after them.
const GUTTER: u32 = 4;
/// How many rows at the bottom of the page are rug rather than code.
const RUG_ROWS: u32 = 5;
/// The rows a piece of furniture needs under it, so nothing stands on the rug.
const FLOOR_MARGIN: u32 = RUG_ROWS + 1;
/// How many rows at the bottom are left to the app's own label.
///
/// The label is drawn by the text renderer in screen space, so the page keeps the
/// last line clear: a hint printed over a line of code is a hint nobody reads.
const LABEL_ROWS: u32 = 1;

/// How far a token's centre may be from the page's edge, in cells.
pub const TOKEN_RADIUS: f32 = 0.4;
/// The longest word the kitten will bat: wider than that and it is a line, not a
/// token.
const UNDER_PAW: usize = 8;
/// The fastest a batted token may travel, in cells per second.
const MAX_SPEED: f32 = 14.0;
/// The fastest a batted token may spin.
const MAX_SPIN: f32 = 16.0;
/// How fast a token loses its speed to the page, per second.
///
/// The page has no gravity: a word the kitten bats slides away, turns, and
/// stops where the friction leaves it. A page that pulled its own letters down
/// would be a page with a floor, and this one is a sheet of paper seen head on.
const DRAG: f32 = 1.6;

/// One loose token: a word the kitten can knock about.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The characters of the token, in order.
    pub text: Vec<char>,
    /// One style per character, from the flavour's lexer.
    pub styles: Vec<u8>,
    /// Centre, in cells: `x` right, `y` up the page, `z` out of it.
    pub pos: [f32; 3],
    /// Velocity along the page, in cells per second.
    pub vel: [f32; 2],
    /// Turn in the page's own plane, in radians.
    pub roll: f32,
    /// Tipped off the page, in radians.
    pub pitch: f32,
    /// Spin, in radians per second.
    pub spin: f32,
    /// True once it has stopped moving.
    pub rest: bool,
}

impl Token {
    /// The token's own width, in cells.
    pub fn width(&self) -> f32 {
        self.text.len() as f32 * GLYPH_WIDTH
    }

    /// The cards that draw it.
    pub fn cards(&self, alpha: f32, out: &mut Vec<Card>) {
        let width = self.width();
        let (sin, cos) = self.roll.sin_cos();
        for (index, ch) in self.text.iter().copied().enumerate() {
            // Laid out along the token's own turn, so a word that has been
            // knocked sideways still spells itself.
            let along = index as f32 * GLYPH_WIDTH - width * 0.5 + GLYPH_WIDTH * 0.5;
            out.push(Card {
                ch,
                style: self.styles.get(index).copied().unwrap_or(0),
                pos: [
                    self.pos[0] + along * cos,
                    self.pos[1] + along * sin,
                    self.pos[2],
                ],
                yaw: 0.0,
                pitch: self.pitch,
                roll: self.roll,
                size: [GLYPH_WIDTH, 1.0],
                alpha,
                kind: CardKind::Glyph,
            });
        }
    }
}

/// A speck of dust, drifting in front of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mote {
    pub pos: [f32; 3],
    /// How far it wanders from where it started.
    pub drift: f32,
    /// Where it is in its wander, so no two move together.
    pub phase: f32,
    /// Its size, in cells.
    pub size: f32,
}

/// What one page is made of.
#[derive(Debug, Clone)]
pub struct Page {
    pub flavour: Flavour,
    pub seed: u64,
    /// Which regeneration this is, counted from the start of the session.
    pub index: u32,
    /// Cells across and down.
    pub columns: u32,
    pub rows: u32,
    /// The document: one card per character.
    pub text: Vec<Card>,
    /// Loose tokens.
    pub tokens: Vec<Token>,
    /// Dust.
    pub motes: Vec<Mote>,
}

impl Page {
    /// Lays out the page for one regeneration.
    pub fn new(flavour: Flavour, seed: u64, index: u32, columns: u32, rows: u32) -> Self {
        let columns = columns.max(24);
        let rows = rows.max(8);
        let mut rng = Rng::new(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let lines = flavour.lines();

        // The last line belongs to the app's label, so the page stops above it.
        let body = rows.saturating_sub(LABEL_ROWS).max(8);

        let mut text = Vec::with_capacity((columns * rows) as usize);
        let props = furniture(&mut rng, columns, body);
        let mut taken = vec![false; (columns * rows) as usize];
        // A prop owns its whole box, not just the cells it draws ink in: a table
        // with code showing through it is a table drawn on a page, not furniture
        // standing on one.
        for prop in &props {
            for down in 0..prop.down {
                for across in 0..prop.across {
                    let (row, column) = (prop.row + down, prop.column + across);
                    if row < rows && column < columns {
                        taken[(row * columns + column) as usize] = true;
                    }
                }
            }
        }

        // The gutter: a line number per row, dim, right-aligned, the way the
        // editor draws it.
        for row in 0..body {
            let number = format!("{:>3} ", row + 1);
            let chars: Vec<char> = number.chars().take(GUTTER as usize).collect();
            for (index, ch) in chars.into_iter().enumerate() {
                text.push(cell(row, index as u32, rows, ch, 4, 0.55));
            }
        }

        // The code: every row above the rug, taken from the flavour's material in
        // a seeded order so two pages are two files.
        let mut cursor = rng.below(lines.len());
        for row in 0..body.saturating_sub(RUG_ROWS) {
            let line = lines[cursor % lines.len()];
            cursor = cursor.wrapping_add(1 + rng.below(2));
            let styles = flavour.styles(line);
            let chars: Vec<char> = line.chars().collect();
            // Occasionally indented, so the page is not a wall of flush-left text.
            let indent = if line.starts_with(' ') { 0 } else { rng.below(6) as u32 };
            for (index, ch) in chars.iter().copied().enumerate() {
                if ch == ' ' || ch == '\t' {
                    continue;
                }
                let column = GUTTER + indent + index as u32;
                if column >= columns {
                    break;
                }
                if taken[(row * columns + column) as usize] {
                    continue;
                }
                text.push(cell(
                    row,
                    column,
                    rows,
                    ch,
                    styles.get(index).copied().unwrap_or(0),
                    1.0,
                ));
            }
        }

        // The rug: punctuation lying about at the bottom of the page, the way a
        // floor is scattered with the small things a room is used for.
        let marks = ['.', ',', ';', '{', '}', '(', ')', ':', '=', '*', '+', '-', '~', '\''];
        for row in body.saturating_sub(RUG_ROWS)..body {
            for column in 0..columns {
                if taken[(row * columns + column) as usize] || rng.below(7) != 0 {
                    continue;
                }
                let mut card = cell(row, column, rows, marks[rng.below(marks.len())], 7, 0.3);
                card.roll = rng.range(-0.5, 0.5);
                card.size = [GLYPH_WIDTH * 0.9, 0.9];
                text.push(card);
            }
        }

        // The furniture, drawn in its own cleared box: a document with a table and
        // a plant standing on it.
        for prop in &props {
            for (down, across, ch) in &prop.cells {
                let (row, column) = (prop.row + down, prop.column + across);
                if row >= rows || column >= columns {
                    continue;
                }
                text.push(cell(row, column, rows, *ch, 2, 0.95));
            }
        }

        let tokens = loose_tokens(&mut rng, flavour, columns, rows);
        let motes = (0..40)
            .map(|index| Mote {
                pos: [
                    rng.range(0.0, columns as f32 * GLYPH_WIDTH),
                    rng.range(1.0, rows as f32),
                    rng.range(0.1, 1.6),
                ],
                drift: rng.range(0.3, 1.4),
                phase: index as f32 * 0.37 + rng.range(0.0, std::f32::consts::TAU),
                size: rng.range(0.03, 0.08),
            })
            .collect();

        Self {
            flavour,
            seed,
            index,
            columns,
            rows,
            text,
            tokens,
            motes,
        }
    }

    /// The width of the page, in line heights.
    pub fn width(&self) -> f32 {
        self.columns as f32 * GLYPH_WIDTH
    }

    /// The corner label: which flavour, which page.
    pub fn label(&self) -> String {
        format!("{} · page {}", self.flavour.name(), self.index + 1)
    }

    /// Moves every loose token by `dt` seconds.
    ///
    /// Fixed steps are the caller's job: one step of `1/120` and one of `1/60` are
    /// not the same integration, and a jump of half a second -- which a loaded
    /// machine will produce -- would send a token off the page.
    pub fn step_tokens(&mut self, dt: f32) {
        let width = self.width();
        let top = self.rows as f32;
        for token in &mut self.tokens {
            if token.rest {
                continue;
            }
            token.pos[0] += token.vel[0] * dt;
            token.pos[1] += token.vel[1] * dt;
            token.roll += token.spin * dt;
            // Paper drag, not a floor: the word keeps going until it does not.
            let keep = (1.0 - DRAG * dt).max(0.0);
            token.vel[0] *= keep;
            token.vel[1] *= keep;
            token.spin *= (1.0 - 2.4 * dt).max(0.0);
            // A sliding token settles flat as it slows: a card that stops standing
            // on its edge looks broken.
            token.pitch += (0.0 - token.pitch) * 3.0 * dt;
            if token.vel[0].hypot(token.vel[1]) < 0.06 && token.spin.abs() < 0.3 {
                token.vel = [0.0, 0.0];
                token.spin = 0.0;
                token.pitch = 0.0;
                token.rest = true;
                token.roll %= std::f32::consts::TAU;
            }
            // The edge of the page is the edge of the world.
            let half = token.width() * 0.5;
            if token.pos[0] < half {
                token.pos[0] = half;
                token.vel[0] = token.vel[0].abs() * 0.4;
            }
            if token.pos[0] > width - half {
                token.pos[0] = width - half;
                token.vel[0] = -token.vel[0].abs() * 0.4;
            }
            let low = TOKEN_RADIUS;
            if token.pos[1] < low {
                token.pos[1] = low;
                token.vel[1] = token.vel[1].abs() * 0.4;
            }
            if token.pos[1] > top - low {
                token.pos[1] = top - low;
                token.vel[1] = -token.vel[1].abs() * 0.4;
            }
        }
    }

    /// Pushes a token near `at` away from the kitten.
    ///
    /// The paw is not an engine: a token the kitten keeps its paw on for a dozen
    /// frames takes one shove, not a dozen, which is why the push is damped by
    /// paper drag rather than summed.
    pub fn bat(&mut self, at: [f32; 3], reach: f32, power: f32) {
        for token in &mut self.tokens {
            let dx = token.pos[0] - at[0];
            let dy = token.pos[1] - at[1];
            let distance = dx.hypot(dy);
            if distance > reach {
                continue;
            }
            let scale = power / distance.max(0.4);
            token.vel[0] = (token.vel[0] + dx * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.vel[1] = (token.vel[1] + dy * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.spin = (token.spin + power * 2.2).clamp(-MAX_SPIN, MAX_SPIN);
            token.rest = false;
        }
    }
}

/// One character of the page, placed on the grid.
fn cell(row: u32, column: u32, rows: u32, ch: char, style: u8, alpha: f32) -> Card {
    Card {
        ch,
        style,
        pos: [
            (column as f32 + 0.5) * GLYPH_WIDTH,
            rows as f32 - row as f32 - 0.5,
            0.0,
        ],
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        size: [GLYPH_WIDTH, 1.0],
        alpha,
        kind: CardKind::Glyph,
    }
}

/// One piece of furniture: where its box starts, how big it is, and the
/// characters in it.
struct Prop {
    row: u32,
    column: u32,
    down: u32,
    across: u32,
    cells: Vec<(u32, u32, char)>,
}

/// Two to four pieces of furniture, standing in the lower half of the page.
fn furniture(rng: &mut Rng, columns: u32, rows: u32) -> Vec<Prop> {
    let designs: [&[&str]; 5] = [&TABLE, &PLANT, &LAMP, &SHELF, &FRAME];
    let mut order: Vec<usize> = (0..designs.len()).collect();
    // Shuffle, then take the first few: no page has two tables.
    for index in (1..order.len()).rev() {
        let other = rng.below(index + 1);
        order.swap(index, other);
    }
    let count = 2 + rng.below(3);
    let mut out = Vec::new();
    let mut occupied = vec![false; (columns * rows) as usize];
    let usable = rows.saturating_sub(FLOOR_MARGIN);
    for (place, pick) in order.into_iter().take(count).enumerate() {
        let design = designs[pick];
        let height = design.len() as u32;
        let width = design.iter().map(|line| line.chars().count()).max().unwrap_or(0) as u32;
        if width == 0 || height == 0 || usable < height + 2 || columns < width + 6 {
            continue;
        }
        // Spread them across the page rather than stacking them on one side.
        let slot = (place as f32 + 0.5) / count as f32;
        let centre = slot * (columns - width) as f32;
        // In the lower part of the page, where furniture stands in a document
        // rather than across the first lines of it.
        let lowest = usable.saturating_sub(height);
        let highest = (rows / 3).min(lowest);
        let row = highest + rng.below((lowest - highest) as usize + 1) as u32;
        let column = (centre as u32).min(columns - width - 1).max(GUTTER + 1);
        // Two pieces of furniture in the same cells would be one piece of
        // furniture drawn twice.
        let free = (0..height).all(|down| {
            (0..width).all(|across| {
                let (row, column) = (row + down, column + across);
                row < rows && column < columns && !occupied[(row * columns + column) as usize]
            })
        });
        if !free {
            continue;
        }
        for down in 0..height {
            for across in 0..width {
                let (row, column) = (row + down, column + across);
                if row < rows && column < columns {
                    occupied[(row * columns + column) as usize] = true;
                }
            }
        }
        let mut cells = Vec::new();
        for (down, line) in design.iter().enumerate() {
            for (across, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                cells.push((down as u32, across as u32, ch));
            }
        }
        out.push(Prop {
            row,
            column,
            down: height,
            across: width,
            cells,
        });
    }
    out
}

/// A table: a top, two legs, and a line under it.
const TABLE: [&str; 5] = [
    " ___________ ",
    "|           |",
    "|___________|",
    "  ||     ||  ",
    "  ||     ||  ",
];

/// A plant: leaves, a stem and a pot.
const PLANT: [&str; 5] = [
    "  \\ | /  ",
    " \\ \\|/ / ",
    "   \\|/   ",
    "    |    ",
    "  [___]  ",
];

/// A lamp: a shade, a stem and a foot.
const LAMP: [&str; 5] = [
    "  ,---.  ",
    " (     ) ",
    "  `---'  ",
    "    |    ",
    "  =====  ",
];

/// A shelf: two boards and three books.
const SHELF: [&str; 4] = [
    "+---------+",
    "| [] [] [ |",
    "+---------+",
    "|_________|",
];

/// A framed picture on the page.
const FRAME: [&str; 4] = [
    "+------+",
    "| .  . |",
    "|  --  |",
    "+------+",
];

/// The words lying about for the kitten to bat.
///
/// A token is a whole word off the page, not a slice of one: a fragment the kitten
/// knocks about reads as a rendering fault rather than as something a cat did.
fn loose_tokens(rng: &mut Rng, flavour: Flavour, columns: u32, rows: u32) -> Vec<Token> {
    let lines = flavour.lines();
    let count = 4 + rng.below(4);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let line = lines[rng.below(lines.len())];
        let styles = flavour.styles(line);
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            continue;
        }
        // Every run of non-space characters short enough to be a thing the kitten
        // can carry, picked at random.
        let mut words = Vec::new();
        let mut start = None;
        for (index, ch) in chars.iter().chain([' '].iter()).enumerate() {
            match (ch.is_whitespace(), start) {
                (false, None) => start = Some(index),
                (true, Some(from)) => {
                    let length = index - from;
                    if (1..=UNDER_PAW).contains(&length) {
                        words.push((from, length));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        if words.is_empty() {
            continue;
        }
        let (from, length) = words[rng.below(words.len())];
        let text: Vec<char> = chars[from..from + length].to_vec();
        let token_styles: Vec<u8> = (from..from + length)
            .map(|at| styles.get(at).copied().unwrap_or(0))
            .collect();
        let half = text.len() as f32 * GLYPH_WIDTH * 0.5;
        out.push(Token {
            text,
            styles: token_styles,
            pos: [
                rng.range(half + 1.0, (columns as f32 * GLYPH_WIDTH - half - 1.0).max(half + 1.0)),
                rng.range(1.0, (rows.saturating_sub(FLOOR_MARGIN) as f32).max(2.0)),
                0.0,
            ],
            vel: [0.0, 0.0],
            roll: rng.range(-0.5, 0.5),
            pitch: 0.0,
            spin: 0.0,
            rest: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        Page::new(Flavour::Rust, 0x5EED, 0, 120, 40)
    }

    #[test]
    fn a_page_is_lines_of_code_in_a_gutter() {
        let page = page();
        assert!(page.text.len() > 700, "{}", page.text.len());
        // One line number, three digits and a space, per row.
        let gutter = page
            .text
            .iter()
            .filter(|card| card.style == 4 && card.ch.is_ascii_digit())
            .count();
        assert!(gutter > 40, "{}", gutter);
        // Every card is inside the page, and on the grid.
        let width = page.width();
        for card in &page.text {
            assert!(card.pos[0] > 0.0 && card.pos[0] < width, "{:?}", card.pos);
            assert!(card.pos[1] > 0.0 && card.pos[1] <= page.rows as f32, "{:?}", card.pos);
            assert_eq!(card.pos[2], 0.0);
        }
    }

    #[test]
    fn the_same_seed_lays_out_the_same_page() {
        let a = page();
        let b = page();
        assert_eq!(a.text.len(), b.text.len());
        assert_eq!(a.text[10], b.text[10]);
        assert_eq!(a.tokens, b.tokens);
    }

    #[test]
    fn two_seeds_lay_out_two_pages() {
        let a = Page::new(Flavour::Rust, 1, 0, 120, 40);
        let b = Page::new(Flavour::Rust, 2, 0, 120, 40);
        assert!(a.text.iter().map(|c| c.ch).ne(b.text.iter().map(|c| c.ch)));
    }

    #[test]
    fn the_furniture_stands_on_the_page_and_not_on_the_rug() {
        let page = page();
        // Furniture is drawn in the type style, which code never uses for an
        // outline, so a '|' or a '+' in it is a prop.
        let props: Vec<&Card> = page
            .text
            .iter()
            .filter(|card| matches!(card.ch, '|' | '+') && card.style == 2)
            .collect();
        assert!(!props.is_empty(), "a page with furniture");
        let rug = page.rows - RUG_ROWS;
        for card in props {
            let row = (page.rows as f32 - card.pos[1] - 0.5) as u32;
            assert!(row < rug, "furniture on the rug: row {row}");
        }
    }

    #[test]
    fn a_batted_token_stays_on_the_page() {
        let mut page = page();
        page.bat([page.width() * 0.5, 20.0, 0.0], 6.0, 8.0);
        for _ in 0..(120 * 20) {
            page.step_tokens(1.0 / 120.0);
        }
        for token in &page.tokens {
            assert!(token.pos[0] > 0.0, "{token:?}");
            assert!(token.pos[0] < page.width(), "{token:?}");
            assert!(token.pos[1] > 0.0, "{token:?}");
            assert!(token.pos[1] < page.rows as f32, "{token:?}");
        }
    }

    #[test]
    fn a_batted_token_stops_instead_of_drifting_for_ever() {
        let mut page = page();
        page.bat([page.width() * 0.5, 20.0, 0.0], 8.0, 6.0);
        for _ in 0..(120 * 40) {
            page.step_tokens(1.0 / 120.0);
        }
        assert!(
            page.tokens.iter().all(|token| token.rest),
            "every token came to rest"
        );
    }

    #[test]
    fn the_label_names_the_flavour_and_the_page() {
        let page = Page::new(Flavour::Csv, 9, 2, 100, 30);
        assert_eq!(page.label(), "csv · page 3");
    }
}
