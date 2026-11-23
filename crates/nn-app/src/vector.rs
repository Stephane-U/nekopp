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

//! Vector paths: parsing, flattening, and rasterising.
//!
//! The toolbar icons are drawings, not bitmaps. Each one is a handful of SVG
//! path strings in a 24x24 box, and this module turns those strings into an
//! alpha mask at whatever pixel size the display asks for. That is what makes
//! the icons sharp on a 2x screen and at any toolbar height without shipping a
//! second set of images.
//!
//! The pipeline is the usual one: parse the path into absolute commands, flatten
//! the curves into polylines, then scan-convert the polygons with the nonzero
//! winding rule. Coverage is accumulated at `SUPERSAMPLE` times the output
//! resolution and box-filtered down, which is what gives the edges their
//! anti-aliasing without a second pass over the pixels.
//!
//! Arcs are supported because a rounded corner is the one shape a drawing tool
//! will not emit as a cubic, and an icon set that silently drops them would be
//! wrong in a way that is hard to see.

/// One absolute path command.
///
/// Relative commands, the shorthand forms (`H`, `V`, `S`, `T`) and arcs are all
/// resolved by the parser, so the flattener only ever sees these four.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    /// Start a new subpath at this point.
    Move(f32, f32),
    /// A straight segment to this point.
    Line(f32, f32),
    /// A cubic segment: two control points then the end point.
    Cubic([f32; 6]),
    /// A quadratic segment: one control point then the end point.
    Quad([f32; 4]),
    /// Close the current subpath back to its start.
    Close,
}

/// How many samples per output pixel, in each direction, coverage is measured at.
///
/// Four is the point where a diagonal edge stops looking stepped at the sizes a
/// toolbar uses; eight doubles the work for a difference no one can see at 24
/// pixels.
pub const SUPERSAMPLE: u32 = 4;

/// Parses SVG path data into absolute commands.
///
/// Returns `None` for malformed input rather than guessing: a truncated path is
/// a bug in the icon table, and a half-drawn icon is harder to notice than a
/// missing one.
pub fn parse(d: &str) -> Option<Vec<Cmd>> {
    let mut lex = Lexer::new(d.as_bytes());
    let mut out = Vec::new();
    // The current point, the start of the current subpath, and the last control
    // point, which is what `S` and `T` reflect.
    let mut cur = [0.0f32, 0.0f32];
    let mut start = [0.0f32, 0.0f32];
    let mut ctrl: Option<[f32; 2]> = None;
    let mut last = 0u8;

    while let Some(cmd) = lex.peek_command() {
        lex.bump();
        let rel = cmd.is_ascii_lowercase();
        let upper = cmd.to_ascii_uppercase();
        // `M` and `m` take a pair and then behave as `L` for any further pairs,
        // which is the one place the command letter changes meaning mid-run.
        let mut first = true;
        loop {
            let base = if rel { cur } else { [0.0, 0.0] };
            match upper {
                b'M' => {
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    if first {
                        out.push(Cmd::Move(x, y));
                        start = [x, y];
                    } else {
                        out.push(Cmd::Line(x, y));
                    }
                    cur = [x, y];
                    ctrl = None;
                }
                b'L' => {
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Line(x, y));
                    cur = [x, y];
                    ctrl = None;
                }
                b'H' => {
                    let x = base[0] + lex.number()?;
                    out.push(Cmd::Line(x, cur[1]));
                    cur = [x, cur[1]];
                    ctrl = None;
                }
                b'V' => {
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Line(cur[0], y));
                    cur = [cur[0], y];
                    ctrl = None;
                }
                b'C' => {
                    let x1 = base[0] + lex.number()?;
                    let y1 = base[1] + lex.number()?;
                    let x2 = base[0] + lex.number()?;
                    let y2 = base[1] + lex.number()?;
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Cubic([x1, y1, x2, y2, x, y]));
                    ctrl = Some([x2, y2]);
                    cur = [x, y];
                }
                b'S' => {
                    // The first control point is the reflection of the previous
                    // one through the current point, or the current point itself
                    // when the previous command was not a cubic.
                    let r = match (last.to_ascii_uppercase(), ctrl) {
                        (b'C', Some(c)) => [2.0 * cur[0] - c[0], 2.0 * cur[1] - c[1]],
                        _ => cur,
                    };
                    let x2 = base[0] + lex.number()?;
                    let y2 = base[1] + lex.number()?;
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Cubic([r[0], r[1], x2, y2, x, y]));
                    ctrl = Some([x2, y2]);
                    cur = [x, y];
                }
                b'Q' => {
                    let x1 = base[0] + lex.number()?;
                    let y1 = base[1] + lex.number()?;
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Quad([x1, y1, x, y]));
                    ctrl = Some([x1, y1]);
                    cur = [x, y];
                }
                b'T' => {
                    let r = match (last.to_ascii_uppercase(), ctrl) {
                        (b'Q', Some(c)) => [2.0 * cur[0] - c[0], 2.0 * cur[1] - c[1]],
                        _ => cur,
                    };
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    out.push(Cmd::Quad([r[0], r[1], x, y]));
                    ctrl = Some(r);
                    cur = [x, y];
                }
                b'A' => {
                    let rx = lex.number()?.abs();
                    let ry = lex.number()?.abs();
                    let rot = lex.number()?;
                    let large = lex.number()? != 0.0;
                    let sweep = lex.number()? != 0.0;
                    let x = base[0] + lex.number()?;
                    let y = base[1] + lex.number()?;
                    arc_to_cubics(cur, [rx, ry], rot, large, sweep, [x, y], &mut out);
                    cur = [x, y];
                    ctrl = None;
                }
                b'Z' => {
                    out.push(Cmd::Close);
                    cur = start;
                    ctrl = None;
                }
                _ => return None,
            }
            last = cmd;
            first = false;
            // A new command letter ends the run; so does running out of numbers.
            if lex.peek_command().is_some() || !lex.has_number() {
                break;
            }
        }
    }
    Some(out)
}

/// Converts an SVG elliptical arc into cubic segments.
///
/// The endpoint parameterisation is turned into a centre parameterisation, the
/// sweep is split into quarter-turn pieces, and each piece is approximated by
/// the standard cubic whose control points sit `4/3 * tan(theta/4)` along the
/// tangents. That is accurate to about a thousandth of the radius, which is far
/// below one pixel at icon sizes.
fn arc_to_cubics(
    from: [f32; 2],
    radii: [f32; 2],
    rotation: f32,
    large: bool,
    sweep: bool,
    to: [f32; 2],
    out: &mut Vec<Cmd>,
) {
    let (mut rx, mut ry) = (radii[0], radii[1]);
    if rx == 0.0 || ry == 0.0 {
        out.push(Cmd::Line(to[0], to[1]));
        return;
    }
    let phi = rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx2 = (from[0] - to[0]) / 2.0;
    let dy2 = (from[1] - to[1]) / 2.0;
    let x1 = cos_phi * dx2 + sin_phi * dy2;
    let y1 = -sin_phi * dx2 + cos_phi * dy2;
    // Scale the radii up when they are too small to span the endpoints, which is
    // what the specification asks for and what keeps the square root real.
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cos_phi * cx1 - sin_phi * cy1 + (from[0] + to[0]) / 2.0;
    let cy = sin_phi * cx1 + cos_phi * cy1 + (from[1] + to[1]) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let ux = (x1 - cx1) / rx;
    let uy = (y1 - cy1) / ry;
    let vx = (-x1 - cx1) / rx;
    let vy = (-y1 - cy1) / ry;
    let theta = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = (delta.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / steps as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut t = theta;
    let mut p = from;
    for _ in 0..steps {
        let t2 = t + step;
        let (s1, c1) = t.sin_cos();
        let (s2, c2) = t2.sin_cos();
        let map = |ex: f32, ey: f32| -> [f32; 2] {
            [
                cx + rx * cos_phi * ex - ry * sin_phi * ey,
                cy + rx * sin_phi * ex + ry * cos_phi * ey,
            ]
        };
        let c1p = map(c1 - k * s1, s1 + k * c1);
        let c2p = map(c2 + k * s2, s2 - k * c2);
        let end = map(c2, s2);
        out.push(Cmd::Cubic([c1p[0], c1p[1], c2p[0], c2p[1], end[0], end[1]]));
        p = end;
        t = t2;
    }
    // The arithmetic above lands a hair off the requested endpoint; snapping it
    // keeps a chain of arcs from drifting away from the shape it is tracing.
    if let Some(Cmd::Cubic(c)) = out.last_mut() {
        c[4] = to[0];
        c[5] = to[1];
    }
    let _ = p;
}

/// Flattens commands into polylines, one per subpath.
///
/// `tolerance` is the largest distance, in path units, a flattened segment may
/// stray from the true curve. Half a pixel is the useful setting: below that the
/// extra points cost time and change nothing.
pub fn flatten(cmds: &[Cmd], tolerance: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur = [0.0f32, 0.0f32];
    let mut start = [0.0f32, 0.0f32];
    let tol = tolerance.max(0.01);
    for cmd in cmds {
        match *cmd {
            Cmd::Move(x, y) => {
                out.push(vec![[x, y]]);
                cur = [x, y];
                start = [x, y];
            }
            Cmd::Line(x, y) => {
                if out.is_empty() {
                    out.push(vec![cur]);
                }
                out.last_mut().unwrap().push([x, y]);
                cur = [x, y];
            }
            Cmd::Cubic(c) => {
                if out.is_empty() {
                    out.push(vec![cur]);
                }
                let path = out.last_mut().unwrap();
                flatten_cubic(cur, [c[0], c[1]], [c[2], c[3]], [c[4], c[5]], tol, 0, path);
                cur = [c[4], c[5]];
            }
            Cmd::Quad(q) => {
                if out.is_empty() {
                    out.push(vec![cur]);
                }
                let path = out.last_mut().unwrap();
                flatten_quad(cur, [q[0], q[1]], [q[2], q[3]], tol, 0, path);
                cur = [q[2], q[3]];
            }
            Cmd::Close => {
                if let Some(path) = out.last_mut() {
                    if path.len() > 1 && path[0] != cur {
                        path.push(start);
                    }
                }
                cur = start;
            }
        }
    }
    out
}

/// The largest distance from `p` to the segment `a`-`b`.
fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

/// Recursion cap, so a degenerate curve cannot exhaust the stack.
const MAX_DEPTH: u32 = 16;

fn flatten_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    tol: f32,
    depth: u32,
    out: &mut Vec<[f32; 2]>,
) {
    if depth >= MAX_DEPTH
        || (distance_to_segment(p1, p0, p3) <= tol && distance_to_segment(p2, p0, p3) <= tol)
    {
        out.push(p3);
        return;
    }
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let p0123 = mid(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, tol, depth + 1, out);
    flatten_cubic(p0123, p123, p23, p3, tol, depth + 1, out);
}

fn flatten_quad(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    tol: f32,
    depth: u32,
    out: &mut Vec<[f32; 2]>,
) {
    if depth >= MAX_DEPTH || distance_to_segment(p1, p0, p2) <= tol {
        out.push(p2);
        return;
    }
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p012 = mid(p01, p12);
    flatten_quad(p0, p01, p012, tol, depth + 1, out);
    flatten_quad(p012, p12, p2, tol, depth + 1, out);
}

/// Scan-converts closed polygons into an 8-bit coverage mask.
///
/// The polygons are filled with the nonzero winding rule, which is what SVG
/// uses by default and what lets a hole be cut by drawing it the other way
/// round. Coverage is measured at `SUPERSAMPLE` times the output size and
/// averaged down, so an edge that crosses a pixel diagonally gets a value
/// between zero and 255 instead of a hard step.
///
/// The mask is `width * height` bytes, row-major, top row first, and is meant to
/// be used as an alpha channel.
pub fn rasterise(subpaths: &[Vec<[f32; 2]>], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut acc = vec![0.0f32; w * h];
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let ss = SUPERSAMPLE as usize;
    // Every edge, with the direction it runs in, so the winding can be counted.
    // The coordinates are scaled into supersample space here, once, so the
    // scanline loop below can step a whole subpixel at a time.
    let scale = ss as f32;
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for path in subpaths {
        if path.len() < 2 {
            continue;
        }
        for i in 0..path.len() {
            let a = path[i];
            let b = path[(i + 1) % path.len()];
            if a[1] == b[1] {
                continue;
            }
            let dir = if b[1] > a[1] { 1 } else { -1 };
            let (top, bottom) = if a[1] < b[1] { (a, b) } else { (b, a) };
            edges.push((
                top[0] * scale,
                top[1] * scale,
                bottom[0] * scale,
                bottom[1] * scale,
                dir,
            ));
        }
    }
    if edges.is_empty() {
        return vec![0; w * h];
    }
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..h * ss {
        let y = row as f32 + 0.5;
        crossings.clear();
        for &(x0, y0, x1, y1, dir) in &edges {
            if y < y0 || y >= y1 {
                continue;
            }
            let t = (y - y0) / (y1 - y0);
            crossings.push((x0 + t * (x1 - x0), dir));
        }
        if crossings.len() < 2 {
            continue;
        }
        crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut winding = 0;
        let mut span_start = 0.0f32;
        for &(x, dir) in &crossings {
            let before = winding;
            winding += dir;
            if before == 0 && winding != 0 {
                span_start = x;
            } else if before != 0 && winding == 0 {
                add_span(&mut acc, row / ss, w, ss, span_start, x);
            }
        }
    }
    let total = (ss * ss) as f32;
    acc.iter()
        .map(|&c| ((c / total) * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// Adds one supersampled row's coverage for the span `[from, to)` to `row`.
fn add_span(acc: &mut [f32], row: usize, width: usize, ss: usize, from: f32, to: f32) {
    let lo = from.max(0.0);
    let hi = to.min((width * ss) as f32);
    if hi <= lo {
        return;
    }
    let first = lo.floor() as usize;
    let last = (hi.ceil() as usize).min(width * ss);
    for sub in first..last {
        let a = lo.max(sub as f32);
        let b = hi.min(sub as f32 + 1.0);
        if b <= a {
            continue;
        }
        let column = sub / ss;
        if column < width {
            acc[row * width + column] += b - a;
        }
    }
}

/// How a path is painted.
///
/// Notepad++'s toolbar icons are line drawings, so most of the icon table is
/// strokes; the few solid shapes — a record dot, a play triangle — are fills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// Fill the path with the nonzero winding rule.
    Fill,
    /// Draw the path as a line of this width, with round joins and caps.
    Stroke(f32),
}

/// Turns polylines into the polygons that cover them.
///
/// A stroke is the union of a rectangle per segment and a disc per vertex. The
/// union is not computed geometrically: every piece is emitted with the same
/// winding direction, so the nonzero rule fills the overlaps once and the seams
/// disappear. Round joins fall out of the discs for free, which is why there is
/// no miter arithmetic here.
pub fn stroke(subpaths: &[Vec<[f32; 2]>], width: f32) -> Vec<Vec<[f32; 2]>> {
    let half = (width * 0.5).max(0.05);
    let mut out = Vec::new();
    for path in subpaths {
        if path.len() < 2 {
            // A lone point still gets a dot, so a path of one `M` is visible.
            if let Some(&p) = path.first() {
                out.push(disc(p, half));
            }
            continue;
        }
        for pair in path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                continue;
            }
            let (nx, ny) = (-dy / len * half, dx / len * half);
            out.push(clockwise(vec![
                [a[0] + nx, a[1] + ny],
                [b[0] + nx, b[1] + ny],
                [b[0] - nx, b[1] - ny],
                [a[0] - nx, a[1] - ny],
            ]));
        }
        for &p in path {
            out.push(disc(p, half));
        }
    }
    out
}

/// A regular polygon approximating a circle, wound clockwise.
fn disc(centre: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    const SIDES: usize = 12;
    (0..SIDES)
        .map(|k| {
            let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
            [centre[0] + radius * a.cos(), centre[1] + radius * a.sin()]
        })
        .collect()
}

/// Reverses a polygon when its signed area is negative.
///
/// The stroke pieces have to agree on a direction or the nonzero rule would
/// cancel them where they overlap; normalising here means the segment code does
/// not have to reason about which way round a given segment runs.
fn clockwise(mut poly: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    let mut area = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    if area < 0.0 {
        poly.reverse();
    }
    poly
}

/// Rasterises a list of painted paths into one mask.
///
/// This is the entry point the icon table uses: each icon is a handful of
/// `(path data, paint)` pairs, and the result is a single alpha mask that can be
/// tinted with any colour at draw time.
pub fn render_parts(parts: &[(&str, Paint)], view: f32, width: u32, height: u32) -> Vec<u8> {
    let mut polys: Vec<Vec<[f32; 2]>> = Vec::new();
    for (d, paint) in parts {
        let Some(cmds) = parse(d) else { continue };
        let sub = flatten(&cmds, 0.4);
        match paint {
            Paint::Fill => polys.extend(sub),
            Paint::Stroke(w) => polys.extend(stroke(&sub, *w)),
        }
    }
    let sx = width as f32 / view;
    let sy = height as f32 / view;
    for path in &mut polys {
        for p in path.iter_mut() {
            p[0] *= sx;
            p[1] *= sy;
        }
    }
    rasterise(&polys, width, height)
}

/// Parses, flattens and rasterises one path into a `width` x `height` mask.
///
/// The path is scaled from its own `view` box to the output size, so the same
/// drawing works at 16 pixels and at 64.
pub fn render(d: &str, view: f32, width: u32, height: u32) -> Vec<u8> {
    let Some(cmds) = parse(d) else {
        return vec![0; (width * height) as usize];
    };
    let mut subpaths = flatten(&cmds, 0.4);
    let sx = width as f32 / view;
    let sy = height as f32 / view;
    for path in &mut subpaths {
        for p in path.iter_mut() {
            p[0] *= sx;
            p[1] *= sy;
        }
    }
    rasterise(&subpaths, width, height)
}

/// Rasterises several paths into one mask, so an icon can be built from parts.
pub fn render_all(paths: &[&str], view: f32, width: u32, height: u32) -> Vec<u8> {
    let mut all = Vec::new();
    for d in paths {
        if let Some(cmds) = parse(d) {
            all.extend(flatten(&cmds, 0.4));
        }
    }
    let sx = width as f32 / view;
    let sy = height as f32 / view;
    for path in &mut all {
        for p in path.iter_mut() {
            p[0] *= sx;
            p[1] *= sy;
        }
    }
    rasterise(&all, width, height)
}

/// A cursor over path data.
struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a [u8]) -> Self {
        Self { s, i: 0 }
    }

    fn skip_separators(&mut self) {
        while self.i < self.s.len() {
            let b = self.s[self.i];
            if b == b' ' || b == b',' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn peek_command(&mut self) -> Option<u8> {
        self.skip_separators();
        let b = *self.s.get(self.i)?;
        if b.is_ascii_alphabetic() {
            Some(b)
        } else {
            None
        }
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    /// Whether a number follows, without consuming it.
    fn has_number(&mut self) -> bool {
        self.skip_separators();
        matches!(self.s.get(self.i), Some(b) if *b == b'-' || *b == b'+' || *b == b'.' || b.is_ascii_digit())
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'-') | Some(b'+')) {
            self.i += 1;
        }
        let mut digits = false;
        while matches!(self.s.get(self.i), Some(b) if b.is_ascii_digit()) {
            self.i += 1;
            digits = true;
        }
        if matches!(self.s.get(self.i), Some(b'.')) {
            self.i += 1;
            while matches!(self.s.get(self.i), Some(b) if b.is_ascii_digit()) {
                self.i += 1;
                digits = true;
            }
        }
        if !digits {
            return None;
        }
        if matches!(self.s.get(self.i), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-') | Some(b'+')) {
                self.i += 1;
            }
            let mut exp_digits = false;
            while matches!(self.s.get(self.i), Some(b) if b.is_ascii_digit()) {
                self.i += 1;
                exp_digits = true;
            }
            if !exp_digits {
                self.i = save;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_parses_into_four_lines_and_a_close() {
        let cmds = parse("M 0 0 L 10 0 L 10 10 L 0 10 Z").expect("parses");
        assert_eq!(cmds.len(), 5);
        assert_eq!(cmds[0], Cmd::Move(0.0, 0.0));
        assert_eq!(cmds[4], Cmd::Close);
    }

    #[test]
    fn relative_commands_accumulate() {
        let cmds = parse("m 5 5 l 10 0 l 0 10 z").expect("parses");
        assert_eq!(cmds[0], Cmd::Move(5.0, 5.0));
        assert_eq!(cmds[1], Cmd::Line(15.0, 5.0));
        assert_eq!(cmds[2], Cmd::Line(15.0, 15.0));
    }

    #[test]
    fn a_repeated_pair_after_a_move_becomes_a_line() {
        let cmds = parse("M 0 0 10 0 10 10").expect("parses");
        assert_eq!(cmds[0], Cmd::Move(0.0, 0.0));
        assert_eq!(cmds[1], Cmd::Line(10.0, 0.0));
        assert_eq!(cmds[2], Cmd::Line(10.0, 10.0));
    }

    #[test]
    fn horizontal_and_vertical_shorthand_keep_the_other_axis() {
        let cmds = parse("M 3 4 H 9 V 12").expect("parses");
        assert_eq!(cmds[1], Cmd::Line(9.0, 4.0));
        assert_eq!(cmds[2], Cmd::Line(9.0, 12.0));
    }

    #[test]
    fn a_smooth_cubic_reflects_the_previous_control_point() {
        let cmds = parse("M 0 0 C 0 10 10 10 10 0 S 20 -10 20 0").expect("parses");
        let Cmd::Cubic(c) = cmds[2] else {
            panic!("expected a cubic")
        };
        // The previous second control point was (10, 10) and the current point
        // is (10, 0), so the reflection is (10, -10).
        assert_eq!((c[0], c[1]), (10.0, -10.0));
    }

    #[test]
    fn numbers_may_be_signed_decimal_and_exponential() {
        let cmds = parse("M -1.5 .5 L 1e2 -2E-1").expect("parses");
        assert_eq!(cmds[0], Cmd::Move(-1.5, 0.5));
        assert_eq!(cmds[1], Cmd::Line(100.0, -0.2));
    }

    #[test]
    fn a_truncated_path_is_rejected() {
        assert!(parse("M 0 0 L 10").is_none());
        assert!(parse("M 0 0 Q 1").is_none());
    }

    #[test]
    fn an_unknown_command_is_rejected() {
        assert!(parse("M 0 0 K 1 1").is_none());
    }

    #[test]
    fn an_arc_becomes_cubics_that_end_where_asked() {
        let cmds = parse("M 0 0 A 10 10 0 0 1 20 0").expect("parses");
        let mut last = [0.0f32, 0.0];
        for cmd in &cmds {
            if let Cmd::Cubic(c) = cmd {
                last = [c[4], c[5]];
            }
        }
        assert!((last[0] - 20.0).abs() < 0.01, "ended at {last:?}");
        assert!(last[1].abs() < 0.01, "ended at {last:?}");
    }

    #[test]
    fn a_zero_radius_arc_degrades_to_a_line() {
        let cmds = parse("M 0 0 A 0 0 0 0 1 10 10").expect("parses");
        assert_eq!(cmds[1], Cmd::Line(10.0, 10.0));
    }

    #[test]
    fn flattening_a_line_yields_its_endpoints() {
        let cmds = parse("M 0 0 L 10 0").unwrap();
        let paths = flatten(&cmds, 0.5);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], vec![[0.0, 0.0], [10.0, 0.0]]);
    }

    #[test]
    fn flattening_a_curve_yields_more_than_two_points() {
        let cmds = parse("M 0 0 C 0 20 20 20 20 0").unwrap();
        let paths = flatten(&cmds, 0.2);
        assert!(paths[0].len() > 4, "got {} points", paths[0].len());
    }

    #[test]
    fn a_closed_subpath_returns_to_its_start() {
        let cmds = parse("M 0 0 L 10 0 L 10 10 Z").unwrap();
        let paths = flatten(&cmds, 0.5);
        assert_eq!(paths[0].last(), Some(&[0.0, 0.0]));
    }

    #[test]
    fn a_filled_square_covers_its_middle_and_not_its_corner() {
        let mask = render("M 2 2 H 22 V 22 H 2 Z", 24.0, 24, 24);
        assert_eq!(mask.len(), 24 * 24);
        assert_eq!(mask[12 * 24 + 12], 255, "the middle is solid");
        assert_eq!(mask[0], 0, "the corner is empty");
    }

    #[test]
    fn a_hole_drawn_the_other_way_round_is_empty() {
        // Outer square clockwise, inner square anticlockwise: nonzero winding
        // leaves the middle at zero.
        let mask = render(
            "M 2 2 H 22 V 22 H 2 Z M 8 8 V 16 H 16 V 8 Z",
            24.0,
            24,
            24,
        );
        assert_eq!(mask[12 * 24 + 12], 0, "the hole is empty");
        assert_eq!(mask[5 * 24 + 12], 255, "the ring is solid");
    }

    #[test]
    fn an_edge_that_crosses_a_pixel_diagonally_is_partly_covered() {
        // A triangle whose hypotenuse passes through the middle of the image.
        let mask = render("M 0 0 L 24 0 L 24 24 Z", 24.0, 24, 24);
        // A pixel straddling the hypotenuse must be neither empty nor solid.
        let mut partial = 0;
        for &v in &mask {
            if v > 0 && v < 255 {
                partial += 1;
            }
        }
        assert!(partial > 10, "only {partial} partly covered pixels");
    }

    #[test]
    fn rendering_at_a_larger_size_keeps_the_shape() {
        let small = render("M 2 2 H 22 V 22 H 2 Z", 24.0, 24, 24);
        let large = render("M 2 2 H 22 V 22 H 2 Z", 24.0, 96, 96);
        assert_eq!(small[12 * 24 + 12], 255);
        assert_eq!(large[48 * 96 + 48], 255);
        assert_eq!(large[0], 0);
    }

    #[test]
    fn a_malformed_path_renders_empty_rather_than_panicking() {
        let mask = render("M 0 0 L", 24.0, 8, 8);
        assert_eq!(mask, vec![0; 64]);
    }

    #[test]
    fn several_paths_combine_into_one_mask() {
        let mask = render_all(&["M 0 0 H 10 V 10 H 0 Z", "M 14 14 H 24 V 24 H 14 Z"], 24.0, 24, 24);
        assert_eq!(mask[5 * 24 + 5], 255);
        assert_eq!(mask[19 * 24 + 19], 255);
        assert_eq!(mask[12 * 24 + 12], 0);
    }

    #[test]
    fn an_empty_path_list_renders_empty() {
        assert_eq!(render_all(&[], 24.0, 4, 4), vec![0; 16]);
    }
}
