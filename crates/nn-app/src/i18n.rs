//! Translations.
//!
//! Notepad++ ships about ninety localizations as XML files in `localization/`,
//! and a user drops one in and picks it from a dialog. This module reads the
//! same files.
//!
//! ## What ships
//!
//! Notepad++'s own files stopped being readable at version 8: they key on a
//! `menuId` attribute now, and the menu ids they use are not the labels the
//! editor draws. Reading them would need a table mapping several hundred ids to
//! English text, which is the table this module was built to avoid.
//!
//! So the editor carries its own translations, in [`BUNDLED`]: the second
//! schema below, one file per language, compiled in with `include_str!`. A
//! single binary is the whole installation, and a feature that needs a data
//! directory beside it is a feature that breaks when the binary is copied
//! somewhere else.
//!
//! A file in the `localization` directory still wins over the bundled one of the
//! same name. That is what makes a translation editable: copy `german.xml` out,
//! fix a label, and the fix is picked up without recompiling.
//!
//! ## Why a pair of files
//!
//! A Notepad++ localization is keyed by *command id*, not by English text:
//! `french.xml` says `<Item id="1" name="Nouveau"/>` and nothing in the file
//! says that id 1 is "New". The mapping lives in Notepad++'s source, as a table
//! of `IDM_*` constants.
//!
//! Copying that table would mean a second copy of it here, drifting from the
//! first. Instead the table is *derived*: `english.xml` says
//! `<Item id="1" name="New"/>`, so reading both files gives
//! `"New" -> "Nouveau"` without a single hard-coded id. That is why
//! [`Catalog::parse`] takes two documents.
//!
//! The cost is that `english.xml` has to be present. It ships with Notepad++,
//! so a user who has a localization has it too; when it is missing the editor
//! stays in English rather than guessing.
//!
//! ## The second schema
//!
//! A file whose `id` is not a number is read as a direct mapping:
//! `<Item id="new" name="Nouveau"/>` translates the action named `new`. That
//! is what a translation written for this editor looks like -- no pair needed,
//! no id table, and it survives a menu item being renamed in English.

use std::collections::HashMap;
use std::path::Path;

use nn_core::xml;

/// Whether a label is the same word in every language.
///
/// Palette names, programming language names and single-character submenu
/// letters are proper nouns: a French user's copy of "HTML" says "HTML". The
/// case-conversion rows are samples of the effect rather than words. None of
/// them has, or should have, an entry in a translation file, and a completeness
/// test that demanded one would be asking for a translation of a brand name.
pub fn is_literal(label: &str) -> bool {
    if label.chars().count() == 1 {
        return true;
    }
    const SAMPLES: [&str; 5] = ["cmd", "lowercase", "UPPERCASE", "iNVERT cASE", "ranDOm CasE"];
    if SAMPLES.contains(&label) {
        return true;
    }
    if crate::themes::CATALOGUE.iter().any(|p| p.name == label) {
        return true;
    }
    nn_core::LANGS.iter().any(|lang| lang.name == label)
}

/// Why a localization could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file has no `<Native-Langue>` element.
    NotALanguage,
    /// The file has no `<Menu>` element.
    NoMenu,
    /// The file could not be read.
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotALanguage => write!(f, "not a Notepad++ localization file"),
            Self::NoMenu => write!(f, "the file has no <Menu> section"),
            Self::Io(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

/// One installed localization, as the picker shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// The name the file gives itself, e.g. `Français`.
    pub name: String,
    /// The English name, e.g. `French`.
    ///
    /// The picker sorts by this and shows it, so the list reads A to Z whatever
    /// scripts its rows are written in. A list sorted by `name` puts `العربية`
    /// between `한국어` and `Русский`, which is alphabetical only to a machine.
    pub english: String,
    /// The file it came from, e.g. `french.xml`.
    ///
    /// A file name and not a path: it is what the settings file records, and a
    /// stored path would break the moment the configuration moved -- and would
    /// have no value at all for a translation that is compiled in.
    pub file: String,
}

/// The translations that ship inside the binary.
///
/// `(file name, source)`. The file name is what the settings file records and
/// what [`Catalog::load`] looks up, so it must not change between releases: a
/// user who picked `german.xml` would silently fall back to English.
pub const BUNDLED: &[(&str, &str)] = &[
    (
        "chinese-simplified.xml",
        include_str!("../../../localization/chinese-simplified.xml"),
    ),
    (
        "chinese-traditional.xml",
        include_str!("../../../localization/chinese-traditional.xml"),
    ),
    (
        "czech.xml",
        include_str!("../../../localization/czech.xml"),
    ),
    (
        "danish.xml",
        include_str!("../../../localization/danish.xml"),
    ),
    (
        "dutch.xml",
        include_str!("../../../localization/dutch.xml"),
    ),
    (
        "finnish.xml",
        include_str!("../../../localization/finnish.xml"),
    ),
    (
        "french.xml",
        include_str!("../../../localization/french.xml"),
    ),
    (
        "german.xml",
        include_str!("../../../localization/german.xml"),
    ),
    (
        "greek.xml",
        include_str!("../../../localization/greek.xml"),
    ),
    (
        "hebrew.xml",
        include_str!("../../../localization/hebrew.xml"),
    ),
    (
        "italian.xml",
        include_str!("../../../localization/italian.xml"),
    ),
    (
        "japanese.xml",
        include_str!("../../../localization/japanese.xml"),
    ),
    (
        "korean.xml",
        include_str!("../../../localization/korean.xml"),
    ),
    (
        "norwegian.xml",
        include_str!("../../../localization/norwegian.xml"),
    ),
    (
        "polish.xml",
        include_str!("../../../localization/polish.xml"),
    ),
    (
        "portuguese-brazil.xml",
        include_str!("../../../localization/portuguese-brazil.xml"),
    ),
    (
        "portuguese-portugal.xml",
        include_str!("../../../localization/portuguese-portugal.xml"),
    ),
    (
        "romanian.xml",
        include_str!("../../../localization/romanian.xml"),
    ),
    (
        "russian.xml",
        include_str!("../../../localization/russian.xml"),
    ),
    (
        "spanish.xml",
        include_str!("../../../localization/spanish.xml"),
    ),
    (
        "swedish.xml",
        include_str!("../../../localization/swedish.xml"),
    ),
    (
        "turkish.xml",
        include_str!("../../../localization/turkish.xml"),
    ),
    (
        "ukrainian.xml",
        include_str!("../../../localization/ukrainian.xml"),
    ),
];

/// The bundled translation called `file`, if one ships under that name.
pub fn bundled(file: &str) -> Option<&'static str> {
    BUNDLED
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, xml)| *xml)
}

/// A loaded translation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    /// The name the file gives itself.
    name: String,
    /// The English name, when the file carries one.
    english: String,
    /// Lowercased English label to translated label.
    ///
    /// Lowercased because Notepad++ is inconsistent about capitalisation
    /// between its own English file and its menus -- "Word completion" against
    /// "Word Completion" -- and a translation that misses because of a capital
    /// letter is a translation that silently does nothing.
    entries: HashMap<String, String>,
}

impl Catalog {
    /// The language's own name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The English name, or the language's own name when the file has none.
    pub fn english_name(&self) -> &str {
        if self.english.is_empty() {
            &self.name
        } else {
            &self.english
        }
    }

    /// How many labels the catalog translates.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog translates nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The translation of `english`, or `english` itself when there is none.
    ///
    /// Falling back to the input is what makes a partial translation usable: a
    /// file that covers the menus but not the status bar leaves the status bar
    /// in English instead of blank.
    pub fn tr<'a>(&'a self, english: &'a str) -> &'a str {
        match self.entries.get(&english.to_lowercase()) {
            Some(translated) => translated,
            None => english,
        }
    }

    /// Whether the catalog has a translation for `english`.
    ///
    /// The difference between "this language spells it that way" and "this
    /// language has never heard of it" is invisible in [`Catalog::tr`], which
    /// returns the input either way. A coverage test has to see it, because a
    /// label nobody translated is a label drawn in a second language.
    pub fn translates(&self, english: &str) -> bool {
        self.entries.contains_key(&english.to_lowercase())
    }

    /// Reads a localization.
    ///
    /// `english` is the reference file that turns ids into English labels. It
    /// is `None` for a file that keys on action names directly.
    pub fn parse(english: Option<&str>, target: &str) -> Result<Self, Error> {
        let name = language_name(target).ok_or(Error::NotALanguage)?;
        let english_name = language_english(target).unwrap_or_default();
        let target_items = items(target)?;
        let mut entries = HashMap::new();
        match english {
            Some(english) => {
                let english_items = items(english)?;
                // Keyed by section as well as id: `<Main>` and `<SubEntries>`
                // number their items independently, so id 0 is both "File" and
                // "Edit". Joining on the id alone would translate one of them
                // with the other's text.
                let by_id: HashMap<(&str, &str), &str> = english_items
                    .iter()
                    .map(|(section, id, label)| {
                        ((section.as_str(), id.as_str()), label.as_str())
                    })
                    .collect();
                for (section, id, label) in &target_items {
                    // A numeric id needs the reference file; a name does not.
                    let key = match by_id.get(&(section.as_str(), id.as_str())) {
                        Some(english_label) => english_label.to_lowercase(),
                        None => id.to_lowercase(),
                    };
                    entries.insert(key, label.clone());
                }
            }
            None => {
                // Without a reference file a numeric id cannot be matched to
                // an English label, so only name-keyed items are usable. They
                // are the hand-written form: `<Item id="new" name="Nouveau"/>`.
                for (_, id, label) in &target_items {
                    if id.parse::<u32>().is_err() {
                        entries.insert(id.to_lowercase(), label.clone());
                    }
                }
            }
        }
        Ok(Self {
            name,
            english: english_name,
            entries,
        })
    }

    /// Reads `file`, from `dir` when it is there and from the binary otherwise.
    ///
    /// `dir` is `None` when the platform gives no configuration directory, which
    /// is not a reason to have no translations: the bundled ones still load.
    /// `english.xml` beside the file is used as the id table when present, which
    /// is what makes a Notepad++-format file work.
    pub fn load(dir: Option<&Path>, file: &str) -> Result<Self, Error> {
        if let Some(dir) = dir {
            let path = dir.join(file);
            if path.is_file() {
                let target = std::fs::read_to_string(&path)
                    .map_err(|err| Error::Io(format!("{file}: {err}")))?;
                let english = std::fs::read_to_string(dir.join("english.xml")).ok();
                return Self::parse(english.as_deref(), &target);
            }
        }
        match bundled(file) {
            Some(source) => Self::parse(None, source),
            None => Err(Error::Io(format!("{file}: no such localization"))),
        }
    }

    /// Every localization available, sorted by English name.
    ///
    /// The bundled translations come first in the sense that they are always
    /// there, and a file of the same name in `dir` replaces one: an edited copy
    /// beside the settings is a deliberate act and outranks what we ship.
    /// `english.xml` is skipped either way -- it is the reference file, not a
    /// translation, and offering it would give a language that changes nothing.
    pub fn languages(dir: Option<&Path>) -> Vec<Language> {
        let mut found: Vec<Language> = Vec::new();
        if let Some(dir) = dir {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return Self::languages(None);
            };
            let mut installed: Vec<Language> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("xml") {
                    continue;
                }
                let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if file == "english.xml" {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Some(name) = language_name(&text) else {
                    continue;
                };
                let english = language_english(&text).unwrap_or_else(|| english_name_of(file));
                installed.push(Language {
                    name,
                    english,
                    file: file.to_string(),
                });
            }
            installed.sort_by(|a, b| a.file.cmp(&b.file));
            found.extend(installed);
        }
        for (file, source) in BUNDLED {
            if found.iter().any(|language| language.file == *file) {
                continue;
            }
            let Some(name) = language_name(source) else {
                continue;
            };
            let english = language_english(source).unwrap_or_else(|| english_name_of(file));
            found.push(Language {
                name,
                english,
                file: (*file).to_string(),
            });
        }
        found.sort_by(|a, b| {
            a.english
                .to_lowercase()
                .cmp(&b.english.to_lowercase())
                .then_with(|| a.file.cmp(&b.file))
        });
        found
    }
}

/// The `name` attribute of the file's `<Native-Langue>` element.
fn language_name(text: &str) -> Option<String> {
    let element = xml::elements(text, "Native-Langue").into_iter().next()?;
    xml::attribute(&element, "name").map(xml::unescape)
}

/// The `english` attribute of the file's `<Native-Langue>` element, if any.
///
/// Not part of Notepad++'s schema; ours. A file that does not carry one is
/// labelled from its file name instead, which is the best a third-party file
/// with no English name in it can offer.
fn language_english(text: &str) -> Option<String> {
    let element = xml::elements(text, "Native-Langue").into_iter().next()?;
    xml::attribute(&element, "english").map(xml::unescape)
}

/// `chinese-simplified.xml` -> `Chinese Simplified`.
fn english_name_of(file: &str) -> String {
    let stem = file.strip_suffix(".xml").unwrap_or(file);
    let mut out = String::new();
    for (index, word) in stem.split(['-', '_']).enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// The `(section, id, name)` triples of a file's menu items.
///
/// Both `<Main>` and `<SubEntries>` are read, because a menu title is as much a
/// label as a menu row. The sections are sliced out first: the scanner does not
/// track nesting, and a `<MessageBoxes>` item has an id in a different
/// namespace that would otherwise collide.
fn items(text: &str) -> Result<Vec<(String, String, String)>, Error> {
    if !text.contains("<Menu>") {
        return Err(Error::NoMenu);
    }
    let mut found = Vec::new();
    for section in ["Main", "SubEntries"] {
        let Some(body) = section_of(text, section) else {
            continue;
        };
        for element in xml::elements(body, "Item") {
            let (Some(id), Some(name)) = (
                xml::attribute(&element, "id"),
                xml::attribute(&element, "name"),
            ) else {
                continue;
            };
            // Both attributes are unescaped: an id is an English label, and a
            // label is allowed to contain the ampersand in "Match Case & Whole
            // Word". Leaving the id escaped keys the entry on `&amp;`, which no
            // menu row will ever ask for, so the row falls back to English while
            // its translation sits in the file looking correct.
            found.push((
                section.to_string(),
                xml::unescape(id),
                xml::unescape(name),
            ));
        }
    }
    Ok(found)
}

/// The text between `<name>` and `</name>`, or `None` when the section is
/// absent.
fn section_of<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A pair of files in Notepad++'s own shape: ids, with English as the
    /// reference.
    const ENGLISH: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <Native-Langue name="English" filename="english.xml" version="8.6">
        <Menu>
            <Main>
                <Item id="0" name="File"/>
                <Item id="1" name="New"/>
                <Item id="2" name="Open"/>
                <Item id="3" name="Save All"/>
            </Main>
            <SubEntries>
                <Item id="0" name="Edit"/>
                <Item id="1" name="Search"/>
            </SubEntries>
        </Menu>
        <MessageBoxes>
            <Item id="0" name="Save file"/>
        </MessageBoxes>
    </Native-Langue>
</NotepadPlus>
"#;

    const FRENCH: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<NotepadPlus>
    <Native-Langue name="Francais" filename="french.xml" version="8.6">
        <Menu>
            <Main>
                <Item id="0" name="Fichier"/>
                <Item id="1" name="Nouveau"/>
                <Item id="2" name="Ouvrir"/>
                <Item id="3" name="Enregistrer tout"/>
            </Main>
            <SubEntries>
                <Item id="0" name="Edition"/>
                <Item id="1" name="Recherche"/>
            </SubEntries>
        </Menu>
    </Native-Langue>
</NotepadPlus>
"#;

    fn french() -> Catalog {
        Catalog::parse(Some(ENGLISH), FRENCH).expect("a catalog")
    }

    #[test]
    fn a_pair_of_files_gives_english_to_french() {
        let catalog = french();
        assert_eq!(catalog.tr("New"), "Nouveau");
        assert_eq!(catalog.tr("Save All"), "Enregistrer tout");
        assert_eq!(catalog.tr("Edit"), "Edition");
    }

    #[test]
    fn the_language_names_itself() {
        assert_eq!(french().name(), "Francais");
    }

    #[test]
    fn a_label_the_file_does_not_cover_stays_english() {
        assert_eq!(french().tr("Print Now"), "Print Now");
    }

    #[test]
    fn matching_ignores_capitalisation() {
        // Notepad++'s English file says "Word completion"; the menu says
        // "Word Completion". A translation that missed on a capital letter
        // would silently do nothing.
        let catalog = french();
        assert_eq!(catalog.tr("new"), "Nouveau");
        assert_eq!(catalog.tr("NEW"), "Nouveau");
    }

    #[test]
    fn a_message_box_item_is_not_a_menu_item() {
        // `<MessageBoxes>` has its own id space; reading it would map id 0 to
        // "Save file" and overwrite "File".
        assert_eq!(french().tr("File"), "Fichier");
    }

    #[test]
    fn a_file_keyed_by_action_name_needs_no_reference() {
        let own = r#"<NotepadPlus>
            <Native-Langue name="Deutsch" filename="german.xml">
                <Menu>
                    <Main>
                        <Item id="new" name="Neu"/>
                        <Item id="open" name="Offnen"/>
                    </Main>
                </Menu>
            </Native-Langue>
        </NotepadPlus>"#;
        let catalog = Catalog::parse(None, own).expect("a catalog");
        assert_eq!(catalog.tr("new"), "Neu");
        assert_eq!(catalog.tr("open"), "Offnen");
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn a_file_without_a_language_element_is_rejected() {
        assert_eq!(
            Catalog::parse(None, "<NotepadPlus></NotepadPlus>"),
            Err(Error::NotALanguage)
        );
    }

    #[test]
    fn a_file_without_a_menu_is_rejected() {
        let text = r#"<NotepadPlus><Native-Langue name="X" filename="x.xml"/></NotepadPlus>"#;
        assert_eq!(Catalog::parse(None, text), Err(Error::NoMenu));
    }

    #[test]
    fn an_empty_catalog_translates_nothing() {
        let text = r#"<NotepadPlus><Native-Langue name="X" filename="x.xml">
            <Menu><Main></Main></Menu></Native-Langue></NotepadPlus>"#;
        let catalog = Catalog::parse(None, text).expect("a catalog");
        assert!(catalog.is_empty());
        assert_eq!(catalog.tr("New"), "New");
    }

    #[test]
    fn an_escaped_label_is_unescaped() {
        let text = r#"<NotepadPlus><Native-Langue name="X" filename="x.xml">
            <Menu><Main><Item id="new" name="Fichier &amp; Modifier"/></Main></Menu>
            </Native-Langue></NotepadPlus>"#;
        let catalog = Catalog::parse(None, text).expect("a catalog");
        assert_eq!(catalog.tr("new"), "Fichier & Modifier");
    }

    /// A scratch directory that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("npp-i18n-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch directory");
            Self(dir)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).expect("scratch file");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_directory_lists_its_languages_and_skips_english() {
        let scratch = Scratch::new("list");
        scratch.write("english.xml", ENGLISH);
        scratch.write("french.xml", FRENCH);
        scratch.write("notes.txt", "not a language");
        let found = Catalog::languages(Some(&scratch.0));
        // The bundled set, with the installed `french.xml` in place of ours
        // rather than beside it.
        assert_eq!(found.len(), BUNDLED.len());
        let french: Vec<&Language> = found.iter().filter(|l| l.file == "french.xml").collect();
        assert_eq!(french.len(), 1);
        assert_eq!(french[0].name, "Francais");
    }

    #[test]
    fn loading_a_file_uses_the_english_beside_it() {
        let scratch = Scratch::new("load");
        scratch.write("english.xml", ENGLISH);
        scratch.write("french.xml", FRENCH);
        let catalog = Catalog::load(Some(&scratch.0), "french.xml").expect("a catalog");
        assert_eq!(catalog.tr("New"), "Nouveau");
    }

    #[test]
    fn loading_without_an_english_file_falls_back_to_action_names() {
        let scratch = Scratch::new("noenglish");
        scratch.write("french.xml", FRENCH);
        let catalog = Catalog::load(Some(&scratch.0), "french.xml").expect("a catalog");
        // The ids are numbers and there is no reference, so nothing matches an
        // English label -- but the file still loads rather than failing.
        assert_eq!(catalog.tr("New"), "New");
        assert_eq!(catalog.len(), 0);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let scratch = Scratch::new("missing");
        assert!(matches!(
            Catalog::load(Some(&scratch.0), "nope.xml"),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn a_missing_directory_still_lists_the_bundled_translations() {
        // A user with no `localization` directory has no translations of their
        // own and all of ours, which is the point of compiling them in.
        let found = Catalog::languages(Some(Path::new("/nonexistent-npp-i18n")));
        assert_eq!(found.len(), BUNDLED.len());
        assert_eq!(Catalog::languages(None).len(), BUNDLED.len());
    }

    #[test]
    fn a_bundled_translation_loads_with_no_directory_at_all() {
        let catalog = Catalog::load(None, "german.xml").expect("a catalog");
        assert_eq!(catalog.name(), "Deutsch");
        assert_eq!(catalog.english_name(), "German");
        assert_eq!(catalog.tr("File"), "Datei");
        assert_eq!(catalog.tr("Save As..."), "Speichern unter...");
    }

    #[test]
    fn every_bundled_translation_parses_and_translates_something() {
        for (file, source) in BUNDLED {
            let catalog = Catalog::parse(None, source)
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            assert!(!catalog.is_empty(), "{file} translates nothing");
            assert!(!catalog.name().is_empty(), "{file} has no name");
            assert!(language_english(source).is_some(), "{file} has no English name");
        }
    }

    #[test]
    fn every_bundled_translation_covers_every_menu_title() {
        // A file that missed a title would leave one menu in English while the
        // rest of the bar was translated, which reads as a bug rather than as
        // an incomplete translation.
        let titles = [
            "File", "Edit", "Search", "View", "Encoding", "Language", "Settings", "Tools", "Macro",
            "Run", "Plugins", "Window", "Help",
        ];
        for (file, source) in BUNDLED {
            let catalog = Catalog::parse(None, source).expect("a catalog");
            for title in titles {
                assert!(
                    catalog.entries.contains_key(&title.to_lowercase()),
                    "{file} does not translate {title}"
                );
            }
            // Presence alone would also pass for a file that copied the English,
            // so nearly every title has to read differently. Nearly and not
            // entirely because some do not: Italian calls the File menu `File`,
            // and Italian for Macro is `Macro`.
            let differing = titles
                .iter()
                .filter(|title| catalog.tr(title) != **title)
                .count();
            assert!(
                differing >= titles.len() - 2,
                "{file} leaves {} of {} titles in English",
                titles.len() - differing,
                titles.len()
            );
        }
    }

    #[test]
    fn the_bundled_file_names_are_unique_and_sorted() {
        let mut names: Vec<&str> = BUNDLED.iter().map(|(file, _)| *file).collect();
        let sorted = {
            let mut copy = names.clone();
            copy.sort_unstable();
            copy
        };
        assert_eq!(names, sorted, "the table is not in file-name order");
        names.dedup();
        assert_eq!(names.len(), BUNDLED.len(), "a file name appears twice");
    }

    #[test]
    fn the_languages_are_sorted_by_english_name() {
        let found = Catalog::languages(None);
        let english: Vec<String> = found.iter().map(|l| l.english.clone()).collect();
        let mut sorted = english.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(english, sorted);
        // And the list reads A to Z, starting with the scripts a reader expects.
        assert_eq!(english.first().map(String::as_str), Some("Chinese (Simplified)"));
    }

    #[test]
    fn an_english_name_is_derived_from_a_file_that_has_none() {
        assert_eq!(english_name_of("chinese-simplified.xml"), "Chinese Simplified");
        assert_eq!(english_name_of("french.xml"), "French");
        assert_eq!(english_name_of("portuguese_brazil.xml"), "Portuguese Brazil");
    }

    #[test]
    fn an_installed_file_outranks_the_bundled_one_of_the_same_name() {
        let scratch = Scratch::new("override");
        scratch.write(
            "german.xml",
            r#"<NotepadPlus><Native-Langue name="Deutsch (eigen)" english="German" filename="german.xml">
                <Menu><Main><Item id="File" name="Datei2"/></Main></Menu></Native-Langue></NotepadPlus>"#,
        );
        let found = Catalog::languages(Some(&scratch.0));
        let german: Vec<&Language> = found.iter().filter(|l| l.file == "german.xml").collect();
        assert_eq!(german.len(), 1, "the file is offered twice");
        assert_eq!(german[0].name, "Deutsch (eigen)");
        let catalog = Catalog::load(Some(&scratch.0), "german.xml").expect("a catalog");
        assert_eq!(catalog.tr("File"), "Datei2");
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// Every label the window draws outside the menu bar.
    ///
    /// Each module is asked for its own list rather than a copy kept here: a
    /// widget that grows a label adds it to its own module's list or it is not
    /// translated, and the test below fails before it can ship in English.
    fn chrome_labels() -> Vec<&'static str> {
        let mut labels: Vec<&'static str> = Vec::new();
        labels.extend(crate::prefs::Preferences::labels());
        labels.extend(crate::statusbar::StatusBar::labels());
        labels.extend(crate::prompt::PromptBar::labels());
        labels.extend(crate::findbar::FindBar::labels());
        labels.extend(crate::finddialog::FindDialog::labels());
        labels.extend(crate::about::AboutBox::labels());
        labels.extend(crate::pluginlist::PluginList::labels());
        labels.extend(crate::toolbar::labels());
        labels.sort_unstable();
        labels.dedup();
        labels
    }

    /// No bundled language may leave a word of the window in English.
    ///
    /// The menu bar has the same test in `menu`, next to the rows it is about.
    /// This one is about everything else: the Preferences panel, the status bar,
    /// the dialogs. A language that covers the menus and not the panel says
    /// "Fichier" at the top of the window and "Preferences" in the middle of it.
    #[test]
    fn every_bundled_language_translates_every_window_label() {
        let labels = chrome_labels();
        assert!(labels.len() > 80, "{} labels", labels.len());
        for (file, source) in BUNDLED {
            let catalog = Catalog::parse(None, source).expect(file);
            let missing: Vec<&str> = labels
                .iter()
                .copied()
                .filter(|label| !is_literal(label))
                .filter(|label| !catalog.translates(label))
                .collect();
            assert!(
                missing.is_empty(),
                "{file} leaves {} in English: {missing:?}",
                missing.len()
            );
        }
    }
}
