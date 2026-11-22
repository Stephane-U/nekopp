//! The line commands, by name, without a window.
//!
//! [`crate::lines`] holds the functions; this module is the table that names
//! them and the one place a name is turned into a call. The names are the action
//! names the keymap file already uses -- `sort_asc`, `trim_trailing` -- so a
//! command found by `list_shortcuts`, written in `shortcuts.xml`, pressed as a
//! chord, or sent by an MCP client is the same word for the same function.
//!
//! ## What is here, and what is not
//!
//! A command belongs in this table when its whole result is a function of the
//! text it is given. That is why sorting, trimming, case and comment markers are
//! here and why `duplicate_line` is not: duplicating "the line" needs a caret to
//! say which line, and a caller that sends text instead of a caret has no answer.
//! The caret commands are reachable in a running window through its API
//! (`npp --call`), which is where a caret exists.

use std::fmt;

use crate::lines::{self, Case, SortKey, SortOptions, SortOrder};

/// An option a command reads.
///
/// The set is closed rather than a string key so an option added to [`Options`]
/// cannot be spelled wrongly at a call site, and so the JSON schema an MCP client
/// is shown is generated from the same list the dispatcher uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// Which comparison `sort_lines` uses.
    SortKey,
    /// Which direction `sort_lines` sorts in.
    SortOrder,
    /// Whether `sort_lines` tells `A` from `a`.
    CaseSensitive,
    /// Tab width, in columns, for the tab/space and indent commands.
    Width,
    /// What `split_lines` splits on.
    Delimiter,
    /// The marker `toggle_comment` and `single_line_comment` add.
    Prefix,
    /// What `block_comment` opens with.
    BlockOpen,
    /// What `block_comment` closes with.
    BlockClose,
    /// The seed `random_case` and `randomize_line_order` shuffle with.
    Seed,
}

impl Param {
    /// The spelling the API uses, which is also the JSON field name.
    pub fn name(self) -> &'static str {
        match self {
            Param::SortKey => "sort_key",
            Param::SortOrder => "sort_order",
            Param::CaseSensitive => "case_sensitive",
            Param::Width => "width",
            Param::Delimiter => "delimiter",
            Param::Prefix => "prefix",
            Param::BlockOpen => "block_open",
            Param::BlockClose => "block_close",
            Param::Seed => "seed",
        }
    }
}

/// One command that is a function of text.
#[derive(Debug)]
pub struct Command {
    /// The action name, as `npp`'s keymap spells it.
    pub name: &'static str,
    /// The options this command reads, in the order a schema should show them.
    pub params: &'static [Param],
}

const fn command(name: &'static str, params: &'static [Param]) -> Command {
    Command { name, params }
}

const WIDTH: &[Param] = &[Param::Width];
const COMMENT: &[Param] = &[Param::Prefix];
const BLOCK: &[Param] = &[Param::BlockOpen, Param::BlockClose];
const SORT: &[Param] = &[Param::SortKey, Param::SortOrder, Param::CaseSensitive];

/// Every command that is a function of text.
///
/// Alphabetical by function family rather than by menu, because a reader looking
/// for "is trimming here" looks for trimming. `sort_asc`, `sort_desc` and
/// `sort_numeric` are the menu's three rows; `sort_lines` is the submenu's general
/// case, which is what a caller with options wants.
pub const COMMANDS: &[Command] = &[
    command("sort_asc", &[]),
    command("sort_desc", &[]),
    command("sort_numeric", &[]),
    command("sort_lines", SORT),
    command("dedupe_consecutive", &[]),
    command("dedupe_all", &[]),
    command("reverse_line_order", &[]),
    command("randomize_line_order", &[Param::Seed]),
    command("join_lines", &[]),
    command("split_lines", &[Param::Delimiter]),
    command("remove_empty", &[]),
    command("remove_empty_with_blank", &[]),
    command("trim_trailing", &[]),
    command("trim_leading", &[]),
    command("trim_both", &[]),
    command("trim_all", &[]),
    command("eol_to_space", &[]),
    command("upper_case", &[]),
    command("lower_case", &[]),
    command("title_case", &[]),
    command("proper_case", &[]),
    command("proper_case_blend", &[]),
    command("sentence_case", &[]),
    command("sentence_case_blend", &[]),
    command("invert_case", &[]),
    command("random_case", &[Param::Seed]),
    command("toggle_comment", COMMENT),
    command("single_line_comment", COMMENT),
    command("single_line_uncomment", COMMENT),
    command("block_comment", BLOCK),
    command("block_uncomment", BLOCK),
    command("tabs_to_spaces", WIDTH),
    command("spaces_to_tabs", WIDTH),
    command("spaces_to_tabs_all", WIDTH),
    command("spaces_to_tabs_leading", WIDTH),
    command("indent", &[]),
    command("unindent", WIDTH),
];

impl Command {
    /// Whether this command reads `param`.
    pub fn reads(&self, param: Param) -> bool {
        self.params.contains(&param)
    }
}

/// The table, for a caller that wants to list what it can run.
pub fn commands() -> &'static [Command] {
    COMMANDS
}

/// The command called `name`, if there is one.
///
/// Compared after lowercasing and with `-` read as `_`, because a name arrives
/// from a language model or a shell script as often as from a keymap file, and
/// `Trim Trailing` and `trim-trailing` both obviously mean the one command.
pub fn find(name: &str) -> Option<&'static Command> {
    let wanted = normalise(name);
    COMMANDS.iter().find(|one| one.name == wanted)
}

/// Whether `name` is a command this table can run.
pub fn knows(name: &str) -> bool {
    find(name).is_some()
}

fn normalise(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

/// What a command is run with.
///
/// One struct rather than an argument per command: the caller is a dispatcher
/// that has to build it before it knows which command it has, and the defaults
/// are the ones the editor itself uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Direction, key and case rule for `sort_lines`.
    pub sort: SortOptions,
    /// Tab width in columns. Four is the editor's default and Notepad++'s.
    pub width: usize,
    /// What `split_lines` splits on.
    pub delimiter: String,
    /// The marker the line-comment commands add and remove.
    pub prefix: String,
    /// What `block_comment` opens with.
    pub block_open: String,
    /// What `block_comment` closes with.
    pub block_close: String,
    /// The seed the shuffling commands use, so a run can be repeated.
    pub seed: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sort: SortOptions::default(),
            width: 4,
            delimiter: ",".to_string(),
            prefix: "//".to_string(),
            block_open: "/*".to_string(),
            block_close: "*/".to_string(),
            seed: 0,
        }
    }
}

impl Options {
    /// The width, as at least one column.
    ///
    /// Zero is not a tab width: it would divide by zero in the conversion and
    /// produce a line that gets longer forever. A caller that sends 0 gets the
    /// default rather than a hang.
    fn columns(&self) -> usize {
        self.width.max(1)
    }
}

/// Why a command could not be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No command by that name.
    Unknown(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unknown(name) => write!(
                f,
                "no command called {name:?}; run `list_commands` for the ones that work on text"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Runs `name` over `text`.
///
/// The whole text is the block: a caller that wants a selection transformed
/// sends the selection. Whether the text ended with a terminator, and whether
/// that terminator was CRLF, is carried through by the functions in
/// [`crate::lines`], so a command run over a whole file cannot add a newline to
/// the end of it or turn its line endings into the other kind.
pub fn run(name: &str, text: &str, options: &Options) -> Result<String, Error> {
    let command = find(name).ok_or_else(|| Error::Unknown(name.to_string()))?;
    let width = options.columns();
    let out = match command.name {
        "sort_asc" => lines::sort(text, SortOptions::default()),
        "sort_desc" => lines::sort(
            text,
            SortOptions {
                order: SortOrder::Descending,
                ..SortOptions::default()
            },
        ),
        "sort_numeric" => lines::sort(
            text,
            SortOptions {
                key: SortKey::Numeric,
                ..SortOptions::default()
            },
        ),
        "sort_lines" => lines::sort(text, options.sort),
        "dedupe_consecutive" => lines::dedupe_consecutive(text),
        "dedupe_all" => lines::dedupe_all(text),
        "reverse_line_order" => lines::reverse_order(text),
        "randomize_line_order" => lines::randomize_order(text, options.seed),
        "join_lines" => lines::join(text),
        "split_lines" => lines::split(text, &options.delimiter),
        "remove_empty" => lines::remove_empty(text),
        "remove_empty_with_blank" => lines::remove_empty_with_blank(text),
        "trim_trailing" => lines::trim_trailing(text),
        "trim_leading" => lines::trim_leading(text),
        "trim_both" => lines::trim_both(text),
        "trim_all" => lines::trim_all(text),
        "eol_to_space" => lines::eol_to_space(text),
        "upper_case" => lines::convert_case(text, Case::Upper),
        "lower_case" => lines::convert_case(text, Case::Lower),
        "title_case" => lines::convert_case(text, Case::Title),
        "proper_case" => lines::convert_case(text, Case::Title),
        "proper_case_blend" => lines::convert_case(text, Case::TitleBlend),
        "sentence_case" => lines::convert_case(text, Case::Sentence),
        "sentence_case_blend" => lines::convert_case(text, Case::SentenceBlend),
        "invert_case" => lines::convert_case(text, Case::Invert),
        "random_case" => lines::convert_case_seeded(text, Case::Random, options.seed),
        "toggle_comment" => lines::toggle_comment(text, &options.prefix),
        "single_line_comment" => lines::single_line_comment(text, &options.prefix),
        "single_line_uncomment" => lines::single_line_uncomment(text, &options.prefix),
        "block_comment" => lines::block_comment(text, &options.block_open, &options.block_close),
        "block_uncomment" => {
            lines::block_uncomment(text, &options.block_open, &options.block_close)
        }
        "tabs_to_spaces" => lines::tabs_to_spaces(text, width),
        "spaces_to_tabs" => lines::spaces_to_tabs(text, width),
        "spaces_to_tabs_all" => lines::spaces_to_tabs_all(text, width),
        "spaces_to_tabs_leading" => lines::spaces_to_tabs(text, width),
        "indent" => lines::indent(text),
        "unindent" => lines::unindent(text, width),
        // `find` said the name is in the table, and the table is exhaustive
        // above: reaching this arm means the two lists disagree.
        other => return Err(Error::Unknown(other.to_string())),
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "b\na\nc\n";

    #[test]
    fn every_command_runs_on_the_sample() {
        for command in COMMANDS {
            let out = run(command.name, SAMPLE, &Options::default())
                .unwrap_or_else(|error| panic!("{}: {error}", command.name));
            assert!(
                !out.is_empty() || SAMPLE.is_empty(),
                "{} emptied the text",
                command.name
            );
        }
    }

    #[test]
    fn every_command_survives_empty_text() {
        // The property is that an empty document is not a special case: a command
        // either returns nothing or wraps nothing, but it does not panic, and it
        // does not invent a line to work on.
        for command in COMMANDS {
            let out = run(command.name, "", &Options::default()).unwrap();
            assert!(
                out.trim().is_empty(),
                "{} invented text: {out:?}",
                command.name
            );
        }
    }

    #[test]
    fn names_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for command in COMMANDS {
            assert!(seen.insert(command.name), "{} is listed twice", command.name);
            assert!(
                command
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "{} is not a snake_case action name",
                command.name
            );
        }
    }

    #[test]
    fn every_param_is_spelled_consistently() {
        // The nine spellings are the API: they are the JSON fields a client
        // sends and the names `Options` documents. A tenth spelling appearing in
        // the table without a field behind it is the failure this catches.
        let known = [
            Param::SortKey,
            Param::SortOrder,
            Param::CaseSensitive,
            Param::Width,
            Param::Delimiter,
            Param::Prefix,
            Param::BlockOpen,
            Param::BlockClose,
            Param::Seed,
        ];
        let names: Vec<&str> = known.iter().map(|one| one.name()).collect();
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        for command in COMMANDS {
            for param in command.params {
                assert!(
                    names.contains(&param.name()),
                    "{} reads {:?}, which is not one of the nine",
                    command.name,
                    param
                );
            }
        }
    }

    #[test]
    fn a_name_is_read_the_way_a_person_would_type_it() {
        assert!(knows("sort_asc"));
        assert!(knows("Sort Asc"));
        assert!(knows("SORT-ASC"));
        assert!(knows("  trim_trailing  "));
        assert!(!knows("sort_sideways"));
    }

    #[test]
    fn an_unknown_command_says_so() {
        let error = run("sort_sideways", "a\n", &Options::default()).unwrap_err();
        assert_eq!(error, Error::Unknown("sort_sideways".to_string()));
        assert!(error.to_string().contains("list_commands"), "{error}");
    }

    #[test]
    fn the_three_sort_rows_are_the_three_menu_rows() {
        assert_eq!(run("sort_asc", "b\na\n", &Options::default()).unwrap(), "a\nb\n");
        assert_eq!(
            run("sort_desc", "a\nb\n", &Options::default()).unwrap(),
            "b\na\n"
        );
        // Ten before nine: the numeric sort is the whole reason the row exists.
        assert_eq!(
            run("sort_numeric", "10\n9\n", &Options::default()).unwrap(),
            "9\n10\n"
        );
        assert_eq!(run("sort_asc", "10\n9\n", &Options::default()).unwrap(), "10\n9\n");
    }

    #[test]
    fn the_general_sort_takes_its_three_options() {
        let options = Options {
            sort: SortOptions {
                order: SortOrder::Descending,
                key: SortKey::Length,
                case_sensitive: false,
            },
            ..Options::default()
        };
        assert_eq!(
            run("sort_lines", "a\nccc\nbb\n", &options).unwrap(),
            "ccc\nbb\na\n"
        );
    }

    #[test]
    fn dedupe_keeps_adjacent_runs_but_dedupe_all_keeps_one_of_each() {
        let text = "a\nb\na\na\n";
        assert_eq!(
            run("dedupe_consecutive", text, &Options::default()).unwrap(),
            "a\nb\na\n"
        );
        assert_eq!(run("dedupe_all", text, &Options::default()).unwrap(), "a\nb\n");
    }

    #[test]
    fn the_trimming_commands_trim_what_they_say() {
        let text = "  a  \n\tb\t\n";
        assert_eq!(
            run("trim_trailing", text, &Options::default()).unwrap(),
            "  a\n\tb\n"
        );
        assert_eq!(
            run("trim_leading", text, &Options::default()).unwrap(),
            "a  \nb\t\n"
        );
        assert_eq!(
            run("trim_both", text, &Options::default()).unwrap(),
            "a\nb\n"
        );
    }

    #[test]
    fn the_case_commands_differ_from_each_other() {
        let text = "hello world\n";
        let options = Options::default();
        assert_eq!(run("upper_case", text, &options).unwrap(), "HELLO WORLD\n");
        assert_eq!(run("lower_case", text, &options).unwrap(), "hello world\n");
        assert_eq!(run("title_case", text, &options).unwrap(), "Hello World\n");
        assert_eq!(
            run("invert_case", "Hello\n", &options).unwrap(),
            "hELLO\n"
        );
        // The forced and blended versions of the same case differ on text that
        // is already partly capitalised, which is the only reason both exist.
        let mixed = "hELLO wORLD\n";
        assert_eq!(
            run("proper_case", mixed, &options).unwrap(),
            "Hello World\n"
        );
        assert_eq!(
            run("proper_case_blend", mixed, &options).unwrap(),
            "HELLO WORLD\n"
        );
    }

    #[test]
    fn the_seeded_commands_repeat_with_the_seed() {
        let seven = Options {
            seed: 7,
            ..Options::default()
        };
        let seven_again = Options {
            seed: 7,
            ..Options::default()
        };
        let text = "the quick brown fox jumps over the lazy dog\n";
        assert_eq!(
            run("random_case", text, &seven).unwrap(),
            run("random_case", text, &seven_again).unwrap()
        );

        // Whatever order the shuffle picks, the lines are all still there: a
        // randomise that lost a line would be a delete with a misleading name.
        let shuffled = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let out = run("randomize_line_order", shuffled, &seven).unwrap();
        let mut lines: Vec<&str> = out.lines().collect();
        lines.sort_unstable();
        assert_eq!(lines, vec!["a", "b", "c", "d", "e", "f", "g", "h"]);
        assert_eq!(
            out,
            run("randomize_line_order", shuffled, &seven_again).unwrap()
        );
    }

    #[test]
    fn commenting_twice_gives_the_text_back() {
        let text = "let x = 1;\nlet y = 2;\n";
        let options = Options::default();
        let once = run("toggle_comment", text, &options).unwrap();
        assert!(once.starts_with("// "), "{once:?}");
        assert_eq!(run("toggle_comment", &once, &options).unwrap(), text);
        // The same marker, spelled as an option, is what the comment commands add.
        let prefix = Options {
            prefix: "#".to_string(),
            ..Options::default()
        };
        assert!(run("single_line_comment", text, &prefix)
            .unwrap()
            .starts_with("# "));
        assert_eq!(
            run(
                "single_line_uncomment",
                &run("single_line_comment", text, &prefix).unwrap(),
                &prefix
            )
            .unwrap(),
            text
        );
    }

    #[test]
    fn a_block_comment_opens_and_closes_around_the_block() {
        let text = "a\nb\n";
        let options = Options::default();
        let wrapped = run("block_comment", text, &options).unwrap();
        assert!(wrapped.starts_with("/*"), "{wrapped:?}");
        assert!(wrapped.trim_end().ends_with("*/"), "{wrapped:?}");
        assert_eq!(run("block_uncomment", &wrapped, &options).unwrap(), text);
    }

    #[test]
    fn the_tab_width_is_the_width_that_is_asked_for() {
        let two = Options {
            width: 2,
            ..Options::default()
        };
        assert_eq!(
            run("tabs_to_spaces", "\tx\n", &two).unwrap(),
            "  x\n"
        );
        assert_eq!(
            run("spaces_to_tabs", "  x\n", &two).unwrap(),
            "\tx\n"
        );
        assert_eq!(
            run("indent", "a\n", &two).unwrap(),
            "\ta\n"
        );
        assert_eq!(
            run("unindent", "\t\ta\n", &two).unwrap(),
            "\ta\n"
        );
        // A width of zero is a caller's mistake, but it must not hang or grow
        // the line: it is read as one column.
        let none = Options {
            width: 0,
            ..Options::default()
        };
        assert_eq!(run("tabs_to_spaces", "\tx\n", &none).unwrap(), " x\n");
    }

    #[test]
    fn splitting_and_joining_are_inverses() {
        let options = Options {
            delimiter: ";".to_string(),
            ..Options::default()
        };
        let split = run("split_lines", "a;b;c\n", &options).unwrap();
        assert_eq!(split, "a\nb\nc\n");
        assert_eq!(run("join_lines", &split, &Options::default()).unwrap(), "a b c\n");
    }

    #[test]
    fn a_crlf_block_stays_crlf() {
        let text = "a  \r\nb  \r\n";
        let out = run("trim_trailing", text, &Options::default()).unwrap();
        assert_eq!(out, "a\r\nb\r\n");
    }

    #[test]
    fn a_file_without_a_final_newline_does_not_gain_one() {
        let out = run("sort_asc", "b\na", &Options::default()).unwrap();
        assert_eq!(out, "a\nb");
        let out = run("trim_trailing", "a  ", &Options::default()).unwrap();
        assert_eq!(out, "a");
    }

    #[test]
    fn the_eol_to_space_command_joins_a_block_into_one_line() {
        // The newline that ends the block is the block's, not the join's: the
        // lines are joined and the terminator stays where it was.
        assert_eq!(
            run("eol_to_space", "a\nb\n", &Options::default()).unwrap(),
            "a b\n"
        );
        assert_eq!(
            run("eol_to_space", "a\nb", &Options::default()).unwrap(),
            "a b"
        );
    }

    #[test]
    fn a_command_reads_only_the_options_it_declares() {
        for command in COMMANDS {
            let declared = command.params.len();
            assert!(
                declared <= 3,
                "{} reads {declared} options",
                command.name
            );
            if command.reads(Param::Width) {
                assert!(
                    matches!(
                        command.name,
                        "tabs_to_spaces"
                            | "spaces_to_tabs"
                            | "spaces_to_tabs_all"
                            | "spaces_to_tabs_leading"
                            | "unindent"
                    ),
                    "{} reads a width it does not use",
                    command.name
                );
            }
        }
    }
}
