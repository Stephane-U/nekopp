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

//! The built-in languages, as Notepad++'s Language menu lists them.
//!
//! One table rather than a hundred and forty-seven lexers. Every language here
//! is described by the same five things -- how a comment starts, how a block
//! comment opens and closes, which characters quote a string, how an apostrophe
//! ends, and which words are keywords -- and `ConfigLexer` reads that
//! description. A hand-written lexer per language would be a hundred and
//! forty-seven files that differ only in their tables.
//!
//! The model is deliberately shallow. It does not know that Lua's long brackets
//! nest or that Elixir and Perl have heredocs. Those degrade to "the rest of the
//! line is a string", which is wrong for a few lines and right for the thousands
//! around them. A lexer that guessed at them and got the common case wrong
//! would be worse.

/// How a single-quoted literal ends, in the languages that have one.
///
/// `'` is the one quote that is not always a quote. It opens a string in
/// JavaScript and holds a character in Rust; in Haskell it is the tail of an
/// identifier, in Julia an operator, and in a YAML value an apostrophe inside a
/// word. Treating it as a quote that always opens a string -- which the table
/// did, by listing it in `quotes` and nothing else -- made one apostrophe colour
/// every line below it as a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apostrophe {
    /// `'` holds one character or one escape, as in Rust's `'a'` or Go's rune.
    ///
    /// A run that does not close within a few characters is something else and
    /// stays punctuation: a lifetime, a type parameter, a quote macro, a
    /// transpose.
    Char,
    /// `'` opens a string that ends on the line it started on.
    ///
    /// An apostrophe that does not close on its own line is punctuation, so it
    /// cannot run into the lines below.
    Line,
    /// `'` opens a string that may run across lines, as in the shells, Perl,
    /// Ruby, PHP and R.
    Multiline,
}

/// One language, described by what a line-at-a-time lexer needs to know.
#[derive(Debug, Clone, Copy)]
pub struct LangSpec {
    /// The label the Language menu shows.
    pub name: &'static str,
    /// The prefixes that start a comment running to the end of the line.
    pub line: &'static [&'static str],
    /// The pair that opens and closes a block comment.
    pub block: Option<(&'static str, &'static str)>,
    /// The characters that quote a string.
    ///
    /// `'` is listed only where it opens a literal. A language that does not
    /// quote with it -- Makefile, Dockerfile, Haskell, Julia -- leaves it out,
    /// so a prime or an adjoint is never mistaken for a string.
    pub quotes: &'static [char],
    /// How a literal opened by `'` ends; read only when `quotes` lists `'`.
    pub apostrophe: Apostrophe,
    /// The words drawn as keywords.
    pub keywords: &'static [&'static str],
    /// Whether keywords are matched without regard to case.
    ///
    /// True for the languages that were designed before case mattered -- Ada,
    /// Pascal, SQL, the BASICs -- where `BEGIN` and `begin` are the same word.
    pub fold_case: bool,
}

impl LangSpec {
    /// The stable key a saved shortcut uses.
    ///
    /// Derived from the name rather than stored, so a language cannot have a
    /// name and a key that disagree. `Fortran (free form)` becomes
    /// `fortran_free_form` and `MS-DOS Style` becomes `ms_dos_style`.
    ///
    /// `+` and `#` are spelled out rather than dropped, because dropping them
    /// makes `C`, `C#` and `C++` the same key. A shortcut file that named `C`
    /// would then select whichever of the three came first.
    pub fn key(&self) -> String {
        let mut out = String::with_capacity(self.name.len());
        for c in self.name.chars() {
            match c {
                '+' => out.push_str("plus"),
                '#' => out.push_str("sharp"),
                c if c.is_ascii_alphanumeric() => out.push(c.to_ascii_lowercase()),
                _ => {
                    if !out.ends_with('_') && !out.is_empty() {
                        out.push('_');
                    }
                }
            }
        }
        while out.ends_with('_') {
            out.pop();
        }
        out
    }

    /// Whether `word` is one of this language's keywords.
    pub fn is_keyword(&self, word: &str) -> bool {
        if self.fold_case {
            self.keywords
                .iter()
                .any(|k| k.eq_ignore_ascii_case(word))
        } else {
            self.keywords.contains(&word)
        }
    }
}

/// Every built-in language, in the order the Language menu lists them.
pub const LANGS: [LangSpec; 147] = [
    LangSpec {
        name: "ActionScript",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["class", "const", "extends", "function", "get", "implements", "import", "interface", "internal", "native", "new", "null", "override", "package", "private", "protected", "public", "return", "set", "static", "super", "switch", "this", "throw", "try", "typeof", "use", "var", "void", "while", "with", "dynamic", "final", "each", "in", "if", "else", "for", "do", "break", "continue", "case", "default", "delete", "true", "false"],
        fold_case: false,
    },
    LangSpec {
        name: "Ada",
        line: &["--"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["abort", "abs", "abstract", "accept", "access", "aliased", "all", "and", "array", "at", "begin", "body", "case", "constant", "declare", "delay", "delta", "digits", "do", "else", "elsif", "end", "entry", "exception", "exit", "for", "function", "generic", "goto", "if", "in", "interface", "is", "limited", "loop", "mod", "new", "not", "null", "of", "or", "others", "out", "overriding", "package", "pragma", "private", "procedure", "protected", "raise", "range", "record", "rem", "renames", "requeue", "return", "reverse", "select", "separate", "some", "subtype", "synchronized", "tagged", "task", "terminate", "then", "type", "until", "use", "when", "while", "with", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "APL",
        line: &["⍝"],
        block: None,
        quotes: &['\''],
        apostrophe: Apostrophe::Line,
        keywords: &["Access", "Case", "CaseList", "Class", "Disposable", "Else", "ElseIf", "EndClass", "EndFor", "EndIf", "EndInterface", "EndNamespace", "EndOperator", "EndSections", "EndSelect", "EndTrap", "EndWith", "For", "Function", "GoTo", "Hold", "If", "In", "Interface", "Namespace", "Operator", "Repeat", "Return", "Sections", "Select", "Trap", "Until", "While", "With"],
        fold_case: false,
    },
    LangSpec {
        name: "AsciiDoc",
        line: &["//"],
        block: Some(("////", "////")),
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "admonition", "asciimath", "audio", "caution", "center", "code", "colophon", "comment", "contents", "csv-table", "dedication", "discrete", "example", "figure", "footnote", "highlight", "image", "important", "include", "index", "inverse", "kbd", "latexmath", "listing", "literal", "menu", "note", "open", "pagebreak", "pass", "partintro", "quote", "raw", "replace", "role", "sectnum", "section", "sidebar", "source", "stem", "table", "target-notes", "tip", "toc", "verse", "video", "warning"],
        fold_case: false,
    },
    LangSpec {
        name: "ASN.1",
        line: &["--"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["ABSENT", "ABSTRACT-SYNTAX", "ALL", "APPLICATION", "AUTOMATIC", "BEGIN", "BIT", "BMPString", "BOOLEAN", "BY", "CHARACTER", "CHOICE", "CLASS", "COMPONENTS", "CONSTRAINED", "CONTAINING", "DEFAULT", "DEFINITIONS", "EMBEDDED", "ENCODED", "END", "ENUMERATED", "EXCEPT", "EXPLICIT", "EXPORTS", "EXTENSIBILITY", "EXTERNAL", "FALSE", "FROM", "GeneralizedTime", "GeneralString", "GraphicString", "IA5String", "IDENTIFIER", "IMPLICIT", "IMPLIED", "IMPORTS", "INCLUDES", "INSTANCE", "INTEGER", "INTERSECTION", "ISO646String", "MAX", "MIN", "MINUS-INFINITY", "NULL", "NumericString", "OBJECT", "ObjectDescriptor", "OCTET", "OF", "OPTIONAL", "PATTERN", "PDVString", "PLUS-INFINITY", "PRESENT", "PrintableString", "PRIVATE", "REAL", "RELATIVE-OID", "SEQUENCE", "SET", "SIZE", "STRING", "SYNTAX", "T61String", "TAGS", "TRUE", "TYPE-IDENTIFIER", "UNION", "UNIQUE", "UNIVERSAL", "UTCTime", "UTF8String", "VideotexString", "VisibleString", "WITH"],
        fold_case: false,
    },
    LangSpec {
        name: "ASP",
        line: &["'"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["dim", "set", "if", "then", "else", "elseif", "end", "for", "next", "while", "wend", "do", "loop", "function", "sub", "class", "const", "byval", "byref", "call", "case", "select", "exit", "on", "error", "resume", "response", "request", "server", "session", "application", "true", "false", "nothing"],
        fold_case: true,
    },
    LangSpec {
        name: "Assembly",
        line: &[";"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["mov", "add", "sub", "mul", "div", "inc", "dec", "and", "or", "xor", "not", "shl", "shr", "cmp", "jmp", "je", "jne", "jz", "jnz", "jg", "jl", "jge", "jle", "call", "ret", "push", "pop", "lea", "nop", "int", "section", "global", "extern", "db", "dw", "dd", "equ", "org", "times", "byte", "word", "dword", "qword"],
        fold_case: false,
    },
    LangSpec {
        name: "Astro",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["as", "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "else", "export", "extends", "false", "for", "from", "function", "get", "if", "implements", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "private", "props", "public", "readonly", "return", "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "AutoIt",
        line: &[";"],
        block: Some(("#cs", "#ce")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "byref", "case", "const", "continuecase", "continueloop", "default", "dim", "do", "else", "elseif", "endfunc", "endif", "endselect", "endswitch", "endwith", "enum", "exit", "exitloop", "false", "for", "func", "global", "if", "in", "local", "next", "not", "null", "or", "redim", "return", "select", "static", "step", "switch", "then", "to", "true", "until", "volatile", "wend", "while", "with"],
        fold_case: true,
    },
    LangSpec {
        name: "AviSynth",
        line: &["#"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["function", "return", "global", "clip", "bool", "int", "float", "string", "true", "false", "if", "else", "try", "catch", "last"],
        fold_case: true,
    },
    LangSpec {
        name: "AWK",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["BEGIN", "END", "break", "case", "close", "continue", "default", "delete", "do", "else", "exit", "exp", "fflush", "for", "func", "function", "getline", "gsub", "if", "in", "index", "int", "length", "log", "match", "next", "nextfile", "print", "printf", "rand", "return", "sin", "split", "sprintf", "sqrt", "srand", "sub", "substr", "switch", "system", "tolower", "toupper", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "BaanC",
        line: &["|"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["domain", "table", "select", "from", "where", "order", "by", "group", "function", "extern", "static", "long", "double", "string", "if", "then", "else", "endif", "for", "while", "endfor", "return"],
        fold_case: true,
    },
    LangSpec {
        name: "Batch",
        line: &["rem ", "::"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["echo", "set", "if", "else", "goto", "call", "for", "in", "do", "exit", "not", "exist", "defined", "errorlevel", "shift", "pause", "start", "cd", "copy", "del", "move", "ren", "md", "rd", "type"],
        fold_case: true,
    },
    LangSpec {
        name: "BibTeX",
        line: &["%"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "address", "annote", "article", "author", "book", "booktitle", "chapter", "comment", "conference", "doi", "edition", "editor", "howpublished", "incollection", "inproceedings", "institution", "isbn", "issn", "journal", "keywords", "mastersthesis", "misc", "month", "note", "number", "organization", "pages", "phdthesis", "preamble", "proceedings", "publisher", "school", "series", "string", "techreport", "title", "type", "unpublished", "url", "volume", "year"],
        fold_case: false,
    },
    LangSpec {
        name: "Blade",
        line: &["//"],
        block: Some(("{{--", "--}}")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["auth", "break", "can", "cannot", "case", "component", "csrf", "default", "each", "else", "elseif", "empty", "endcan", "endcannot", "endcomponent", "enderror", "endfor", "endforeach", "endforelse", "endif", "endisset", "endonce", "endphp", "endsection", "endunless", "endverbatim", "error", "extends", "for", "foreach", "forelse", "fragment", "guest", "if", "include", "includeIf", "includeUnless", "includeWhen", "isset", "json", "method", "once", "php", "props", "push", "section", "session", "show", "slot", "stack", "switch", "unless", "use", "verbatim", "while", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Blitzbasic",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "or", "not", "xor", "shl", "shr", "if", "then", "else", "elseif", "endif", "for", "to", "step", "next", "while", "wend", "repeat", "until", "forever", "function", "end", "function", "return", "type", "field", "global", "local", "const", "dim", "select", "case", "default", "exit"],
        fold_case: true,
    },
    LangSpec {
        name: "C",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register", "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "C#",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char", "checked", "class", "const", "continue", "decimal", "default", "delegate", "do", "double", "else", "enum", "event", "explicit", "extern", "false", "finally", "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int", "interface", "internal", "is", "lock", "long", "namespace", "new", "null", "object", "operator", "out", "override", "params", "private", "protected", "public", "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof", "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true", "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual", "void", "volatile", "while", "var", "async", "await"],
        fold_case: false,
    },
    LangSpec {
        name: "C++",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["alignas", "alignof", "and", "asm", "auto", "bool", "break", "case", "catch", "char", "class", "const", "constexpr", "const_cast", "continue", "decltype", "default", "delete", "do", "double", "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false", "float", "for", "friend", "goto", "if", "inline", "int", "long", "mutable", "namespace", "new", "noexcept", "nullptr", "operator", "or", "private", "protected", "public", "register", "reinterpret_cast", "return", "short", "signed", "sizeof", "static", "static_assert", "static_cast", "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typeid", "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "wchar_t", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Caml",
        line: &[],
        block: Some(("(*", "*)")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "assert", "asr", "begin", "class", "constraint", "do", "done", "downto", "else", "end", "exception", "external", "false", "for", "fun", "function", "functor", "if", "in", "include", "inherit", "initializer", "land", "lazy", "let", "lor", "lsl", "lsr", "lxor", "match", "method", "mod", "module", "mutable", "new", "object", "of", "open", "or", "private", "rec", "sig", "struct", "then", "to", "true", "try", "type", "val", "virtual", "when", "while", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "Clojure",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "assert", "binding", "case", "catch", "comment", "cond", "condp", "def", "definterface", "defmacro", "defmethod", "defmulti", "defn", "defonce", "defprotocol", "defrecord", "defstruct", "deftype", "do", "doseq", "dotimes", "doto", "fn", "for", "future", "if", "if-let", "if-not", "import", "in-ns", "lazy-seq", "let", "letfn", "loop", "monitor-enter", "monitor-exit", "new", "ns", "or", "quote", "recur", "refer", "require", "throw", "try", "use", "var", "when", "when-let", "when-not", "while", "with-open"],
        fold_case: false,
    },
    LangSpec {
        name: "CMake",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["add_custom_command", "add_executable", "add_library", "add_subdirectory", "cmake_minimum_required", "configure_file", "else", "elseif", "endforeach", "endfunction", "endif", "endmacro", "endwhile", "file", "find_package", "foreach", "function", "if", "include", "install", "list", "macro", "message", "option", "project", "return", "set", "string", "target_include_directories", "target_link_libraries", "unset", "while"],
        fold_case: true,
    },
    LangSpec {
        name: "COBOL",
        line: &["*>"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["accept", "add", "alter", "call", "cancel", "close", "compute", "configuration", "continue", "data", "declare", "delete", "display", "divide", "division", "else", "end", "end-if", "end-perform", "end-read", "end-string", "environment", "evaluate", "exit", "file", "from", "generate", "go", "goback", "if", "initialize", "input", "inspect", "into", "is", "just", "justified", "key", "linkage", "merge", "move", "multiply", "open", "output", "perform", "picture", "procedure", "program-id", "read", "record", "redefines", "release", "replace", "return", "rewrite", "search", "section", "select", "sentence", "set", "sort", "source-computer", "stop", "string", "subtract", "then", "to", "unstring", "using", "value", "varying", "when", "working-storage", "write"],
        fold_case: true,
    },
    LangSpec {
        name: "Crystal",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "alias", "annotation", "as", "asm", "begin", "break", "case", "class", "def", "do", "else", "elsif", "end", "ensure", "enum", "extend", "false", "for", "fun", "if", "in", "include", "instance_sizeof", "lib", "macro", "module", "next", "nil", "of", "offsetof", "out", "pointerof", "private", "protected", "rescue", "require", "return", "select", "self", "sizeof", "struct", "super", "then", "true", "type", "typeof", "uninitialized", "union", "unless", "until", "verbatim", "when", "while", "with", "yield", "__DIR__", "__FILE__", "__LINE__"],
        fold_case: false,
    },
    LangSpec {
        name: "CSound",
        line: &[";"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["sr", "kr", "ksmps", "nchnls", "instr", "endin", "opcode", "endop", "if", "then", "elseif", "else", "endif", "while", "do", "od", "goto", "igoto", "kgoto", "return"],
        fold_case: true,
    },
    LangSpec {
        name: "CoffeeScript",
        line: &["#"],
        block: Some(("###", "###")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "break", "by", "catch", "class", "continue", "debugger", "delete", "do", "else", "extends", "false", "finally", "for", "if", "in", "instanceof", "is", "isnt", "loop", "new", "no", "not", "null", "of", "off", "on", "or", "return", "super", "switch", "then", "this", "throw", "true", "try", "typeof", "unless", "until", "when", "while", "yes"],
        fold_case: false,
    },
    LangSpec {
        name: "CSS",
        line: &[],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["important", "media", "import", "charset", "font-face", "keyframes", "supports", "page", "namespace"],
        fold_case: true,
    },
    LangSpec {
        name: "CUDA",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["__device__", "__forceinline__", "__global__", "__host__", "__launch_bounds__", "__managed__", "__noinline__", "__restrict__", "__shared__", "__syncthreads__", "atomicAdd", "atomicCAS", "blockDim", "blockIdx", "bool", "break", "case", "char", "const", "continue", "default", "dim3", "do", "double", "else", "enum", "extern", "float", "for", "goto", "gridDim", "if", "inline", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "threadIdx", "typedef", "union", "unsigned", "void", "volatile", "warpSize", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "D",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "alias", "align", "asm", "assert", "auto", "body", "bool", "break", "byte", "case", "cast", "catch", "char", "class", "const", "continue", "dchar", "debug", "default", "delegate", "delete", "deprecated", "do", "double", "else", "enum", "export", "extern", "false", "final", "finally", "float", "for", "foreach", "foreach_reverse", "function", "goto", "idouble", "if", "ifloat", "immutable", "import", "in", "inout", "int", "interface", "invariant", "ireal", "is", "lazy", "long", "macro", "mixin", "module", "new", "nothrow", "null", "out", "override", "package", "pragma", "private", "protected", "public", "pure", "real", "ref", "return", "scope", "shared", "short", "static", "struct", "super", "switch", "synchronized", "template", "this", "throw", "true", "try", "typeid", "typeof", "ubyte", "ucent", "uint", "ulong", "union", "unittest", "ushort", "version", "void", "volatile", "wchar", "while", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "Dart",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "as", "assert", "async", "await", "base", "break", "case", "catch", "class", "const", "continue", "covariant", "default", "deferred", "do", "dynamic", "else", "enum", "export", "extends", "extension", "external", "factory", "false", "final", "finally", "for", "get", "hide", "if", "implements", "import", "in", "interface", "is", "late", "library", "mixin", "new", "null", "on", "operator", "part", "required", "rethrow", "return", "sealed", "set", "show", "static", "super", "switch", "sync", "this", "throw", "true", "try", "typedef", "var", "void", "when", "while", "with", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Diff",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Dockerfile",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["ADD", "ARG", "AS", "BUILDPLATFORM", "CMD", "COPY", "ENTRYPOINT", "ENV", "EXPOSE", "FROM", "HEALTHCHECK", "LABEL", "MAINTAINER", "ONBUILD", "RUN", "SHELL", "STOPSIGNAL", "TARGETARCH", "TARGETOS", "TARGETPLATFORM", "USER", "VOLUME", "WORKDIR"],
        fold_case: false,
    },
    LangSpec {
        name: "Elixir",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["after", "alias", "and", "catch", "def", "defdelegate", "defexception", "defguard", "defimpl", "defmacro", "defmodule", "defp", "defprotocol", "defstruct", "do", "else", "end", "false", "fn", "for", "if", "import", "in", "nil", "not", "or", "quote", "raise", "receive", "require", "rescue", "super", "throw", "true", "try", "unless", "unquote", "use", "when", "with", "__CALLER__", "__DIR__", "__ENV__", "__MODULE__", "__STACKTRACE__"],
        fold_case: false,
    },
    LangSpec {
        name: "Elm",
        line: &["--"],
        block: Some(("{-", "-}")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["Bool", "Char", "Cmd", "Dict", "Err", "False", "Float", "Html", "Int", "Just", "List", "Maybe", "Nothing", "Ok", "Program", "Result", "Set", "String", "Sub", "True", "Tuple", "as", "case", "effect", "else", "exposing", "if", "import", "in", "infix", "infixl", "infixr", "let", "module", "of", "port", "then", "type", "where"],
        fold_case: false,
    },
    LangSpec {
        name: "Erlang",
        line: &["%"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["after", "and", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor", "case", "catch", "cond", "div", "end", "fun", "if", "let", "not", "of", "or", "orelse", "receive", "rem", "try", "when", "xor"],
        fold_case: false,
    },
    LangSpec {
        name: "ErrorList",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Escape Sequence (ANSI)",
        line: &[],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "ESCRIPT",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "break", "case", "continue", "default", "do", "downto", "else", "elseif", "end", "for", "function", "if", "in", "not", "or", "repeat", "return", "then", "to", "until", "var", "while"],
        fold_case: true,
    },
    LangSpec {
        name: "F#",
        line: &["//"],
        block: Some(("(*", "*)")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "and", "as", "assert", "async", "base", "begin", "class", "default", "delegate", "do", "done", "down", "downto", "elif", "else", "end", "exception", "extern", "false", "finally", "fixed", "for", "fun", "function", "global", "if", "in", "inherit", "inline", "interface", "internal", "lazy", "let", "match", "member", "module", "mutable", "namespace", "new", "not", "null", "of", "open", "or", "override", "params", "private", "public", "rec", "ref", "return", "static", "struct", "then", "this", "to", "true", "try", "type", "upcast", "use", "val", "void", "when", "while", "with", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Fish",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["abbr", "alias", "and", "argparse", "begin", "break", "builtin", "case", "cd", "command", "contains", "continue", "count", "echo", "else", "end", "eval", "exec", "exit", "false", "for", "function", "if", "in", "math", "not", "or", "printf", "pwd", "random", "read", "return", "set", "source", "status", "string", "switch", "test", "true", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Forth",
        line: &["\\"],
        block: Some(("(", ")")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["dup", "drop", "swap", "over", "rot", "if", "else", "then", "begin", "until", "while", "repeat", "do", "loop", "i", "j", "variable", "constant", "create", "does", "immediate", "postpone", "literal", "recurse", "exit"],
        fold_case: true,
    },
    LangSpec {
        name: "Fortran (free form)",
        line: &["!"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["allocatable", "allocate", "assign", "associate", "block", "call", "case", "class", "close", "common", "contains", "continue", "cycle", "data", "deallocate", "default", "dimension", "do", "else", "elseif", "end", "enddo", "endif", "entry", "enum", "enumerator", "equivalence", "error", "stop", "exit", "extends", "external", "final", "forall", "format", "function", "generic", "goto", "if", "implicit", "in", "include", "inquire", "intent", "interface", "intrinsic", "module", "namelist", "none", "nullify", "only", "open", "operator", "optional", "parameter", "pointer", "print", "private", "procedure", "program", "protected", "public", "pure", "read", "real", "recursive", "result", "return", "rewind", "save", "select", "sequence", "stop", "subroutine", "target", "then", "type", "use", "value", "volatile", "wait", "where", "while", "write"],
        fold_case: true,
    },
    LangSpec {
        name: "Fortran (fixed form)",
        line: &["!"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["allocatable", "allocate", "assign", "associate", "block", "call", "case", "class", "close", "common", "contains", "continue", "cycle", "data", "deallocate", "default", "dimension", "do", "else", "elseif", "end", "enddo", "endif", "entry", "equivalence", "error", "stop", "exit", "external", "final", "format", "function", "goto", "if", "implicit", "include", "inquire", "intent", "interface", "intrinsic", "module", "namelist", "none", "nullify", "open", "operator", "optional", "parameter", "pointer", "print", "private", "procedure", "program", "protected", "public", "pure", "read", "real", "recursive", "result", "return", "rewind", "save", "select", "sequence", "stop", "subroutine", "target", "then", "type", "use", "value", "volatile", "wait", "where", "while", "write"],
        fold_case: true,
    },
    LangSpec {
        name: "Freebasic",
        line: &["'"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "asm", "base", "byref", "byval", "call", "case", "class", "close", "const", "continue", "data", "declare", "defbyte", "defdbl", "defint", "deflng", "defsng", "defstr", "dim", "do", "double", "else", "elseif", "end", "enum", "exit", "export", "extern", "for", "function", "get", "gosub", "goto", "if", "input", "integer", "let", "line", "long", "loop", "mod", "next", "not", "or", "print", "private", "public", "redim", "return", "select", "shared", "short", "single", "static", "step", "string", "sub", "then", "to", "type", "union", "until", "wend", "while", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "GDScript",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "assert", "await", "break", "breakpoint", "class", "class_name", "const", "continue", "elif", "else", "enum", "export", "extends", "false", "for", "func", "if", "in", "is", "match", "master", "namespace", "not", "null", "onready", "or", "pass", "preload", "remote", "return", "self", "setget", "signal", "static", "tool", "true", "var", "while", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Gleam",
        line: &["//"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["BitString", "Bool", "Error", "False", "Float", "Int", "List", "Nil", "None", "Ok", "Result", "Some", "String", "True", "UtfCodepoint", "as", "assert", "auto", "case", "const", "echo", "external", "fn", "if", "import", "let", "opaque", "panic", "pub", "todo", "type", "use"],
        fold_case: false,
    },
    LangSpec {
        name: "GLSL",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["attribute", "bool", "break", "buffer", "case", "centroid", "coherent", "const", "continue", "default", "discard", "do", "double", "else", "false", "flat", "float", "for", "highp", "if", "in", "inout", "int", "invariant", "layout", "lowp", "mat2", "mat3", "mat4", "mediump", "noperspective", "out", "patch", "precision", "precise", "readonly", "restrict", "return", "sample", "shared", "smooth", "struct", "subroutine", "switch", "true", "uint", "uniform", "varying", "vec2", "vec3", "vec4", "void", "volatile", "while", "writeonly"],
        fold_case: false,
    },
    LangSpec {
        name: "Go",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "bool", "byte", "complex64", "complex128", "error", "float32", "float64", "int", "int8", "int16", "int32", "int64", "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64", "uintptr", "true", "false", "nil", "iota"],
        fold_case: false,
    },
    LangSpec {
        name: "GraphQL",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["Boolean", "Float", "ID", "Int", "String", "directive", "enum", "extend", "false", "fragment", "implements", "input", "interface", "mutation", "null", "on", "query", "repeatable", "scalar", "schema", "subscription", "true", "type", "union"],
        fold_case: false,
    },
    LangSpec {
        name: "Groovy",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "as", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "def", "default", "do", "double", "else", "enum", "extends", "false", "final", "finally", "float", "for", "goto", "if", "implements", "import", "in", "instanceof", "int", "interface", "long", "native", "new", "null", "package", "private", "protected", "public", "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this", "threadsafe", "throw", "throws", "trait", "transient", "true", "try", "var", "void", "volatile", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Gui4Cli",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["if", "else", "endif", "for", "next", "while", "endwhile", "goto", "gosub", "return", "window", "control", "button", "text", "edit", "list"],
        fold_case: true,
    },
    LangSpec {
        name: "Haskell",
        line: &["--"],
        block: Some(("{-", "-}")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["case", "class", "data", "default", "deriving", "do", "else", "if", "import", "in", "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "then", "type", "where", "forall", "mdo", "proc", "rec"],
        fold_case: false,
    },
    LangSpec {
        name: "HCL",
        line: &["#", "//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["backend", "can", "check", "connection", "contains", "count", "data", "depends_on", "dynamic", "else", "false", "for", "for_each", "if", "import", "in", "locals", "module", "moved", "null", "output", "provider", "provisioner", "required_providers", "required_version", "resource", "terraform", "true", "try", "variable", "version", "workspace"],
        fold_case: false,
    },
    LangSpec {
        name: "HLSL",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["Texture2D", "TextureCube", "blend_state", "cbuffer", "centroid", "column_major", "compile", "const", "depth_stencil_state", "discard", "do", "double", "else", "extern", "false", "float", "float2", "float3", "float4", "for", "groupshared", "half", "if", "in", "inline", "inout", "int", "int2", "int3", "int4", "linear", "matrix", "namespace", "nointerpolation", "noperspective", "numthreads", "out", "packoffset", "pass", "point", "precise", "rasterizer_state", "register", "return", "row_major", "sample", "sampler", "sampler2D", "samplerCube", "shared", "static", "string", "struct", "switch", "technique", "texture", "true", "typedef", "uint", "uniform", "vector", "void", "volatile", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Hollywood",
        line: &[";"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "break", "case", "const", "continue", "default", "do", "else", "elseif", "endfor", "endfunction", "endif", "endswitch", "endwhile", "false", "for", "foreach", "function", "global", "goto", "if", "local", "nil", "not", "or", "repeat", "return", "switch", "then", "to", "true", "until", "while"],
        fold_case: true,
    },
    LangSpec {
        name: "HTML",
        line: &[],
        block: Some(("<!--", "-->")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: true,
    },
    LangSpec {
        name: "INI file",
        line: &[";", "#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: true,
    },
    LangSpec {
        name: "Inno Setup",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["begin", "const", "end", "else", "for", "if", "procedure", "function", "var", "while", "do", "then", "case", "of", "repeat", "until", "type", "array", "record", "string", "integer", "boolean"],
        fold_case: true,
    },
    LangSpec {
        name: "Intel HEX",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Java",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native", "new", "package", "private", "protected", "public", "return", "short", "static", "strictfp", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void", "volatile", "while", "true", "false", "null", "var", "record", "sealed", "permits"],
        fold_case: false,
    },
    LangSpec {
        name: "JavaScript",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        apostrophe: Apostrophe::Line,
        keywords: &["async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "with", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Jinja",
        line: &[],
        block: Some(("{#", "#}")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "autoescape", "block", "call", "cycle", "do", "else", "elif", "endautoescape", "endblock", "endcall", "endfilter", "endfor", "endif", "endmacro", "endraw", "endset", "endtrans", "endwith", "extends", "false", "filter", "for", "from", "if", "import", "in", "include", "is", "loop", "macro", "none", "not", "or", "raw", "recursive", "set", "super", "trans", "true", "varargs", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "JSON",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["true", "false", "null"],
        fold_case: false,
    },
    LangSpec {
        name: "JSON5",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["true", "false", "null", "Infinity", "NaN"],
        fold_case: false,
    },
    LangSpec {
        name: "JSP",
        line: &[],
        block: Some(("<%--", "--%>")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: true,
    },
    LangSpec {
        name: "Julia",
        line: &["#"],
        block: Some(("#=", "=#")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "baremodule", "begin", "break", "catch", "const", "continue", "do", "else", "elseif", "end", "export", "false", "finally", "for", "function", "global", "if", "import", "in", "isa", "let", "local", "macro", "module", "mutable", "primitive", "quote", "return", "struct", "true", "try", "using", "where", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Kotlin",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "actual", "annotation", "as", "break", "by", "catch", "class", "companion", "const", "constructor", "continue", "crossinline", "data", "do", "else", "enum", "expect", "external", "false", "final", "finally", "for", "fun", "if", "import", "in", "infix", "init", "inline", "inner", "interface", "internal", "is", "lateinit", "noinline", "null", "object", "open", "operator", "out", "override", "package", "private", "protected", "public", "reified", "return", "sealed", "super", "suspend", "tailrec", "this", "throw", "true", "try", "typealias", "val", "var", "vararg", "when", "where", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "LaTeX",
        line: &["%"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["begin", "end", "documentclass", "usepackage", "newcommand", "renewcommand", "def", "let", "section", "subsection", "subsubsection", "paragraph", "chapter", "part", "label", "ref", "cite", "bibliography", "bibliographystyle", "include", "input", "item", "textbf", "textit", "emph", "footnote", "title", "author", "date", "maketitle", "tableofcontents"],
        fold_case: false,
    },
    LangSpec {
        name: "Less",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "calc", "charset", "default", "each", "extend", "false", "font-face", "for", "hide", "if", "import", "important", "iscolor", "isem", "iskeyword", "isnumber", "ispercentage", "ispixel", "isruleset", "isstring", "isunit", "isurl", "keyframes", "layer", "media", "mixin", "namespace", "not", "null", "or", "page", "plugin", "property", "show", "supports", "true", "url", "use", "var", "viewport", "when", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "LISP",
        line: &[";"],
        block: Some(("#|", "|#")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["defun", "defmacro", "defvar", "defparameter", "defconstant", "lambda", "let", "let*", "if", "cond", "case", "when", "unless", "progn", "prog1", "prog2", "setq", "setf", "quote", "function", "loop", "do", "dolist", "dotimes", "return", "from", "return-from", "block", "tagbody", "go", "catch", "throw", "unwind-protect", "handler-case", "eval"],
        fold_case: false,
    },
    LangSpec {
        name: "Lua",
        line: &["--"],
        block: Some(("--[[", "]]")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Makefile",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["define", "endef", "else", "endif", "export", "ifdef", "ifeq", "ifndef", "ifneq", "include", "override", "private", "sinclude", "undefine", "unexport", "vpath"],
        fold_case: false,
    },
    LangSpec {
        name: "Markdown",
        line: &[],
        block: Some(("<!--", "-->")),
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Matlab",
        line: &["%"],
        block: Some(("%{", "%}")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["break", "case", "catch", "classdef", "continue", "else", "elseif", "end", "for", "function", "global", "if", "otherwise", "parfor", "persistent", "return", "spmd", "switch", "try", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Meson",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["add_global_arguments", "add_languages", "add_project_arguments", "add_test_setup", "alias_target", "and", "assert", "benchmark", "both_libraries", "break", "build_target", "configuration_data", "configure_file", "continue", "custom_target", "declare_dependency", "dependency", "disabler", "elif", "else", "endif", "environment", "error", "executable", "false", "files", "find_program", "foreach", "generator", "get_option", "get_variable", "if", "import", "in", "include_directories", "install_data", "install_headers", "install_man", "install_subdir", "is_disabler", "is_variable", "jar", "join_paths", "library", "message", "not", "or", "project", "range", "run_command", "set_variable", "shared_library", "shared_module", "static_library", "subdir", "subdir_done", "subproject", "summary", "test", "true", "vcs_tag", "warning"],
        fold_case: false,
    },
    LangSpec {
        name: "Microsoft Transact-SQL",
        line: &["--"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["add", "all", "alter", "and", "any", "as", "asc", "authorization", "backup", "begin", "between", "break", "browse", "bulk", "by", "cascade", "case", "check", "checkpoint", "close", "clustered", "coalesce", "collate", "column", "commit", "compute", "constraint", "contains", "continue", "create", "cross", "current", "current_date", "current_time", "current_timestamp", "current_user", "cursor", "database", "dbcc", "deallocate", "declare", "default", "delete", "deny", "desc", "disk", "distinct", "distributed", "double", "drop", "dump", "else", "end", "errlvl", "escape", "except", "exec", "execute", "exists", "exit", "external", "fetch", "file", "fillfactor", "for", "foreign", "freetext", "from", "full", "function", "goto", "grant", "group", "having", "holdlock", "identity", "identity_insert", "identitycol", "if", "in", "index", "inner", "insert", "intersect", "into", "is", "join", "key", "kill", "left", "like", "lineno", "load", "merge", "national", "nocheck", "nonclustered", "not", "null", "nullif", "of", "off", "offsets", "on", "open", "option", "or", "order", "outer", "over", "percent", "pivot", "plan", "precision", "primary", "print", "proc", "procedure", "public", "raiserror", "read", "readtext", "reconfigure", "references", "replication", "restore", "restrict", "return", "revert", "revoke", "right", "rollback", "rowcount", "rowguidcol", "rule", "save", "schema", "select", "session_user", "set", "setuser", "shutdown", "some", "statistics", "system_user", "table", "tablesample", "textsize", "then", "to", "top", "tran", "transaction", "trigger", "truncate", "tsequal", "union", "unique", "unpivot", "update", "updatetext", "use", "user", "values", "varying", "view", "waitfor", "when", "where", "while", "with", "writetext"],
        fold_case: true,
    },
    LangSpec {
        name: "MMIXAL",
        line: &["%"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["ADD", "ADDU", "AND", "ANDN", "BDIF", "BEV", "BN", "BNN", "BNP", "BNZ", "BOD", "BP", "BZ", "CMP", "CMPU", "CSN", "CSNN", "CSNP", "CSNZ", "CSP", "CSWAP", "CSZ", "DIV", "DIVU", "FADD", "FCMP", "FCMPE", "FDIV", "FEQL", "FEQLE", "FINT", "FIX", "FIXU", "FLOT", "FLOTU", "FMUL", "FREM", "FSQRT", "FSUB", "FUN", "FUNE", "GET", "GETA", "GO", "INCH", "INCL", "INCM", "INCMH", "INCML", "JMP", "LDB", "LDBU", "LDHT", "LDO", "LDOU", "LDSF", "LDT", "LDTU", "LDUNC", "LDVTS", "LDW", "LDWU", "MOR", "MUL", "MULU", "MUX", "MXOR", "NAND", "NEG", "NEGU", "NOR", "NXOR", "ODIF", "OR", "ORN", "ORH", "ORL", "ORM", "PBOD", "PREGO", "PRELD", "PREST", "PRI", "PUSHGO", "PUSHJ", "PUT", "RESUME", "SADD", "SAVE", "SETH", "SETL", "SETMH", "SETML", "SETSFL", "SFLOT", "SFLOTU", "SL", "SLU", "SR", "SRU", "STB", "STBU", "STCO", "STHT", "STO", "STOU", "STSF", "STT", "STTU", "STUNC", "STW", "STWU", "SUB", "SUBU", "SYNC", "SYNCD", "SYNCID", "TDIF", "TRAP", "TRIP", "UNSAVE", "WDIF", "XOR", "ZSEV", "ZSN", "ZSNN", "ZSNP", "ZSNZ", "ZSP", "ZSZ"],
        fold_case: true,
    },
    LangSpec {
        name: "MS-DOS Style",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Mustache",
        line: &[],
        block: Some(("{{!", "}}")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["comment", "each", "else", "end", "first", "if", "index", "inverted", "last", "length", "partial", "section", "this", "unless", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "MySQL",
        line: &["--", "#"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        apostrophe: Apostrophe::Multiline,
        keywords: &["add", "after", "alter", "and", "as", "asc", "auto_increment", "before", "begin", "between", "by", "call", "case", "char", "check", "column", "commit", "constraint", "create", "cross", "database", "default", "delete", "desc", "distinct", "drop", "else", "end", "exists", "foreign", "from", "full", "grant", "group", "having", "if", "in", "index", "inner", "insert", "int", "into", "is", "join", "key", "left", "like", "limit", "not", "null", "on", "or", "order", "outer", "primary", "references", "right", "select", "set", "table", "then", "union", "unique", "update", "use", "using", "values", "view", "when", "where"],
        fold_case: true,
    },
    LangSpec {
        name: "Nim",
        line: &["#"],
        block: Some(("#[", "]#")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["addr", "and", "as", "asm", "bind", "block", "break", "case", "cast", "concept", "const", "continue", "converter", "defer", "discard", "distinct", "div", "do", "elif", "else", "end", "enum", "except", "export", "finally", "for", "from", "func", "if", "import", "in", "include", "interface", "is", "isnot", "iterator", "let", "macro", "method", "mixin", "mod", "nil", "not", "notin", "object", "of", "or", "out", "proc", "ptr", "raise", "ref", "return", "shl", "shr", "static", "template", "try", "tuple", "type", "using", "var", "when", "while", "xor", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Ninja",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["build", "command", "console", "default", "depfile", "deps", "depth", "description", "dyndep", "generator", "in", "in_newline", "include", "msvc_deps_prefix", "out", "phony", "pool", "restat", "rule", "rspfile", "rspfile_content", "subninja"],
        fold_case: false,
    },
    LangSpec {
        name: "Nix",
        line: &["#"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["abort", "and", "assert", "attrNames", "attrValues", "baseNameOf", "builtins", "concatStringsSep", "deepSeq", "derivation", "dirOf", "elem", "elemAt", "else", "false", "fetchGit", "fetchTarball", "filter", "foldl", "functionArgs", "genList", "getAttr", "hasAttr", "head", "if", "import", "in", "inherit", "isNull", "length", "let", "listToAttrs", "map", "mapAttrs", "null", "or", "pathExists", "placeholder", "readDir", "readFile", "rec", "removeAttrs", "scopedImport", "seq", "sort", "super", "then", "throw", "toPath", "toString", "trace", "true", "tryEval", "typeOf", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "Nncrontab",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "NSIS",
        line: &[";"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["Section", "SectionEnd", "Function", "FunctionEnd", "If", "IfFileExists", "Else", "EndIf", "Goto", "Label", "Call", "Return", "StrCpy", "StrCmp", "IntOp", "IntCmp", "MessageBox", "SetOutPath", "File", "WriteRegStr", "ReadRegStr", "Delete", "Rename", "CreateDirectory", "RMDir", "DeleteRegKey", "WriteUninstaller", "Var"],
        fold_case: true,
    },
    LangSpec {
        name: "Nushell",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["alias", "break", "catch", "cd", "const", "continue", "cp", "def", "def-env", "describe", "do", "each", "echo", "else", "error", "export", "extern", "for", "from", "get", "group-by", "help", "hide", "if", "in", "insert", "let", "loop", "ls", "make", "match", "module", "mut", "mv", "open", "overlay", "print", "reduce", "reject", "return", "rm", "save", "select", "sort-by", "source", "source-env", "table", "throw", "to", "try", "type", "update", "upsert", "use", "version", "where", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Objective-C",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "id", "self", "super", "nil", "YES", "NO", "interface", "implementation", "end", "property", "synthesize", "protocol", "selector", "class", "import"],
        fold_case: false,
    },
    LangSpec {
        name: "Org",
        line: &["# "],
        block: Some(("#+BEGIN_COMMENT", "#+END_COMMENT")),
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["AUTHOR", "BEGIN_SRC", "CAPTION", "DATE", "END_SRC", "FILETAGS", "HTML_HEAD", "INCLUDE", "LABEL", "LATEX_HEADER", "LINK", "MACRO", "NAME", "OPTIONS", "PRIORITIES", "PROPERTY", "RESULTS", "SEQ_TODO", "SETUPFILE", "STARTUP", "TAGS", "TITLE", "TODO", "TYP_TODO", "attr_html", "begin_center", "begin_comment", "begin_example", "begin_export", "begin_quote", "begin_src", "begin_verse", "end_center", "end_comment", "end_example", "end_export", "end_quote", "end_src", "end_verse"],
        fold_case: false,
    },
    LangSpec {
        name: "OScript",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["break", "by", "case", "catch", "continue", "default", "do", "downto", "else", "end", "endcase", "endfor", "endif", "endrepeat", "endtry", "for", "function", "if", "in", "loop", "object", "repeat", "return", "switch", "try", "until", "var", "while"],
        fold_case: true,
    },
    LangSpec {
        name: "Pascal",
        line: &["//"],
        block: Some(("{", "}")),
        quotes: &['\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "array", "asm", "begin", "case", "class", "const", "constructor", "destructor", "div", "do", "downto", "else", "end", "except", "exports", "file", "finalization", "finally", "for", "function", "goto", "if", "implementation", "in", "inherited", "initialization", "inline", "interface", "is", "label", "library", "mod", "nil", "not", "object", "of", "on", "operator", "or", "out", "packed", "procedure", "program", "property", "raise", "record", "repeat", "resourcestring", "set", "shl", "shr", "string", "then", "threadvar", "to", "try", "type", "unit", "until", "uses", "var", "while", "with", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "Perl",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["my", "our", "local", "sub", "if", "elsif", "else", "unless", "while", "until", "for", "foreach", "do", "last", "next", "redo", "return", "package", "use", "require", "BEGIN", "END", "and", "or", "not", "eq", "ne", "lt", "gt", "le", "ge", "cmp", "x", "qw", "q", "qq", "m", "s", "tr", "y", "print", "printf", "die", "warn", "eval"],
        fold_case: false,
    },
    LangSpec {
        name: "PHP",
        line: &["//", "#"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone", "const", "continue", "declare", "default", "do", "echo", "else", "elseif", "empty", "enddeclare", "endfor", "endforeach", "endif", "endswitch", "endwhile", "enum", "extends", "final", "finally", "fn", "for", "foreach", "function", "global", "goto", "if", "implements", "include", "include_once", "instanceof", "insteadof", "interface", "isset", "list", "match", "namespace", "new", "or", "print", "private", "protected", "public", "readonly", "require", "require_once", "return", "static", "switch", "throw", "trait", "try", "unset", "use", "var", "while", "xor", "yield", "true", "false", "null"],
        fold_case: false,
    },
    LangSpec {
        name: "PostgreSQL",
        line: &["--"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["all", "alter", "and", "as", "asc", "between", "by", "case", "check", "column", "constraint", "create", "cross", "default", "delete", "desc", "distinct", "drop", "else", "end", "except", "exists", "false", "foreign", "from", "full", "function", "grant", "group", "having", "if", "in", "index", "inner", "insert", "into", "is", "join", "left", "like", "limit", "not", "null", "on", "or", "order", "outer", "partition", "primary", "references", "returning", "revoke", "right", "rollback", "schema", "select", "set", "table", "then", "true", "truncate", "union", "unique", "update", "using", "values", "view", "when", "where", "with"],
        fold_case: true,
    },
    LangSpec {
        name: "PostScript",
        line: &["%"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["add", "sub", "mul", "div", "idiv", "mod", "neg", "abs", "ceiling", "floor", "round", "truncate", "sqrt", "sin", "cos", "atan", "exp", "ln", "log", "rand", "srand", "copy", "index", "roll", "clear", "count", "exec", "if", "ifelse", "for", "repeat", "loop", "exit", "stop", "stopped", "load", "store", "def", "begin", "end", "dict", "array", "string", "length", "get", "put", "getinterval", "putinterval", "findfont", "scalefont", "setfont", "show", "moveto", "lineto", "curveto", "closepath", "stroke", "fill", "newpath", "grestore", "gsave", "translate", "scale", "rotate"],
        fold_case: false,
    },
    LangSpec {
        name: "PowerShell",
        line: &["#"],
        block: Some(("<#", "#>")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["begin", "break", "catch", "class", "continue", "data", "define", "do", "dynamicparam", "else", "elseif", "end", "enum", "exit", "filter", "finally", "for", "foreach", "from", "function", "hidden", "if", "in", "param", "process", "return", "static", "switch", "throw", "trap", "try", "until", "using", "var", "while", "workflow"],
        fold_case: true,
    },
    LangSpec {
        name: "Prolog",
        line: &["%"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["append", "arg", "assert", "asserta", "assertz", "atom", "bagof", "between", "call", "catch", "consult", "copy_term", "discontiguous", "dynamic", "else", "endif", "ensure_loaded", "fail", "false", "findall", "forall", "format", "functor", "ground", "halt", "if", "ignore", "include", "initialization", "is", "is_list", "length", "listing", "member", "meta_predicate", "module", "multifile", "nl", "number", "once", "op", "print", "public", "read", "retract", "retractall", "set_prolog_flag", "setof", "throw", "true", "use_module", "var", "write"],
        fold_case: false,
    },
    LangSpec {
        name: "Properties",
        line: &["#", "!"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "Protocol Buffers",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["bool", "bytes", "double", "enum", "extend", "extensions", "false", "fixed32", "fixed64", "float", "group", "import", "int32", "int64", "map", "max", "message", "oneof", "option", "optional", "package", "public", "repeated", "required", "reserved", "returns", "rpc", "service", "sfixed32", "sfixed64", "sint32", "sint64", "stream", "string", "syntax", "to", "true", "uint32", "uint64", "weak"],
        fold_case: false,
    },
    LangSpec {
        name: "Puppet",
        line: &["#"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["after", "and", "before", "case", "class", "cron", "debug", "default", "define", "else", "elsif", "err", "exec", "fail", "false", "file", "function", "group", "host", "if", "include", "info", "inherit", "in", "mount", "node", "notice", "notify", "or", "package", "require", "resource", "schedule", "service", "ssh_authorized_key", "sshkey", "stage", "subscribe", "tag", "tidy", "true", "undef", "unless", "user", "warning"],
        fold_case: false,
    },
    LangSpec {
        name: "Purebasic",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "array", "as", "break", "byref", "byval", "case", "compilerendif", "compilerif", "compilerelse", "compilerselect", "compilercase", "compilerdefault", "compilerendselect", "compilerselect", "continue", "data", "dataSection", "declare", "default", "define", "dim", "do", "double", "else", "elseif", "end", "enddatasection", "endif", "endselect", "endenumeration", "endinterface", "endprocedure", "endstructure", "endstructureunion", "endwith", "enumeration", "extends", "fake", "return", "for", "foreach", "forever", "global", "gosub", "goto", "if", "include", "includeBinary", "includefile", "includepath", "input", "integer", "interface", "list", "long", "loop", "map", "module", "newlist", "next", "not", "or", "procedure", "protected", "prototype", "read", "real", "redim", "repeat", "restore", "return", "select", "shared", "short", "static", "step", "string", "structure", "structureunion", "swap", "threaded", "to", "type", "until", "use", "wend", "while", "with", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "PureScript",
        line: &["--"],
        block: Some(("{-", "-}")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["Aff", "Array", "Boolean", "Char", "Effect", "Either", "IO", "Int", "Maybe", "Number", "String", "Unit", "ado", "case", "class", "data", "derive", "do", "else", "false", "forall", "foreign", "if", "import", "in", "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "then", "true", "type", "where"],
        fold_case: false,
    },
    LangSpec {
        name: "Python",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        // A single quote never spans lines in Python; the triple quote does,
        // and a delimiter of three is more than a line-at-a-time lexer can
        // count. Its body is coloured as code rather than the file after it
        // being coloured as a string.
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "True", "False", "None", "match", "case"],
        fold_case: false,
    },
    LangSpec {
        name: "R",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["if", "else", "repeat", "while", "function", "for", "in", "next", "break", "TRUE", "FALSE", "NULL", "Inf", "NaN", "NA", "library", "require", "return"],
        fold_case: false,
    },
    LangSpec {
        name: "Racket",
        line: &[";"],
        block: Some(("#|", "|#")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "begin", "case", "cond", "cons", "define", "define-syntax", "define-values", "delay", "display", "do", "else", "error", "for", "for/fold", "for/list", "force", "if", "lambda", "let", "let-values", "letrec", "list", "match", "module", "module+", "newline", "not", "null", "or", "parameterize", "provide", "quasiquote", "quote", "raise", "require", "struct", "syntax-rules", "unless", "unquote", "values", "when", "with-handlers"],
        fold_case: false,
    },
    LangSpec {
        name: "Raku",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["my", "our", "has", "sub", "method", "class", "role", "module", "use", "need", "if", "elsif", "else", "unless", "while", "until", "for", "loop", "repeat", "return", "last", "next", "redo", "die", "say", "print", "put", "given", "when", "default", "state", "constant", "enum", "subset", "multi", "proto", "only", "augment", "supersede"],
        fold_case: false,
    },
    LangSpec {
        name: "REBOL",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["all", "any", "attempt", "break", "case", "catch", "compose", "do", "either", "else", "exit", "for", "forall", "foreach", "forever", "forskip", "func", "function", "halt", "if", "loop", "map", "next", "not", "now", "or", "print", "probe", "reduce", "repeat", "return", "reverse", "switch", "throw", "try", "unless", "until", "while"],
        fold_case: true,
    },
    LangSpec {
        name: "Registry",
        line: &[";"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["Windows", "Registry", "Editor", "Version", "HKEY_LOCAL_MACHINE", "HKEY_CURRENT_USER", "HKEY_CLASSES_ROOT", "HKEY_USERS", "HKEY_CURRENT_CONFIG", "dword", "hex"],
        fold_case: true,
    },
    LangSpec {
        name: "Resource file",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["ACCELERATORS", "BITMAP", "CAPTION", "CHECKBOX", "CLASS", "COMBOBOX", "CONTROL", "CTEXT", "CURSOR", "DEFPUSHBUTTON", "DIALOG", "DIALOGEX", "EDITTEXT", "EXSTYLE", "FONT", "GROUPBOX", "ICON", "LANGUAGE", "LISTBOX", "LTEXT", "MENU", "MENUEX", "MENUITEM", "POPUP", "PUSHBUTTON", "RADIOBUTTON", "RCDATA", "RTEXT", "SCROLLBAR", "SEPARATOR", "STRINGTABLE", "STYLE", "VERSION", "VERSIONINFO", "VALUE", "BLOCK", "BEGIN", "END"],
        fold_case: true,
    },
    LangSpec {
        name: "reStructuredText",
        line: &[".."],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["admonition", "attention", "caution", "class", "code", "compound", "container", "contents", "csv-table", "danger", "date", "default-role", "directive", "epigraph", "error", "figure", "footer", "header", "highlights", "hint", "image", "important", "include", "index", "line-block", "list-table", "math", "meta", "note", "parsed-literal", "pull-quote", "raw", "replace", "role", "rubric", "sectnum", "section", "sidebar", "sourcecode", "table", "target-notes", "tip", "title", "topic", "unicode", "warning"],
        fold_case: false,
    },
    LangSpec {
        name: "Ruby",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["alias", "and", "begin", "break", "case", "class", "def", "defined", "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until", "when", "while", "yield", "require", "require_relative", "attr_accessor", "attr_reader", "attr_writer", "include", "extend", "prepend", "lambda", "proc", "raise"],
        fold_case: false,
    },
    LangSpec {
        name: "Rust",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield", "union", "macro_rules"],
        fold_case: false,
    },
    LangSpec {
        name: "SAS",
        line: &[],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["data", "set", "run", "proc", "var", "by", "class", "model", "output", "input", "infile", "file", "put", "if", "then", "else", "do", "end", "where", "merge", "keep", "drop", "rename", "format", "informat", "length", "retain", "array", "select", "when", "otherwise", "goto", "return", "stop"],
        fold_case: true,
    },
    LangSpec {
        name: "Sass",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "at-root", "calc", "charset", "content", "debug", "each", "else", "error", "extend", "false", "font-face", "for", "forward", "from", "function", "hide", "if", "important", "import", "include", "keyframes", "layer", "media", "mixin", "namespace", "not", "null", "or", "page", "placeholder", "return", "show", "supports", "true", "url", "use", "var", "warn", "while", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "SCSS",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "as", "at-root", "calc", "charset", "content", "debug", "each", "else", "error", "extend", "false", "font-face", "for", "forward", "from", "function", "hide", "if", "important", "import", "include", "keyframes", "layer", "media", "mixin", "namespace", "not", "null", "or", "page", "placeholder", "return", "show", "supports", "true", "url", "use", "var", "warn", "while", "with"],
        fold_case: false,
    },
    LangSpec {
        name: "Sed",
        line: &["#"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["D", "F", "G", "H", "N", "P", "Q", "R", "T", "W", "a", "b", "c", "d", "e", "g", "h", "i", "l", "n", "p", "q", "r", "s", "t", "w", "x", "y", "z"],
        fold_case: false,
    },
    LangSpec {
        name: "Shell",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "function", "in", "select", "time", "return", "exit", "break", "continue", "local", "export", "readonly", "declare", "typeset", "unset", "shift", "eval", "exec", "trap", "set", "source", "alias", "echo", "printf", "read", "cd", "test"],
        fold_case: false,
    },
    LangSpec {
        name: "Scheme",
        line: &[";"],
        block: Some(("#|", "|#")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["define", "lambda", "let", "let*", "letrec", "if", "cond", "case", "and", "or", "not", "begin", "do", "set!", "quote", "quasiquote", "unquote", "unquote-splicing", "delay", "force", "call/cc", "call-with-current-continuation", "map", "for-each", "apply"],
        fold_case: false,
    },
    LangSpec {
        name: "Smalltalk",
        line: &["\""],
        block: None,
        quotes: &['\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["self", "super", "nil", "true", "false", "thisContext", "ifTrue", "ifFalse", "whileTrue", "whileFalse", "do", "collect", "select", "reject", "detect", "inject", "timesRepeat", "to", "do", "to", "by", "new", "class", "subclass", "instanceVariableNames"],
        fold_case: false,
    },
    LangSpec {
        name: "Solidity",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "address", "anonymous", "as", "assembly", "bool", "break", "bytes", "bytes32", "calldata", "catch", "constant", "constructor", "continue", "contract", "delete", "do", "else", "emit", "enum", "error", "event", "external", "fallback", "false", "for", "function", "if", "immutable", "import", "indexed", "int", "interface", "internal", "is", "library", "mapping", "memory", "modifier", "new", "override", "payable", "pragma", "private", "public", "pure", "receive", "require", "return", "returns", "revert", "storage", "string", "struct", "super", "this", "true", "try", "type", "uint", "uint256", "unchecked", "using", "view", "virtual", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Spice",
        line: &[],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["subckt", "ends", "model", "param", "include", "lib", "global", "temp", "options", "tran", "ac", "dc", "op", "print", "plot", "probe", "measure", "alter"],
        fold_case: true,
    },
    LangSpec {
        name: "SQL",
        line: &["--"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Multiline,
        keywords: &["add", "all", "alter", "and", "any", "as", "asc", "authorization", "begin", "between", "by", "cascade", "case", "check", "column", "commit", "constraint", "create", "cross", "current", "current_date", "current_time", "current_timestamp", "cursor", "database", "default", "delete", "desc", "distinct", "drop", "else", "end", "escape", "except", "exists", "fetch", "for", "foreign", "from", "full", "grant", "group", "having", "in", "index", "inner", "insert", "intersect", "into", "is", "join", "key", "left", "like", "limit", "natural", "not", "null", "offset", "on", "only", "open", "or", "order", "outer", "primary", "references", "revoke", "right", "rollback", "schema", "select", "set", "table", "then", "to", "transaction", "trigger", "union", "unique", "update", "user", "using", "values", "view", "when", "where", "with"],
        fold_case: true,
    },
    LangSpec {
        name: "Structured Text",
        line: &["//"],
        block: Some(("(*", "*)")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["and", "array", "case", "do", "else", "elsif", "end_case", "end_for", "end_if", "end_repeat", "end_while", "exit", "for", "function", "if", "mod", "not", "of", "or", "procedure", "program", "repeat", "return", "then", "to", "type", "until", "var", "while", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "Svelte",
        line: &["//"],
        block: Some(("<!--", "-->")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["animate", "await", "bind", "body", "catch", "class", "component", "const", "context", "debug", "each", "element", "else", "export", "false", "fragment", "from", "head", "html", "if", "in", "import", "let", "module", "on", "options", "out", "script", "self", "slot", "style", "then", "transition", "true", "undefined", "use", "window"],
        fold_case: false,
    },
    LangSpec {
        name: "Swift",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["associatedtype", "class", "deinit", "enum", "extension", "fileprivate", "func", "import", "init", "inout", "internal", "let", "open", "operator", "private", "protocol", "public", "rethrows", "static", "struct", "subscript", "typealias", "var", "break", "case", "continue", "default", "defer", "do", "else", "fallthrough", "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as", "catch", "false", "is", "nil", "super", "self", "Self", "throw", "throws", "true", "try"],
        fold_case: false,
    },
    LangSpec {
        name: "S-Record",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "SystemVerilog",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["always", "always_comb", "always_ff", "always_latch", "assert", "assign", "begin", "bit", "byte", "case", "class", "const", "constraint", "do", "else", "end", "endcase", "endclass", "endfunction", "endgenerate", "endinterface", "endmodule", "endpackage", "endtask", "enum", "extends", "extern", "final", "for", "foreach", "forever", "fork", "function", "generate", "genvar", "if", "implements", "import", "initial", "inout", "input", "int", "integer", "interface", "join", "localparam", "logic", "longint", "module", "new", "output", "package", "parameter", "program", "property", "rand", "randc", "real", "repeat", "return", "signed", "string", "struct", "task", "this", "typedef", "union", "void", "while", "wire"],
        fold_case: false,
    },
    LangSpec {
        name: "TCL",
        line: &["#"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["after", "append", "array", "break", "catch", "cd", "close", "concat", "continue", "dict", "else", "elseif", "eof", "error", "eval", "exec", "exit", "expr", "fblocked", "fconfigure", "fcopy", "file", "fileevent", "flush", "for", "foreach", "format", "gets", "glob", "global", "history", "if", "incr", "info", "interp", "join", "lappend", "lindex", "linsert", "list", "llength", "lrange", "lreplace", "lsearch", "lset", "lsort", "namespace", "open", "package", "pid", "proc", "puts", "pwd", "read", "regexp", "regsub", "rename", "return", "scan", "seek", "set", "socket", "source", "split", "string", "subst", "switch", "tell", "time", "trace", "unset", "update", "uplevel", "upvar", "variable", "vwait", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Tektronix extended HEX",
        line: &[],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "TeX",
        line: &["%"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["begin", "end", "def", "let", "newcommand", "renewcommand", "documentclass", "usepackage", "section", "subsection", "subsubsection", "paragraph", "chapter", "part", "label", "ref", "cite", "bibliography", "include", "input", "item", "textbf", "textit", "emph", "footnote", "title", "author", "date", "maketitle", "tableofcontents", "hbox", "vbox", "hskip", "vskip", "par"],
        fold_case: false,
    },
    LangSpec {
        name: "Thrift",
        line: &["//", "#"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["async", "binary", "bool", "byte", "const", "cpp_type", "double", "enum", "exception", "extends", "false", "i16", "i32", "i64", "i8", "include", "list", "map", "namespace", "oneway", "optional", "required", "senum", "service", "set", "string", "struct", "throws", "true", "typedef", "union", "void"],
        fold_case: false,
    },
    LangSpec {
        name: "TOML",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["true", "false"],
        fold_case: false,
    },
    LangSpec {
        name: "txt2tags",
        line: &["%"],
        block: None,
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: false,
    },
    LangSpec {
        name: "TypeScript",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        apostrophe: Apostrophe::Line,
        keywords: &["abstract", "any", "as", "asserts", "async", "await", "boolean", "break", "case", "catch", "class", "const", "constructor", "continue", "debugger", "declare", "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for", "from", "function", "get", "if", "implements", "import", "in", "infer", "instanceof", "interface", "is", "keyof", "let", "module", "namespace", "never", "new", "null", "number", "object", "of", "package", "private", "protected", "public", "readonly", "require", "return", "set", "static", "string", "super", "switch", "symbol", "this", "throw", "true", "try", "type", "typeof", "undefined", "unique", "unknown", "var", "void", "while", "with", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Typst",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["align", "and", "as", "assert", "auto", "block", "box", "break", "calc", "circle", "columns", "context", "continue", "counter", "document", "else", "emph", "eval", "false", "figure", "for", "grid", "heading", "here", "if", "image", "import", "in", "include", "let", "line", "link", "locate", "math", "measure", "none", "not", "or", "pad", "page", "par", "panic", "place", "query", "rect", "repr", "return", "set", "show", "stack", "state", "strong", "sys", "table", "text", "true", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Vala",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["abstract", "as", "async", "base", "break", "case", "catch", "class", "const", "construct", "continue", "default", "delegate", "delete", "do", "dynamic", "else", "enum", "errordomain", "extern", "false", "finally", "for", "foreach", "get", "if", "in", "inline", "interface", "internal", "is", "lock", "loop", "namespace", "new", "null", "out", "override", "owned", "params", "private", "protected", "public", "ref", "return", "set", "signal", "sizeof", "static", "struct", "switch", "this", "throw", "throws", "true", "try", "typeof", "unichar", "unowned", "using", "value", "var", "virtual", "void", "volatile", "weak", "while", "yield"],
        fold_case: false,
    },
    LangSpec {
        name: "Vim script",
        line: &["\""],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["autocmd", "augroup", "break", "call", "catch", "command", "const", "continue", "echo", "echom", "echomsg", "else", "elseif", "endfor", "endfunction", "endif", "endtry", "endwhile", "execute", "finish", "for", "function", "global", "highlight", "if", "in", "let", "lockvar", "map", "maparg", "normal", "noremap", "plugin", "return", "runtime", "script", "set", "setlocal", "silent", "source", "syntax", "throw", "try", "unlet", "unlockvar", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "Visual Basic",
        line: &["'"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["addhandler", "addressof", "alias", "and", "andalso", "as", "boolean", "byref", "byte", "byval", "call", "case", "catch", "cbool", "cbyte", "cchar", "cdate", "cdbl", "cdec", "char", "cint", "class", "clng", "cobj", "csbyte", "cshort", "csng", "cstr", "ctype", "cuint", "culng", "cushort", "date", "decimal", "declare", "default", "delegate", "dim", "directcast", "do", "double", "each", "else", "elseif", "end", "endif", "enum", "erase", "error", "event", "exit", "false", "finally", "for", "friend", "function", "get", "gettype", "global", "gosub", "goto", "handles", "if", "implements", "imports", "in", "inherits", "integer", "interface", "is", "isnot", "let", "lib", "like", "long", "loop", "me", "mod", "module", "mustinherit", "mustoverride", "mybase", "myclass", "namespace", "narrowing", "new", "next", "not", "nothing", "notinheritable", "notoverridable", "object", "of", "on", "operator", "option", "optional", "or", "orelse", "overloads", "overridable", "overrides", "paramarray", "partial", "private", "property", "protected", "public", "raiseevent", "readonly", "redim", "rem", "removehandler", "resume", "return", "sbyte", "select", "set", "shadows", "shared", "short", "single", "static", "step", "stop", "string", "structure", "sub", "synclock", "then", "throw", "to", "true", "try", "trycast", "typeof", "uinteger", "ulong", "ushort", "using", "variant", "wend", "when", "while", "widening", "with", "withevents", "writeonly", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "Visual Prolog",
        line: &["%"],
        block: Some(("/*", "*/")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["class", "clauses", "constants", "domains", "else", "elseif", "endclass", "endimplement", "endif", "endinterface", "facts", "goal", "if", "implement", "in", "include", "interface", "namespace", "open", "predicates", "properties", "supports", "then", "this"],
        fold_case: true,
    },
    LangSpec {
        name: "VHDL",
        line: &["--"],
        block: None,
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["abs", "access", "after", "alias", "all", "and", "architecture", "array", "assert", "attribute", "begin", "block", "body", "buffer", "bus", "case", "component", "configuration", "constant", "disconnect", "downto", "else", "elsif", "end", "entity", "exit", "file", "for", "function", "generate", "generic", "group", "guarded", "if", "impure", "in", "inertial", "inout", "is", "label", "library", "linkage", "literal", "loop", "map", "mod", "nand", "new", "next", "nor", "not", "null", "of", "on", "open", "or", "others", "out", "package", "port", "postponed", "procedure", "process", "protected", "pure", "range", "record", "register", "reject", "rem", "report", "return", "rol", "ror", "select", "severity", "shared", "signal", "sla", "sll", "sra", "srl", "subtype", "then", "to", "transport", "type", "unaffected", "units", "until", "use", "variable", "wait", "when", "while", "with", "xnor", "xor"],
        fold_case: true,
    },
    LangSpec {
        name: "Verilog",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["always", "and", "assign", "automatic", "begin", "buf", "bufif0", "bufif1", "case", "casex", "casez", "cell", "cmos", "config", "deassign", "default", "defparam", "design", "disable", "edge", "else", "end", "endcase", "endconfig", "endfunction", "endgenerate", "endmodule", "endprimitive", "endspecify", "endtable", "endtask", "event", "for", "force", "forever", "fork", "function", "generate", "genvar", "highz0", "highz1", "if", "ifnone", "incdir", "include", "initial", "inout", "input", "instance", "integer", "join", "large", "liblist", "library", "localparam", "macromodule", "medium", "module", "nand", "negedge", "nmos", "nor", "noshowcancelled", "not", "notif0", "notif1", "or", "output", "parameter", "pmos", "posedge", "primitive", "pull0", "pull1", "pulldown", "pullup", "pulsestyle_onevent", "pulsestyle_ondetect", "rcmos", "real", "realtime", "reg", "release", "repeat", "rnmos", "rpmos", "rtran", "rtranif0", "rtranif1", "scalared", "showcancelled", "signed", "small", "specify", "specparam", "strong0", "strong1", "supply0", "supply1", "table", "task", "time", "tran", "tranif0", "tranif1", "tri", "tri0", "tri1", "triand", "trior", "trireg", "unsigned", "use", "vectored", "wait", "wand", "weak0", "weak1", "while", "wire", "wor", "xnor", "xor"],
        fold_case: false,
    },
    LangSpec {
        name: "Vue",
        line: &["//"],
        block: Some(("<!--", "-->")),
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["beforeCreate", "beforeMount", "beforeUpdate", "components", "computed", "created", "data", "defineComponent", "emits", "export", "extends", "false", "filters", "from", "import", "inject", "key", "methods", "mixins", "mounted", "onMounted", "props", "provide", "reactive", "ref", "setup", "slot", "style", "template", "transition", "true", "unmounted", "updated", "watch"],
        fold_case: false,
    },
    LangSpec {
        name: "WGSL",
        line: &["//"],
        block: Some(("/*", "*/")),
        quotes: &[],
        apostrophe: Apostrophe::Line,
        keywords: &["alias", "as", "array", "atomic", "bitcast", "bool", "break", "case", "const", "continue", "continuing", "default", "discard", "else", "enable", "f16", "f32", "false", "fn", "for", "function", "i32", "if", "let", "loop", "mat2x2", "mat3x3", "mat4x4", "override", "private", "read", "read_write", "return", "sampler", "storage", "struct", "switch", "texture_2d", "true", "u32", "uniform", "var", "vec2", "vec3", "vec4", "while", "workgroup", "write"],
        fold_case: false,
    },
    LangSpec {
        name: "Wolfram",
        line: &[],
        block: Some(("(*", "*)")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &["Block", "Cases", "Clear", "Cos", "D", "Do", "Exp", "Expand", "Factor", "False", "Fold", "For", "Function", "Graphics", "Hold", "If", "Integrate", "Length", "ListPlot", "Log", "Map", "Module", "NDSolve", "Nest", "Null", "Options", "Part", "Plot", "Plot3D", "Print", "Range", "Replace", "ReplaceAll", "Return", "Rule", "Select", "Set", "SetDelayed", "Show", "Simplify", "Sin", "Solve", "Sqrt", "Switch", "Table", "Total", "True", "Which", "While", "With"],
        fold_case: false,
    },
    LangSpec {
        name: "Zig",
        line: &["//"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Char,
        keywords: &["addrspace", "align", "allowzero", "and", "anyframe", "anytype", "asm", "async", "await", "break", "callconv", "catch", "comptime", "const", "continue", "defer", "else", "enum", "errdefer", "error", "export", "extern", "fn", "for", "if", "inline", "linksection", "noalias", "noinline", "nosuspend", "opaque", "or", "orelse", "packed", "pub", "resume", "return", "struct", "suspend", "switch", "test", "threadlocal", "try", "union", "unreachable", "usingnamespace", "var", "volatile", "while"],
        fold_case: false,
    },
    LangSpec {
        name: "KIXtart",
        line: &[";"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["if", "else", "endif", "while", "loop", "do", "until", "for", "each", "next", "function", "endfunction", "return", "select", "case", "endselect", "global", "dim", "redim", "break", "exit", "goto", "call"],
        fold_case: true,
    },
    LangSpec {
        name: "XML",
        line: &[],
        block: Some(("<!--", "-->")),
        quotes: &['"'],
        apostrophe: Apostrophe::Line,
        keywords: &[],
        fold_case: true,
    },
    LangSpec {
        name: "YAML",
        line: &["#"],
        block: None,
        quotes: &['"', '\''],
        apostrophe: Apostrophe::Line,
        keywords: &["true", "false", "null", "yes", "no", "on", "off"],
        fold_case: true,
    },
];

/// The language called `name`, if there is one.
pub fn by_name(name: &str) -> Option<usize> {
    LANGS.iter().position(|spec| spec.name == name)
}

/// The language Notepad++ would pick for a file with this extension.
///
/// The extension is matched without its dot and without regard to case. Only
/// the extensions that are unambiguous are listed: `.h` could be C or C++ and
/// `.m` could be Objective-C or Matlab, so both are left to the user rather
/// than guessed at. A file whose extension is not here opens with the built-in
/// lexer, which is what Notepad++ does for an unknown extension too.
pub fn for_extension(ext: &str) -> Option<usize> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let name = match ext.as_str() {
        "as" => "ActionScript",
        "ada" | "adb" | "ads" => "Ada",
        "apl" => "APL",
        "adoc" | "asciidoc" => "AsciiDoc",
        "asp" | "asa" => "ASP",
        "asm" | "s" | "inc" => "Assembly",
        "astro" => "Astro",
        "au3" => "AutoIt",
        "avs" => "AviSynth",
        "awk" => "AWK",
        "bat" | "cmd" | "nt" => "Batch",
        "bib" => "BibTeX",
        "blade" => "Blade",
        "bb" | "pb" => "Blitzbasic",
        "c" => "C",
        "cs" => "C#",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "ino" => "C++",
        "ml" | "mli" => "Caml",
        "clj" | "cljs" | "cljc" | "edn" => "Clojure",
        "cmake" => "CMake",
        "cob" | "cbl" => "COBOL",
        "cr" => "Crystal",
        "csd" | "orc" | "sco" | "udo" => "CSound",
        "coffee" => "CoffeeScript",
        "css" => "CSS",
        "cu" | "cuh" => "CUDA",
        "d" => "D",
        "dart" => "Dart",
        "diff" | "patch" => "Diff",
        "dockerfile" => "Dockerfile",
        "ex" | "exs" => "Elixir",
        "elm" => "Elm",
        "erl" | "hrl" => "Erlang",
        "fs" | "fsi" | "fsx" => "F#",
        "fish" => "Fish",
        "f" | "f90" | "f95" | "f2k" | "f77" => "Fortran (free form)",
        "bas" | "bi" => "Freebasic",
        "gd" => "GDScript",
        "gleam" => "Gleam",
        "glsl" => "GLSL",
        "go" => "Go",
        "graphql" | "gql" => "GraphQL",
        "groovy" | "gradle" | "gvy" => "Groovy",
        "gui" => "Gui4Cli",
        "hs" | "lhs" | "las" => "Haskell",
        "hcl" | "tf" | "tfvars" => "HCL",
        "hlsl" | "fx" => "HLSL",
        "hly" => "Hollywood",
        "html" | "htm" | "xhtml" | "shtml" | "shtm" | "hta" => "HTML",
        "ini" | "inf" | "reg" | "url" => "INI file",
        "iss" => "Inno Setup",
        "hex" => "Intel HEX",
        "java" => "Java",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "jinja" | "jinja2" | "j2" => "Jinja",
        "json" => "JSON",
        "json5" => "JSON5",
        "jsp" | "jspf" => "JSP",
        "jl" => "Julia",
        "kix" => "KIXtart",
        "kt" | "kts" => "Kotlin",
        "tex" | "sty" | "cls" => "LaTeX",
        "less" => "Less",
        "lisp" | "lsp" | "el" => "LISP",
        "lua" => "Lua",
        "mak" | "mk" => "Makefile",
        "md" | "markdown" | "mdown" | "mkd" => "Markdown",
        "m" => "Matlab",
        "sql" => "Microsoft Transact-SQL",
        "mms" => "MMIXAL",
        "mustache" => "Mustache",
        "mysql" => "MySQL",
        "nim" | "nims" => "Nim",
        "ninja" => "Ninja",
        "nix" => "Nix",
        "nsi" | "nsh" => "NSIS",
        "nu" => "Nushell",
        "mm" => "Objective-C",
        "org" => "Org",
        "os" => "OScript",
        "pas" | "pp" | "dpr" => "Pascal",
        "pl" | "pm" | "plx" => "Perl",
        "php" | "php3" | "php4" | "php5" | "phtml" => "PHP",
        "psql" | "pgsql" => "PostgreSQL",
        "ps" | "eps" => "PostScript",
        "ps1" | "psm1" | "psd1" => "PowerShell",
        "prolog" => "Prolog",
        "properties" | "conf" => "Properties",
        "proto" => "Protocol Buffers",
        "pure" => "Purebasic",
        "purs" => "PureScript",
        "py" | "pyw" => "Python",
        "r" | "rdata" => "R",
        "rkt" | "rktl" | "rktd" => "Racket",
        "raku" | "rakumod" | "p6" => "Raku",
        "reb" | "r3" => "REBOL",
        "rst" => "reStructuredText",
        "rb" | "rbw" => "Ruby",
        "rs" => "Rust",
        "sas" => "SAS",
        "sass" => "Sass",
        "scss" => "SCSS",
        "sed" => "Sed",
        "sh" | "bash" | "zsh" | "ksh" => "Shell",
        "scm" | "ss" => "Scheme",
        "st" => "Smalltalk",
        "sol" => "Solidity",
        "spice" | "cir" => "Spice",
        "svelte" => "Svelte",
        "swift" => "Swift",
        "sv" | "svh" => "SystemVerilog",
        "tcl" => "TCL",
        "thrift" => "Thrift",
        "toml" => "TOML",
        "t2t" => "txt2tags",
        "ts" | "tsx" => "TypeScript",
        "typ" => "Typst",
        "vala" | "vapi" => "Vala",
        "vim" => "Vim script",
        "vb" | "vbs" => "Visual Basic",
        "pro" => "Visual Prolog",
        "vhd" | "vhdl" => "VHDL",
        "v" => "Verilog",
        "vue" => "Vue",
        "wgsl" => "WGSL",
        "wl" | "wls" | "nb" => "Wolfram",
        "xml" | "xsd" | "xsl" | "xslt" | "svg" | "rss" => "XML",
        "yaml" | "yml" => "YAML",
        "zig" => "Zig",
        _ => return None,
    };
    by_name(name)
}

/// The language whose key is `key`, if there is one.
pub fn by_key(key: &str) -> Option<usize> {
    LANGS.iter().position(|spec| spec.key() == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_a_name_and_a_key() {
        for spec in LANGS {
            assert!(!spec.name.is_empty());
            assert!(!spec.key().is_empty(), "{}", spec.name);
            assert!(spec.key().is_ascii(), "{}", spec.name);
        }
    }

    #[test]
    fn no_two_languages_share_a_name_or_a_key() {
        for (index, spec) in LANGS.iter().enumerate() {
            for other in &LANGS[index + 1..] {
                assert_ne!(spec.name, other.name);
                assert_ne!(spec.key(), other.key(), "{} and {}", spec.name, other.name);
            }
        }
    }

    #[test]
    fn the_order_is_the_one_notepads_own_menu_uses() {
        // Notepad++ groups its Language menu by first letter and lists each
        // group in the order its resource file declares, which is not quite
        // alphabetical: `LaTeX` precedes `LISP`, and `VHDL` precedes `Verilog`.
        // Copying the order rather than sorting means the two menus read the
        // same way, which is the point of the exercise.
        //
        // The assertions are relative rather than absolute, so inserting a
        // language shifts its neighbours' indices without breaking the test.
        let names: Vec<&str> = LANGS.iter().map(|spec| spec.name).collect();
        let at = |name: &str| {
            names
                .iter()
                .position(|n| *n == name)
                .unwrap_or_else(|| panic!("{name} is not in the table"))
        };
        assert_eq!(names[0], "ActionScript");
        assert!(at("LaTeX") < at("LISP"));
        assert!(at("VHDL") < at("Verilog"));
        // The three rows Notepad++ keeps outside the letter groups come last.
        assert_eq!(&names[names.len() - 3..], &["KIXtart", "XML", "YAML"]);
    }

    #[test]
    fn every_group_is_contiguous() {
        // The menu builds its letter submenus by walking the table once, so a
        // language filed under the wrong letter would produce two `S` submenus.
        // The three ungrouped rows at the end are not filed under their letters
        // and are skipped here, the same way the menu skips them -- `KIXtart`
        // would otherwise look like a second `K` group.
        let mut seen: Vec<char> = Vec::new();
        for spec in &LANGS[..LANGS.len() - 3] {
            let letter = spec.name.chars().next().unwrap().to_ascii_uppercase();
            if seen.last() != Some(&letter) {
                assert!(!seen.contains(&letter), "{} is out of its group", spec.name);
                seen.push(letter);
            }
        }
    }

    #[test]
    fn a_name_and_a_key_find_the_same_language() {
        for (index, spec) in LANGS.iter().enumerate() {
            assert_eq!(by_name(spec.name), Some(index));
            assert_eq!(by_key(&spec.key()), Some(index));
        }
        assert_eq!(by_name("Klingon"), None);
        assert_eq!(by_key("klingon"), None);
    }

    #[test]
    fn an_extension_finds_a_language() {
        assert_eq!(for_extension("py"), by_name("Python"));
        assert_eq!(for_extension(".PY"), by_name("Python"));
        assert_eq!(for_extension("rs"), by_name("Rust"));
        assert_eq!(for_extension("cpp"), by_name("C++"));
        assert_eq!(for_extension("yml"), by_name("YAML"));
        assert_eq!(for_extension("zzz"), None);
        assert_eq!(for_extension(""), None);
    }

    #[test]
    fn every_extension_names_a_language_that_exists() {
        // The table is a match arm, so a typo in a name would silently return
        // None rather than fail to compile. This walks the arms the tests know
        // about and checks the lookup resolves.
        for ext in [
            "py", "rs", "c", "cpp", "js", "ts", "html", "css", "json", "xml", "apl", "adoc",
            "asciidoc", "astro", "awk", "bib", "blade", "clj", "cljs", "cljc", "edn", "cr", "cu",
            "cuh", "dart", "dockerfile", "ex", "exs", "elm", "fs", "fsi", "fsx", "fish", "gleam",
            "glsl", "graphql", "gql", "groovy", "gradle", "gvy", "hcl", "tf", "tfvars", "hlsl",
            "fx", "jinja", "jinja2", "j2", "jl", "kt", "kts", "less", "md", "markdown", "mdown",
            "mkd", "mustache", "mysql", "ninja", "nix", "nu", "org", "psql", "pgsql", "prolog",
            "proto", "purs", "rkt", "rktl", "rktd", "rst", "sass", "scss", "sed", "sol", "svelte",
            "sv", "svh", "thrift", "typ", "vala", "vapi", "vim", "vue", "wgsl", "wl", "wls", "nb",
            "zig",
        ] {
            assert!(for_extension(ext).is_some(), "{ext}");
        }
        // The arms that changed hands resolve to their new owner, and the
        // languages the new arms name are the ones the table actually has.
        assert_eq!(for_extension("sv"), by_name("SystemVerilog"));
        assert_eq!(for_extension("svh"), by_name("SystemVerilog"));
        assert_eq!(for_extension("fs"), by_name("F#"));
        assert_eq!(for_extension("md"), by_name("Markdown"));
    }

    #[test]
    fn keywords_are_matched_the_way_the_language_matches_them() {
        let ada = &LANGS[by_name("Ada").unwrap()];
        assert!(ada.is_keyword("begin"));
        assert!(ada.is_keyword("BEGIN"));
        let rust = &LANGS[by_name("Rust").unwrap()];
        assert!(rust.is_keyword("fn"));
        assert!(!rust.is_keyword("FN"));
    }
}
