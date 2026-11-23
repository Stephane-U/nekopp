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

//! A minimal PDF writer for printed pages.
//!
//! There is no print backend in the process: the way to get ink on paper on
//! Linux is to hand a file to CUPS, and the format CUPS handles best without a
//! filter chain is PDF. So printing is two steps -- lay the document out (see
//! [`crate::print`]), then write those pages as a PDF and pipe it to `lp`.
//!
//! The writer is deliberately small. It emits one page object per page, one
//! content stream per page, and two base-14 fonts: Courier and Courier-Bold.
//! Base-14 fonts need no embedding, which is what keeps this under a few
//! hundred lines instead of a font subsetter.
//!
//! Text is encoded as WinAnsi (Latin-1 plus a handful of typographic
//! characters). A character outside that set becomes `?`: a printed page with
//! one wrong glyph is better than a PDF that will not open, and the editor's
//! own display is where a missing glyph should be noticed.

use crate::print::{Layout, PageSetup};

/// The width of a Courier glyph, as a fraction of the font size.
///
/// Courier is one of the base-14 fonts and is strictly monospaced at 0.6 em,
/// which is why the layout can be pure arithmetic.
pub const COURIER_WIDTH: f32 = 0.6;

/// Escapes a string for a PDF literal and encodes it as WinAnsi bytes.
///
/// The three characters that must be escaped are the ones that would otherwise
/// end the literal or start an escape. Everything above Latin-1 becomes `?`.
fn escape(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '(' => out.extend_from_slice(b"\\("),
            ')' => out.extend_from_slice(b"\\)"),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' | '\r' => out.push(b' '),
            _ => {
                let code = ch as u32;
                if code < 0x100 {
                    out.push(code as u8);
                } else {
                    out.push(b'?');
                }
            }
        }
    }
    out
}

/// One run of text at a position, in points from the bottom-left corner.
struct Run {
    x: f32,
    y: f32,
    size: f32,
    bold: bool,
    text: String,
}

/// Builds the content stream for one page.
fn content(runs: &[Run]) -> Vec<u8> {
    let mut out = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        out.extend_from_slice(b"BT\n");
        out.extend_from_slice(format!("/{} {} Tf\n", if run.bold { "F2" } else { "F1" }, run.size).as_bytes());
        out.extend_from_slice(format!("1 0 0 1 {:.2} {:.2} Tm\n", run.x, run.y).as_bytes());
        out.push(b'(');
        out.extend_from_slice(&escape(&run.text));
        out.extend_from_slice(b") Tj\nET\n");
    }
    out
}

/// The x position of a zone: left, centre or right.
fn zone_x(zone: usize, text: &str, char_width: f32, setup: &PageSetup) -> f32 {
    let (width, _) = setup.paper.size(setup.landscape);
    let text_width = text.chars().count() as f32 * char_width;
    match zone {
        0 => setup.margins.left,
        1 => ((width - text_width) / 2.0).max(setup.margins.left),
        _ => (width - setup.margins.right - text_width).max(setup.margins.left),
    }
}

/// Renders a laid-out document as a PDF.
///
/// `char_width` must be the same width the layout was computed with, or the
/// header zones will not line up with the text.
pub fn render(layout: &Layout, setup: &PageSetup, char_width: f32) -> Vec<u8> {
    let (width, height) = setup.paper.size(setup.landscape);
    let line_height = setup.line_height;
    let size = setup.font_size;
    let digits = layout
        .pages
        .iter()
        .flat_map(|page| page.lines.iter())
        .filter_map(|line| line.number)
        .max()
        .unwrap_or(1)
        .to_string()
        .len();

    // One content stream per page.
    let mut streams: Vec<Vec<u8>> = Vec::new();
    for page in &layout.pages {
        let mut runs: Vec<Run> = Vec::new();
        // The header sits in the top margin, the footer in the bottom one.
        let header_y = height - setup.margins.top + 0.35 * line_height;
        for (zone, text) in page.header.iter().enumerate() {
            if !text.is_empty() {
                runs.push(Run {
                    x: zone_x(zone, text, char_width, setup),
                    y: header_y,
                    size,
                    bold: true,
                    text: text.clone(),
                });
            }
        }
        let footer_y = setup.margins.bottom - 0.65 * line_height;
        for (zone, text) in page.footer.iter().enumerate() {
            if !text.is_empty() {
                runs.push(Run {
                    x: zone_x(zone, text, char_width, setup),
                    y: footer_y,
                    size,
                    bold: false,
                    text: text.clone(),
                });
            }
        }
        for (row, line) in page.lines.iter().enumerate() {
            let baseline = height - setup.margins.top - (row as f32 + 1.0) * line_height
                + 0.25 * line_height;
            if let Some(number) = line.number {
                let text = number.to_string();
                // Right-aligned in the gutter, so the digits line up.
                let x = setup.margins.left + (digits as f32 - text.chars().count() as f32) * char_width;
                runs.push(Run {
                    x,
                    y: baseline,
                    size,
                    bold: false,
                    text,
                });
            }
            let x = if setup.line_numbers {
                setup.margins.left + (digits + 2) as f32 * char_width
            } else {
                setup.margins.left
            };
            runs.push(Run {
                x,
                y: baseline,
                size,
                bold: false,
                text: line.text.clone(),
            });
        }
        streams.push(content(&runs));
    }

    // Objects: 1 catalog, 2 pages, 3 Courier, 4 Courier-Bold, then a page and a
    // stream per page.
    let page_count = streams.len();
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 5 + index * 2))
        .collect();
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            page_count
        )
        .into_bytes(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    for (index, stream) in streams.iter().enumerate() {
        let stream_object = 6 + index * 2;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                width, height, stream_object
            )
            .into_bytes(),
        );
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        objects.push(body);
    }

    // Assemble, recording the offset of each object for the cross-reference
    // table.
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::{self, Fields, PageSetup};

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

    fn pdf(text: &str) -> Vec<u8> {
        let setup = setup();
        let layout = print::layout(text, &setup, 6.0, &fields());
        render(&layout, &setup, 6.0)
    }

    fn as_text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn the_file_starts_with_a_header_and_ends_with_eof() {
        let text = as_text(&pdf("hello"));
        assert!(text.starts_with("%PDF-1.4\n"));
        assert!(text.ends_with("%%EOF\n"));
    }

    #[test]
    fn the_catalog_and_page_tree_are_present() {
        let text = as_text(&pdf("hello"));
        assert!(text.contains("/Type /Catalog /Pages 2 0 R"));
        assert!(text.contains("/Type /Pages /Kids [5 0 R] /Count 1"));
    }

    #[test]
    fn both_fonts_are_declared() {
        let text = as_text(&pdf("hello"));
        assert!(text.contains("/BaseFont /Courier "));
        assert!(text.contains("/BaseFont /Courier-Bold "));
    }

    #[test]
    fn the_media_box_is_the_paper_size() {
        let text = as_text(&pdf("hello"));
        assert!(text.contains("/MediaBox [0 0 595.00 842.00]"), "{text}");
    }

    #[test]
    fn landscape_swaps_the_media_box() {
        let mut setup = setup();
        setup.landscape = true;
        let layout = print::layout("hello", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        assert!(text.contains("/MediaBox [0 0 842.00 595.00]"), "{text}");
    }

    #[test]
    fn the_text_is_drawn() {
        let text = as_text(&pdf("hello"));
        assert!(text.contains("(hello) Tj"), "{text}");
    }

    #[test]
    fn a_long_document_makes_one_page_object_per_page() {
        let mut setup = setup();
        setup.header = String::new();
        setup.footer = String::new();
        let rows = setup.rows();
        let body = (0..rows + 1).map(|i| format!("{i}")).collect::<Vec<_>>().join("\n");
        let layout = print::layout(&body, &setup, 6.0, &fields());
        assert_eq!(layout.len(), 2);
        let text = as_text(&render(&layout, &setup, 6.0));
        assert!(text.contains("/Count 2"), "{text}");
        assert_eq!(text.matches("/Type /Page ").count(), 2);
    }

    #[test]
    fn the_page_count_in_the_trailer_matches_the_objects() {
        let text = as_text(&pdf("hello"));
        // Four fixed objects plus a page and a stream.
        assert!(text.contains("/Size 7"), "{text}");
    }

    /// Finds a byte pattern, so offsets can be checked against the real file
    /// rather than a lossy string whose length the binary header changes.
    fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
        bytes.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn the_cross_reference_table_points_at_the_objects() {
        let bytes = pdf("hello");
        let xref = find(&bytes, b"xref\n0 7\n").expect("an xref table");
        let table = String::from_utf8_lossy(&bytes[xref..]).into_owned();
        // The first entry is the free-list head, then one per object.
        let entries: Vec<&str> = table.lines().skip(3).take(6).collect();
        assert_eq!(entries.len(), 6);
        for (index, entry) in entries.iter().enumerate() {
            let offset: usize = entry[..10].parse().expect("an offset");
            let expected = format!("{} 0 obj", index + 1);
            assert!(
                bytes[offset..].starts_with(expected.as_bytes()),
                "object {} is not at {}",
                index + 1,
                offset
            );
        }
    }

    #[test]
    fn the_startxref_points_at_the_table() {
        let bytes = pdf("hello");
        let start = find(&bytes, b"startxref\n").expect("a startxref");
        let tail = String::from_utf8_lossy(&bytes[start + 10..]).into_owned();
        let offset: usize = tail.lines().next().unwrap().parse().expect("an offset");
        assert!(bytes[offset..].starts_with(b"xref"));
    }

    #[test]
    fn parentheses_and_backslashes_are_escaped() {
        let text = as_text(&pdf("f(a)\\b"));
        assert!(text.contains("(f\\(a\\)\\\\b) Tj"), "{text}");
    }

    #[test]
    fn a_character_outside_latin1_becomes_a_question_mark() {
        let text = as_text(&pdf("a\u{4e00}b"));
        assert!(text.contains("(a?b) Tj"), "{text}");
    }

    #[test]
    fn a_latin1_character_is_written_as_one_byte() {
        let bytes = pdf("caf\u{e9}");
        // The e-acute is 0xE9 in WinAnsi.
        assert!(bytes.windows(4).any(|w| w == b"caf\xe9"));
    }

    #[test]
    fn the_header_is_drawn_bold_and_the_footer_is_not() {
        let mut setup = setup();
        setup.header = "title".to_string();
        setup.footer = "$(PAGE)".to_string();
        let layout = print::layout("body", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        assert!(text.contains("/F2 10 Tf"), "{text}");
        assert!(text.contains("(title) Tj"), "{text}");
        assert!(text.contains("(1) Tj"), "{text}");
    }

    #[test]
    fn the_header_sits_above_the_text_area() {
        let mut setup = setup();
        setup.header = "title".to_string();
        let layout = print::layout("body", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        // 842 - 56.7 + 0.35 * 12 = 789.5.
        assert!(text.contains("1 0 0 1 56.70 789.50 Tm"), "{text}");
    }

    #[test]
    fn the_first_body_row_sits_below_the_top_margin() {
        let text = as_text(&pdf("body"));
        // 842 - 56.7 - 12 + 3 = 776.3.
        assert!(text.contains("1 0 0 1 56.70 776.30 Tm"), "{text}");
    }

    #[test]
    fn the_footer_sits_in_the_bottom_margin() {
        let text = as_text(&pdf("body"));
        // 56.7 - 0.65 * 12 = 48.9.
        assert!(text.contains("1 0 0 1 56.70 48.90 Tm"), "{text}");
    }

    #[test]
    fn a_centred_zone_is_centred() {
        let mut setup = setup();
        setup.footer = "\t$(PAGE)".to_string();
        let layout = print::layout("body", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        // One character at 6 points, centred on 595: (595 - 6) / 2 = 294.5.
        assert!(text.contains("1 0 0 1 294.50 48.90 Tm"), "{text}");
    }

    #[test]
    fn a_right_zone_ends_at_the_right_margin() {
        let mut setup = setup();
        setup.footer = "\t\t$(PAGE)".to_string();
        let layout = print::layout("body", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        // 595 - 56.7 - 6 = 532.3.
        assert!(text.contains("1 0 0 1 532.30 48.90 Tm"), "{text}");
    }

    #[test]
    fn line_numbers_are_drawn_in_the_gutter() {
        let mut setup = setup();
        setup.line_numbers = true;
        let layout = print::layout("one\ntwo", &setup, 6.0, &fields());
        let text = as_text(&render(&layout, &setup, 6.0));
        // The number at the left margin, the text two columns further in.
        assert!(text.contains("1 0 0 1 56.70 776.30 Tm\n(1) Tj"), "{text}");
        assert!(text.contains("1 0 0 1 74.70 776.30 Tm\n(one) Tj"), "{text}");
    }

    #[test]
    fn an_empty_document_still_makes_a_page() {
        let text = as_text(&pdf(""));
        assert!(text.contains("/Count 1"), "{text}");
    }

    #[test]
    fn the_stream_length_matches_the_stream() {
        let bytes = pdf("hello");
        let text = as_text(&bytes);
        let start = text.find("/Length ").expect("a length");
        let length: usize = text[start + 8..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .expect("a number");
        let body = text.find("stream\n").expect("a stream") + 7;
        // The declared length must cover exactly the drawing commands, so a
        // reader stops before the `endstream` keyword.
        assert!(text[body + length..].starts_with("\nendstream"));
        assert!(text[body..body + length].contains("(hello) Tj"));
    }

    #[test]
    fn courier_is_six_tenths_of_an_em() {
        assert!((COURIER_WIDTH - 0.6).abs() < f32::EPSILON);
    }
}
