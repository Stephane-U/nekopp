//! The room the kitten lives in.
//!
//! A room is generated from one seed and then stands still: the walls are papered
//! with lines of code, the floor carries a rug of punctuation, a few pieces of
//! furniture stand against the walls, and some tokens lie about for the kitten to
//! bat. Every five minutes the whole thing is rebuilt from the next seed, which is
//! what keeps the mode from becoming wallpaper.
//!
//! The geometry is in characters: one unit is one line of the editor's text, so a
//! room is a few characters wide and a kitten is two and a half of them. Nothing
//! here knows about pixels, the window, or the GPU.

use super::corpus::{self, Flavour};
use super::{Block, Card, CardKind, Rng, Shade};

/// How wide one glyph card is, in character heights.
///
/// Monospace cells are about 0.55 of the line height in every font this editor
/// ships with; the exact ratio is the renderer's business, and this is the
/// spacing the room is laid out with.
pub const GLYPH_WIDTH: f32 = 0.55;

/// Half the depth of the room, from the back wall to the camera side.
pub const HALF_DEPTH: f32 = 9.0;
/// Half the width of the room.
pub const HALF_WIDTH: f32 = 8.0;
/// How high the walls are.
pub const WALL_HEIGHT: f32 = 8.0;
/// The z of the back wall.
pub const BACK: f32 = -HALF_DEPTH;

/// Radius of a token, for its collisions with the floor.
pub const TOKEN_RADIUS: f32 = 0.5;
/// The fastest a batted token may travel, in characters per second.
///
/// Without a ceiling a token the kitten keeps swatting climbs out of the room,
/// and the room has no ceiling to stop it.
const MAX_SPEED: f32 = 9.0;
/// The fastest a batted token may spin.
const MAX_SPIN: f32 = 14.0;

/// One loose token: a word or a brace the kitten can knock about.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The characters of the token, in order.
    pub text: Vec<char>,
    /// One style per character, from the flavour's lexer.
    pub styles: Vec<u8>,
    /// Centre, in characters above the floor.
    pub pos: [f32; 3],
    /// Velocity, in characters per second.
    pub vel: [f32; 3],
    /// Facing, in radians.
    pub yaw: f32,
    /// How far it has tipped, in radians.
    pub pitch: f32,
    /// Spin, in radians per second.
    pub spin: f32,
    /// True once it has stopped moving.
    pub rest: bool,
}

impl Token {
    /// The token's own width, in characters.
    pub fn width(&self) -> f32 {
        self.text.len() as f32 * GLYPH_WIDTH
    }

    /// The cards that draw it.
    pub fn cards(&self, alpha: f32, out: &mut Vec<Card>) {
        let width = self.width();
        let (sin, cos) = self.yaw.sin_cos();
        for (index, ch) in self.text.iter().copied().enumerate() {
            // Laid out along the token's own facing, then tipped by `pitch`.
            let offset = index as f32 * GLYPH_WIDTH - width * 0.5 + GLYPH_WIDTH * 0.5;
            out.push(Card {
                ch,
                style: self.styles.get(index).copied().unwrap_or(0),
                pos: [
                    self.pos[0] + offset * cos,
                    self.pos[1],
                    self.pos[2] - offset * sin,
                ],
                yaw: self.yaw,
                pitch: self.pitch,
                scale: 1.0,
                alpha,
                kind: CardKind::Glyph,
            });
        }
    }
}

/// A speck of dust, drifting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mote {
    pub pos: [f32; 3],
    /// How far it wanders from where it started.
    pub drift: f32,
    /// Where it is in its wander, so no two move together.
    pub phase: f32,
    /// Its size, in characters.
    pub size: f32,
}

/// What one room is made of.
#[derive(Debug, Clone)]
pub struct Room {
    pub flavour: Flavour,
    pub seed: u64,
    /// Which regeneration this is, counted from the start of the session.
    pub index: u32,
    /// The code on the walls, already placed.
    pub walls: Vec<Card>,
    /// The rug: punctuation lying flat on the floor.
    pub floor: Vec<Card>,
    /// Furniture.
    pub blocks: Vec<Block>,
    /// Loose tokens.
    pub tokens: Vec<Token>,
    /// Dust.
    pub motes: Vec<Mote>,
}

impl Room {
    /// Builds the room for one regeneration.
    pub fn new(flavour: Flavour, seed: u64, index: u32) -> Self {
        let mut rng = Rng::new(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let lines = flavour.lines();
        let mut walls = Vec::new();
        // Seven rows of code on the back wall, centred, fading at the edges so
        // the wall does not end in a hard vertical edge.
        for row in 0..7 {
            let line = lines[(row + rng.below(lines.len())) % lines.len()];
            let styles = flavour.styles(line);
            let chars: Vec<char> = line.chars().collect();
            let width = chars.len() as f32 * GLYPH_WIDTH;
            // One line per row, rising from just above the floor to just under
            // the top of the wall the camera sees.
            let y = 1.18 + row as f32 * 1.0;
            for (column, ch) in chars.iter().copied().enumerate() {
                let x = column as f32 * GLYPH_WIDTH - width * 0.5;
                // A line of code is wider than the room: the wall shows the part
                // of it that fits, the way a real wall of text is cropped by the
                // corner it runs into.
                if x.abs() > HALF_WIDTH - 0.1 {
                    continue;
                }
                let edge = (1.0 - (x.abs() / (HALF_WIDTH - 1.0)).powi(2)).clamp(0.0, 1.0);
                walls.push(Card {
                    ch,
                    style: styles.get(column).copied().unwrap_or(0),
                    pos: [x, y, BACK + 0.03],
                    yaw: 0.0,
                    pitch: 0.0,
                    scale: 1.0,
                    alpha: 0.32 + 0.68 * edge,
                    kind: CardKind::Glyph,
                });
            }
        }
        // The side walls carry the same lines, at a right angle, four rows each.
        for (side, sign) in [(-1.0, -1.0f32), (1.0, 1.0f32)] {
            for row in 0..4 {
                let line = lines[(row * 3 + rng.below(lines.len())) % lines.len()];
                let styles = flavour.styles(line);
                let chars: Vec<char> = line.chars().collect();
                let width = chars.len() as f32 * GLYPH_WIDTH;
                for (column, ch) in chars.iter().copied().enumerate() {
                    let along = column as f32 * GLYPH_WIDTH - width * 0.5;
                    let z = -2.0 + along * sign;
                    if z < BACK + 0.1 || z > 3.0 {
                        continue;
                    }
                    let edge = (1.0 - (along.abs() / (HALF_DEPTH - 1.0)).powi(2)).clamp(0.0, 1.0);
                    walls.push(Card {
                        ch,
                        style: styles.get(column).copied().unwrap_or(0),
                        pos: [side * (HALF_WIDTH - 0.02), 1.6 + row as f32 * 1.1, z],
                        // The line faces into the room, so it reads from the
                        // middle of it rather than through the wall.
                        yaw: -sign * std::f32::consts::FRAC_PI_2,
                        pitch: 0.0,
                        scale: 1.0,
                        alpha: (0.2 + 0.5 * edge) * 0.9,
                        kind: CardKind::Glyph,
                    });
                }
            }
        }

        // A rug of punctuation, lying flat: `;` and `{` are the two characters a
        // code editor's floor is made of.
        let mut floor = Vec::new();
        let marks = ['.', ',', ';', '{', '}', '(', ')', ':', '=', '*', '+', '-'];
        let mut x = -HALF_WIDTH + 1.0;
        while x < HALF_WIDTH - 1.0 {
            let mut z = -HALF_DEPTH + 1.5;
            while z < 1.0 {
                let ch = marks[rng.below(marks.len())];
                let shade = (rng.above() * 0.14) as f32;
                floor.push(Card {
                    ch,
                    style: 7,
                    pos: [x + shade, 0.01, z - shade],
                    yaw: rng.range(-0.2, 0.2),
                    pitch: -std::f32::consts::FRAC_PI_2,
                    scale: 0.7,
                    alpha: 0.16,
                    kind: CardKind::Glyph,
                });
                z += 1.5;
            }
            x += 1.4;
        }

        let blocks = furniture(&mut rng, flavour);
        let tokens = loose_tokens(&mut rng, flavour);
        let motes = (0..48)
            .map(|index| Mote {
                pos: [
                    rng.range(-HALF_WIDTH + 1.0, HALF_WIDTH - 1.0),
                    rng.range(0.5, 6.0),
                    rng.range(BACK + 0.5, 1.0),
                ],
                drift: rng.range(0.3, 1.4),
                phase: index as f32 * 0.37 + rng.range(0.0, std::f32::consts::TAU),
                size: rng.range(0.04, 0.12),
            })
            .collect();

        Self {
            flavour,
            seed,
            index,
            walls,
            floor,
            blocks,
            tokens,
            motes,
        }
    }

    /// The corner label: which flavour, which room.
    pub fn label(&self) -> String {
        format!("{} · room {}", self.flavour.name(), self.index + 1)
    }

    /// Moves every loose token by `dt` seconds.
    ///
    /// Fixed steps are the caller's job: one step of `1/120` and one of `1/60` are
    /// not the same integration, and a jump of half a second -- which a loaded
    /// machine will produce -- would send a token through the floor.
    pub fn step_tokens(&mut self, dt: f32) {
        const GRAVITY: f32 = -16.0;
        const RESTITUTION: f32 = 0.34;
        for token in &mut self.tokens {
            if token.rest {
                continue;
            }
            token.vel[1] += GRAVITY * dt;
            token.pos[0] += token.vel[0] * dt;
            token.pos[1] += token.vel[1] * dt;
            token.pos[2] += token.vel[2] * dt;
            token.yaw += token.spin * dt;
            token.spin *= 1.0 - 2.4 * dt;
            // A tumbling token tips toward flat as it slows: a card that lands
            // standing up looks broken.
            token.pitch += (0.0 - token.pitch) * 3.0 * dt;

            let half = TOKEN_RADIUS * token.pitch.cos().abs().max(0.35);
            if token.pos[1] < half {
                token.pos[1] = half;
                if token.vel[1] < 0.0 {
                    token.vel[1] = -token.vel[1] * RESTITUTION;
                    if token.vel[1].abs() < 0.6 {
                        token.vel[1] = 0.0;
                    }
                }
                token.vel[0] *= 1.0 - 5.0 * dt;
                token.vel[2] *= 1.0 - 5.0 * dt;
                if token.vel[0].hypot(token.vel[2]) < 0.05 && token.vel[1] == 0.0 {
                    token.rest = true;
                    token.pitch *= 0.9;
                }
            }
            // The walls are solid, and so is the back of the room.
            let limit = HALF_WIDTH - TOKEN_RADIUS;
            if token.pos[0] < -limit {
                token.pos[0] = -limit;
                token.vel[0] = token.vel[0].abs() * 0.4;
            }
            if token.pos[0] > limit {
                token.pos[0] = limit;
                token.vel[0] = -token.vel[0].abs() * 0.4;
            }
            let back = BACK + TOKEN_RADIUS;
            if token.pos[2] < back {
                token.pos[2] = back;
                token.vel[2] = token.vel[2].abs() * 0.4;
            }
            if token.pos[2] > 3.0 {
                token.pos[2] = 3.0;
                token.vel[2] = -token.vel[2].abs() * 0.4;
            }
        }
    }

    /// Pushes a token near `at` away from the kitten.
    ///
    /// The paw is not an engine: a token the kitten keeps its paw on for a dozen
    /// frames takes one shove, not a dozen, which is why the vertical speed is a
    /// maximum here and not a sum.
    pub fn bat(&mut self, at: [f32; 3], reach: f32, power: f32) {
        for token in &mut self.tokens {
            let dx = token.pos[0] - at[0];
            let dz = token.pos[2] - at[2];
            let distance = dx.hypot(dz);
            if distance > reach {
                continue;
            }
            let scale = power / distance.max(0.4);
            token.vel[0] = (token.vel[0] + dx * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.vel[2] = (token.vel[2] + dz * scale).clamp(-MAX_SPEED, MAX_SPEED);
            token.vel[1] = token.vel[1].max(power * 1.6).min(MAX_SPEED);
            token.spin = (token.spin + power * 3.0).clamp(-MAX_SPIN, MAX_SPIN);
            token.rest = false;
        }
    }
}

/// What one piece of furniture is built by: it takes the seed, the flavour and
/// the place it stands, and returns its blocks.
type Recipe = fn(&mut Rng, Flavour, f32, f32) -> Vec<Block>;

/// Two to four pieces of furniture, against the walls and out of the way.
fn furniture(rng: &mut Rng, flavour: Flavour) -> Vec<Block> {
    let mut blocks = Vec::new();
    let recipes: [Recipe; 5] = [table, shelf, lamp, plant, screen];
    let count = 2 + rng.below(3);
    let mut slots: Vec<usize> = (0..recipes.len()).collect();
    // Shuffle the recipes, then take the first `count`: no room has two tables.
    for index in (1..slots.len()).rev() {
        let other = rng.below(index + 1);
        slots.swap(index, other);
    }
    for (place, recipe) in slots.into_iter().take(count).enumerate() {
        let side = if place % 2 == 0 { -1.0 } else { 1.0 };
        let depth = -6.0 + (place as f32) * 2.4;
        blocks.extend(recipes[recipe](rng, flavour, side * (HALF_WIDTH - 2.2), depth));
    }
    blocks
}

/// A block, sized and placed.
fn block(pos: [f32; 3], size: [f32; 3], shade: Shade) -> Block {
    Block {
        pos,
        size,
        yaw: 0.0,
        shade,
    }
}

fn table(rng: &mut Rng, _flavour: Flavour, x: f32, z: f32) -> Vec<Block> {
    let yaw = rng.range(-0.2, 0.2);
    let mut blocks = vec![block([x, 1.0, z], [2.6, 0.16, 1.5], Shade::Wood)];
    for (dx, dz) in [(-1.1, -0.55), (1.1, -0.55), (-1.1, 0.55), (1.1, 0.55)] {
        blocks.push(block([x + dx, 0.5, z + dz], [0.16, 1.0, 0.16], Shade::Wood));
    }
    for piece in &mut blocks {
        piece.yaw = yaw;
    }
    blocks
}

fn shelf(rng: &mut Rng, _flavour: Flavour, x: f32, z: f32) -> Vec<Block> {
    let mut blocks = Vec::new();
    for dz in [-1.0, 1.0] {
        blocks.push(block([x, 1.6, z + dz * 0.5], [0.18, 3.2, 0.18], Shade::Wood));
    }
    for level in 0..3 {
        blocks.push(block(
            [x, 0.9 + level as f32 * 1.05, z],
            [0.16, 0.12, 1.1],
            Shade::Wood,
        ));
    }
    blocks.push(block([x, 3.2, z], [0.2, 0.16, 1.2], Shade::Wood));
    let _ = rng;
    blocks
}

fn lamp(rng: &mut Rng, _flavour: Flavour, x: f32, z: f32) -> Vec<Block> {
    let _ = rng;
    vec![
        block([x, 0.08, z], [0.5, 0.16, 0.5], Shade::Metal),
        block([x, 1.4, z], [0.12, 2.6, 0.12], Shade::Metal),
        block([x, 2.8, z], [0.7, 0.34, 0.7], Shade::Metal),
        block([x, 2.72, z], [0.42, 0.22, 0.42], Shade::Glow),
    ]
}

fn plant(rng: &mut Rng, _flavour: Flavour, x: f32, z: f32) -> Vec<Block> {
    let mut blocks = vec![block([x, 0.35, z], [0.8, 0.7, 0.8], Shade::Pot)];
    for stem in 0..3 {
        let lean = rng.range(-0.25, 0.25);
        blocks.push(block(
            [x + lean * 0.4, 1.1, z + lean * 0.3],
            [0.1, 1.1, 0.1],
            Shade::Leaf,
        ));
        blocks.push(block(
            [x + lean, 1.7 + stem as f32 * 0.12, z + lean * 0.6],
            [0.5, 0.28, 0.34],
            Shade::Leaf,
        ));
    }
    blocks
}

fn screen(rng: &mut Rng, _flavour: Flavour, x: f32, z: f32) -> Vec<Block> {
    let _ = rng;
    vec![
        block([x, 0.4, z], [0.9, 0.8, 0.5], Shade::Metal),
        block([x, 1.6, z], [1.6, 1.6, 0.12], Shade::Metal),
        block([x, 1.6, z - 0.08], [1.4, 1.4, 0.04], Shade::Glow),
    ]
}

/// Six tokens from the corpus, lying on the floor.
fn loose_tokens(rng: &mut Rng, flavour: Flavour) -> Vec<Token> {
    let mut pool: Vec<(Vec<char>, Vec<u8>)> = Vec::new();
    for line in flavour.lines() {
        let styles = flavour.styles(line);
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        for word in corpus::tokens(flavour, line) {
            // Colours are per line, so a token's styles are the slice of the
            // line it came from: the tokens are in the order they appear.
            let at = find(&chars, &word, start);
            start = at.map(|at| at + word.len()).unwrap_or(start);
            let slice: Vec<u8> = match at {
                Some(at) => (0..word.chars().count())
                    .map(|offset| styles.get(at + offset).copied().unwrap_or(0))
                    .collect(),
                None => vec![0; word.chars().count()],
            };
            pool.push((word.chars().collect(), slice));
        }
    }
    pool.sort_by_key(|(text, _)| text.len());
    let mut tokens = Vec::new();
    for _ in 0..6 {
        if pool.is_empty() {
            break;
        }
        // Short tokens: a card the kitten can bat, not a whole line.
        let pick = rng.below(pool.len().min(10));
        let (text, styles) = pool[pick].clone();
        tokens.push(Token {
            text,
            styles,
            pos: [
                rng.range(-HALF_WIDTH + 1.5, HALF_WIDTH - 1.5),
                TOKEN_RADIUS,
                rng.range(-7.0, 0.5),
            ],
            vel: [0.0, 0.0, 0.0],
            yaw: rng.range(-std::f32::consts::PI, std::f32::consts::PI),
            pitch: rng.range(-0.2, 0.2),
            spin: 0.0,
            rest: true,
        });
    }
    tokens
}

/// Where `word` starts in `chars`, at or after `from`.
fn find(chars: &[char], word: &str, from: usize) -> Option<usize> {
    let wanted: Vec<char> = word.chars().collect();
    if wanted.is_empty() {
        return None;
    }
    (from..chars.len())
        .take_while(|start| start + wanted.len() <= chars.len())
        .find(|start| chars[*start..start + wanted.len()] == wanted[..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(seed: u64) -> Room {
        Room::new(Flavour::Rust, seed, 0)
    }

    #[test]
    fn a_room_is_the_same_every_time_it_is_built_from_one_seed() {
        let a = room(7);
        let b = room(7);
        assert_eq!(a.walls.len(), b.walls.len());
        assert_eq!(a.walls[3], b.walls[3]);
        assert_eq!(a.tokens.len(), b.tokens.len());
        assert_eq!(a.tokens[0].text, b.tokens[0].text);
        assert_eq!(a.blocks.len(), b.blocks.len());
    }

    #[test]
    fn two_seeds_make_two_rooms() {
        let a = room(7);
        let b = room(8);
        assert_ne!(a.walls, b.walls);
    }

    #[test]
    fn a_wall_is_papered_with_more_than_a_hundred_cards() {
        let room = room(1);
        assert!(room.walls.len() > 100, "{} cards", room.walls.len());
        assert!(room.floor.len() > 40, "{} floor marks", room.floor.len());
        assert!(!room.blocks.is_empty());
        assert_eq!(room.tokens.len(), 6);
        assert!(!room.motes.is_empty());
    }

    #[test]
    fn nothing_is_generated_outside_the_room() {
        for seed in 0..8 {
            let room = room(seed);
            for card in room.walls.iter().chain(&room.floor) {
                assert!(card.pos[0].abs() <= HALF_WIDTH + 0.01, "{:?}", card.pos);
                assert!(card.pos[1] >= 0.0 && card.pos[1] <= WALL_HEIGHT, "{:?}", card.pos);
                assert!(card.pos[2] >= BACK - 0.1 && card.pos[2] <= 3.2, "{:?}", card.pos);
            }
            for token in &room.tokens {
                assert!(token.pos[0].abs() <= HALF_WIDTH);
                assert!(token.pos[2] >= BACK && token.pos[2] <= 3.0);
            }
        }
    }

    #[test]
    fn a_dropped_token_lands_and_stays_on_the_floor() {
        let mut room = room(3);
        let token = &mut room.tokens[0];
        token.pos[1] = 3.0;
        token.vel = [1.0, 0.0, 0.5];
        token.rest = false;
        for _ in 0..(120 * 6) {
            room.step_tokens(1.0 / 120.0);
        }
        let token = &room.tokens[0];
        assert!(token.rest, "{token:?}");
        assert!(token.pos[1] >= TOKEN_RADIUS * 0.34, "{token:?}");
        assert!(token.pos[1] < 1.0, "it fell through the floor: {token:?}");
        assert!(token.pos[0].abs() <= HALF_WIDTH);
    }

    #[test]
    fn a_batted_token_moves_away_from_the_paw() {
        let mut room = room(4);
        let token = &mut room.tokens[0];
        token.pos = [0.0, TOKEN_RADIUS, 0.0];
        token.rest = true;
        room.bat([-1.0, 0.5, 0.0], 2.0, 4.0);
        let token = &room.tokens[0];
        assert!(!token.rest);
        assert!(token.vel[0] > 0.0, "{:?}", token.vel);
        assert!(token.vel[1] > 0.0, "{:?}", token.vel);
    }

    #[test]
    fn a_token_draws_one_card_per_character() {
        let room = room(2);
        let token = &room.tokens[0];
        let mut cards = Vec::new();
        token.cards(1.0, &mut cards);
        assert_eq!(cards.len(), token.text.len());
        assert_eq!(cards[0].ch, token.text[0]);
        assert!(cards.iter().all(|card| card.kind == CardKind::Glyph));
        // The cards are spread along the token's facing, so the widest gap is
        // bounded by the card width.
        if cards.len() > 1 {
            let gap = (cards[1].pos[0] - cards[0].pos[0]).abs();
            assert!(gap > 0.2 && gap < 0.7, "{gap}");
        }
    }

    #[test]
    fn the_floor_is_laid_at_ground_level_and_the_label_names_the_room() {
        let room = room(5);
        assert!(room.floor.iter().all(|card| card.pos[1] < 0.1));
        assert_eq!(room.label(), "rust · room 1");
    }

    #[test]
    fn eight_flavours_give_eight_different_rooms() {
        let mut widths = Vec::new();
        for (index, flavour) in Flavour::ALL.iter().copied().enumerate() {
            let room = Room::new(flavour, 1, index as u32);
            assert_eq!(room.flavour, flavour);
            widths.push(room.walls.len());
        }
        assert!(widths.iter().any(|width| *width != widths[0]));
    }

    #[test]
    fn a_room_holds_no_more_than_four_pieces_of_furniture() {
        for seed in 0..16 {
            let room = room(seed);
            // Every recipe is between four and six blocks.
            assert!(room.blocks.len() >= 4, "{} blocks", room.blocks.len());
            assert!(room.blocks.len() <= 30, "{} blocks", room.blocks.len());
            for piece in &room.blocks {
                assert!(piece.pos[0].abs() <= HALF_WIDTH + 0.5);
                assert!(piece.pos[1] >= 0.0 && piece.pos[1] <= 4.0);
            }
        }
    }

    #[test]
    fn the_same_step_of_time_gives_the_same_motion() {
        let mut a = room(11);
        let mut b = room(11);
        for room in [&mut a, &mut b] {
            let token = &mut room.tokens[0];
            token.pos[1] = 2.5;
            token.vel = [0.7, 0.0, -0.3];
            token.rest = false;
            for _ in 0..60 {
                room.step_tokens(1.0 / 120.0);
            }
        }
        assert_eq!(a.tokens[0].pos, b.tokens[0].pos);
    }

    #[test]
    fn a_character_in_a_line_can_be_found_for_its_style() {
        let chars: Vec<char> = "let x = 1;".chars().collect();
        assert_eq!(find(&chars, "x", 0), Some(4));
        assert_eq!(find(&chars, "x", 5), None);
        assert_eq!(find(&chars, "1", 0), Some(8));
        assert_eq!(find(&chars, "", 0), None);
    }
}
