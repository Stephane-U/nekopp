//! The command line.
//!
//! Notepad++ takes a file and a handful of switches; a Linux editor is also
//! expected to work as `$EDITOR`, which means `--wait` and reading a document
//! from standard input. Both live here rather than in `main.rs` because
//! `main.rs` has no test module -- the policy is that anything with a decision
//! in it belongs in a library module where a test can reach it.
//!
//! The parser is hand-written. A derive-based one would pull a dependency in
//! for eleven flags, and the interesting part is not the parsing but the
//! *rules*: which switch wins when two are given, what `-` means, and what
//! happens to a file whose name starts with a dash.

use std::path::PathBuf;

/// What the command line asked for.
///
/// `files` is ordered: the first file is the one that gets focus, the rest open
/// as tabs behind it. That is the order the shell expanded them in, which is
/// the order the user typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// The documents to open, in the order they were named.
    pub files: Vec<PathBuf>,
    /// Block until every named document is closed.
    pub wait: bool,
    /// Start a second process even when one is already running.
    pub new_instance: bool,
    /// Read the document from standard input instead of a file.
    pub stdin: bool,
    /// The line to put the caret on, one-based.
    pub line: Option<usize>,
    /// The column to put the caret on, one-based.
    pub column: Option<usize>,
    /// Print the usage text and exit.
    pub help: bool,
    /// Print the version and exit.
    pub version: bool,
    /// Skip the hardware adapter and use the software rasterizer.
    pub software: bool,
    /// One API call to send to the running editor and print the answer of.
    ///
    /// A call is not a launch: a process that carries one hands it to the window
    /// that is already running, prints what comes back, and exits without
    /// touching the GPU. That is why it is here rather than in a second program:
    /// the caller wants the *running* editor, and finding it is this binary's
    /// job.
    pub call: Option<String>,
    /// Whether to serve the REST face for this run.
    ///
    /// A listener is not a thing to start behind the user's back, so it is off
    /// unless this switch or the `rest` setting asks for it. `--rest=PORT` names
    /// the port; a bare `--rest` takes the one in the settings file.
    pub rest: bool,
    /// The port `--rest` named, if it named one.
    pub rest_port: Option<u16>,
}

impl Args {
    /// The caret position as zero-based `(line, column)`, when one was asked
    /// for.
    ///
    /// The command line is one-based because that is what a compiler error and
    /// a `grep -n` result say, and the editor is zero-based because that is
    /// what an index is. Converting in one place means the two callers that
    /// need it -- the first launch and a handed-off request -- cannot disagree.
    ///
    /// A line without a column starts at column 1, which is where a line
    /// starts; a column without a line starts at line 1, for the same reason.
    pub fn caret(&self) -> Option<(usize, usize)> {
        if self.line.is_none() && self.column.is_none() {
            return None;
        }
        Some((
            self.line.unwrap_or(1).saturating_sub(1),
            self.column.unwrap_or(1).saturating_sub(1),
        ))
    }
}

/// Why a command line was rejected.
///
/// A `String` rather than an enum: the message is shown to the user verbatim
/// and there is nothing a caller wants to branch on beyond "it failed".
pub type Error = String;

/// The switches this editor understands, as `(short, long, help)`.
///
/// One table, used by both the parser and the usage text, so a switch cannot be
/// documented and unimplemented or the other way round. A short letter of `\0`
/// means there is none: `--rest` is a listener rather than a file operation, and
/// the three letters left unclaimed are worth more to a switch typed constantly.
const SWITCHES: &[(char, &str, &str)] = &[
    ('n', "new-instance", "start a new process even if one is running"),
    ('w', "wait", "block until the document is closed"),
    ('l', "line", "put the caret on this line (one-based)"),
    ('c', "column", "put the caret on this column (one-based)"),
    ('s', "software", "draw with the software rasterizer, not the GPU"),
    ('C', "call", "send a JSON call to the running editor and print the answer"),
    ('\0', "rest", "serve the REST face on 127.0.0.1 (--rest=PORT names the port)"),
    ('h', "help", "print this text"),
    ('V', "version", "print the version"),
];

/// The usage text, built from [`SWITCHES`].
///
/// The program name is `npp`, not the product name: this text is printed by a
/// shell, and what a shell needs is the word it has to type. The product name
/// appears in `--version` and in the window title, which are read by people.
pub fn usage() -> String {
    let mut text = String::from(
        "usage: npp [switches] [file ...]\n\n\
         A file named - reads the document from standard input.\n\n\
         --call sends a JSON call to the editor that is already running and\n\
         prints its answer: `npp --call '{\"method\":\"document.list\"}'`.\n\
         Nothing opens.\n\n\
         --rest serves the same calls over HTTP on 127.0.0.1, for a client that\n\
         would rather fetch a URL: `--rest=8085`, then `GET /status`. The port\n\
         goes straight after the switch, so a file called 8085 still opens.\n\n\
         switches:\n",
    );
    for (short, long, help) in SWITCHES {
        if *short == '\0' {
            text.push_str(&format!("      --{long:<14}  {help}\n"));
        } else {
            text.push_str(&format!("  -{short}, --{long:<14}  {help}\n"));
        }
    }
    text
}

/// Parses a command line, without the program name.
///
/// `--` ends the switches: everything after it is a file, which is how a file
/// called `-w` gets opened. A lone `-` is standard input, not a switch, because
/// that is what every other Unix tool means by it.
pub fn parse<I, S>(args: I) -> Result<Args, Error>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut out = Args::default();
    let mut rest = args.into_iter().map(Into::into).peekable();
    let mut switches_done = false;
    while let Some(arg) = rest.next() {
        if switches_done {
            out.files.push(PathBuf::from(arg));
            continue;
        }
        if arg == "--" {
            switches_done = true;
            continue;
        }
        if arg == "-" {
            out.stdin = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (long, None),
            };
            match name {
                "new-instance" => out.new_instance = true,
                "wait" => out.wait = true,
                "help" => out.help = true,
                "version" => out.version = true,
                "software" => out.software = true,
                "call" => out.call = Some(text(name, inline, &mut rest)?),
                "rest" => {
                    out.rest = true;
                    // The port is inline only. A bare `--rest` must not eat the
                    // next argument: `npp --rest 8085` is a listener and a file
                    // called 8085, and only the user knows which was meant.
                    if let Some(value) = inline {
                        out.rest_port = Some(port(&value)?);
                    }
                }
                "line" => out.line = Some(number(name, inline, &mut rest)?),
                "column" => out.column = Some(number(name, inline, &mut rest)?),
                other => return Err(format!("unknown switch --{other}")),
            }
            continue;
        }
        if let Some(short) = arg.strip_prefix('-') {
            // A cluster of short switches: `-nw` is `-n -w`. A switch that
            // takes a value ends the cluster and takes the rest of the word, so
            // `-l20` and `-l 20` both work.
            let mut chars = short.chars();
            while let Some(c) = chars.next() {
                match c {
                    'n' => out.new_instance = true,
                    'w' => out.wait = true,
                    'h' => out.help = true,
                    'V' => out.version = true,
                    's' => out.software = true,
                    'C' => {
                        let tail: String = chars.by_ref().collect();
                        out.call = Some(if tail.is_empty() {
                            rest.next().ok_or_else(|| "-C needs a JSON call".to_string())?
                        } else {
                            tail
                        });
                        break;
                    }
                    'l' | 'c' => {
                        let tail: String = chars.by_ref().collect();
                        let value = if tail.is_empty() {
                            rest.next()
                                .ok_or_else(|| format!("-{c} needs a number"))?
                        } else {
                            tail
                        };
                        let parsed = parse_number(&value)
                            .ok_or_else(|| format!("-{c} needs a number, not {value:?}"))?;
                        if c == 'l' {
                            out.line = Some(parsed);
                        } else {
                            out.column = Some(parsed);
                        }
                        break;
                    }
                    other => return Err(format!("unknown switch -{other}")),
                }
            }
            continue;
        }
        out.files.push(PathBuf::from(arg));
    }
    Ok(out)
}

/// The value of a long switch that takes a number, from `--name=value` or the
/// next argument.
fn number<I>(name: &str, inline: Option<String>, rest: &mut I) -> Result<usize, Error>
where
    I: Iterator<Item = String>,
{
    let value = match inline {
        Some(value) => value,
        None => rest
            .next()
            .ok_or_else(|| format!("--{name} needs a number"))?,
    };
    parse_number(&value).ok_or_else(|| format!("--{name} needs a number, not {value:?}"))
}

/// The value of a long switch that takes text, from `--name=value` or the next
/// argument.
fn text<I>(name: &str, inline: Option<String>, rest: &mut I) -> Result<String, Error>
where
    I: Iterator<Item = String>,
{
    match inline {
        Some(value) => Ok(value),
        None => rest.next().ok_or_else(|| format!("--{name} needs a value")),
    }
}

/// A port number, rejecting zero and anything outside the range.
fn port(text: &str) -> Result<u16, Error> {
    match text.parse::<u32>() {
        Ok(value) if (1..=65535).contains(&value) => Ok(value as u16),
        _ => Err(format!(
            "--rest needs a port between 1 and 65535, not {text:?}"
        )),
    }
}

/// A one-based position, rejecting zero and anything that is not a number.
///
/// Zero is rejected rather than clamped: a caller that passes `--line 0` has
/// made a mistake, and silently opening at line 1 hides it.
fn parse_number(text: &str) -> Option<usize> {
    text.parse::<usize>().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        parse(list.iter().copied()).expect("a valid command line")
    }

    #[test]
    fn no_arguments_asks_for_an_empty_document() {
        let a = args(&[]);
        assert!(a.files.is_empty());
        assert!(!a.stdin);
        assert!(!a.wait);
    }

    #[test]
    fn a_bare_word_is_a_file() {
        assert_eq!(args(&["notes.txt"]).files, vec![PathBuf::from("notes.txt")]);
    }

    #[test]
    fn files_keep_the_order_they_were_named_in() {
        let a = args(&["b.txt", "a.txt"]);
        assert_eq!(a.files, vec![PathBuf::from("b.txt"), PathBuf::from("a.txt")]);
    }

    #[test]
    fn a_call_is_read_in_any_of_its_three_spellings() {
        let json = r#"{"method":"app.status"}"#;
        assert_eq!(args(&["--call", json]).call.as_deref(), Some(json));
        assert_eq!(args(&[&format!("--call={json}")]).call.as_deref(), Some(json));
        assert_eq!(args(&["-C", json]).call.as_deref(), Some(json));
    }

    #[test]
    fn a_call_may_sit_between_file_names() {
        let a = args(&["notes.txt", "-C", "{}", "more.txt"]);
        assert_eq!(a.call.as_deref(), Some("{}"));
        assert_eq!(
            a.files,
            vec![PathBuf::from("notes.txt"), PathBuf::from("more.txt")]
        );
    }

    #[test]
    fn a_call_with_nothing_after_it_says_what_is_missing() {
        let error = parse(["-C"]).unwrap_err();
        assert!(error.contains("JSON call"), "{error}");
    }

    #[test]
    fn a_lone_dash_is_standard_input() {
        let a = args(&["-"]);
        assert!(a.stdin);
        assert!(a.files.is_empty());
    }

    #[test]
    fn a_dash_after_the_terminator_is_a_file() {
        let a = args(&["--", "-"]);
        assert!(!a.stdin);
        assert_eq!(a.files, vec![PathBuf::from("-")]);
    }

    #[test]
    fn a_file_named_like_a_switch_needs_the_terminator() {
        let a = args(&["--", "-w"]);
        assert!(!a.wait);
        assert_eq!(a.files, vec![PathBuf::from("-w")]);
    }

    #[test]
    fn the_long_switches_are_recognised() {
        let a = args(&["--wait", "--new-instance", "x.txt"]);
        assert!(a.wait);
        assert!(a.new_instance);
        assert_eq!(a.files, vec![PathBuf::from("x.txt")]);
    }

    #[test]
    fn short_switches_cluster() {
        let a = args(&["-nw"]);
        assert!(a.new_instance);
        assert!(a.wait);
    }

    #[test]
    fn a_short_switch_takes_the_rest_of_the_word_as_its_value() {
        assert_eq!(args(&["-l20"]).line, Some(20));
    }

    #[test]
    fn a_short_switch_takes_the_next_word_as_its_value() {
        assert_eq!(args(&["-l", "20"]).line, Some(20));
    }

    #[test]
    fn a_long_switch_takes_an_inline_value() {
        assert_eq!(args(&["--line=7"]).line, Some(7));
    }

    #[test]
    fn a_long_switch_takes_the_next_word_as_its_value() {
        assert_eq!(args(&["--column", "3"]).column, Some(3));
    }

    #[test]
    fn a_position_must_be_a_positive_number() {
        assert!(parse(["--line", "0"]).is_err());
        assert!(parse(["--line", "x"]).is_err());
        assert!(parse(["--line"]).is_err());
    }

    #[test]
    fn an_unknown_switch_is_an_error() {
        assert!(parse(["--frobnicate"]).is_err());
        assert!(parse(["-q"]).is_err());
    }

    #[test]
    fn help_and_version_are_switches() {
        assert!(args(&["--help"]).help);
        assert!(args(&["-V"]).version);
    }

    #[test]
    fn software_is_a_switch_in_both_spellings() {
        assert!(args(&["--software"]).software);
        assert!(args(&["-s"]).software);
        // Off by default: the hardware path is the one that should be taken
        // without being asked for.
        assert!(!args(&[]).software);
    }

    #[test]
    fn software_clusters_with_other_switches() {
        let a = args(&["-sn"]);
        assert!(a.software);
        assert!(a.new_instance);
    }

    #[test]
    fn the_usage_text_documents_every_switch() {
        let text = usage();
        for (short, long, _) in SWITCHES {
            if *short != '\0' {
                assert!(text.contains(&format!("-{short}")), "missing -{short}");
            }
            assert!(text.contains(&format!("--{long}")), "missing --{long}");
        }
    }

    #[test]
    fn rest_is_a_listener_and_its_port_is_inline() {
        let a = args(&["--rest"]);
        assert!(a.rest);
        assert_eq!(a.rest_port, None);
        let a = args(&["--rest=9000"]);
        assert!(a.rest);
        assert_eq!(a.rest_port, Some(9000));
    }

    #[test]
    fn a_bare_rest_does_not_swallow_the_next_argument() {
        // `npp --rest 8085` is a listener and a file called 8085: only the user
        // knows which one was meant, so neither is guessed at.
        let a = args(&["--rest", "8085"]);
        assert_eq!(a.rest_port, None);
        assert_eq!(a.files, vec![PathBuf::from("8085")]);
    }

    #[test]
    fn a_port_that_is_not_a_port_is_refused() {
        for value in ["--rest=0", "--rest=70000", "--rest=http"] {
            assert!(parse([value.to_string()]).is_err(), "{value}");
        }
    }

    #[test]
    fn a_caret_position_is_converted_to_zero_based() {
        assert_eq!(args(&["--line", "3", "--column", "7"]).caret(), Some((2, 6)));
    }

    #[test]
    fn a_line_without_a_column_starts_at_the_first_column() {
        assert_eq!(args(&["--line", "3"]).caret(), Some((2, 0)));
    }

    #[test]
    fn a_column_without_a_line_starts_at_the_first_line() {
        assert_eq!(args(&["--column", "5"]).caret(), Some((0, 4)));
    }

    #[test]
    fn no_position_asks_for_no_caret_move() {
        assert_eq!(args(&["x.txt"]).caret(), None);
    }

    #[test]
    fn a_switch_after_a_file_is_still_a_switch() {
        // Notepad++ accepts `file.txt -w`, and so does every editor that lets
        // the shell put the file first.
        let a = args(&["file.txt", "-w"]);
        assert!(a.wait);
        assert_eq!(a.files, vec![PathBuf::from("file.txt")]);
    }
}
