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

//! The render pass.
//!
//! Owns every GPU resource that outlives a frame and knows how to draw one frame
//! of the editor into a texture view. It does not know about windows or surfaces,
//! which is what lets the benchmark drive it offscreen.

use cosmic_text::{FontSystem, SwashCache};
use glyphon::{Cache, Resolution, TextArea, TextAtlas, TextRenderer, Viewport};
use nn_core::Editor;

use crate::browser::Browser;
use crate::changelog::ChangeHistory;
use crate::charset::CharPanel;
use crate::chrome::Chrome;
use crate::complete::{CompletionPopup, ParamHint};
use crate::context::ContextMenu;
use crate::about::AboutBox;
use crate::peeker::DocumentPeeker;
use crate::findbar::FindBar;
use crate::finddialog::FindDialog;
use crate::folder::FolderPanel;
use crate::functionlist::FunctionList;
use crate::layout::{self, Split};
use crate::menu::MenuBar;
use crate::pluginlist::PluginList;
use crate::prefs::Preferences;
use crate::playground::Playground;
use crate::print::PrintPreview;
use crate::styles::StyleConfigurator;
use crate::shortcuts::Shortcuts;
use crate::prompt::PromptBar;
use crate::iconrender::IconRenderer;
use crate::quad::QuadRenderer;
use crate::results::ResultsPanel;
use crate::statusbar::StatusBar;
use crate::tabbar::TabBar;
use crate::toolbar::Toolbar;
use crate::tooltip::Tooltip;
use crate::view::EditorView;

/// The font size every piece of window furniture is shaped with, at scale 1.
///
/// The menu bar, the toolbar, the tab strip, the status bar and the panels are
/// built at startup with the metrics this implies, and are rebuilt only when the
/// display's density changes. Zoom does not touch them, which is deliberate: zoom
/// is a property of the document, not of the window. Notepad++ behaves the same
/// way — `Ctrl` and the wheel make the text bigger and leave the furniture where
/// it is — and it is the only reading that makes sense, because a menu bar that
/// grew with the font would push the text off the bottom of the window at 40px.
pub const UI_FONT_SIZE: f32 = 15.0;

/// The line height every piece of window furniture is laid out with.
///
/// Must match the metrics the chrome widgets are constructed with in `main.rs`;
/// the two are kept in step by that file using these constants rather than
/// repeating the numbers. The renderer needs the value because it is what turns
/// a bar's height into a number of pixels, and it must not be the document's
/// line height: the bars would grow while their text stayed 15px, which is what
/// left the menu bar 59px tall with a 15px label pinned to its top edge.
pub const UI_LINE_HEIGHT: f32 = 20.0;

/// The two numbers the window furniture is built with, at one display density.
///
/// A pair rather than two parameters because they are always needed together and
/// always in the same ratio: a caller that scaled the line height and forgot the
/// font size would get a bar with a 15px label floating in a 40px row, which is
/// exactly the fault the constants above were split apart to stop.
///
/// Rounded to whole pixels, because that is the size the glyphs are rasterised
/// at and a fractional one lands on half a pixel: `cosmic-text` will honour 18.75
/// but the stems come out smeared where 19 would have been sharp. The rounding is
/// the reason this is not simply a multiplier kept in `State`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiMetrics {
    /// The size every piece of furniture is shaped at, in pixels.
    pub font_size: f32,
    /// The height of one row of it, in pixels.
    pub line_height: f32,
}

impl UiMetrics {
    /// The furniture metrics a display of `scale` density asks for.
    pub fn at(scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Self {
            font_size: (UI_FONT_SIZE * scale).round(),
            line_height: (UI_LINE_HEIGHT * scale).round(),
        }
    }

    /// How far this is from the size the constants describe.
    ///
    /// Read back off the line height rather than kept as a third field: it is the
    /// number the icons and the hit tests are scaled by, and two answers to "how
    /// big is everything" would be one too many.
    pub fn scale(self) -> f32 {
        self.line_height / UI_LINE_HEIGHT
    }

    /// The metrics `cosmic-text` wants for a buffer of this furniture.
    pub fn metrics(self) -> cosmic_text::Metrics {
        cosmic_text::Metrics::new(self.font_size, self.line_height)
    }
}

impl Default for UiMetrics {
    fn default() -> Self {
        Self::at(1.0)
    }
}

/// One pane's mutable state, as the renderer needs it.
///
/// A struct rather than three parameters because the renderer takes a slice of
/// them: one entry for a single view, two for a split. The borrow checker is the
/// reason it is a slice of disjoint field borrows rather than a method on the
/// application state.
pub struct PaneRef<'a> {
    pub editor: &'a mut Editor,
    pub view: &'a mut EditorView,
    pub tabs: &'a mut TabBar,
}

/// Draws the editor.
pub struct Renderer {
    quads: QuadRenderer,
    /// A second quad batch drawn after the text, for anything that has to cover
    /// glyphs: an open menu, a tooltip, a popup.
    overlay: QuadRenderer,
    /// The toolbar's icons, drawn from a vector atlas.
    icons: IconRenderer,
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    viewport: Viewport,
}

impl Renderer {
    /// Builds the pipelines and glyph atlas for a target of `format`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::with_font_system(device, queue, format, crate::fonts::font_system())
    }

    /// The same, with a font system already built.
    ///
    /// Scanning the installed fonts costs more than everything else this
    /// constructor does, and it needs nothing but the CPU, so the caller can
    /// start it on a thread while the graphics device is being opened and hand
    /// the answer over here. That is why this exists: to keep the two waits
    /// running at the same time instead of one after the other.
    pub fn with_font_system(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_system: FontSystem,
    ) -> Self {
        let quads = QuadRenderer::new(device, format);
        let overlay = QuadRenderer::new(device, format);
        // The atlas is built at the size the default font implies; the first
        // frame resizes it if the window turns out to be on a denser display.
        let icons = IconRenderer::new(device, queue, format, 23);
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let viewport = Viewport::new(device, &cache);

        Self {
            quads,
            overlay,
            icons,
            font_system,
            swash_cache: SwashCache::new(),
            atlas,
            text_renderer,
            viewport,
        }
    }

/// The font system, for measuring text before the first frame.
    pub fn font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// Tells glyphon the target size.
    pub fn resize(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        self.viewport.update(queue, Resolution { width, height });
    }

    /// Draws one frame of `panes` into `target`.
    ///
    /// The argument list is the frame: every widget that draws something, plus
    /// the target. Bundling them into a struct would move the same list one
    /// indirection away without making it shorter.
    ///
    /// `panes` holds one entry for a single view and two for a split. The bars —
    /// menu, toolbar, prompt, find, results, status, function list — are window
    /// furniture and are drawn once, spanning the whole width; only the editor
    /// and its tab strip are per pane.
    ///
    /// `panes` is ordered by focus: `panes[0]` is the one the caret is in, which
    /// is what the status bar reads. `focused_second` says where that pane sits
    /// on screen, so the document that is being edited stays in its own half of
    /// the window when the focus moves to the other one.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        panes: &mut [PaneRef<'_>],
        split: Split,
        ratio: f32,
        focused_second: bool,
        find: &mut FindBar,
        dialog: &mut FindDialog,
        menu: &mut MenuBar,
        toolbar: &mut Toolbar,
        prompt: &mut PromptBar,
        status: &mut StatusBar,
        results: &mut ResultsPanel,
        history: &mut ChangeHistory,
        charset: &mut CharPanel,
        functions: &mut FunctionList,
        folder: &mut FolderPanel,
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
        playground: &mut Playground,
        chrome: &Chrome,
        ui: f32,
        cursor_y: f32,
        width: f32,
        height: f32,
    ) {
        // The playground is not a layer over the window: it replaces it. Taking
        // the whole frame rather than drawing over the editor is what lets the
        // mode hide the tabs, the menu and the document behind it without any of
        // them being laid out, prepared or painted.
        if playground.is_active() {
            let theme = panes[0].view.theme();
            self.render_playground(device, queue, target, playground, &theme, width, height);
            return;
        }
        // The chrome's measurements are the same for every pane, so they are read
        // once from the first one. `Theme` is `Copy`, which is what lets the rest
        // of this function talk about colours without holding a borrow on a view
        // it is about to mutate.
        let padding = panes[0].view.padding();
        let char_width = panes[0].view.char_width();
        let theme = panes[0].view.theme();
        // Two line heights exist in this window and the distinction matters. The
        // document's belongs to the view and changes with zoom; `ui` belongs to
        // the window, and changes only when the display's density does. Every bar
        // below is measured and laid out with `ui`, so zooming moves the text and
        // nothing else. Passing the document's height here is what made the menu
        // bar grow to 59px at 40px while its labels stayed 15px, and it also
        // rebuilt the toolbar's icon atlas on every notch.

        // Which furniture is on screen. Every height below is zero when its bar
        // is hidden, so the layout arithmetic needs no special case: a hidden
        // bar simply occupies nothing and the panes grow to fill the space.
        let show_menu = chrome.shows_menu(cursor_y, ui);
        let show_toolbar = chrome.shows_toolbar();
        let show_tabs = chrome.shows_tabs();
        let show_status = chrome.shows_status();
        let show_prompt = chrome.shows_prompt();
        let show_results = chrome.shows_results(results.is_open());
        let show_history = chrome.shows_history(history.is_open());
        let show_functions = chrome.shows_functions();
        let show_folder = chrome.shows_folder() && folder.is_open();
        let show_minimap = chrome.shows_minimap();

        let status_height = if show_status {
            status.height(ui)
        } else {
            0.0
        };
        let results_height = if show_results {
            results.height(ui)
        } else {
            0.0
        };
        let history_height = if show_history {
            history.height(ui)
        } else {
            0.0
        };
        // The bar is measured after it is prepared: a button's width comes from
        // the shaped label, and a first frame laid out with the widths still at
        // zero fits every button on one row while the frame itself draws two --
        // the second row then hangs over the status bar. Preparing here also
        // sizes the label column, which the fields are placed against.
        find.prepare(&mut self.font_system);
        let bar_height = find.height(width, ui, padding);
        let functions_width = if show_functions {
            functions.width(char_width)
        } else {
            0.0
        };
        // The workspace panel takes width off the left edge, so it is added to
        // the inset rather than subtracted from each pane: the panes divide what
        // is left, and a split still splits the space the user can see.
        let folder_width = if show_folder {
            folder.width(char_width)
        } else {
            0.0
        };
        let menu_height = if show_menu {
            menu.height(ui)
        } else {
            0.0
        };
        let toolbar_height = if show_toolbar {
            toolbar.height(ui)
        } else {
            0.0
        };
        let tab_height = if show_tabs {
            panes[0].tabs.height(ui)
        } else {
            0.0
        };
        // The vertical strip takes width instead of height, and each pane has its
        // own column, so the inset is per pane rather than per window.
        let tab_width = if show_tabs {
            panes[0].tabs.width()
        } else {
            0.0
        };
        let prompt_height = if show_prompt {
            prompt.height(ui, padding)
        } else {
            0.0
        };
        let bars_top = menu_height + toolbar_height;

        // The area the panes divide between them: below the menu, toolbar, and
        // tab strips, above the find bar, results, and status bar.
        let panes_top = bars_top;
        let panes_height = (height
            - bars_top
            - status_height
            - results_height
            - history_height
            - bar_height)
            .max(1.0);
        // The text column is capped and centred in distraction-free mode. The
        // inset is applied to the whole pane area before it is divided, not to
        // each pane afterwards: insetting each half of a split would take twice
        // the margin out of half the width and could leave a pane a pixel wide.
        // Applied here, the gutter, the caret, the selection rectangles and the
        // hit testing all move together and none of them can disagree.
        let inset = chrome.inset(width, char_width);
        let left = inset + folder_width;
        let (first, second) = layout::panes(
            split,
            left,
            panes_top,
            width - 2.0 * inset - folder_width,
            panes_height,
            ratio,
        );
        let rects = [
            Some(layout::inset_left(first, tab_width)),
            second.map(|rect| layout::inset_left(rect, tab_width)),
        ];

        self.quads.begin();
        menu.prepare(&mut self.font_system);
        for pane in panes.iter_mut() {
            pane.tabs.prepare(&mut self.font_system);
        }
        prompt.prepare(&mut self.font_system);
        status.update(panes[0].editor);
        status.prepare(&mut self.font_system);
        results.prepare(&mut self.font_system, width);
        history.prepare(&mut self.font_system, width);
        functions.prepare(&mut self.font_system, functions_width, panes_height);
        folder.prepare(
            &mut self.font_system,
            folder_width,
            panes_height - tab_height - prompt_height,
        );
        complete.set_viewport(height);
        complete.prepare(&mut self.font_system);
        param_hint.set_viewport(height);
        param_hint.prepare(&mut self.font_system, &theme);
        dialog.set_viewport(width, height);
        dialog.prepare(&mut self.font_system, find);
        tooltip.set_viewport(width, height);
        tooltip.prepare(&mut self.font_system, &theme);
        context.set_viewport(width, height);
        context.prepare(&mut self.font_system);
        peeker.set_viewport(width, height);
        peeker.prepare(&mut self.font_system);
        about.set_viewport(width, height);
        about.prepare(&mut self.font_system, &theme);
        prefs.set_viewport(width, height);
        prefs.prepare(&mut self.font_system);
        plugin_list.set_viewport(width, height);
        plugin_list.prepare(&mut self.font_system);
        print.set_viewport(width, height);
        print.prepare(&mut self.font_system);
        styles.set_viewport(width, height);
        styles.prepare(&mut self.font_system);
        shortcuts.set_viewport(width, height);
        shortcuts.prepare(&mut self.font_system);
        browser.set_viewport(width, height);
        browser.prepare(&mut self.font_system);
        charset.set_viewport(width, height);
        charset.prepare(&mut self.font_system);

        // The editor paints its own background over the whole window, so every
        // bar is queued after it. The menu goes last: an open dropdown has to
        // cover the tab strip and the text, not sit under them.
        //
        // Each pane is told where it lives before it prepares, because a view
        // emits absolute coordinates: it has to know its own origin, its own
        // height, and how much of its right edge the function list is covering.
        //
        // `panes` is ordered by focus and `rects` by position, and the two are
        // not the same order once the focused pane is the one on the right (or
        // at the bottom): a click there moves the focus, and moving the focus
        // must not move the documents. `position` is what maps one to the other.
        let pane_count = panes.len();
        for (index, pane) in panes.iter_mut().enumerate() {
            let position = if focused_second {
                pane_count - 1 - index
            } else {
                index
            };
            let Some(rect) = rects[position] else { continue };
            let last = position + 1 == pane_count;
            pane.view.set_origin(rect.x, rect.y);
            pane.view.minimap_mut().set_suppressed(!show_minimap);
            pane.view.set_right_dock(if last { functions_width } else { 0.0 });
            pane.view.set_content_top(tab_height + prompt_height);
            pane.view
                .set_text_height(rect.height - tab_height - prompt_height);
            pane.view.resize(pane.editor, rect.width, rect.height);
            pane.view
                .prepare(pane.editor, &mut self.font_system, &mut self.quads);
            // The width, not the y: the strip's bounds are what clips its
            // titles, and passing the pane's top edge here clipped every title
            // to the first few characters.
            if show_tabs {
                // The column sits to the left of the pane's text, so the strip's
                // own origin is the pane's left edge before the inset.
                let column = pane.tabs.width();
                pane.tabs
                    .set_bounds(rect.x - column, rect.y, rect.width + column);
                pane.tabs.push_quads(&mut self.quads, ui, &theme);
            }
        }
        if let Some(divider) = layout::divider(
            split,
            left,
            panes_top,
            width - 2.0 * inset - folder_width,
            panes_height,
            ratio,
        ) {
            self.quads.push(
                divider.x,
                divider.y,
                divider.width,
                divider.height,
                theme.divider.to_linear(),
            );
        }
        if show_toolbar {
            toolbar.push_quads(&mut self.quads, width, menu_height, ui, &theme);
        }
        // One atlas, one upload, one draw call for every vector glyph on screen:
        // the toolbar's thirty-two buttons and the tab strip's crosses, pins and
        // pencils. The atlas is rebuilt only when the icon size changes, which is
        // when the font or the display density does.
        let wanted = toolbar.icon_size(ui).round() as u32;
        self.icons.set_size(device, queue, wanted);
        self.icons.begin();
        if show_toolbar {
            toolbar.push_icons(&mut self.icons, width, menu_height, ui, &theme);
        }
        if show_tabs {
            for pane in panes.iter() {
                pane.tabs.push_icons(&mut self.icons, ui, &theme);
            }
        }
        self.icons.upload(device, queue, width, height);
        if show_prompt {
            prompt.push_quads(
            &mut self.quads,
            width,
                bars_top + tab_height,
                ui,
                padding,
                &theme,
            );
        }
        find.push_quads(
            &mut self.quads,
            width,
            height - status_height - history_height - results_height - bar_height,
            ui,
            padding,
            &theme,
        );
        if show_results {
            results.push_quads(
                &mut self.quads,
                width,
                height - status_height - history_height - results_height,
                ui,
                &theme,
            );
        }
        if show_history {
            history.push_quads(
                &mut self.quads,
                width,
                height - status_height - history_height,
                ui,
                &theme,
            );
        }
        if show_status {
            status.push_quads(
                &mut self.quads,
                width,
                height - status_height,
                ui,
                &theme,
            );
        }
        // The function list is docked to the window's right edge, so it spans
        // the whole pane area rather than one pane's.
        if show_functions {
            functions.push_quads(
                &mut self.quads,
                width,
                panes_top + tab_height + prompt_height,
                panes_height - tab_height - prompt_height,
                &theme,
            );
        }
        // The workspace panel is docked to the window's left edge, below the
        // menu and toolbar and above the status bar, so it spans the pane area
        // rather than one pane's.
        if show_folder {
            folder.push_quads(
                &mut self.quads,
                panes_top,
                panes_height,
                &theme,
            );
        }
        if show_menu {
            menu.push_quads(&mut self.quads, width, ui, &theme);
        }
        self.quads.upload(device, queue, width, height);

        self.overlay.begin();
        menu.push_dropdown_quads(&mut self.overlay, ui, &theme, width);
        complete.push_quads(&mut self.overlay, ui, &theme);
        // The hint sits under the caret, which is where the popup also opens, so
        // it goes above the popup: when both are up the hint is the one you are
        // reading while you type the arguments.
        param_hint.push_quads(&mut self.overlay, ui, &theme);
        // Above the completion popup: a right-click while the list is open has
        // to put the menu on top of it, not behind it.
        context.push_quads(&mut self.overlay, ui, &theme);
        // The tooltip hangs off the toolbar, which is drawn in the first pass,
        // so it has to be in the overlay to cover it. It goes above the popups
        // and below the modals: a label is an annotation on the window, and a
        // dialog is a replacement for it.
        tooltip.push_quads(&mut self.overlay, ui, &theme);
        // The search sheet is a dialog, so it goes above the tooltip and below
        // the peeker and the About box, which answer to a key and cover more.
        dialog.push_quads(&mut self.overlay, find, ui, &theme);
        // The peeker covers the window, so it goes above the context menu.
        peeker.push_quads(&mut self.overlay, ui, &theme);
        // The About box is a dialog, so it goes above the peeker and below the
        // print preview, which is the only thing that covers a whole window.
        about.push_quads(&mut self.overlay, ui, &theme);
        // The print preview covers the whole window, so it goes above every
        // other overlay: nothing should be visible through a sheet of paper.
        print.push_quads(&mut self.overlay, &theme);
        // The shortcut editor is opened from the preferences dialog and covers
        // it, so only one of the two is ever painted.
        if !shortcuts.is_open() {
            prefs.push_quads(&mut self.overlay, &theme);
            plugin_list.push_quads(&mut self.overlay, &theme);
        }
        shortcuts.push_quads(&mut self.overlay, &theme);
        styles.push_quads(&mut self.overlay, &theme);
        browser.push_quads(&mut self.overlay, &theme);
        // The character grid is a palette, not a modal: it is opened from the
        // Edit menu while the caret is in the text and stays up while several
        // characters are picked, so it goes above everything else.
        charset.push_quads(&mut self.overlay, &theme);
        self.overlay.upload(device, queue, width, height);

        let mut areas: Vec<TextArea<'_>> = Vec::new();
        for pane in panes.iter() {
            if show_tabs {
                areas.extend(pane.tabs.text_areas(ui, &theme));
            }
            areas.extend(pane.view.text_areas(&*pane.editor));
        }
        if show_prompt {
            areas.extend(prompt.text_areas(
                width,
                bars_top + tab_height,
                ui,
                padding,
                &theme,
            ));
        }
        areas.extend(find.text_areas(
            width,
            height - status_height - history_height - results_height - bar_height,
            ui,
            padding,
            &theme,
        ));
        if show_results {
            areas.extend(results.text_areas(
                width,
                height - status_height - history_height - results_height,
                ui,
                padding,
                &theme,
            ));
        }
        if show_history {
            areas.extend(history.text_areas(
                width,
                height - status_height - history_height,
                ui,
                padding,
                &theme,
            ));
        }
        if show_functions {
            areas.extend(functions.text_areas(
                width,
                panes_top + tab_height + prompt_height,
                panes_height - tab_height - prompt_height,
                padding,
                &theme,
            ));
        }
        if show_folder {
            areas.extend(folder.text_areas(
                panes_top,
                panes_height,
                padding,
                &theme,
            ));
        }
        if show_menu {
            areas.extend(menu.text_areas(width, ui, &theme));
        }
        if show_status {
            areas.extend(status.text_areas(
                width,
                height - status_height,
                ui,
                padding,
                &theme,
            ));
        }
        if let Err(err) = self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        ) {
            eprintln!("npp: text prepare failed: {err:?}");
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(theme.background.to_wgpu()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.quads.draw(&mut pass);
            if show_toolbar || show_tabs {
                self.icons.draw(&mut pass);
            }
            if let Err(err) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                eprintln!("npp: text render failed: {err:?}");
            }
            self.overlay.draw(&mut pass);
        }
        // Submitted before the second prepare: glyphon recreates its vertex
        // buffer when the glyph count grows, and destroying a buffer that a
        // recorded but unsubmitted draw still references is a validation error.
        queue.submit(Some(encoder.finish()));

        // The menu is drawn in a second pass because glyphs and rectangles are
        // separate pipelines: a dropdown background queued with the editor's
        // rectangles would still be painted under the editor's text.
        // Only when a dropdown is open: the bar's own titles are already drawn in
        // the first pass, and a second pass costs a full prepare.
        let mut menu_areas = menu.dropdown_areas(ui, &theme, width);
        menu_areas.extend(complete.text_areas(ui, &theme));
        menu_areas.extend(param_hint.text_areas(ui, &theme));
        menu_areas.extend(tooltip.text_areas(ui, &theme));
        menu_areas.extend(dialog.text_areas(find, ui, &theme));
        menu_areas.extend(context.text_areas(ui, &theme));
        menu_areas.extend(peeker.text_areas(ui, &theme));
        menu_areas.extend(about.text_areas(ui, &theme));
        if !shortcuts.is_open() {
            menu_areas.extend(prefs.text_areas(&theme));
            menu_areas.extend(plugin_list.text_areas(&theme));
        }
        menu_areas.extend(print.text_areas(&theme));
        menu_areas.extend(shortcuts.text_areas(&theme));
        menu_areas.extend(styles.text_areas(&theme));
        menu_areas.extend(browser.text_areas(&theme));
        menu_areas.extend(browser.filter_areas(&theme));
        menu_areas.extend(charset.text_areas(&theme));
        if !menu_areas.is_empty() {
            if let Err(err) = self.text_renderer.prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                menu_areas,
                &mut self.swash_cache,
            ) {
                eprintln!("npp: menu prepare failed: {err:?}");
            }
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("menu"),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Err(err) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                eprintln!("npp: menu render failed: {err:?}");
            }
            drop(pass);
            queue.submit(Some(encoder.finish()));
        }

        // Ends the frame for the glyph atlas, and it is not optional.
        //
        // glyphon's atlas is a packer with an LRU cache, and its only way to make
        // room for a new glyph is to evict one that is *not* in use. "In use" is
        // a set that `prepare` fills and `trim` empties, so a frame that never
        // trims leaves every glyph ever drawn marked as live: the packer cannot
        // evict anything, `try_allocate` fails, the atlas doubles until it hits
        // the device's maximum texture size, and then every `prepare` returns
        // `AtlasFull` and the whole text pass is dropped -- menu bar, document
        // and status bar at once, with nothing on screen to say why.
        //
        // Zooming is what made it reachable. Each font size rasterises its own
        // copy of every glyph, so stepping from 15px to 40px one notch at a time
        // caches twenty-six complete alphabets, and the atlas fills long before
        // the user has done anything unusual.
        //
        // Called once, after both passes, rather than between them: the second
        // pass draws dropdowns and modals, and a trim in the middle would let the
        // second `prepare` evict a glyph the first pass had just queued.
        self.atlas.trim();
    }

    /// Draws the playground, which is the whole window.
    ///
    /// Two passes rather than one, because glyphon's pipeline has no depth state
    /// and wgpu requires a pipeline's depth attachment to match the pass it is
    /// used in. So the room is drawn with a depth buffer, then the label is
    /// drawn on top of it with the depth attachment dropped and the colour
    /// loaded rather than cleared. One submit, so the two are one frame.
    #[allow(clippy::too_many_arguments)]
    fn render_playground(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        playground: &mut Playground,
        theme: &crate::theme::Theme,
        width: f32,
        height: f32,
    ) {
        playground.prepare(
            device,
            queue,
            &mut self.font_system,
            theme,
            width.max(1.0) as u32,
            height.max(1.0) as u32,
        );

        let areas = playground.text_areas(theme, height);
        if let Err(err) = self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        ) {
            eprintln!("npp: playground text failed: {err:?}");
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("playground"),
        });
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("playground pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(theme.background.to_wgpu()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: playground.depth_view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The borrow of the pass is taken by a `let mut` so the borrow of
            // `playground` for its depth view ends with the descriptor.
            let mut pass = pass;
            playground.draw(&mut pass);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("playground label"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Err(err) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                eprintln!("npp: playground label failed: {err:?}");
            }
        }
        queue.submit(Some(encoder.finish()));
        self.atlas.trim();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_furniture_doubles_with_the_display() {
        let one = UiMetrics::at(1.0);
        assert_eq!((one.font_size, one.line_height), (UI_FONT_SIZE, UI_LINE_HEIGHT));
        assert_eq!(one.scale(), 1.0);
        let two = UiMetrics::at(2.0);
        assert_eq!((two.font_size, two.line_height), (30.0, 40.0));
        assert_eq!(two.scale(), 2.0);
        assert_eq!(two.metrics(), cosmic_text::Metrics::new(30.0, 40.0));
        // Whole pixels, because that is the size a glyph is rasterised at: 18.75
        // would be honoured by the shaper and come out with smeared stems.
        let quarter = UiMetrics::at(1.25);
        assert_eq!((quarter.font_size, quarter.line_height), (19.0, 25.0));
    }

    #[test]
    fn a_nonsense_scale_is_the_plain_one() {
        // Not zero: a window whose furniture is nothing is worse than a window
        // that ignored the display.
        let one = UiMetrics::at(1.0);
        assert_eq!(UiMetrics::at(0.0), one);
        assert_eq!(UiMetrics::at(-2.0), one);
        assert_eq!(UiMetrics::at(f32::NAN), one);
        assert_eq!(UiMetrics::default(), one);
    }
}
