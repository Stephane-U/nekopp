//! The toolbar icons, as vector drawings.
//!
//! Each icon is a set of SVG path strings in a 24x24 box, split into two
//! colours: `ink` for the body of the drawing and `accent` for the part that
//! carries the meaning — the plus on New, the arrow on Undo, the dot on Record.
//! Notepad++ draws its toolbar the same way, and the split is what lets the
//! icons follow the theme instead of being baked into a bitmap.
//!
//! The set and its order are Notepad++'s: the same thirty-two buttons, in the
//! same nine groups, so a user who knows where Print is knows where it is here.
//!
//! Nothing here is rasterised at compile time. `vector::render_parts` turns a
//! drawing into an alpha mask at whatever size the display asks for, which is
//! what keeps the icons sharp on a high-density screen.

use crate::vector::Paint;

/// The stroke width of the body of an icon, in the 24-unit box.
const INK: Paint = Paint::Stroke(1.7);
/// The stroke width of the accent. Slightly heavier, so the part that carries
/// the meaning still reads when the icon is 16 pixels across.
const ACCENT: Paint = Paint::Stroke(2.1);
/// A solid shape.
const SOLID: Paint = Paint::Fill;
/// The stroke width of a tab mark.
///
/// Heavier than [`INK`]: a tab mark is drawn at fourteen pixels rather than
/// twenty-three, and a 1.7-unit stroke at that size thins to a hairline.
const TAB: Paint = Paint::Stroke(2.2);

/// One icon.
pub struct Icon {
    /// The name the toolbar refers to it by.
    pub name: &'static str,
    /// The body of the drawing.
    pub ink: &'static [(&'static str, Paint)],
    /// The part that carries the meaning.
    pub accent: &'static [(&'static str, Paint)],
}

/// The icon table, in Notepad++'s toolbar order.
pub const ICONS: &[Icon] = &[
    Icon {
        name: "new",
        ink: &[
            ("M 5 2.5 H 13 L 17.5 7 V 21.5 H 5 Z", INK),
            ("M 13 2.5 V 7 H 17.5", INK),
        ],
        accent: &[("M 16 12.5 V 19", ACCENT), ("M 12.75 15.75 H 19.25", ACCENT)],
    },
    Icon {
        name: "open",
        ink: &[],
        accent: &[
            ("M 2.5 19.5 V 6.5 H 9.5 L 11.5 9 H 19 V 12", ACCENT),
            ("M 2.5 19.5 L 5.5 12 H 21.5 L 18.5 19.5 Z", ACCENT),
        ],
    },
    Icon {
        name: "save",
        ink: &[("M 3.5 3.5 H 17 L 20.5 7 V 20.5 H 3.5 Z", INK)],
        accent: &[
            ("M 7 3.5 H 15 V 9 H 7 Z", ACCENT),
            ("M 7 14 H 17 V 20.5 H 7 Z", ACCENT),
        ],
    },
    Icon {
        name: "saveall",
        ink: &[
            ("M 2.5 7.5 H 12 L 15 10.5 V 21.5 H 2.5 Z", INK),
            ("M 7.5 2.5 H 17 L 20.5 6 V 17 H 7.5 Z", INK),
        ],
        accent: &[
            ("M 10.5 2.5 H 16 V 7 H 10.5 Z", ACCENT),
            ("M 10.5 12 H 17.5 V 17 H 10.5 Z", ACCENT),
        ],
    },
    Icon {
        name: "close",
        ink: &[
            ("M 4.5 2.5 H 13 L 17.5 7 V 21.5 H 4.5 Z", INK),
            ("M 13 2.5 V 7 H 17.5", INK),
        ],
        accent: &[("M 8 11 L 14 17", ACCENT), ("M 14 11 L 8 17", ACCENT)],
    },
    Icon {
        name: "closeall",
        ink: &[
            ("M 2.5 6.5 H 9.5 L 13 10 V 21.5 H 2.5 Z", INK),
            ("M 7.5 2.5 H 14.5 L 18 6 V 17.5 H 7.5 Z", INK),
        ],
        accent: &[("M 10 9.5 L 15.5 15", ACCENT), ("M 15.5 9.5 L 10 15", ACCENT)],
    },
    Icon {
        name: "print",
        ink: &[
            ("M 3.5 9 H 20.5 V 16 H 3.5 Z", INK),
            ("M 6.5 9 V 3.5 H 17.5 V 9", INK),
        ],
        accent: &[("M 6.5 13.5 H 17.5 V 20.5 H 6.5 Z", ACCENT)],
    },
    Icon {
        name: "cut",
        ink: &[],
        accent: &[
            ("M 6.5 3.5 L 15.5 17", ACCENT),
            ("M 17.5 3.5 L 8.5 17", ACCENT),
            ("M 3.5 20 A 2.2 2.2 0 1 0 7.9 20 A 2.2 2.2 0 1 0 3.5 20", ACCENT),
            ("M 16.1 20 A 2.2 2.2 0 1 0 20.5 20 A 2.2 2.2 0 1 0 16.1 20", ACCENT),
        ],
    },
    Icon {
        name: "copy",
        ink: &[],
        accent: &[
            ("M 9 2.5 H 19 V 15", ACCENT),
            ("M 5 6.5 H 15 V 21.5 H 5 Z", ACCENT),
        ],
    },
    Icon {
        name: "paste",
        ink: &[("M 9 2.5 H 15 V 6.5 H 9 Z", INK)],
        accent: &[("M 5 4.5 H 19 V 21.5 H 5 Z", ACCENT)],
    },
    Icon {
        name: "undo",
        ink: &[],
        accent: &[
            ("M 4.5 9.5 H 14 A 5.5 5.5 0 0 1 14 20.5 H 9", ACCENT),
            ("M 8.5 5 L 4 9.5 L 8.5 14", ACCENT),
        ],
    },
    Icon {
        name: "redo",
        ink: &[],
        accent: &[
            ("M 19.5 9.5 H 10 A 5.5 5.5 0 0 0 10 20.5 H 15", ACCENT),
            ("M 15.5 5 L 20 9.5 L 15.5 14", ACCENT),
        ],
    },
    Icon {
        name: "find",
        ink: &[],
        accent: &[
            ("M 3.5 10.5 A 7 7 0 1 0 17.5 10.5 A 7 7 0 1 0 3.5 10.5", ACCENT),
            ("M 15.5 15.5 L 20.5 20.5", ACCENT),
        ],
    },
    Icon {
        name: "replace",
        ink: &[],
        accent: &[
            ("M 4.5 9 A 7.5 7.5 0 0 1 19.5 9", ACCENT),
            ("M 16 5.5 L 19.5 9 L 16 12.5", ACCENT),
            ("M 19.5 15 A 7.5 7.5 0 0 1 4.5 15", ACCENT),
            ("M 8 11.5 L 4.5 15 L 8 18.5", ACCENT),
        ],
    },
    Icon {
        name: "zoom_in",
        ink: &[("M 10.5 7 V 14", INK), ("M 7 10.5 H 14", INK)],
        accent: &[
            ("M 3.5 10.5 A 7 7 0 1 0 17.5 10.5 A 7 7 0 1 0 3.5 10.5", ACCENT),
            ("M 15.5 15.5 L 20.5 20.5", ACCENT),
        ],
    },
    Icon {
        name: "zoom_out",
        ink: &[("M 7 10.5 H 14", INK)],
        accent: &[
            ("M 3.5 10.5 A 7 7 0 1 0 17.5 10.5 A 7 7 0 1 0 3.5 10.5", ACCENT),
            ("M 15.5 15.5 L 20.5 20.5", ACCENT),
        ],
    },
    Icon {
        name: "sync_v",
        ink: &[
            ("M 3.5 2.5 H 20.5 V 8 H 3.5 Z", INK),
            ("M 3.5 16 H 20.5 V 21.5 H 3.5 Z", INK),
        ],
        accent: &[
            ("M 12 9 V 15", ACCENT),
            ("M 9 11.5 L 12 8.5 L 15 11.5", ACCENT),
            ("M 9 12.5 L 12 15.5 L 15 12.5", ACCENT),
        ],
    },
    Icon {
        name: "sync_h",
        ink: &[
            ("M 2.5 3.5 H 8 V 20.5 H 2.5 Z", INK),
            ("M 16 3.5 H 21.5 V 20.5 H 16 Z", INK),
        ],
        accent: &[
            ("M 9 12 H 15", ACCENT),
            ("M 11.5 9 L 8.5 12 L 11.5 15", ACCENT),
            ("M 12.5 9 L 15.5 12 L 12.5 15", ACCENT),
        ],
    },
    Icon {
        name: "wrap",
        ink: &[],
        accent: &[
            ("M 3.5 7 H 15 A 4 4 0 0 1 15 15 H 7", ACCENT),
            ("M 10.5 11.5 L 7 15 L 10.5 18.5", ACCENT),
        ],
    },
    Icon {
        name: "all_chars",
        ink: &[],
        accent: &[
            ("M 8 4 H 17 V 20", ACCENT),
            ("M 8 4 H 12.5 A 4 4 0 0 1 12.5 12 H 8 Z", ACCENT),
        ],
    },
    Icon {
        name: "indent_guide",
        ink: &[
            ("M 8 5 H 20.5", INK),
            ("M 8 12 H 20.5", INK),
            ("M 8 19 H 20.5", INK),
        ],
        accent: &[("M 4 2.5 V 21.5", ACCENT)],
    },
    Icon {
        name: "udl",
        ink: &[],
        accent: &[
            ("M 8 7 L 3 12 L 8 17", ACCENT),
            ("M 16 7 L 21 12 L 16 17", ACCENT),
            ("M 13.5 5 L 10.5 19", ACCENT),
        ],
    },
    Icon {
        name: "doc_map",
        ink: &[],
        accent: &[
            ("M 12 6 C 9 3.5 5 3.5 2.5 4.5 V 19 C 5 18 9 18 12 20.5", ACCENT),
            ("M 12 6 C 15 3.5 19 3.5 21.5 4.5 V 19 C 19 18 15 18 12 20.5", ACCENT),
            ("M 12 6 V 20.5", ACCENT),
        ],
    },
    Icon {
        name: "doc_list",
        ink: &[],
        accent: &[
            ("M 5 2.5 H 14 L 19 7.5 V 21.5 H 5 Z", ACCENT),
            ("M 14 2.5 V 7.5 H 19", ACCENT),
            ("M 8 11 H 16", ACCENT),
            ("M 8 14.5 H 16", ACCENT),
            ("M 8 18 H 13", ACCENT),
        ],
    },
    Icon {
        name: "func_list",
        ink: &[
            ("M 10 20.5 V 6 A 2.5 2.5 0 0 1 12.5 3.5", INK),
            ("M 6.5 9 H 12.5", INK),
        ],
        accent: &[("M 14 9 L 20 19", ACCENT), ("M 20 9 L 14 19", ACCENT)],
    },
    Icon {
        name: "file_browser",
        ink: &[
            ("M 2.5 4.5 H 21.5 V 17.5 H 2.5 Z", INK),
            ("M 9 17.5 V 21", INK),
            ("M 15 17.5 V 21", INK),
            ("M 7 21 H 17", INK),
        ],
        accent: &[("M 6 8 H 11 L 12.5 9.5 H 18 V 15 H 6 Z", ACCENT)],
    },
    Icon {
        name: "monitoring",
        ink: &[],
        accent: &[
            ("M 2.5 12 C 6 6 18 6 21.5 12 C 18 18 6 18 2.5 12 Z", ACCENT),
            ("M 9 12 A 3 3 0 1 0 15 12 A 3 3 0 1 0 9 12", ACCENT),
        ],
    },
    Icon {
        name: "start_record",
        ink: &[],
        accent: &[("M 4 12 A 8 8 0 1 0 20 12 A 8 8 0 1 0 4 12", SOLID)],
    },
    Icon {
        name: "stop_record",
        ink: &[],
        accent: &[("M 5 5 H 19 V 19 H 5 Z", SOLID)],
    },
    Icon {
        name: "playback",
        ink: &[("M 3.5 3.5 H 20.5 V 20.5 H 3.5 Z", INK)],
        accent: &[("M 9.5 7.5 L 17 12 L 9.5 16.5 Z", SOLID)],
    },
    Icon {
        name: "run_multiple",
        ink: &[
            ("M 2.5 6 H 11", INK),
            ("M 2.5 11 H 11", INK),
            ("M 2.5 16 H 8", INK),
        ],
        accent: &[("M 13 8 L 21 13 L 13 18 Z", SOLID)],
    },
    Icon {
        name: "save_macro",
        ink: &[
            ("M 3.5 3.5 H 17 L 20.5 7 V 20.5 H 3.5 Z", INK),
            ("M 7 3.5 H 15 V 9 H 7 Z", INK),
        ],
        accent: &[("M 10 12 L 17 16 L 10 20 Z", SOLID)],
    },
    // The four below are not toolbar buttons. They are the marks Notepad++ puts
    // on a tab: the close cross, the pin in its two states, and the pencil that
    // says the document has unsaved edits. They live in the same table because
    // they are drawn by the same rasteriser into the same atlas, and a second
    // table would mean a second atlas for four glyphs.
    Icon {
        name: "tab_close",
        ink: &[("M 7 7 L 17 17", TAB), ("M 17 7 L 7 17", TAB)],
        accent: &[],
    },
    Icon {
        name: "tab_pin",
        ink: &[
            ("M 7 3.5 H 17 L 15.5 9 H 8.5 Z", TAB),
            ("M 6 9 H 18 V 11.5 H 6 Z", TAB),
            ("M 12 11.5 V 20.5", TAB),
        ],
        accent: &[],
    },
    Icon {
        name: "tab_pin_on",
        ink: &[("M 12 11.5 V 20.5", TAB)],
        accent: &[
            ("M 7 3.5 H 17 L 15.5 9 H 8.5 Z", SOLID),
            ("M 6 9 H 18 V 11.5 H 6 Z", SOLID),
        ],
    },
    Icon {
        name: "tab_unsaved",
        ink: &[],
        accent: &[
            ("M 5.5 18.5 L 7 14 L 15.5 5.5 L 18.5 8.5 L 10 17 Z", SOLID),
            ("M 7 14 L 10 17", TAB),
        ],
    },
    // The pair that scrolls a strip too full to show every tab. They are the
    // only marks in this table drawn from a fixed rectangle rather than from
    // the tab they belong to, which is why they take their own argument.
    Icon {
        name: "tab_left",
        ink: &[("M 15 5 L 8 12 L 15 19", TAB)],
        accent: &[],
    },
    Icon {
        name: "tab_right",
        ink: &[("M 9 5 L 16 12 L 9 19", TAB)],
        accent: &[],
    },
];

/// The box the drawings are laid out in.
pub const VIEW: f32 = 24.0;

/// The index of the icon called `name`.
pub fn index(name: &str) -> Option<usize> {
    ICONS.iter().position(|icon| icon.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector;

    #[test]
    fn every_icon_has_a_unique_name() {
        let mut seen: Vec<&str> = Vec::new();
        for icon in ICONS {
            assert!(!icon.name.is_empty());
            assert!(!seen.contains(&icon.name), "{} twice", icon.name);
            seen.push(icon.name);
        }
    }

    #[test]
    fn every_icon_draws_something() {
        for icon in ICONS {
            let mut parts: Vec<(&str, Paint)> = Vec::new();
            parts.extend(icon.ink.iter().copied());
            parts.extend(icon.accent.iter().copied());
            assert!(!parts.is_empty(), "{} is empty", icon.name);
            let mask = vector::render_parts(&parts, VIEW, 24, 24);
            let covered = mask.iter().filter(|&&v| v > 8).count();
            assert!(covered > 20, "{} covers only {covered} pixels", icon.name);
        }
    }

    #[test]
    fn every_path_parses() {
        for icon in ICONS {
            for (d, _) in icon.ink.iter().chain(icon.accent.iter()) {
                assert!(
                    vector::parse(d).is_some(),
                    "{} has a path that does not parse: {d}",
                    icon.name
                );
            }
        }
    }

    #[test]
    fn every_icon_stays_inside_its_box() {
        for icon in ICONS {
            for (d, _) in icon.ink.iter().chain(icon.accent.iter()) {
                let cmds = vector::parse(d).expect("parses");
                for path in vector::flatten(&cmds, 0.4) {
                    for p in path {
                        assert!(
                            (-1.0..=VIEW + 1.0).contains(&p[0])
                                && (-1.0..=VIEW + 1.0).contains(&p[1]),
                            "{} has a point at {p:?}",
                            icon.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_table_is_notepad_plus_plus_s_toolbar_in_order() {
        let names: Vec<&str> = ICONS.iter().map(|i| i.name).collect();
        assert_eq!(
            names,
            vec![
                "new",
                "open",
                "save",
                "saveall",
                "close",
                "closeall",
                "print",
                "cut",
                "copy",
                "paste",
                "undo",
                "redo",
                "find",
                "replace",
                "zoom_in",
                "zoom_out",
                "sync_v",
                "sync_h",
                "wrap",
                "all_chars",
                "indent_guide",
                "udl",
                "doc_map",
                "doc_list",
                "func_list",
                "file_browser",
                "monitoring",
                "start_record",
                "stop_record",
                "playback",
                "run_multiple",
                "save_macro",
                "tab_close",
                "tab_pin",
                "tab_pin_on",
                "tab_unsaved",
                "tab_left",
                "tab_right",
            ]
        );
    }

    #[test]
    fn looking_up_a_name_finds_it() {
        assert_eq!(index("new"), Some(0));
        assert_eq!(index("save_macro"), Some(31));
        assert_eq!(index("nope"), None);
    }

    #[test]
    fn an_icon_renders_at_any_size() {
        let icon = &ICONS[0];
        let mut parts: Vec<(&str, Paint)> = Vec::new();
        parts.extend(icon.ink.iter().copied());
        parts.extend(icon.accent.iter().copied());
        for size in [12u32, 16, 24, 48] {
            let mask = vector::render_parts(&parts, VIEW, size, size);
            assert_eq!(mask.len(), (size * size) as usize);
            assert!(mask.iter().any(|&v| v > 128), "{size}px drew nothing");
        }
    }
}
