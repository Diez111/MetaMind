//! MetMind application library: state, commands, view construction and headless rendering.

pub mod ai;
pub mod anim;
pub mod attach;
pub mod bases;
pub mod baseview;
pub mod calendar;
pub mod canvasview;
pub mod cli;
pub mod clipboard;
pub mod dataview;
pub mod datetime;
pub mod editing;
pub mod excalidraw;
pub mod excalidrawview;
pub mod graphview;
pub mod icon;
pub mod icons;
pub mod instance;
pub mod kanbanview;
pub mod media;
pub mod mentions;
pub mod menu;
pub mod ospref;
pub mod panes;
pub mod pdf;
pub mod plugins;
pub mod popover;
pub mod properties;
pub mod recent;
pub mod rich;
pub mod session;
pub mod settings;
pub mod shell;
pub mod spell;
pub mod split;
pub mod suggest;
pub mod templates;
pub mod update;
pub mod uri;
pub mod video;
pub mod view;
pub mod welcome;
pub mod workspaces;
pub mod youtube;

pub use shell::{App, Key};

use metamind_ui::{Dom, Fonts, Pixmap};
use std::time::{Duration, Instant};

/// How long after a press the next one still counts as its second half.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Limits of the full-screen diagram's zoom, as a multiple of the fit its box gives it.
pub const MIN_DIAGRAM_ZOOM: f32 = 0.4;
pub const MAX_DIAGRAM_ZOOM: f32 = 4.0;

/// The scrollbar colour the theme asks for (`--scrollbar-thumb`), so the bar of the note and
/// the bar of a modal are painted alike and follow the scheme.
pub(crate) fn scrollbar_thumb(dom: &Dom) -> Option<theme_engine::Color> {
    dom.nodes.first()?.computed.var("--scrollbar-thumb").and_then(|v| theme_engine::parse_color(&v))
}

/// One rendered frame and the information needed to map input back onto it.
pub struct Frame {
    pub dom: Dom,
    pub info: view::ViewInfo,
    /// Where the editor caret is, for placing the input method's candidate window.
    pub caret: Option<metamind_ui::Rect>,
    /// What the rasterizer repainted for this frame: empty means the window already showed it.
    pub damage: metamind_ui::Damage,
}

/// Build, style, lay out and paint the window into `pixmap`.
pub fn render(app: &mut App, fonts: &mut Fonts, pixmap: &mut Pixmap, scale: f32) -> Frame {
    render_window(app, fonts, pixmap, scale, None)
}

/// Like `render`, for a window: `solo` is the pane a pop-out window shows alone.
pub fn render_window(
    app: &mut App,
    fonts: &mut Fonts,
    pixmap: &mut Pixmap,
    scale: f32,
    solo: Option<usize>,
) -> Frame {
    let t = Instant::now();
    app.pump_plugins();
    app.refresh_derived();
    app.animate_changes();
    app.busy_tick();
    app.video_tick(std::time::Instant::now());
    app.scale = scale;
    app.step_scrolls();
    app.window_w = pixmap.width() as f32 / scale;
    app.window_h = pixmap.height() as f32 / scale;
    let (mut dom, info) = view::build(app, pixmap.height() as f32 / scale, solo);
    let env = app.env();
    dom.style(&app.theme, env);
    dom.scrollbar_thumb = scrollbar_thumb(&dom);
    dom.layout(fonts, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
    // The box the over-window overlays live in, once it has been laid out: the workspace, with
    // the status bar already out of it. The video player asks for a frame that fits it.
    if let Some(ws) = info.workspace.and_then(|n| dom.nodes.get(n)) {
        app.video_area = (ws.rect.w, ws.rect.h);
    }
    view::scroll_views(app, &mut dom, &info);
    // The suggestion and the composing text sit at the caret, which is only known after layout.
    let caret = view::caret_rect(app, &dom, &info, fonts);
    let mut preedit = None;
    if let Some(caret) = caret.filter(|_| app.suggest.is_some() || !app.preedit.is_empty()) {
        view::add_suggest(app, &mut dom, caret, pixmap.height() as f32 / scale);
        preedit = view::add_preedit(app, &mut dom, &info, caret);
        dom.style(&app.theme, env);
        dom.layout(fonts, pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
    }
    icons::paint(&mut dom);
    view::add_carets(app, &mut dom, &info, fonts);
    if let Some(id) = preedit {
        view::mark_preedit(app, &mut dom, id, fonts);
    }
    view::add_graph(app, &mut dom, &info);
    canvasview::add_canvas(app, &mut dom, &info);
    kanbanview::add_kanban(app, &mut dom, &info, fonts);
    rich::paint(&mut dom, &info.rich, &info.inline, fonts);
    excalidraw::paint(&mut dom, &info.exc, fonts, app.dark);
    excalidrawview::add_exc(app, &mut dom, &info, fonts);
    let mut scene = dom.paint();
    // The tab indicator is measured from the boxes the tab bar got, so it is the one mark that
    // cannot be built before layout. It goes last: it sits on the line that closes the bar.
    if let Some(cmd) = view::tab_indicator(app, &dom, &info) {
        scene.cmds.push(cmd);
    }
    let clear = dom.nodes[0].style.background;
    let damage = metamind_ui::render_damaged(scene, fonts, pixmap, scale, clear, &mut app.damage);
    app.last_frame_us = t.elapsed().as_micros() as u64;
    Frame { dom, info, caret, damage }
}

/// The open pane of a drawn frame, without the tab bar and sidebars, at the pixmap's resolution.
pub fn pane_image(frame: &Frame, pixmap: &Pixmap, scale: f32) -> Result<Pixmap, String> {
    let has = |n: &metamind_ui::dom::Node, c: &str| n.el.classes.iter().any(|x| x == c);
    let nodes = &frame.dom.nodes;
    let root = nodes
        .iter()
        .position(|n| has(n, "workspace-leaf") && has(n, "mod-active"))
        .ok_or("no hay ningún panel abierto")?;
    let r = nodes[root..].iter().find(|n| has(n, "view-content")).ok_or("no hay ningún panel abierto")?.rect;
    let (x0, y0) = ((r.x * scale).round().max(0.0), (r.y * scale).round().max(0.0));
    let x1 = ((r.x + r.w) * scale).round().min(pixmap.width() as f32);
    let y1 = ((r.y + r.h) * scale).round().min(pixmap.height() as f32);
    metamind_ui::IntRect::from_ltrb(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
        .and_then(|rect| pixmap.clone_rect(rect))
        .ok_or_else(|| "el panel no está visible".to_string())
}

impl App {
    /// Finish Copy screenshot with the frame just drawn.
    pub fn copy_screenshot(&mut self, frame: &Frame, pixmap: &Pixmap, scale: f32) {
        self.screenshot = false;
        let copied = pane_image(frame, pixmap, scale).and_then(|pm| {
            if self.system_clipboard {
                clipboard::set_image(&pm)
            } else {
                Err("el portapapeles del sistema está desactivado".to_string())
            }
        });
        self.notice = Some(match copied {
            Ok(()) => "Captura de pantalla copiada".to_string(),
            Err(e) => format!("No se pudo copiar la captura de pantalla: {e}"),
        });
    }

    /// Left click at window coordinates. Two presses in the same spot within [`DOUBLE_CLICK`]
    /// are a double click, which is what swaps a diagram between its drawing and its source.
    pub fn click(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32, shift: bool) {
        // A board shown full screen is a modal of its own: a click outside its box leaves it,
        // and a click inside belongs to the board.
        if self.exc_fullscreen && self.exc_active() && !self.exc_rect(frame).is_some_and(|r| r.contains(x, y))
        {
            self.exc_fullscreen = false;
            self.last_click = None;
            return;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, (px, py))| {
            !self.clicked_dragged
                && now.saturating_duration_since(t) <= DOUBLE_CLICK
                && (x - px).abs() <= 4.0
                && (y - py).abs() <= 4.0
        });
        // The third press starts over: a double click is consumed by what it opened.
        self.last_click = if double { None } else { Some((now, (x, y))) };
        self.clicked_dragged = false;
        if double && self.double_click(frame, x, y) {
            return;
        }
        // A click outside the player's box gives the note its video back, the way the diagram
        // overlay closes on a click on its backdrop. The card keeps its own clicks: pressing it
        // plays or pauses, pressing a control does what the control says.
        if self.leave_video_overlay(frame, x, y) {
            return;
        }
        if self.menu_click(frame, x, y) || !self.press_pane(frame, x, y) || !self.press_stack(frame, x, y) {
            return;
        }
        if let Some((_, a)) = frame.dom.action_at(x, y).filter(|(_, a)| a.starts_with("suggest:")) {
            let a = a.to_string();
            return self.run(&a);
        }
        self.suggest = None;
        self.hover_preview = None;
        if let Some((node, action)) = frame.dom.action_at(x, y) {
            let action = action.to_string();
            if action == "mermaid:zoom" {
                // The block's source is kept with the item that paints it; the full-screen view
                // re-renders the same diagram with the width of the window.
                let src = frame.info.rich.iter().find(|it| it.node == node).and_then(|it| match &it.kind {
                    crate::rich::Kind::Mermaid(src) => Some(src.clone()),
                    _ => None,
                });
                if let Some(src) = src {
                    self.open_diagram_zoom(src);
                }
                return;
            }
            if action == "mermaid:zoom-close" {
                self.close_diagram_zoom();
                return;
            }
            if action == "mermaid:drag" {
                // A press on the diagram itself grabs it: the overlay is dragged from here, and
                // the click never reaches the backdrop that would close it.
                self.diagram_drag = Some(((x, y), self.diagram_zoom_pan));
                return;
            }
            if action == "video:scrub" {
                self.video.scrub_press(frame.dom.nodes[node].rect, x);
                return;
            }
            if !action.starts_with("prop:") && !action.starts_with("settings:") {
                self.commit_prop_edit();
            }
            if !action.starts_with("graph:edit:") && !action.starts_with("graph:insert:") {
                self.graph_end_edit();
            }
            if action != "backlinks:edit-filter" {
                self.backlinks_editing = false;
            }
            if action != "ai:focus-input" {
                self.ai.editing = false;
            }
            if matches!(action.as_str(), "file-explorer:sort-menu" | "ai:profile-menu") {
                let r = frame.dom.nodes[node].rect;
                if action == "file-explorer:sort-menu" {
                    self.sort_menu(r.x, r.y + r.h);
                } else {
                    self.ai_profile_menu(r.x, r.y + r.h);
                }
                return;
            }
            if let Some(side) = action.strip_prefix("side-resize:") {
                let left = side == "left";
                let width = if left { self.left_w } else { self.right_w };
                self.side_drag = Some(shell::SideDrag { left, start_x: x, width });
                return;
            }
            if let Some((k, path)) = action.strip_prefix("pane-resize:").and_then(|a| a.split_once(':')) {
                let path = path.split('.').filter_map(|i| i.parse().ok()).collect();
                let parent = frame.dom.nodes[node].parent;
                if let (Ok(k), Some(p)) = (k.parse(), parent) {
                    let rect = frame.dom.nodes[p].rect;
                    self.pane_drag = Some(split::PaneDrag { path, k, rect });
                }
                return;
            }
            if let Some(key) = action.strip_prefix("graph:slider:") {
                return self.graph_slider_press(key, frame.dom.nodes[node].rect, x);
            }
            let el = &frame.dom.nodes[node].el;
            if el.classes.iter().any(|c| c == "nav-file-title" || c == "nav-folder-title") {
                let source = action.split_once(':').map_or("", |(_, p)| p).to_string();
                let start = (x, y);
                self.explorer_drag =
                    Some(shell::ExplorerDrag { source, action, start, moving: false, target: None });
                return;
            }
            return self.run(&action);
        }
        self.graph_end_edit();
        self.backlinks_editing = false;
        self.ai.editing = false;
        if self.prompt.is_some() {
            return self.run("prompt:dismiss");
        }
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            if rect.contains(x, y) {
                g.press(&rect, x, y);
            }
            return;
        }
        if self.canvas_click(frame, x, y) {
            return;
        }
        if self.kanban_click(frame, fonts, x, y) {
            return;
        }
        if self.exc_click(frame, x, y) {
            return;
        }
        if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
            self.focus = shell::Focus::Editor;
            if let Some(t) = self.tab_mut() {
                let anchor = if shift { t.editor.buf.sels[0].anchor } else { off };
                t.editor.buf.set_sels(vec![editor_core::Sel::new(anchor, off)]);
            }
        }
    }

    /// What a double click does, wherever it lands: a diagram swaps between its drawing and its
    /// source, and a board edits the text under the pointer. In the note the swap is the block's,
    /// in the full-screen view it is the view's, so the second press of a click that just opened
    /// the view belongs to the block it came from. Returns whether it was a double click of ours.
    pub fn double_click(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        if self.exc_active() && self.exc_double_click(frame, x, y) {
            return true;
        }
        if self.kanban_active() && self.kanban_double_click(frame, x, y) {
            return true;
        }
        let note = self.tabs.get(self.active).map(|t| t.path.clone()).unwrap_or_default();
        let fresh = self.diagram_zoom_since.is_some_and(|t| t.elapsed() <= DOUBLE_CLICK);
        // A diagram drawn in the note, or the source a previous double click left there.
        let mut note_block = frame
            .info
            .rich
            .iter()
            .find(|it| {
                matches!(it.kind, crate::rich::Kind::Mermaid(_))
                    && frame.dom.nodes[it.node].rect.contains(x, y)
            })
            .and_then(|it| frame.info.blocks.iter().find(|b| b.node == it.node))
            .map(|b| b.src_start);
        if note_block.is_none() {
            note_block = frame
                .info
                .blocks
                .iter()
                .find(|b| b.display.contains("```mermaid") && frame.dom.nodes[b.node].rect.contains(x, y))
                .map(|b| b.src_start);
        }
        // Inside a view that was already open, the double click is the view's own.
        if self.diagram_zoom.is_some() && !fresh {
            self.diagram_zoom_source = !self.diagram_zoom_source;
            return true;
        }
        let Some(start) = note_block else { return false };
        let key = (note, start);
        if !self.mermaid_source.remove(&key) {
            self.mermaid_source.insert(key);
        }
        // The block now shows its own source; the view it opened on the first press goes away.
        if self.diagram_zoom.is_some() {
            self.close_diagram_zoom();
        }
        true
    }

    /// Open the full-screen view of a diagram, from the note block that holds it.
    pub fn open_diagram_zoom(&mut self, src: String) {
        self.diagram_zoom = Some(src);
        self.diagram_zoom_pan = (0.0, 0.0);
        self.diagram_zoom_scale = 1.0;
        self.diagram_zoom_source = false;
        self.diagram_zoom_since = Some(Instant::now());
    }

    /// Close the full-screen diagram and forget how it was left.
    pub fn close_diagram_zoom(&mut self) {
        self.diagram_zoom = None;
        self.diagram_zoom_pan = (0.0, 0.0);
        self.diagram_zoom_scale = 1.0;
        self.diagram_zoom_source = false;
        self.diagram_zoom_since = None;
        self.diagram_drag = None;
    }

    /// Show the player over the window, or back in its note. The decoder is asked for a frame that
    /// fits the workspace, so the picture is as sharp as the box allows.
    pub fn set_video_fullscreen(&mut self, on: bool) {
        if !self.video.is_open() {
            self.video_fullscreen = false;
            return;
        }
        self.video_fullscreen = on;
        self.fit_video_overlay();
    }

    /// Ask the decoder for a frame that fits the box the overlay really has: the workspace of the
    /// last frame (the status bar is already out of it), less the air the card keeps around
    /// itself and its control row. A frame sized to the window instead is what used to push the
    /// controls past the bottom edge of the modal.
    pub fn fit_video_overlay(&mut self) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let cap = if self.video_fullscreen {
            let area = if self.video_area.0 > 0.0 && self.video_area.1 > 0.0 {
                self.video_area
            } else {
                // The first frame of the overlay has no layout of its own yet.
                (self.window_w.max(240.0), self.window_h.max(160.0))
            };
            crate::video::overlay_cap(area, scale)
        } else {
            ((640.0 * scale) as u32, (360.0 * scale) as u32)
        };
        self.video.set_cap(cap);
    }

    /// A press in a pane other than the focused one focuses it first. Returns whether the press
    /// should go on: only tab bar actions do, as the rest of that pane was laid out unfocused.
    pub(crate) fn press_pane(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let hit = frame.info.panes.iter().find(|(_, leaf, _)| frame.dom.nodes[*leaf].rect.contains(x, y));
        let Some(&(p, ..)) = hit.filter(|(p, ..)| *p != self.pane) else { return true };
        self.focus_pane(p);
        frame.dom.action_at(x, y).is_some_and(|(_, a)| {
            ["tab:", "close-tab:", "toggle-pin:"].iter().any(|t| a.starts_with(t))
                || a == "file-explorer:new-file"
        })
    }

    /// A press on a stacked tab other than the active one makes it active first. Returns whether
    /// the press should go on, as for `press_pane`.
    pub(crate) fn press_stack(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let hit = frame.info.stack.iter().find(|e| frame.dom.nodes[e.2].rect.contains(x, y));
        let Some(&(_, tab, ..)) = hit.filter(|e| e.0 == self.pane && e.1 != self.active) else { return true };
        self.run(&format!("tab:{tab}"));
        frame
            .dom
            .action_at(x, y)
            .is_some_and(|(_, a)| ["close-tab:", "toggle-pin:"].iter().any(|t| a.starts_with(t)))
    }

    /// Extend the selection while dragging with the button held.
    pub fn drag(&mut self, frame: &Frame, fonts: &mut Fonts, x: f32, y: f32) {
        self.clicked_dragged = true;
        if let Some(((sx, sy), (px, py))) = self.diagram_drag {
            // The diagram follows the pointer from where it was grabbed.
            self.diagram_zoom_pan = (px + (x - sx), py + (y - sy));
            return;
        }
        if self.pane_drag.is_some() {
            return self.pane_drag_to(x, y);
        }
        if self.side_drag.is_some() {
            return self.side_drag_to(x);
        }
        if self.explorer_drag.is_some() {
            return self.explorer_drag_to(frame, x, y);
        }
        if self.video.scrub_drag(x) {
            return;
        }
        if self.graph_slider_drag(x) {
            return;
        }
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            return g.drag(&rect, x, y);
        }
        if self.canvas_drag(frame, x, y) {
            return;
        }
        if self.kanban_drag(frame, x, y) {
            return;
        }
        if self.exc_drag(frame, x, y) {
            return;
        }
        if let Some(off) = view::offset_at(&frame.dom, &frame.info, fonts, x, y) {
            if let Some(t) = self.tab_mut() {
                let anchor = t.editor.buf.sels[0].anchor;
                t.editor.buf.set_sels(vec![editor_core::Sel::new(anchor, off)]);
            }
        }
    }

    /// Mouse wheel. `lines` is positive when scrolling down.
    pub fn wheel(&mut self, frame: &Frame, x: f32, y: f32, lines: f32) {
        self.wheel_mod(frame, x, y, lines, false);
    }

    /// Mouse wheel with Ctrl (or Cmd) held: on a board this zooms about the pointer instead of
    /// scrolling it.
    pub fn ctrl_wheel(&mut self, frame: &Frame, x: f32, y: f32, lines: f32) {
        self.wheel_mod(frame, x, y, lines, true);
    }

    fn wheel_mod(&mut self, frame: &Frame, x: f32, y: f32, lines: f32, ctrl: bool) {
        self.menu = None;
        self.hover_preview = None;
        if self.exc_wheel(frame, x, y, lines, ctrl) {
            return;
        }
        if self.kanban_wheel(frame, x, y, lines, ctrl) {
            return;
        }
        if self.diagram_zoom.is_some() {
            // Wheel over the full-screen diagram zooms it, around its centre.
            let step = (-lines / 6.0).exp();
            self.diagram_zoom_scale =
                (self.diagram_zoom_scale * step).clamp(MIN_DIAGRAM_ZOOM, MAX_DIAGRAM_ZOOM);
            return;
        }
        if let (Some(rect), Some(g)) = (graph_rect(frame), &mut self.graph_view) {
            if rect.contains(x, y) && !in_graph_panel(frame, x, y) {
                return g.wheel(&rect, x, y, lines / 3.0);
            }
        }
        if self.canvas_wheel(frame, x, y, lines) {
            return;
        }
        let Some(node) = frame.dom.scroller_at(x, y) else { return };
        if Some(node) == frame.info.editor {
            return self.scroll_editor(lines);
        }
        if let Some(&(p, ..)) = frame.info.panes.iter().find(|e| e.2 == Some(node)) {
            return self.scroll_pane(p, lines);
        }
        if let Some(&(p, tab, ..)) = frame.info.stack.iter().find(|e| e.3 == Some(node)) {
            return self.scroll_tab(p, tab, lines);
        }
        if let Some(id) = frame.dom.nodes[node].el.id.clone() {
            let cur = frame.dom.nodes[node].scroll_y;
            self.scrolls.insert(id, (cur + lines * 40.0).clamp(0.0, frame.dom.max_scroll(node)));
        }
    }

    /// Middle click: closes the tab under the pointer, or grabs a board.
    pub fn middle_click(&mut self, frame: &Frame, x: f32, y: f32) {
        self.menu = None;
        if self.exc_active() {
            self.exc_middle_press(frame, x, y);
            return;
        }
        if !self.press_pane(frame, x, y) {
            return;
        }
        let Some((_, action)) = frame.dom.action_at(x, y) else { return };
        match action {
            "graph:focus" => self.run("graph:close"),
            "canvas:focus" => self.run("canvas:close"),
            "base:focus" => self.run("base:close"),
            a => {
                if let Some(i) = a.strip_prefix("tab:").or_else(|| a.strip_prefix("close-tab:")) {
                    let cmd = format!("close-tab:{i}");
                    self.run(&cmd);
                }
            }
        }
    }

    /// The left button was released.
    pub fn release(&mut self) {
        if self.diagram_drag.take().is_some() {
            return;
        }
        if self.video.scrub_release() {
            return;
        }
        if self.pane_drag.take().is_some() {
            return;
        }
        if self.side_drag.take().is_some() {
            return;
        }
        if let Some(d) = self.explorer_drag.take() {
            match (d.moving, d.target) {
                (false, _) => self.run(&d.action),
                (true, Some(folder)) => self.move_into(&d.source, &folder),
                (true, None) => {}
            }
            return;
        }
        if self.graph_slider_release() {
            return;
        }
        if let Some(kind) = self.graph_view.as_mut().and_then(graphview::GraphView::release) {
            self.graph_clicked(kind);
        }
        self.canvas_release();
        self.exc_release();
        self.kanban_release();
    }

    /// The pointer moved with no button held. Returns true when the frame must be redrawn.
    pub fn hover(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        let action = frame.dom.action_at(x, y).map(|(_, a)| a.to_string());
        let changed = action != self.hover_action;
        self.hover_action = action;
        let graph = match (graph_rect(frame), &mut self.graph_view) {
            (Some(rect), Some(g)) => {
                // Over the settings panel no node is hovered.
                let x = if in_graph_panel(frame, x, y) { f32::NEG_INFINITY } else { x };
                g.hover_at(&rect, x, y)
            }
            _ => false,
        };
        let kanban = self.kanban_hover(x, y);
        graph || kanban || changed
    }
}

impl App {
    /// Track a sidebar edge drag: the sidebar grows or shrinks by how far the pointer moved from
    /// where it was grabbed, within the limits a sidebar has.
    pub(crate) fn side_drag_to(&mut self, x: f32) {
        let Some(d) = self.side_drag else { return };
        let delta = x - d.start_x;
        let w = if d.left { d.width + delta } else { d.width - delta };
        let w = w.clamp(shell::SIDE_W_MIN, shell::SIDE_W_MAX);
        if d.left {
            self.left_w = w;
        } else {
            self.right_w = w;
        }
    }

    /// Close the over-window player when a press fell outside its card, the way a press on the
    /// diagram overlay's backdrop closes that one. Returns whether it closed.
    fn leave_video_overlay(&mut self, frame: &Frame, x: f32, y: f32) -> bool {
        if !self.video_fullscreen || player_at(frame, x, y) {
            return false;
        }
        self.set_video_fullscreen(false);
        true
    }

    /// Track an explorer drag: past a few pixels it becomes a move, aimed at the folder
    /// under the pointer, the folder of the file under it, or the root on empty space.
    fn explorer_drag_to(&mut self, frame: &Frame, x: f32, y: f32) {
        let Some(d) = &mut self.explorer_drag else { return };
        if !d.moving && (x - d.start.0).abs().max((y - d.start.1).abs()) < 4.0 {
            return;
        }
        d.moving = true;
        let inside = frame.dom.by_id("files").is_some_and(|n| frame.dom.nodes[n].rect.contains(x, y));
        let folder = inside.then(|| match frame.dom.action_at(x, y) {
            Some((_, a)) if a.starts_with("toggle-folder:") => a["toggle-folder:".len()..].to_string(),
            Some((_, a)) if a.starts_with("open:") => {
                vault_core::path::parent(&a["open:".len()..]).to_string()
            }
            _ => String::new(),
        });
        let src = d.source.as_str();
        let nested = |f: &str| f == src || f.starts_with(&format!("{src}/"));
        d.target = folder.filter(|f| !nested(f) && f != vault_core::path::parent(src));
    }
}

fn in_graph_panel(frame: &Frame, x: f32, y: f32) -> bool {
    frame.info.graph_panel.is_some_and(|n| frame.dom.nodes[n].rect.contains(x, y))
}

/// Whether the point is inside the player card of the frame. While the overlay is up the note
/// draws no player of its own, so the card found here is always the one over the window.
fn player_at(frame: &Frame, x: f32, y: f32) -> bool {
    frame.dom.nodes.iter().any(|n| n.el.classes.iter().any(|c| c == "video-player") && n.rect.contains(x, y))
}

/// The graph pane of a frame, when the graph is showing.
fn graph_rect(frame: &Frame) -> Option<metamind_ui::Rect> {
    frame.info.graph.map(|n| frame.dom.nodes[n].rect)
}
