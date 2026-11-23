//! Offscreen frame-time benchmark.
//!
//! Renders the editor into a texture with no surface and no vsync, so the numbers
//! are the cost of producing a frame rather than the cost of waiting for a
//! display. Three scenarios are measured because they stress different parts of
//! the pipeline:
//!
//! * `idle`   — nothing changed. Everything is cached; this is the floor.
//! * `scroll` — one new line enters the view per frame, so one line is shaped.
//! * `type`   — one character is inserted per frame, so one line is reshaped.
//!
//! Usage: `bench [file] [width] [height] [frames] [dump.ppm] [flags...]`
//!
//! Flags are order-independent: `fold`, `menu`, `wrap`, `bookmarks`, `light`,
//! `recent`, `block`, `lines`, `sorted`, `results`, `brace`, `funcs`, `smart`,
//! `complete`, `prefs`, `carets`, `context`, `tabmenu`, `peeker`, and
//! `find=PATTERN` / `tabs=N`.

use std::path::{Path, PathBuf};

/// The optional flags, in any order after the five positional arguments.
#[derive(Default)]
struct Flags {
    find: Option<String>,
    tabs: usize,
    fold: bool,
    menu: bool,
    edit_menu: bool,
    wrap: bool,
    bookmarks: bool,
    light: bool,
    recent: bool,
    block: bool,
    carets: bool,
    context: bool,
    tab_menu: bool,
    divider_menu: bool,
    peeker: bool,
    pinned: bool,
    lines_menu: bool,
    view_menu: bool,
    settings_menu: bool,
    settings_import: bool,
    run_menu: bool,
    sorted: bool,
    results: bool,
    brace: bool,
    funcs: bool,
    smart: bool,
    complete: bool,
    prefs: bool,
    styles: bool,
    shortcuts: bool,
    settings: Option<String>,
    split: Option<Split>,
    flip: bool,
    distraction: bool,
    distraction_bare: bool,
    history: bool,
    charset: bool,
    incr: bool,
    nonvolatile: bool,
    func_complete: bool,
    doc_complete: bool,
    path_complete: bool,
    param_hint: bool,
    tooltip: bool,
    folder: bool,
    project: bool,
    folder_filter: Option<String>,
    find_projects: Option<String>,
    export_shortcuts: bool,
    udl: Option<String>,
    lang: Option<String>,
    prefs_lang: bool,
    picker: bool,
    lang_picker: bool,
    print_preview: bool,
    print_page: usize,
    print_pdf: Option<String>,
    language_menu: bool,
    language_group: bool,
    plugins: Option<String>,
    plugin_menu: bool,
    plugin_manager: Option<String>,
    encoding_menu: bool,
    character_sets: bool,
    plugin_run: Option<String>,
    tab_layout: nn_app::TabLayout,
    /// Which tab of the floating Search window to draw, when it is drawn.
    floating_tab: Option<String>,
    /// The transparency the floating Search window is drawn at, 0 to 0.9.
    floating_transparency: f32,
    /// Draw the playground instead of the editor.
    playground: bool,
    /// How many seconds of the room's own clock to run before drawing it.
    playground_time: f32,
    /// The seed the room is built from.
    playground_seed: u64,
}
use std::time::Instant;

use nn_app::plugin::Context;
use nn_app::{
    Browser, Catalog, ChangeHistory, ChangeRow, CharPanel, Chrome, CompletionPopup, ContextMenu,
    AboutBox, DocumentPeeker,
    EditorView, FindBar, FindDialog, FolderPanel,
    FunctionList, Item, Keymap, MenuBar, Mode, PaneRef, ParamHint,
    PluginList, PluginMenu, PluginRow, Preferences, PrintPreview, PromptBar, Renderer, ResultsPanel, Settings, Shortcuts, Split,
    StatusBar,
    StyleConfigurator, TabBar, Theme, Toolbar, Tooltip,
};
use nn_core::{CLikeLexer, Document, Editor, Highlighter};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

fn main() {
    let mut args = std::env::args().skip(1);
    // An empty first argument means "use the synthetic document".
    let path = args.next().filter(|p| !p.is_empty()).map(PathBuf::from);
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1920);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1080);
    let frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(600);
    // Optional fifth argument: write one rendered frame here as a binary PPM.
    let dump = args.next().filter(|p| !p.is_empty()).map(PathBuf::from);

    // Everything after that is a flag, in any order. Positional flags were a
    // trap: one missing empty string silently shifted every later flag into the
    // previous slot, and the run still looked like it worked.
    let mut flags = Flags::default();
    for arg in args {
        let (key, value) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        match key {
            "find" => flags.find = value,
            "tabs" => flags.tabs = value.and_then(|v| v.parse().ok()).unwrap_or(1),
            "fold" => flags.fold = true,
            "menu" => flags.menu = true,
            "edit_menu" => flags.edit_menu = true,
            "wrap" => flags.wrap = true,
            "bookmarks" => flags.bookmarks = true,
            "light" => flags.light = true,
            "recent" => flags.recent = true,
            "block" => flags.block = true,
            "carets" => flags.carets = true,
            "context" => flags.context = true,
            "peeker" => flags.peeker = true,
            "pinned" => flags.pinned = true,
            "tabmenu" => flags.tab_menu = true,
            "divider_menu" => flags.divider_menu = true,
            "lines" => flags.lines_menu = true,
            "view" => flags.view_menu = true,
            "settings_menu" => flags.settings_menu = true,
            "settings_import" => flags.settings_import = true,
            "run_menu" => flags.run_menu = true,
            "sorted" => flags.sorted = true,
            "results" => flags.results = true,
            "brace" => flags.brace = true,
            "funcs" => flags.funcs = true,
            "smart" => flags.smart = true,
            "complete" => flags.complete = true,
            "prefs" => flags.prefs = true,
            "styles" => flags.styles = true,
            "shortcuts" => flags.shortcuts = true,
            "settings" => flags.settings = value,
            "split" => flags.split = Some(Split::Vertical),
            "split_h" => flags.split = Some(Split::Horizontal),
            // The focused pane drawn second, which is what a split looks like
            // after a click in the right-hand (or lower) view.
            "flip" => flags.flip = true,
            "distraction" => flags.distraction = true,
            "distraction_bare" => flags.distraction_bare = true,
            "history" => flags.history = true,
            "charset" => flags.charset = true,
            "incr" => flags.incr = true,
            "nonvolatile" => flags.nonvolatile = true,
            "func_complete" => flags.func_complete = true,
            "doc_complete" => flags.doc_complete = true,
            "path_complete" => flags.path_complete = true,
            "param_hint" => flags.param_hint = true,
            "tooltip" => flags.tooltip = true,
            "folder" => flags.folder = true,
            "project" => flags.project = true,
            "folder_filter" => flags.folder_filter = value,
            "find_projects" => flags.find_projects = value,
            "export_shortcuts" => flags.export_shortcuts = true,
            "udl" => flags.udl = value,
            "lang" => flags.lang = value,
            "prefs_lang" => flags.prefs_lang = true,
            "picker" => flags.picker = true,
            "lang_picker" => flags.lang_picker = true,
            "print" => flags.print_preview = true,
            "print_page" => flags.print_page = value.and_then(|v| v.parse().ok()).unwrap_or(0),
            "print_pdf" => flags.print_pdf = value,
            "language_menu" => flags.language_menu = true,
            "language_group" => flags.language_group = true,
            "plugins" => flags.plugins = value,
            "plugin_menu" => flags.plugin_menu = true,
            "plugin_manager" => flags.plugin_manager = value,
            "encoding_menu" => flags.encoding_menu = true,
            "character_sets" => flags.character_sets = true,
            "plugin_run" => flags.plugin_run = value,
            "floating_tab" => flags.floating_tab = value,
            "floating_transparency" => {
                flags.floating_transparency = value.and_then(|v| v.parse().ok()).unwrap_or(0.0)
            }
            "playground" => flags.playground = true,
            "playground_time" => {
                flags.playground_time = value
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0)
                    .clamp(0.0, 3600.0)
            }
            "playground_seed" => {
                flags.playground_seed = value.and_then(|v| v.parse().ok()).unwrap_or(0x5EED)
            }
            "vtabs" => flags.tab_layout = nn_app::TabLayout::Vertical,
            "mtabs" => flags.tab_layout = nn_app::TabLayout::MultiLine,
            other => eprintln!("bench: unknown flag {other}"),
        }
    }

    // The chrome policy is a value, so the bench can pose the window in any
    // mode without a second code path.
    let mut chrome = Chrome::new(if flags.distraction || flags.distraction_bare {
        Mode::DistractionFree
    } else {
        Mode::Normal
    });
    if flags.distraction {
        // Pin the menu open so a dump shows the reveal band working rather than
        // an empty strip where the menu would be.
        chrome.pin_menu(true);
    }
    // The pointer position the chrome policy is evaluated against. The bare
    // variant parks it in the middle of the window, which is where it would be
    // while reading, so the menu bar is genuinely absent rather than merely
    // unpinned.
    let cursor_y = if flags.distraction_bare { 400.0 } else { 0.0 };

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        ..Default::default()
    }))
    .expect("no GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bench device"),
        ..Default::default()
    }))
    .expect("request device");

    let info = adapter.get_info();
    println!("adapter: {} ({:?})", info.name, info.backend);
    println!("target:  {width}x{height}, {frames} frames per scenario");

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&Default::default());

    let doc = match &path {
        Some(p) => Document::from_file(p).expect("read input file"),
        None => Document::from_text(&synthetic(20_000)),
    };
    println!("document: {} lines", doc.len_lines());

    let mut renderer = Renderer::new(&device, &queue, FORMAT);
    // The page runs on its own clock, so a dump at `playground_time=180` is the
    // same page at the same moment every run: the seed is a flag and the steps
    // are counted, never timed.
    let mut playground = {
        let now = Instant::now();
        let mut playground = nn_app::Playground::new(
            &device,
            &queue,
            FORMAT,
            renderer.font_system(),
            flags.playground_seed,
            now,
        );
        if flags.playground {
            playground.enter(flags.playground_seed, now);
            playground.resize(width, height);
            // Stepped at the same rate the mode runs at on screen, so a dump at
            // `playground_time=9.4` is the frame the reader would see and not a
            // coarser run of the same seed.
            const STEP: f32 = 1.0 / 120.0;
            let steps = (flags.playground_time / STEP).round() as usize;
            for _ in 0..steps {
                playground.step(STEP);
            }
        }
        playground
    };
    let mut view = EditorView::new(renderer.font_system(), Theme::DARK);
    let mut tab_bar = TabBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut menu = MenuBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
        Keymap::default(),
    );
    let mut toolbar = Toolbar::new();
    let mut prompt = PromptBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut results = ResultsPanel::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut history = ChangeHistory::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut charset = CharPanel::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut peeker = DocumentPeeker::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut about = AboutBox::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut context = ContextMenu::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut prefs = Preferences::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut plugin_list = PluginList::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut print = PrintPreview::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut styles = StyleConfigurator::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut shortcuts = Shortcuts::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut browser = Browser::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut complete = CompletionPopup::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut param_hint = ParamHint::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    // The benchmark never hovers the toolbar, so the tooltip stays closed; it is
    // threaded through anyway because the renderer draws it in the same pass as
    // the popups and a second code path for "no tooltip" would be a second thing
    // to keep in step.
    let mut tooltip = Tooltip::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut functions = FunctionList::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut workspace = FolderPanel::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut status = StatusBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut find = FindBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    let mut dialog = FindDialog::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        view.char_width(),
    );
    if flags.tabs > 1 {
        let mut titles: Vec<nn_app::TabInfo> = (0..flags.tabs)
            .map(|i| nn_app::TabInfo::new(format!("file{i}.rs")))
            .collect();
        if flags.pinned {
            // Two pinned tabs at the front, one painted, so the marker and the
            // colour are both visible in one frame.
            titles[0].pinned = true;
            titles[1].pinned = true;
            titles[1].colour = nn_app::TabColour::Green;
            titles[2].colour = nn_app::TabColour::Blue;
        }
        tab_bar.set_tabs(titles, 0);
    }
    tab_bar.set_layout(flags.tab_layout);
    let mut editor = Editor::new(doc);
    let mut highlighter = Highlighter::new(Box::new(CLikeLexer));
    highlighter.highlight_all(&mut editor.doc);
    let folder = nn_core::fold::BraceFolder;
    folder.apply(&mut editor.doc);
    if flags.fold {
        editor.doc.fold_all();
    }
    if flags.block {
        editor.select_block((2, 4), (9, 14));
    }
    if flags.peeker {
        let titles: Vec<String> = (0..6).map(|i| format!("file{i}.rs")).collect();
        peeker.show(titles, 2);
    }
    if flags.context {
        // The same list the application builds, so the screenshot cannot show a
        // menu the real one would not.
        context.show(
            320.0,
            140.0,
            nn_app::context::editor_entries(true, false, true, false),
        );
    }
    if flags.divider_menu {
        // The splitter's own menu, drawn where the splitter is: the point of the
        // flag is to check the row's wording against the orientation beside it.
        let split = flags.split.unwrap_or(Split::Vertical);
        let (x, y) = if split == Split::Vertical {
            (width as f32 / 2.0, 200.0)
        } else {
            (40.0, 340.0)
        };
        context.show(x, y, nn_app::context::divider_entries(split));
    }
    if flags.tab_menu {
        context.show(
            120.0,
            62.0,
            nn_app::context::tab_entries(
                flags.tabs.max(3),
                1,
                false,
                false,
                nn_app::TabColour::None,
            ),
        );
    }
    if flags.carets {
        // Built through the same calls the keyboard uses, so the screenshot
        // proves the caret stack rather than only the renderer.
        editor.goto_line(2);
        for _ in 0..3 {
            editor.add_caret_below();
        }
        // One keystroke, four lines: the point of the feature, and the only way
        // to see it in a still frame.
        editor.insert("// ");
    }
    if flags.recent {
        menu.set_recent(&[
            "main.rs".to_string(),
            "document.rs".to_string(),
            "notes.txt".to_string(),
        ]);
    }
    if let Some(path) = &flags.settings {
        // The whole path from a settings file to the shortcut column and the
        // syntax colours, so a rebinding or a restyle can be checked without a
        // pointer.
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let settings = Settings::parse(&text);
        menu.set_keymap(settings.keymap);
        let labels: Vec<String> = settings
            .runs
            .commands()
            .iter()
            .map(|command| command.label())
            .collect();
        menu.set_runs(&labels);
        view.set_theme(nn_app::styles::effective_theme(
            settings.theme.theme(None),
            &settings.styles,
        ));
    }
    if let Some(path) = &flags.lang {
        // A localization file, loaded the way the editor loads it: the English
        // reference is read from beside it.
        let path = std::path::Path::new(path);
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match Catalog::load(Some(dir), &file) {
            Ok(catalog) => {
                // Every widget that can draw a word gets the same catalog, which
                // is what `Main::apply_language` does: a screenshot where the
                // menus are translated and the panel is not would be a
                // screenshot of a bug that does not exist.
                menu.set_catalog(Some(catalog.clone()));
                prefs.set_catalog(Some(catalog.clone()));
                status.set_catalog(Some(catalog.clone()));
                prompt.set_catalog(Some(catalog.clone()));
                find.set_catalog(Some(catalog.clone()));
                about.set_catalog(Some(catalog.clone()));
                toolbar.set_catalog(Some(catalog));
            }
            Err(error) => eprintln!("bench: {error}"),
        }
        prefs.set_languages(&Catalog::languages(Some(dir)));
    }
    if flags.menu {
        menu.click(2.0, 2.0, view.line_height(), 900.0);
    }
    if flags.edit_menu {
        menu.open_menu(1);
    }
    if flags.lines_menu {
        menu.open_menu(1);
    }
    if flags.view_menu {
        menu.open_menu(3);
    }
    if flags.encoding_menu {
        menu.open_menu(4);
    }
    if flags.character_sets {
        // The Encoding menu's sixth row is "Character sets", and its first
        // script is "Arabic": two levels down, which is as deep as the menu goes.
        menu.open_menu(4);
        menu.set_hover(Some(5));
        menu.open_submenu();
        menu.set_hover(Some(0));
        menu.open_submenu();
    }
    if flags.settings_menu {
        menu.open_menu(6);
    }
    if flags.settings_import {
        // The Settings menu's fifth row is `Import`.
        menu.open_menu(6);
        menu.set_hover(Some(4));
        menu.open_submenu();
    }
    if flags.run_menu {
        menu.open_menu(9);
    }
    let mut brace_line = None;
    if flags.brace {
        let text = editor.doc.text();
        if let Some(byte) = text.find("pub struct Document {") {
            let pos = text[..byte].chars().count() + "pub struct Document ".chars().count();
            editor.set_caret(pos);
            view.set_brace(editor.matching_brace());
            brace_line = Some(editor.doc.char_to_line(pos));
        }
    }
    if flags.funcs {
        functions.set_symbols(nn_core::scan_symbols(&editor.doc.text()));
    }
    if flags.folder || flags.project {
        // The document's own directory, so the dump shows a real tree rather
        // than a fixture that has to be kept in step with the source.
        let root = editor
            .doc
            .path()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let roots = if flags.project {
            vec![root.clone(), root.join("bin")]
        } else {
            vec![root]
        };
        let mut tree = nn_core::Tree::scan(&roots);
        if let Some(filter) = &flags.folder_filter {
            tree.set_filter(filter);
        }
        let title = if flags.project {
            "Project 1"
        } else {
            "Folder as Workspace"
        };
        workspace.show(tree, title);
        if flags.folder_filter.is_some() {
            workspace.focus_filter();
        }
    }
    let mut smart_line = None;
    if flags.smart {
        let text = editor.doc.text();
        if let Some(byte) = text.find("fn sync_rows") {
            let pos = text[..byte].chars().count() + 3;
            editor.set_caret(pos);
            smart_line = Some(editor.doc.char_to_line(pos));
        }
        if let Some(word) = editor.highlight_word() {
            let word = editor.doc.slice(word);
            view.set_smart(editor.occurrences(&word, 2000));
        }
    }
    if flags.results {
        let dir = std::path::Path::new("/opt/DEV/nn/crates/nn-core/src");
        let hits = nn_core::FindInFiles::new("struct ", dir).run().unwrap_or_default();
        results.set_hits(hits);
    }
    if flags.sorted {
        editor.select_range(0..editor.doc.len_chars());
        editor.transform_lines(|text| nn_core::lines::sort(text, Default::default()));
    }
    if flags.wrap {
        view.set_wrap(true);
    }
    if flags.bookmarks {
        for line in (0..editor.doc.len_lines()).step_by(7) {
            editor.doc.toggle_bookmark(line);
        }
    }
    if flags.light {
        view.set_theme(Theme::LIGHT);
    }
    view.resize(&mut editor, width as f32, height as f32);

    // A second pane, for the split-view screenshots. Its document is a short
    // one so the two panes are visibly different files rather than the same
    // text twice.
    let mut other_editor = Editor::new(Document::from_text(SECOND_DOC));
    let mut other_view = EditorView::new(renderer.font_system(), view.theme());
    let mut other_tab_bar = TabBar::new(
        cosmic_text::Metrics::new(15.0, 20.0),
        cosmic_text::Family::Monospace,
        other_view.char_width(),
    );
    other_tab_bar.set_tabs(vec![nn_app::TabInfo::new("notes.txt")], 0);
    other_view.resize(&mut other_editor, width as f32, height as f32);
    if flags.light {
        other_view.set_theme(Theme::LIGHT);
    }
    if let Some(line) = brace_line.or(smart_line) {
        editor.viewport.scroll_to_top();
        editor.viewport.scroll_lines(line as i64, editor.doc.len_lines());
    }
    renderer.resize(&queue, width, height);
    if flags.prefs {
        prefs.open(&Settings {
            theme: nn_app::ThemeChoice::Dark,
            font_size: 15.0,
            tab_width: 4,
            wrap: true,
            minimap: true,
            function_list: false,
            ..Settings::default()
        });
        prefs.set_selected(2);
    }
    if flags.prefs_lang {
        // The Language row, with the list the editor would have found.
        prefs.set_languages(&Catalog::languages(None));
        prefs.open(&Settings {
            language: flags.lang.clone(),
            ..Settings::default()
        });
        prefs.set_selected(
            nn_app::prefs::Row::ALL
                .iter()
                .position(|row| *row == nn_app::prefs::Row::Language)
                .expect("the language row"),
        );
    }
    if flags.picker {
        // The theme list, open on the theme in force.
        prefs.open(&Settings::default());
        prefs.set_selected(0);
        prefs.open_picker();
    }
    if flags.lang_picker {
        // The language list, open on the language in force.
        prefs.set_languages(&Catalog::languages(None));
        prefs.open(&Settings {
            language: flags.lang.clone(),
            ..Settings::default()
        });
        prefs.set_selected(
            nn_app::prefs::Row::ALL
                .iter()
                .position(|row| *row == nn_app::prefs::Row::Language)
                .expect("the language row"),
        );
        prefs.open_picker();
    }
    if flags.print_preview {
        // The preview of the document the bench loaded, at the default setup.
        let setup = nn_core::print::PageSetup::default();
        let char_width = view.char_width() * setup.font_size / 15.0;
        let name = path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        print.open(
            &editor.doc.text(),
            &name,
            path.as_deref(),
            setup,
            char_width,
        );
        print.set_page(flags.print_page);
        if let Some(path) = &flags.print_pdf {
            // The same bytes `Print Now` would hand to `lp`, so the PDF can be
            // opened by a reader rather than only inspected by a test.
            std::fs::write(path, print.pdf()).expect("write the pdf");
        }
    }
    if flags.styles {
        styles.open(view.theme());
        styles.set_selected(1);
    }
    if flags.shortcuts {
        shortcuts.open(&Keymap::default());
        shortcuts.set_selected(3);
    }
    if flags.complete {
        // Put the caret on a real line so the popup lands where it would in
        // use, not at the top-left corner of the window.
        let text = editor.doc.text();
        if let Some(byte) = text.find("fn sync_rows") {
            let pos = text[..byte].chars().count() + 3;
            editor.set_caret(pos);
            let line = editor.doc.char_to_line(pos);
            editor.viewport.scroll_to_top();
            // A few lines down, so the popup lands under the caret instead of
            // hanging off the top of the window.
            editor
                .viewport
                .scroll_lines(line.saturating_sub(4) as i64, editor.doc.len_lines());
        }
        let (x, y) = view.caret_point(&mut editor);
        let anchor = (x, y + view.line_height());
        complete.show(
            [
                "sync_rows", "sync_rows_lazy", "sync_rows_now", "sync_row_count",
                "sync_rows_after_edit", "sync_rows_for_wrap", "sync_rows_cached",
                "sync_rows_dirty", "sync_rows_visible", "sync_rows_all",
                "sync_rows_range", "sync_rows_end",
            ]
            .iter()
            .map(|s| Item::word(*s))
            .collect(),
            anchor,
        );
        complete.move_selection(2);
    }
    if flags.func_complete {
        // The real scanner on the real document, so the dump proves the wiring
        // rather than a hand-written list.
        let text = editor.doc.text();
        let functions = nn_core::scan_functions(&text);
        let items: Vec<Item> = functions
            .iter()
            .filter(|f| f.name.starts_with("set_c"))
            .take(12)
            .map(|f| Item::function(&f.name, &f.params))
            .collect();
        if let Some(byte) = text.find("pub fn set_caret") {
            let pos = text[..byte].chars().count() + 11;
            editor.set_caret(pos);
            let line = editor.doc.char_to_line(pos);
            editor.viewport.scroll_to_top();
            // A few lines down, so the popup lands under the caret instead of
            // hanging off the top of the window.
            editor
                .viewport
                .scroll_lines(line.saturating_sub(4) as i64, editor.doc.len_lines());
        }
        let (x, y) = view.caret_point(&mut editor);
        complete.show(items, (x, y + view.line_height()));
        complete.move_selection(1);
    }
    if flags.doc_complete {
        let text = editor.doc.text();
        let words = nn_core::collect_words_many(&[&text, "document completion pools every open file"], 20_000);
        let items: Vec<Item> = nn_core::matching_words("doc", &words, 12)
            .into_iter()
            .map(Item::word)
            .collect();
        if let Some(byte) = text.find("use crate::datetime") {
            let pos = text[..byte].chars().count() + 11;
            editor.set_caret(pos);
            let line = editor.doc.char_to_line(pos);
            editor.viewport.scroll_to_top();
            editor
                .viewport
                .scroll_lines(line.saturating_sub(4) as i64, editor.doc.len_lines());
        }
        let (x, y) = view.caret_point(&mut editor);
        complete.show(items, (x, y + view.line_height()));
    }
    if flags.path_complete {
        let dir = std::path::Path::new("crates/nn-app/src");
        let items: Vec<Item> = nn_app::paths::complete_path(dir, "m", 12)
            .into_iter()
            .map(|name| Item {
                label: name.clone(),
                insert: format!("src/{name}"),
                back: 0,
            })
            .collect();
        let text = editor.doc.text();
        if let Some(byte) = text.find("use crate::datetime") {
            let pos = text[..byte].chars().count() + 11;
            editor.set_caret(pos);
            let line = editor.doc.char_to_line(pos);
            editor.viewport.scroll_to_top();
            editor
                .viewport
                .scroll_lines(line.saturating_sub(4) as i64, editor.doc.len_lines());
        }
        let (x, y) = view.caret_point(&mut editor);
        complete.show(items, (x, y + view.line_height()));
    }
    if flags.param_hint {
        // Find a real call in the document whose declaration is also in it, so
        // the hint is produced by the same code the editor runs.
        let text = editor.doc.text();
        let mut found = None;
        for (i, c) in text.char_indices() {
            if c != '(' {
                continue;
            }
            let caret = i + 1;
            let Some(call) = nn_core::call_at(&text, caret) else {
                continue;
            };
            let Some(params) = nn_core::params_of(&text, &call.name) else {
                continue;
            };
            if params.is_empty() || params.contains('\n') {
                continue;
            }
            found = Some((caret, call, params));
            break;
        }
        if let Some((caret, call, params)) = found {
            let pos = text[..caret].chars().count();
            editor.set_caret(pos);
            let line = editor.doc.char_to_line(pos);
            editor.viewport.scroll_to_top();
            // A few lines down, so the popup lands under the caret instead of
            // hanging off the top of the window.
            editor
                .viewport
                .scroll_lines(line.saturating_sub(4) as i64, editor.doc.len_lines());
            let signature = format!("{}({params})", call.name);
            let offset = call.name.len() + 1;
            let active = nn_core::arg_range(&params, call.arg)
                .map(|r| offset + r.start..offset + r.end);
            let (x, y) = view.caret_point(&mut editor);
            param_hint.show(signature, active, (x, y + view.line_height()));
        }
    }

    if flags.tooltip {
        // The real widget, driven through its real API: rest on a button, then
        // tick past the delay. The anchor is the Save button's, computed the way
        // the toolbar computes it, so the dump shows the label where a hover
        // would put it.
        // The anchor is the Save button's left edge and the bar's bottom edge at
        // the default font. Where the button actually sits is covered by a unit
        // test in `toolbar`; this flag exists to show the label, not to measure
        // the layout.
        let now = std::time::Instant::now();
        tooltip.rest(Some(("Save", (62.0, 57.0))), now);
        tooltip.tick(now + nn_app::tooltip::DELAY);
    }

    let split = flags.split.unwrap_or(Split::None);

    // The pane list is rebuilt at each render call rather than held in a local:
    // the find bar borrows the document between the warm-up and the dump, so a
    // `Vec<PaneRef>` that outlived both would not compile.
    macro_rules! panes {
        () => {
            if split.is_split() {
                vec![
                    PaneRef {
                        editor: &mut editor,
                        view: &mut view,
                        tabs: &mut tab_bar,
                    },
                    PaneRef {
                        editor: &mut other_editor,
                        view: &mut other_view,
                        tabs: &mut other_tab_bar,
                    },
                ]
            } else {
                vec![PaneRef {
                    editor: &mut editor,
                    view: &mut view,
                    tabs: &mut tab_bar,
                }]
            }
        };
    }

    // Warm up: shape the first screenful and compile the pipelines.
    for _ in 0..30 {
        renderer.render(
            &device,
            &queue,
            &target,
            &mut panes!(),
            split,
            0.5,
            split.is_split() && flags.flip,
            &mut find,
            &mut dialog,
            &mut menu,
            &mut toolbar,
            &mut prompt,
            &mut status,
            &mut results,
            &mut history,
            &mut charset,
            &mut functions,
            &mut workspace,
            &mut complete,
            &mut param_hint,
            &mut tooltip,
            &mut context,
            &mut peeker,
            &mut about,
            &mut prefs,
            &mut plugin_list,
            &mut print,
            &mut styles,
            &mut shortcuts,
            &mut browser,
            &mut playground,
            &chrome,
            cursor_y,
            width as f32,
            height as f32,
        );
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    if flags.history {
        // A handful of real edits, so the panel shows the shapes it has to
        // render: an insert, a delete, and a replace.
        editor.set_caret(0);
        editor.insert("// header\n");
        editor.break_undo_group();
        editor.set_caret(editor.doc.len_chars());
        editor.insert("\nfn added() {}\n");
        editor.break_undo_group();
        editor.select_range(0..9);
        editor.insert("// changed");
        editor.break_undo_group();
        highlighter.highlight_from(&mut editor.doc, 0);
        let rows: Vec<ChangeRow> = editor
            .history()
            .iter()
            .enumerate()
            .map(|(index, step)| ChangeRow {
                keep: index + 1,
                line: step.line,
                at: step.at,
                what: step.describe(),
            })
            .collect();
        history.set_rows(rows);
    }

    if flags.charset {
        charset.open();
        // Hover a cell in the middle of the grid, so the dump shows the
        // highlight and the readout rather than an untouched table.
        charset.set_hover(Some(0xB7));
    }

    if flags.incr {
        // Incremental search: the caret is the anchor, the query is typed, and
        // the selection follows the first match at or after the anchor.
        let anchor = editor.caret();
        find.open_incremental();
        find.query.set_text("fn ");
        find.refresh(&editor.doc);
        if let Some(range) = find.next_from(anchor) {
            editor.select_range(range);
            // The viewport has not been resized to the pane yet, so
            // `ensure_caret_visible` alone leaves the match off screen. Scroll
            // it into the middle by hand so the dump shows the selection.
            let line = editor.doc.char_to_line(editor.caret());
            editor.viewport.scroll_to_top();
            editor
                .viewport
                .scroll_lines(line as i64 - 6, editor.doc.len_lines());
        }
        view.set_matches(find.matches.clone(), find.current_match());
        println!("incr: {} matches", find.matches.len());
    }

    if flags.nonvolatile {
        // Non-volatile find: the bar closes and the highlights stay.
        find.open_find();
        // A pattern that occurs in the first screenful, so the dump shows the
        // highlights surviving the closed bar rather than an empty viewport.
        find.query.set_text("the");
        find.refresh(&editor.doc);
        find.close(false);
        view.set_matches(find.matches.clone(), find.current_match());
        println!("nonvolatile: {} matches retained", find.matches.len());
    }

    if let Some(path) = &flags.udl {
        // The same path the Language menu takes: parse the file, build a lexer
        // from the first language, re-highlight, and point the palette at it.
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let set = nn_core::UdlSet::parse(&text);
                if let Some(udl) = set.get(0) {
                    let mut hl = Highlighter::new(Box::new(udl.lexer()));
                    hl.highlight_all(&mut editor.doc);
                    view.set_palette(nn_app::Palette::with_udl(Theme::DARK, udl));
                    println!(
                        "udl: {} ext={:?} keywords1={} delimiters={}",
                        udl.name,
                        udl.extensions,
                        udl.keywords[0].len(),
                        udl.delimiters.len()
                    );
                }
                let names: Vec<String> = set.names().iter().map(|n| n.to_string()).collect();
                menu.set_languages(&names);
            }
            Err(err) => eprintln!("bench: cannot read {path}: {err}"),
        }
    }
    if flags.language_menu {
        menu.open_menu(5);
    }
    if flags.language_group {
        // The Language menu's third row is the `A` group: row 0 is plain text
        // and row 1 is the rule under it.
        menu.open_menu(5);
        menu.set_hover(Some(2));
        menu.open_submenu();
    }

    if let Some(dir) = &flags.plugins {
        // The same path the editor takes at startup: load every `.wasm` in the
        // directory, then give the menu the inventory it produces. If no
        // `plugin_run` flag follows, a submenu per plugin is all this run does
        // with them, which is what `menu=` measures.
        let mut plugins = nn_app::Plugins::discover(std::path::Path::new(dir));
        for index in 0..plugins.available() {
            let _ = plugins.enable(index);
        }
        for line in plugins.describe() {
            println!("plugin: {line}");
        }
        let rows: Vec<PluginMenu> = plugins
            .inventory()
            .into_iter()
            .map(|plugin| PluginMenu {
                name: plugin.name,
                enabled: plugin.enabled,
                commands: plugin.commands,
            })
            .collect();
        menu.set_plugins(&rows);
        if let Some(spec) = &flags.plugin_run {
            // `plugin:command`, so one flag can reach any command of any
            // plugin without a second flag to say which plugin.
            let (p, c) = spec.split_once(':').unwrap_or((spec.as_str(), "0"));
            let p: usize = p.parse().unwrap_or(0);
            let c: usize = c.parse().unwrap_or(0);
            let text = editor.doc.text();
            let context = Context {
                text: &text,
                caret: editor.doc.char_to_byte(editor.caret()),
                selection: (
                    editor.doc.char_to_byte(editor.selection_start()),
                    editor.doc.char_to_byte(editor.selection_end()),
                ),
                read_only: false,
            };
            match plugins.run(p, c, &context) {
                Ok(outcome) => {
                    for line in &outcome.log {
                        println!("plugin log: {line}");
                    }
                    if let Some(new) = outcome.text {
                        let len = editor.doc.len_chars();
                        editor.replace_ranges(&[(0..len, new)]);
                        println!("plugin run: {} chars", editor.doc.len_chars());
                    }
                }
                Err(err) => eprintln!("bench: plugin failed: {err}"),
            }
        }
    }
    if flags.plugin_menu {
        menu.open_menu(10);
    }

    if let Some(dir) = &flags.plugin_manager {
        // The manager as the window would show it: every `.wasm` in the
        // directory, each one off until it is switched on.
        let plugins = nn_app::Plugins::discover(std::path::Path::new(dir));
        let rows: Vec<PluginRow> = plugins
            .inventory()
            .into_iter()
            .map(|entry| PluginRow {
                name: entry.name,
                file: entry.file,
                enabled: entry.enabled,
            })
            .collect();
        plugin_list.open(rows);
    }

    if flags.export_shortcuts {
        // The same call the menu item makes, printed rather than written so the
        // dump can be diffed against a previous build.
        let export = nn_app::nppimport::render_shortcuts(&nn_app::Keymap::default());
        println!("{}", export.summary());
        print!("{}", export.text);
    }

    if let Some(pattern) = &flags.find_projects {
        // The same path the menu item takes: every root of every project, one
        // result list.
        let root = editor
            .doc
            .path()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut projects = nn_core::Projects::new();
        let index = projects.add("Project 1").unwrap();
        projects.projects[index].add_root(root.clone());
        projects.projects[index].add_root(root.join("bin"));
        let roots = projects.all_roots();
        let search = nn_core::FindInFiles::new(pattern, &roots[0]);
        match search.run_roots(&roots) {
            Ok(hits) => {
                println!("find in projects: {} hits in {} roots", hits.len(), roots.len());
                results.set_hits(hits);
            }
            Err(err) => eprintln!("bench: bad pattern: {err}"),
        }
    }
    if flags.floating_tab.is_some() {
        // The floating presentation: the same search state, drawn as
        // Notepad++'s window instead of the docked bar.
        find.set_floating(true);
        if let Some(name) = &flags.floating_tab {
            for tab in nn_app::finddialog::TABS {
                if tab.label().eq_ignore_ascii_case(name) {
                    dialog.set_tab(tab);
                }
            }
        }
        dialog.set_transparency(flags.floating_transparency);
    }
    if let Some(pattern) = &flags.find {
        find.open_find();
        find.set_floating(flags.floating_tab.is_some());
        find.query.set_text(pattern);
        find.refresh(&editor.doc);
        view.set_matches(find.matches.clone(), find.current_match());
        println!("find: {} matches", find.matches.len());
    }

    if let Some(path) = &dump {
        renderer.render(
            &device,
            &queue,
            &target,
            &mut panes!(),
            split,
            0.5,
            split.is_split() && flags.flip,
            &mut find,
            &mut dialog,
            &mut menu,
            &mut toolbar,
            &mut prompt,
            &mut status,
            &mut results,
            &mut history,
            &mut charset,
            &mut functions,
            &mut workspace,
            &mut complete,
            &mut param_hint,
            &mut tooltip,
            &mut context,
            &mut peeker,
            &mut about,
            &mut prefs,
            &mut plugin_list,
            &mut print,
            &mut styles,
            &mut shortcuts,
            &mut browser,
            &mut playground,
            &chrome,
            cursor_y,
            width as f32,
            height as f32,
        );
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        dump_frame(&device, &queue, &texture, width, height, path);
    }

    report(
        "idle",
        measure(&device, &queue, &target, &mut renderer, &mut editor, &mut highlighter, &mut view, &mut find, &mut dialog, &mut tab_bar, &mut menu, &mut toolbar, &mut prompt, &mut status, &mut results, &mut history, &mut charset, &mut functions, &mut workspace, &mut complete, &mut param_hint, &mut tooltip, &mut context, &mut peeker, &mut about, &mut prefs, &mut plugin_list, &mut print, &mut styles, &mut shortcuts, &mut browser, &mut playground, &chrome, cursor_y, width, height, frames, |_, _| {}),
    );
    report(
        "scroll",
        measure(&device, &queue, &target, &mut renderer, &mut editor, &mut highlighter, &mut view, &mut find, &mut dialog, &mut tab_bar, &mut menu, &mut toolbar, &mut prompt, &mut status, &mut results, &mut history, &mut charset, &mut functions, &mut workspace, &mut complete, &mut param_hint, &mut tooltip, &mut context, &mut peeker, &mut about, &mut prefs, &mut plugin_list, &mut print, &mut styles, &mut shortcuts, &mut browser, &mut playground, &chrome, cursor_y, width, height, frames, |ed, _| {
            ed.scroll_lines(1);
        }),
    );
    report(
        "type",
        measure(&device, &queue, &target, &mut renderer, &mut editor, &mut highlighter, &mut view, &mut find, &mut dialog, &mut tab_bar, &mut menu, &mut toolbar, &mut prompt, &mut status, &mut results, &mut history, &mut charset, &mut functions, &mut workspace, &mut complete, &mut param_hint, &mut tooltip, &mut context, &mut peeker, &mut about, &mut prefs, &mut plugin_list, &mut print, &mut styles, &mut shortcuts, &mut browser, &mut playground, &chrome, cursor_y, width, height, frames, |ed, hl| {
            let line = ed.doc.char_to_line(ed.caret());
            ed.insert("x");
            hl.highlight_from(&mut ed.doc, line);
            ed.ensure_caret_visible();
        }),
    );
}

/// Copies `texture` back to the CPU and writes it as a binary PPM.
///
/// PPM because it needs no encoder: a header and three bytes per pixel. It is
/// only used to eyeball a frame when no display is attached.
fn dump_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &Path,
) {
    let unpadded = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dump"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("dump"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().expect("map dump buffer");

    let mut out = Vec::with_capacity((width * height * 3) as usize + 32);
    out.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
    for row in 0..height {
        let start = (row * padded) as usize;
        for px in data[start..start + unpadded as usize].chunks_exact(4) {
            // The target is BGRA; PPM wants RGB.
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
    }
    drop(data);
    buffer.unmap();

    std::fs::write(path, out).expect("write dump");
    println!("wrote {}", path.display());
}

/// Runs `frames` frames, applying `step` before each, and returns the frame times
/// in milliseconds.
#[allow(clippy::too_many_arguments)]
fn measure(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    renderer: &mut Renderer,
    editor: &mut Editor,
    highlighter: &mut Highlighter,
    view: &mut EditorView,
    find: &mut FindBar,
    dialog: &mut FindDialog,
    tab_bar: &mut TabBar,
    menu: &mut MenuBar,
    toolbar: &mut Toolbar,
    prompt: &mut PromptBar,
    status: &mut StatusBar,
    results: &mut ResultsPanel,
    history: &mut ChangeHistory,
    charset: &mut CharPanel,
    functions: &mut FunctionList,
    workspace: &mut FolderPanel,
    complete: &mut CompletionPopup,
    param_hint: &mut ParamHint,
    tooltip: &mut Tooltip,
    context: &mut ContextMenu,
    peeker: &mut DocumentPeeker,
    about: &mut AboutBox,
    prefs: &mut Preferences,
    plugin_list: &mut PluginList,
    print: &mut PrintPreview,
    styles: &mut StyleConfigurator,
    shortcuts: &mut Shortcuts,
    browser: &mut Browser,
    playground: &mut nn_app::Playground,
    chrome: &Chrome,
    cursor_y: f32,
    width: u32,
    height: u32,
    frames: usize,
    mut step: impl FnMut(&mut Editor, &mut Highlighter),
) -> Vec<f64> {
    let mut samples = Vec::with_capacity(frames);
    for _ in 0..frames {
        step(editor, highlighter);
        let start = Instant::now();
        renderer.render(
            device,
            queue,
            target,
            &mut [PaneRef {
                editor,
                view,
                tabs: tab_bar,
            }],
            Split::None,
            0.5,
            false,
            find,
            dialog,
            menu,
            toolbar,
            prompt,
            status,
            results,
            history,
            charset,
            functions,
            workspace,
            complete,
            param_hint,
            tooltip,
            context,
            peeker,
            about,
            prefs,
            plugin_list,
            print,
            styles,
            shortcuts,
            browser,
            playground,
            chrome,
            cursor_y,
            width as f32,
            height as f32,
        );
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

/// Prints the percentile summary for one scenario.
fn report(name: &str, mut samples: Vec<f64>) {
    if samples.is_empty() {
        return;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p) as usize];
    let p50 = at(0.50);
    let p95 = at(0.95);
    let p99 = at(0.99);
    let max = *samples.last().unwrap();
    println!(
        "{name:<7} p50 {p50:6.2} ms  p95 {p95:6.2} ms  p99 {p99:6.2} ms  max {max:6.2} ms   \
         ({:5.0} fps at p99)",
        1000.0 / p99.max(0.001)
    );
}

/// A document of `lines` lines of plausible source code.
/// The second pane's document: short, and obviously not the first one.
const SECOND_DOC: &str = "// notes.txt\n\
fn main() {\n\
    let total = 0;\n\
    for item in items {\n\
        total += item;\n\
    }\n\
    println!(\"{total}\");\n\
}\n\
\n\
// The other view scrolls on its own, folds on its own,\n\
// and keeps its own caret.\n";

fn synthetic(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 48);
    for i in 0..lines {
        match i % 5 {
            0 => out.push_str(&format!("fn handler_{i}(input: &str) -> Result<(), Error> {{\n")),
            1 => out.push_str("    let trimmed = input.trim();\n"),
            2 => out.push_str("    if trimmed.is_empty() { return Ok(()); }\n"),
            3 => out.push_str("    // The quick brown fox jumps over the lazy dog.\n"),
            _ => out.push_str("}\n"),
        }
    }
    out
}
