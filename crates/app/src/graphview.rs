//! Graph view: the note graph drawn in the main pane. The force layout runs on a worker thread
//! that sleeps once the simulation has cooled; the UI thread only copies positions.

use crate::shell::App;
use graph_core::{Forces, Graph, GraphSettings, Layout, Local, NodeKind, Sets};
use rustsidian_ui::{Cmd, Rect, TextStyle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use theme_engine::Color;

const MAX_LABELS: usize = 300;
const HIT_SLOP: f32 = 4.0;
/// Width the settings panel takes from the right of the pane, with its margins.
const PANEL_W: f32 = 270.0;
/// Line pixels drawn per frame before links are sampled.
const LINE_PIXEL_BUDGET: f32 = 3.0e6;

#[derive(Default)]
struct SimState {
    pos: Vec<[f32; 2]>,
    /// Nodes held at a position by the pointer.
    holds: Vec<(u32, [f32; 2])>,
    releases: Vec<u32>,
    reheat: bool,
    /// Force sliders changed: apply them and warm the layout gently.
    forces: Option<Forces>,
    /// A frame was shown; the layout may advance one tick, so it moves at display pace.
    tick: bool,
    /// Tick without waiting for frames (headless rendering and tests).
    free_run: bool,
    stop: bool,
}

struct Shared {
    state: Mutex<SimState>,
    wake: Condvar,
    running: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, SimState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand work to the layout thread. It counts as running until it has looked, so the
    /// window keeps supplying frames.
    fn send(&self, f: impl FnOnce(&mut SimState)) {
        f(&mut self.lock());
        self.running.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}

fn simulate(shared: &Shared, graph: &Graph, mut layout: Layout, mut forces: Forces) {
    loop {
        {
            let mut st = shared.lock();
            loop {
                if st.stop {
                    shared.running.store(false, Ordering::Release);
                    return;
                }
                if let Some(f) = st.forces.take() {
                    forces = f;
                    layout.alpha = layout.alpha.max(0.3);
                }
                let mut touched = std::mem::take(&mut st.reheat);
                for (i, p) in st.holds.drain(..) {
                    if let Some(slot) = layout.pos.get_mut(i as usize) {
                        *slot = p;
                        layout.pinned[i as usize] = true;
                        touched = true;
                    }
                }
                for i in st.releases.drain(..) {
                    if let Some(pin) = layout.pinned.get_mut(i as usize) {
                        *pin = false;
                        touched = true;
                    }
                }
                if touched {
                    layout.reheat();
                }
                let settled = layout.is_settled();
                if !settled && (std::mem::take(&mut st.tick) || st.free_run) {
                    break;
                }
                if settled {
                    st.pos.clone_from(&layout.pos);
                }
                shared.running.store(!settled, Ordering::Release);
                st = shared.wake.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        }
        layout.step(graph, &forces);
        shared.lock().pos.clone_from(&layout.pos);
    }
}

struct Press {
    node: Option<u32>,
    at: (f32, f32),
    moved: bool,
}

/// Colors read from the theme (`.graph-view.color-*` rules), in the order of `COLOR_CLASSES`.
pub const COLOR_CLASSES: [&str; 9] = [
    "color-fill",
    "color-fill-focused",
    "color-fill-tag",
    "color-fill-attachment",
    "color-fill-unresolved",
    "color-fill-highlight",
    "color-line",
    "color-line-highlight",
    "color-text",
];

/// Collapsible sections of the settings panel: (`graph.json` key suffix, title).
pub const SECTIONS: [(&str, &str); 4] =
    [("filter", "Filtros"), ("color-groups", "Grupos"), ("display", "Pantalla"), ("forces", "Fuerzas")];

pub struct Slider {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

const fn slider(key: &'static str, label: &'static str, min: f32, max: f32, step: f32) -> Slider {
    Slider { key, label, min, max, step }
}

pub const DEPTH: Slider = slider("depth", "Profundidad", 1.0, 3.0, 1.0);
pub const DISPLAY_SLIDERS: [Slider; 3] = [
    slider("text-fade", "Umbral de desvanecimiento del texto", -3.0, 3.0, 0.01),
    slider("node-size", "Tamaño de nodo", 0.1, 5.0, 0.01),
    slider("line-size", "Grosor de enlaces", 0.1, 5.0, 0.01),
];
pub const FORCE_SLIDERS: [Slider; 4] = [
    slider("center", "Fuerza central", 0.0, 1.0, 0.01),
    slider("repel", "Fuerza de repulsión", 0.0, 20.0, 0.01),
    slider("link", "Fuerza de enlaces", 0.0, 1.0, 0.01),
    slider("distance", "Distancia entre enlaces", 30.0, 500.0, 1.0),
];

/// Colors given to new groups, in turn.
pub const GROUP_COLORS: [u32; 8] =
    [0xe93147, 0xe9973f, 0xe0de71, 0x44cf6e, 0x53dfdd, 0x027aff, 0xa882ff, 0xfa99cd];

/// State of the settings panel over the graph.
#[derive(Default)]
pub struct Panel {
    pub open: bool,
    /// Indexed like `SECTIONS`.
    pub collapsed: [bool; 4],
    /// Text field being edited (`search` or `group:<i>`) and its text.
    pub edit: Option<(String, String)>,
    /// Slider being dragged and its track's x and width in window coordinates.
    slider: Option<(&'static str, f32, f32)>,
}

impl Panel {
    fn from_json(v: &serde_json::Value) -> Panel {
        let flag = |k: &str| v.get(k).and_then(serde_json::Value::as_bool).unwrap_or(false);
        Panel {
            open: !flag("close"),
            collapsed: SECTIONS.map(|(id, _)| flag(&format!("collapse-{id}"))),
            ..Panel::default()
        }
    }

    fn write_into(&self, v: &mut serde_json::Value) {
        let Some(o) = v.as_object_mut() else { return };
        o.insert("close".into(), (!self.open).into());
        for ((id, _), c) in SECTIONS.iter().zip(self.collapsed) {
            o.insert(format!("collapse-{id}"), c.into());
        }
    }
}

pub struct GraphView {
    pub graph: Arc<Graph>,
    pub settings: GraphSettings,
    pub panel: Panel,
    /// Centre note of a local graph; `None` for the whole vault.
    pub local: Option<String>,
    pub depth: u32,
    center: Option<vault_core::FileId>,
    pub pos: Vec<[f32; 2]>,
    /// World point at the centre of the pane, and screen pixels per world unit.
    pub cam: [f32; 2],
    pub zoom: f32,
    /// Until the user pans or zooms, the view follows the bounding box of the graph.
    pub auto_fit: bool,
    pub hover: Option<u32>,
    /// The graph must be rebuilt before the next frame (vault or settings changed).
    pub dirty: bool,
    press: Option<Press>,
    /// Graph centroid the camera follows after a force change.
    anchor: Option<[f32; 2]>,
    shared: Arc<Shared>,
}

impl Drop for GraphView {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
    }
}

fn start(graph: &Arc<Graph>, layout: Layout, forces: Forces) -> Arc<Shared> {
    let shared = Arc::new(Shared {
        state: Mutex::new(SimState { pos: layout.pos.clone(), ..SimState::default() }),
        wake: Condvar::new(),
        running: AtomicBool::new(true),
    });
    let (s, g) = (shared.clone(), graph.clone());
    let spawned = std::thread::Builder::new()
        .name("graph-layout".into())
        .spawn(move || simulate(&s, &g, layout, forces));
    if spawned.is_err() {
        shared.running.store(false, Ordering::Release);
    }
    shared
}

fn centroid(pos: &[[f32; 2]]) -> [f32; 2] {
    let n = pos.len().max(1) as f32;
    let s = pos.iter().fold([0.0; 2], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

fn build_graph(app: &mut App, settings: &GraphSettings, local: Option<&str>, depth: u32) -> Graph {
    let root = app.store.root.clone();
    let vault = &mut app.store.index;
    let query = |q: &str| graph_core::query_set(vault, None, &root, q);
    let sets = Sets {
        filter: if settings.search.trim().is_empty() { None } else { query(&settings.search) },
        groups: settings.color_groups.iter().map(|g| query(&g.query).unwrap_or_default()).collect(),
    };
    let local = local.and_then(|p| vault.id(p)).map(|center| Local {
        center,
        depth,
        backlinks: true,
        forelinks: true,
        interlinks: true,
    });
    let g = graph_core::build(vault, &settings.filters, &sets, local.as_ref());
    app.store.release_cache_pages();
    g
}

impl GraphView {
    pub fn open(app: &mut App, local: Option<String>) -> GraphView {
        let settings = GraphSettings::from_json(&app.config.graph.value);
        let graph = Arc::new(build_graph(app, &settings, local.as_deref(), 1));
        let layout = Layout::new(graph.nodes.len());
        let center = local.as_deref().and_then(|p| app.store.index.id(p));
        GraphView {
            pos: layout.pos.clone(),
            shared: start(&graph, layout, settings.forces),
            graph,
            settings,
            local,
            depth: 1,
            center,
            cam: [0.0; 2],
            zoom: 1.0,
            auto_fit: true,
            hover: None,
            dirty: false,
            press: None,
            anchor: None,
            panel: Panel::from_json(&app.config.graph.value),
        }
    }

    /// Restart the layout from the starting spiral (the Animate button).
    pub fn restart(&mut self) {
        let layout = Layout::new(self.graph.nodes.len());
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        self.pos.clone_from(&layout.pos);
        self.shared = start(&self.graph, layout, self.settings.forces);
        self.press = None;
    }

    pub fn slider_value(&self, key: &str) -> f32 {
        let s = &self.settings;
        match key {
            "depth" => self.depth as f32,
            "text-fade" => s.text_fade,
            "node-size" => s.node_size,
            "line-size" => s.line_size,
            "center" => s.forces.center_strength,
            "repel" => s.forces.repel_strength,
            "link" => s.forces.link_strength,
            "distance" => s.forces.link_distance,
            _ => 0.0,
        }
    }

    fn set_slider(&mut self, key: &str, v: f32) {
        if (self.slider_value(key) - v).abs() < f32::EPSILON {
            return;
        }
        let s = &mut self.settings;
        match key {
            "depth" => {
                self.depth = v as u32;
                self.dirty = true;
                return;
            }
            "text-fade" => s.text_fade = v,
            "node-size" => s.node_size = v,
            "line-size" => s.line_size = v,
            "center" => s.forces.center_strength = v,
            "repel" => s.forces.repel_strength = v,
            "link" => s.forces.link_strength = v,
            "distance" => s.forces.link_distance = v,
            _ => return,
        }
        if FORCE_SLIDERS.iter().any(|f| f.key == key) {
            // Freeze the framing so the graph does not jump while the forces move it.
            self.auto_fit = false;
            self.sync();
            self.anchor = Some(centroid(&self.pos));
            let forces = self.settings.forces;
            self.shared.send(|st| st.forces = Some(forces));
        }
    }

    /// Set the dragged slider from the pointer's x.
    fn slide_to(&mut self, x: f32) {
        let Some((key, x0, w)) = self.panel.slider else { return };
        let Some(sl) =
            std::iter::once(&DEPTH).chain(&DISPLAY_SLIDERS).chain(&FORCE_SLIDERS).find(|s| s.key == key)
        else {
            return;
        };
        // The thumb is 16 px wide, so its centre travels 8 px inside each end of the track.
        let t = ((x - x0 - 8.0) / (w - 16.0).max(1.0)).clamp(0.0, 1.0);
        let v = sl.min + (sl.max - sl.min) * t;
        self.set_slider(key, ((v / sl.step).round() * sl.step).clamp(sl.min, sl.max));
    }

    /// Rebuild after the vault or the settings changed, keeping the positions of nodes that
    /// are still there.
    pub fn rebuild(&mut self, app: &mut App) {
        self.sync();
        let old: HashMap<&NodeKind, [f32; 2]> =
            self.graph.nodes.iter().map(|n| &n.kind).zip(self.pos.iter().copied()).collect();
        let graph = build_graph(app, &self.settings, self.local.as_deref(), self.depth);
        let known: Vec<(usize, [f32; 2])> =
            graph.nodes.iter().enumerate().filter_map(|(i, n)| old.get(&n.kind).map(|p| (i, *p))).collect();
        let layout = Layout::with_positions(graph.nodes.len(), known.into_iter());
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        self.graph = Arc::new(graph);
        self.pos.clone_from(&layout.pos);
        self.shared = start(&self.graph, layout, self.settings.forces);
        self.hover = None;
        self.press = None;
        self.dirty = false;
    }

    /// Whether the layout is still moving (the window keeps redrawing while it is).
    pub fn animating(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    /// Copy the latest positions from the layout thread.
    pub fn sync(&mut self) {
        let st = self.shared.lock();
        if st.pos.len() == self.graph.nodes.len() {
            self.pos.clone_from(&st.pos);
        }
    }

    /// Block until the layout has settled (headless rendering and tests).
    pub fn settle(&mut self) {
        self.shared.send(|st| st.free_run = true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while self.animating() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.sync();
    }

    pub fn title(&self) -> String {
        match &self.local {
            Some(p) => format!("Gráfico de {}", vault_core::path::stem(p)),
            None => "Vista de gráfico".to_string(),
        }
    }

    fn radius(&self, i: usize) -> f32 {
        let deg = self.graph.nodes[i].degree as f32;
        (3.0 + 1.3 * deg.sqrt().min(12.0)) * self.settings.node_size * self.zoom.clamp(0.04, 3.0).sqrt()
    }

    fn fit(&mut self, rect: &Rect) {
        if !self.auto_fit || self.pos.is_empty() {
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &self.pos {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // Keep the graph clear of the settings panel when the pane is wide enough for both.
        let panel = if self.panel.open && rect.w - PANEL_W >= 300.0 { PANEL_W } else { 0.0 };
        let free = rect.w - panel;
        let (w, h) = ((hi[0] - lo[0]).max(1.0), (hi[1] - lo[1]).max(1.0));
        self.zoom = ((free - 80.0).max(40.0) / w).min((rect.h - 80.0).max(40.0) / h).clamp(0.005, 1.5);
        self.cam = [(lo[0] + hi[0]) / 2.0 + panel / 2.0 / self.zoom, (lo[1] + hi[1]) / 2.0];
    }

    /// Where node `i` is drawn in a pane.
    pub fn node_screen(&self, rect: &Rect, i: usize) -> Option<(f32, f32)> {
        self.pos.get(i).map(|p| self.to_screen(rect, *p))
    }

    fn to_screen(&self, rect: &Rect, p: [f32; 2]) -> (f32, f32) {
        (
            rect.x + rect.w / 2.0 + (p[0] - self.cam[0]) * self.zoom,
            rect.y + rect.h / 2.0 + (p[1] - self.cam[1]) * self.zoom,
        )
    }

    fn to_world(&self, rect: &Rect, x: f32, y: f32) -> [f32; 2] {
        [
            (x - rect.x - rect.w / 2.0) / self.zoom + self.cam[0],
            (y - rect.y - rect.h / 2.0) / self.zoom + self.cam[1],
        ]
    }

    /// Topmost node under a point.
    pub fn node_at(&self, rect: &Rect, x: f32, y: f32) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        for (i, p) in self.pos.iter().enumerate() {
            let (sx, sy) = self.to_screen(rect, *p);
            let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
            if d <= self.radius(i) + HIT_SLOP && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i as u32, d));
            }
        }
        best.map(|(i, _)| i)
    }

    pub fn press(&mut self, rect: &Rect, x: f32, y: f32) {
        self.press = Some(Press { node: self.node_at(rect, x, y), at: (x, y), moved: false });
    }

    pub fn drag(&mut self, rect: &Rect, x: f32, y: f32) {
        let Some(press) = &mut self.press else { return };
        let (dx, dy) = (x - press.at.0, y - press.at.1);
        if !press.moved && dx.abs() + dy.abs() < 4.0 {
            return;
        }
        press.moved = true;
        match press.node {
            Some(n) => {
                let w = self.to_world(rect, x, y);
                if let Some(p) = self.pos.get_mut(n as usize) {
                    *p = w;
                }
                self.shared.send(|st| st.holds.push((n, w)));
            }
            None => {
                press.at = (x, y);
                self.auto_fit = false;
                self.cam[0] -= dx / self.zoom;
                self.cam[1] -= dy / self.zoom;
            }
        }
    }

    /// End a press. Returns the node that was clicked (pressed and released without dragging).
    pub fn release(&mut self) -> Option<NodeKind> {
        let press = self.press.take()?;
        let n = press.node?;
        if press.moved {
            self.shared.send(|st| st.releases.push(n));
            return None;
        }
        self.graph.nodes.get(n as usize).map(|n| n.kind.clone())
    }

    /// Zoom about a point. `lines` is positive when scrolling down (zoom out).
    pub fn wheel(&mut self, rect: &Rect, x: f32, y: f32, lines: f32) {
        let before = self.to_world(rect, x, y);
        self.auto_fit = false;
        self.zoom = (self.zoom * 0.9f32.powf(lines)).clamp(0.005, 8.0);
        let after = self.to_world(rect, x, y);
        self.cam[0] += before[0] - after[0];
        self.cam[1] += before[1] - after[1];
    }

    /// Update the hovered node. Returns true when it changed.
    pub fn hover_at(&mut self, rect: &Rect, x: f32, y: f32) -> bool {
        let h = if rect.contains(x, y) { self.node_at(rect, x, y) } else { None };
        std::mem::replace(&mut self.hover, h) != h
    }

    /// Display list for the pane, in window coordinates.
    pub fn paint(&mut self, rect: Rect, colors: &[Color; 9], text: &TextStyle) -> Vec<Cmd> {
        self.sync();
        if self.animating() {
            self.shared.lock().tick = true;
            self.shared.wake.notify_all();
        }
        // While a force change settles, follow the graph's centre so it stays where it was.
        if let Some(a) = self.anchor {
            let c = centroid(&self.pos);
            self.cam[0] += c[0] - a[0];
            self.cam[1] += c[1] - a[1];
            self.anchor = self.animating().then_some(c);
        }
        self.fit(&rect);
        let g = self.graph.clone();
        let n = g.nodes.len().min(self.pos.len());
        let screen: Vec<(f32, f32)> = self.pos[..n].iter().map(|p| self.to_screen(&rect, *p)).collect();
        let focus = self.hover.or(self.press.as_ref().and_then(|p| p.node)).filter(|&h| (h as usize) < n);
        let mut near = vec![false; if focus.is_some() { n } else { 0 }];
        if let Some(h) = focus {
            near[h as usize] = true;
        }
        let margin = 40.0;
        let view = Rect::new(rect.x - margin, rect.y - margin, rect.w + margin * 2.0, rect.h + margin * 2.0);
        let (mut lines, mut hot) = (Vec::new(), Vec::new());
        let arrow = self.settings.show_arrows && self.zoom > 0.25;
        let long = |&(a, b): &(u32, u32)| match (screen.get(a as usize), screen.get(b as usize)) {
            (Some(p), Some(q)) => (p.0 - q.0).abs().max((p.1 - q.1).abs()).min(rect.w.max(rect.h)),
            _ => 0.0,
        };
        let sample = if g.edges.len() > 4096 { 16 } else { 1 };
        let pixels = g.edges.iter().step_by(sample).map(long).sum::<f32>() * sample as f32;
        let stride = (pixels / LINE_PIXEL_BUDGET).ceil().max(1.0) as usize;
        let mut drawn = 0usize;
        // Without a focused node no link is special, so only the sampled ones are visited.
        let step = if focus.is_some() { 1 } else { stride };
        for &(a, b) in g.edges.iter().step_by(step) {
            let (a, b) = (a as usize, b as usize);
            if a >= n || b >= n {
                continue;
            }
            let (p, q) = (screen[a], screen[b]);
            let is_hot = focus.is_some_and(|h| h as usize == a || h as usize == b);
            if is_hot {
                near[a] = true;
                near[b] = true;
            }
            if (p.0 < view.x && q.0 < view.x)
                || (p.1 < view.y && q.1 < view.y)
                || (p.0 > view.x + view.w && q.0 > view.x + view.w)
                || (p.1 > view.y + view.h && q.1 > view.y + view.h)
            {
                continue;
            }
            let out = if is_hot {
                &mut hot
            } else {
                // Dense graphs draw an even sample of their links: past the pixel budget the
                // links are a uniform haze, and drawing all of them costs whole frames.
                drawn += 1;
                if step == 1 && !drawn.is_multiple_of(stride) {
                    continue;
                }
                &mut lines
            };
            out.push([p.0, p.1, q.0, q.1]);
            if arrow {
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                let len = (dx * dx + dy * dy).sqrt();
                if len > 12.0 {
                    let (ux, uy) = (dx / len, dy / len);
                    let r = self.radius(b) + 1.0;
                    let (tx, ty) = (q.0 - ux * r, q.1 - uy * r);
                    let s = 5.0;
                    out.push([tx, ty, tx - (ux + uy * 0.5) * s, ty - (uy - ux * 0.5) * s]);
                    out.push([tx, ty, tx - (ux - uy * 0.5) * s, ty - (uy + ux * 0.5) * s]);
                }
            }
        }
        let dim = |c: Color| if focus.is_some() { c.with_alpha(0.25) } else { c };
        let width = self.settings.line_size.clamp(0.1, 5.0);
        let mut cmds = vec![Cmd::PushClip(rect)];
        cmds.push(Cmd::Lines { segs: lines, width, color: dim(colors[6]) });
        if !hot.is_empty() {
            cmds.push(Cmd::Lines { segs: hot, width: width * 1.5, color: colors[7] });
        }
        // Nodes, batched by color.
        let center = self.center.and_then(|id| g.nodes.iter().position(|nd| nd.kind == NodeKind::Note(id)));
        let mut batches: Vec<(Color, Vec<[f32; 3]>)> = Vec::new();
        let mut visible = Vec::new();
        for (i, node) in g.nodes[..n].iter().enumerate() {
            let (x, y) = screen[i];
            if !view.contains(x, y) {
                continue;
            }
            visible.push(i);
            let base = if focus == Some(i as u32) {
                colors[5]
            } else if center == Some(i) {
                colors[1]
            } else if let Some(cg) = node.group.and_then(|k| self.settings.color_groups.get(k as usize)) {
                Color { r: (cg.rgb >> 16) as u8, g: (cg.rgb >> 8) as u8, b: cg.rgb as u8, a: 255 }
                    .with_alpha(cg.alpha)
            } else {
                match node.kind {
                    NodeKind::Note(_) => colors[0],
                    NodeKind::Tag(_) => colors[2],
                    NodeKind::Attachment(_) => colors[3],
                    NodeKind::Unresolved(_) => colors[4],
                }
            };
            let color = if near.get(i).copied().unwrap_or(true) { base } else { dim(base) };
            let item = [x, y, self.radius(i)];
            match batches.iter_mut().find(|(c, _)| *c == color) {
                Some((_, items)) => items.push(item),
                None => batches.push((color, vec![item])),
            }
        }
        cmds.extend(batches.into_iter().map(|(color, items)| Cmd::Circles { items, color }));
        // Labels fade in smoothly over a zoom range of four times around the threshold. While a
        // node is hovered its neighbours are labelled at full strength and the rest dimmed.
        let threshold = 0.5 * 2f32.powf(-self.settings.text_fade / 1.5);
        let t = ((self.zoom / threshold).log2() / 2.0 + 0.5).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        let alpha = |i: usize| match focus {
            Some(_) if near[i] => 1.0,
            Some(_) => fade * 0.2,
            None => fade,
        };
        // Labels are placed most important first: the hovered node and its neighbours, then by
        // link count. One that would overlap a placed label is left out, as in Obsidian.
        let mut labelled: Vec<usize> = visible.into_iter().filter(|&i| alpha(i) > 0.01).collect();
        labelled.sort_by_key(|&i| {
            (
                std::cmp::Reverse(focus == Some(i as u32)),
                std::cmp::Reverse(near.get(i).copied()),
                std::cmp::Reverse(g.nodes[i].degree),
            )
        });
        let mut placed: Vec<Rect> = Vec::new();
        for i in labelled {
            if placed.len() >= MAX_LABELS {
                break;
            }
            let size = if focus == Some(i as u32) { text.size * 0.9 } else { text.size };
            let label: String = g.nodes[i].label.chars().take(40).collect();
            let w = label.chars().count() as f32 * size * 0.52;
            let (x, y) = (screen[i].0 - w / 2.0, screen[i].1 + self.radius(i) + 3.0);
            let r = Rect::new(x - 2.0, y, w + 4.0, size * 1.25);
            if placed.iter().any(|p| overlaps(p, &r)) {
                continue;
            }
            placed.push(r);
            cmds.push(Cmd::Text {
                x,
                y,
                width: None,
                text: label,
                spans: Vec::new(),
                style: TextStyle { color: colors[8].with_alpha(alpha(i)), size, wrap: false, ..text.clone() },
            });
        }
        cmds.push(Cmd::PopClip);
        cmds
    }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

impl App {
    pub fn graph_animating(&self) -> bool {
        self.show_graph && self.graph_view.as_ref().is_some_and(GraphView::animating)
    }

    pub(crate) fn open_graph(&mut self, local: Option<String>) {
        let view = GraphView::open(self, local);
        self.graph_view = Some(view);
        self.show_graph = true;
    }

    /// Rebuild the graph if the vault or its settings changed since it was built.
    pub(crate) fn refresh_graph_view(&mut self) {
        if !self.show_graph || !self.graph_view.as_ref().is_some_and(|g| g.dirty) {
            return;
        }
        if let Some(mut view) = self.graph_view.take() {
            view.rebuild(self);
            self.graph_view = Some(view);
        }
    }

    /// Change a graph setting, save it to `graph.json` and rebuild.
    fn graph_setting(&mut self, change: impl FnOnce(&mut GraphView)) {
        let Some(view) = &mut self.graph_view else { return };
        change(view);
        view.dirty = true;
        self.save_graph();
    }

    fn save_graph(&mut self) {
        let Some(view) = &self.graph_view else { return };
        view.settings.write_into(&mut self.config.graph.value);
        view.panel.write_into(&mut self.config.graph.value);
        if let Err(e) = self.config.graph.save() {
            self.notice = Some(format!("No se pudo guardar graph.json: {e}"));
        }
    }

    /// A press on a slider track (`key`) at window x.
    pub(crate) fn graph_slider_press(&mut self, key: &str, track: Rect, x: f32) {
        let Some(v) = &mut self.graph_view else { return };
        let Some(sl) =
            std::iter::once(&DEPTH).chain(&DISPLAY_SLIDERS).chain(&FORCE_SLIDERS).find(|s| s.key == key)
        else {
            return;
        };
        v.panel.slider = Some((sl.key, track.x, track.w));
        v.slide_to(x);
    }

    /// Pointer moved with the button held. Returns true when a slider is being dragged.
    pub(crate) fn graph_slider_drag(&mut self, x: f32) -> bool {
        match &mut self.graph_view {
            Some(v) if v.panel.slider.is_some() => {
                v.slide_to(x);
                true
            }
            _ => false,
        }
    }

    /// Button released. Returns true when it ended a slider drag (the value is saved).
    pub(crate) fn graph_slider_release(&mut self) -> bool {
        let Some(v) = &mut self.graph_view else { return false };
        if v.panel.slider.take().is_none() {
            return false;
        }
        self.save_graph();
        true
    }

    /// Put the edited text of a panel field into the settings.
    fn graph_apply_edit(&mut self) {
        let Some(v) = &mut self.graph_view else { return };
        let Some((key, text)) = &v.panel.edit else { return };
        match key.strip_prefix("group:").and_then(|i| i.parse::<usize>().ok()) {
            Some(i) => {
                if let Some(g) = v.settings.color_groups.get_mut(i) {
                    g.query.clone_from(text);
                }
            }
            None => v.settings.search.clone_from(text),
        }
        v.dirty = true;
    }

    /// Finish editing a panel field and save.
    pub(crate) fn graph_end_edit(&mut self) {
        if self.graph_view.as_mut().and_then(|v| v.panel.edit.take()).is_some() {
            self.save_graph();
        }
    }

    pub(crate) fn graph_text(&mut self, text: &str) -> bool {
        let Some((_, buf)) = self.graph_view.as_mut().and_then(|v| v.panel.edit.as_mut()) else {
            return false;
        };
        buf.push_str(text);
        self.graph_apply_edit();
        true
    }

    pub(crate) fn graph_key(&mut self, key: &crate::shell::Key) -> Option<bool> {
        let (_, buf) = self.graph_view.as_mut()?.panel.edit.as_mut()?;
        match key.key.as_str() {
            "Backspace" if key.ctrl => buf.clear(),
            "Backspace" => {
                buf.pop();
            }
            "Enter" | "Escape" | "Tab" => {
                self.graph_end_edit();
                return Some(true);
            }
            _ if key.ctrl || key.alt => return None,
            _ => return Some(true),
        }
        self.graph_apply_edit();
        Some(true)
    }

    fn graph_panel_command(&mut self, command: &str) -> bool {
        let Some(v) = &mut self.graph_view else { return command.starts_with("graph:") };
        if let Some(id) = command.strip_prefix("graph:section:") {
            if let Some(i) = SECTIONS.iter().position(|(s, _)| *s == id) {
                v.panel.collapsed[i] ^= true;
                self.save_graph();
            }
        } else if let Some(key) = command.strip_prefix("graph:edit:") {
            let text = match key.strip_prefix("group:").and_then(|i| i.parse::<usize>().ok()) {
                Some(i) => v.settings.color_groups.get(i).map(|g| g.query.clone()),
                None => Some(v.settings.search.clone()),
            };
            v.panel.edit = text.map(|t| (key.to_string(), t));
        } else if let Some(op) = command.strip_prefix("graph:insert:") {
            if let Some((_, buf)) = &mut v.panel.edit {
                buf.push_str(op);
                self.graph_apply_edit();
            }
        } else if let Some(i) =
            command.strip_prefix("graph:group-color:").and_then(|i| i.parse::<usize>().ok())
        {
            if let Some(g) = v.settings.color_groups.get_mut(i) {
                let at = GROUP_COLORS.iter().position(|&c| c == g.rgb).map_or(0, |p| p + 1);
                g.rgb = GROUP_COLORS[at % GROUP_COLORS.len()];
                self.graph_setting(|_| {});
            }
        } else if let Some(i) =
            command.strip_prefix("graph:group-remove:").and_then(|i| i.parse::<usize>().ok())
        {
            if i < v.settings.color_groups.len() {
                v.panel.edit = None;
                self.graph_setting(|v| {
                    v.settings.color_groups.remove(i);
                });
            }
        } else if command.starts_with("graph:slider:") || command == "graph:panel" {
            // Sliders are set by pointer position in `App::click`; the panel background
            // only stops clicks from reaching the graph.
        } else {
            return false;
        }
        true
    }

    /// Graph commands. Returns false when `command` is not one.
    pub(crate) fn run_graph(&mut self, command: &str) -> bool {
        match command {
            "graph:open" => self.open_graph(None),
            "graph:open-local" => match self.tabs.get(self.active).map(|t| t.path.clone()) {
                Some(p) => self.open_graph(Some(p)),
                None => self.notice = Some("Abre una nota primero para ver su gráfico local.".into()),
            },
            "graph:focus" => self.show_graph = self.graph_view.is_some(),
            "graph:toggle-tags" => self.graph_setting(|v| v.settings.filters.show_tags ^= true),
            "graph:toggle-attachments" => self.graph_setting(|v| v.settings.filters.show_attachments ^= true),
            "graph:toggle-unresolved" => self.graph_setting(|v| v.settings.filters.hide_unresolved ^= true),
            "graph:toggle-orphans" => self.graph_setting(|v| v.settings.filters.show_orphans ^= true),
            "graph:toggle-arrows" => self.graph_setting(|v| v.settings.show_arrows ^= true),
            "graph:cycle-depth" => {
                if let Some(v) = &mut self.graph_view {
                    v.depth = v.depth % 3 + 1;
                    v.dirty = true;
                }
            }
            "graph:reset-view" => {
                if let Some(v) = &mut self.graph_view {
                    v.auto_fit = true;
                }
            }
            "graph:toggle-panel" => {
                if let Some(v) = &mut self.graph_view {
                    v.panel.open ^= true;
                    v.panel.edit = None;
                    self.save_graph();
                }
            }
            "graph:reset-settings" => self.graph_setting(|v| {
                let keep = std::mem::take(&mut v.settings);
                v.settings.color_groups = keep.color_groups;
                v.settings.search = keep.search;
                v.settings.scale = keep.scale;
                v.depth = 1;
            }),
            "graph:animate" => {
                if let Some(v) = &mut self.graph_view {
                    v.auto_fit = true;
                    v.restart();
                }
            }
            "graph:group-new" => {
                if let Some(v) = &mut self.graph_view {
                    let rgb = GROUP_COLORS[v.settings.color_groups.len() % GROUP_COLORS.len()];
                    v.settings.color_groups.push(graph_core::ColorGroup {
                        query: String::new(),
                        rgb,
                        alpha: 1.0,
                    });
                    v.panel.collapsed[1] = false;
                    let i = v.settings.color_groups.len() - 1;
                    v.panel.edit = Some((format!("group:{i}"), String::new()));
                    self.save_graph();
                }
            }
            _ => return self.graph_panel_command(command),
        }
        true
    }

    pub(crate) fn graph_clicked(&mut self, kind: NodeKind) {
        match kind {
            NodeKind::Note(id) | NodeKind::Attachment(id) => {
                if let Some(p) = self.store.index.entry(id).map(|f| f.path.to_string()) {
                    self.open_note(&p);
                }
            }
            NodeKind::Unresolved(name) => self.run(&format!("create:{name}")),
            NodeKind::Tag(tag) => {
                self.search_query = format!("tag:#{}", tag.trim_start_matches('#'));
                self.run("global-search:open");
                self.run_search();
            }
        }
    }
}
