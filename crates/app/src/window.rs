//! The window: winit for events, softbuffer to present the software-rendered frame.
//! Fully event-driven: the loop sleeps until input, a file change or a deadline (autosave,
//! a notice timing out), and redraws at display rate only while something is animating.

use metamind_app::welcome::Welcome;
use metamind_app::{instance, ospref};
use metamind_app::{recent, App, Frame, Key};
use metamind_ui::{Dom, Fonts, Pixmap};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vault_core::fsio::now_ms;
use vault_core::watch::VaultWatcher;
use vault_core::{Progress, VaultStore};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Icon, Theme, Window, WindowId};

enum UserEvent {
    /// Changed paths from a vault watcher, tagged with the generation of the vault it watches.
    Changed(u64, Vec<String>),
    Access(accesskit_winit::Event),
    /// A first-time index finished on its worker thread: the vault, the note to open, the store.
    Indexed(PathBuf, Option<String>, Result<VaultStore, String>),
    /// Another launch asked this window to come forward and show a note of its vault.
    Show(Option<String>),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> UserEvent {
        UserEvent::Access(e)
    }
}

struct Win {
    /// The open vault. `None` while the welcome screen is showing.
    app: Option<App>,
    welcome: Option<Welcome>,
    welcome_dom: Option<Dom>,
    fonts: Fonts,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pixmap: Option<Pixmap>,
    frame: Option<Frame>,
    mods: ModifiersState,
    mouse: (f32, f32),
    dragging: bool,
    animating: bool,
    pointer: bool,
    last_anim: Instant,
    /// Time between animation frames, from the refresh rate of the window's monitor.
    frame_interval: Duration,
    started: Instant,
    profile: bool,
    first_frame_done: bool,
    /// The caret rectangle last given to the input method.
    ime_area: Option<metamind_ui::Rect>,
    proxy: EventLoopProxy<UserEvent>,
    access: Option<accesskit_winit::Adapter>,
    /// Counts opened vaults; a watcher stops when its generation is no longer current.
    generation: Arc<AtomicU64>,
    watcher: Option<std::thread::Thread>,
    /// A vault being indexed for the first time; the welcome screen shows its progress.
    indexing: Option<Arc<Progress>>,
    /// Pop-out windows, each showing one pane.
    pops: Vec<Pop>,
    /// While handling a pop-out window: its id. Its state is then swapped into the fields above.
    solo: Option<u32>,
    /// The vault shown, read by the thread that answers other launches.
    shown: Arc<Mutex<Option<PathBuf>>>,
}

/// A pop-out window and the per-window state that `Win` holds for the main window.
struct Pop {
    id: u32,
    title: String,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pixmap: Option<Pixmap>,
    frame: Option<Frame>,
    mouse: (f32, f32),
    dragging: bool,
    pointer: bool,
    ime_area: Option<metamind_ui::Rect>,
}

fn new_window(
    el: &ActiveEventLoop,
    title: &str,
    size: LogicalSize<f64>,
    visible: bool,
) -> Result<Arc<Window>, String> {
    let icon = Icon::from_rgba(metamind_app::icon::rgba(64), 64, 64).ok();
    let attrs = Window::default_attributes()
        .with_title(title)
        .with_window_icon(icon)
        .with_inner_size(size)
        .with_visible(visible);
    let window = el.create_window(attrs).map_err(|e| format!("no se pudo crear una ventana: {e}"))?;
    window.set_ime_allowed(true);
    Ok(Arc::new(window))
}

fn new_surface(window: &Arc<Window>) -> Result<softbuffer::Surface<Arc<Window>, Arc<Window>>, String> {
    softbuffer::Context::new(window.clone())
        .and_then(|cx| softbuffer::Surface::new(&cx, window.clone()))
        .map_err(|e| format!("no se pudo crear una superficie de dibujo: {e}"))
}

fn named(k: &NamedKey) -> Option<&'static str> {
    Some(match k {
        NamedKey::Enter => "Enter",
        NamedKey::Escape => "Escape",
        NamedKey::Tab => "Tab",
        NamedKey::Backspace => "Backspace",
        NamedKey::Delete => "Delete",
        NamedKey::ArrowLeft => "ArrowLeft",
        NamedKey::ArrowRight => "ArrowRight",
        NamedKey::ArrowUp => "ArrowUp",
        NamedKey::ArrowDown => "ArrowDown",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PageUp",
        NamedKey::PageDown => "PageDown",
        NamedKey::Space => " ",
        NamedKey::F11 => "F11",
        NamedKey::F12 => "F12",
        _ => return None,
    })
}

impl Win {
    /// Animate at the refresh rate of the monitor the window is on (60 Hz when unknown).
    fn measure_refresh_rate(&mut self) {
        let mhz =
            self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz());
        let hz = mhz.map_or(60.0, |m| f64::from(m) / 1000.0).clamp(24.0, 360.0);
        self.frame_interval = Duration::from_secs_f64(1.0 / hz);
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if self.pixmap.as_ref().is_none_or(|p| p.width() != size.width || p.height() != size.height) {
            self.pixmap = Pixmap::new(size.width, size.height);
        }
        let Some(pixmap) = &mut self.pixmap else { return };
        let scale = window.scale_factor() as f32;
        if let Some(welcome) = &mut self.welcome {
            self.welcome_dom = Some(welcome.render(&mut self.fonts, pixmap, scale));
        } else if let Some(app) = &mut self.app {
            let solo = match self.solo {
                Some(id) => match app.window_pane(id) {
                    Some(p) => Some(p),
                    None => return,
                },
                None => None,
            };
            self.frame = Some(metamind_app::render_window(app, &mut self.fonts, pixmap, scale, solo));
            if let (true, Some(frame)) = (app.screenshot, &self.frame) {
                app.copy_screenshot(frame, pixmap, scale);
                // The notice shows on the next frame; the copied one is this frame, without it.
                window.request_redraw();
            }
        } else {
            return;
        }
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        for (dst, px) in buffer.iter_mut().zip(pixmap.data().as_chunks::<4>().0) {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        let _ = buffer.present();
        self.update_access();
        let caret = self.frame.as_ref().and_then(|f| f.caret);
        if caret != self.ime_area {
            self.ime_area = caret;
            if let (Some(c), Some(w)) = (caret, &self.window) {
                w.set_ime_cursor_area(LogicalPosition::new(c.x, c.y), LogicalSize::new(c.w.max(1.0), c.h));
            }
        }
        let Some(app) = &mut self.app else { return };
        if !self.first_frame_done {
            self.first_frame_done = true;
            // The link graph (backlinks) is computed after the first frame is on screen.
            app.defer_graph = false;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            if self.profile {
                eprintln!(
                    "[profile] first frame presented {:?} after process start (frame {:.2} ms)",
                    self.started.elapsed(),
                    app.last_frame_us as f64 / 1000.0
                );
            }
        }
    }

    /// Send the screen-reader tree for the frame just drawn, if a screen reader is listening.
    fn update_access(&mut self) {
        // The screen-reader adapter belongs to the main window.
        if self.solo.is_some() {
            return;
        }
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
        let title = self.title();
        let Some(adapter) = &mut self.access else { return };
        let dom = match (&self.welcome_dom, &self.frame) {
            (Some(d), _) if self.welcome.is_some() => d,
            (_, Some(f)) => &f.dom,
            _ => return,
        };
        adapter.update_if_active(|| crate::a11y::tree(dom, scale, &title));
    }

    /// A screen reader pressed a button: click the middle of it.
    fn activate(&mut self, el: &ActiveEventLoop, target: accesskit::NodeId) {
        let Some(i) = crate::a11y::dom_node(target) else { return };
        let dom = match (&self.welcome_dom, &self.frame) {
            (Some(d), _) if self.welcome.is_some() => d,
            (_, Some(f)) => &f.dom,
            _ => return,
        };
        let Some(r) = dom.nodes.get(i).map(|n| n.rect) else { return };
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        if let Some(welcome) = &mut self.welcome {
            let vault = welcome.click(dom, x, y);
            self.chosen(vault);
        } else if let (Some(app), Some(frame)) = (&mut self.app, &self.frame) {
            app.click(frame, &mut self.fonts, x, y, false);
            app.release();
            self.after_input(el);
        }
        self.request_redraw();
    }

    /// Redraw every window: they all show the same vault.
    fn request_redraw(&self) {
        for w in self.window.iter().chain(self.pops.iter().filter_map(|p| p.window.as_ref())) {
            w.request_redraw();
        }
    }

    /// Run `f` with pop-out window `i`'s state in place of the main window's.
    fn with_pop(&mut self, i: usize, f: impl FnOnce(&mut Win)) {
        let swap = |w: &mut Win, i: usize| {
            let p = &mut w.pops[i];
            std::mem::swap(&mut w.window, &mut p.window);
            std::mem::swap(&mut w.surface, &mut p.surface);
            std::mem::swap(&mut w.pixmap, &mut p.pixmap);
            std::mem::swap(&mut w.frame, &mut p.frame);
            std::mem::swap(&mut w.mouse, &mut p.mouse);
            std::mem::swap(&mut w.dragging, &mut p.dragging);
            std::mem::swap(&mut w.pointer, &mut p.pointer);
            std::mem::swap(&mut w.ime_area, &mut p.ime_area);
        };
        swap(self, i);
        self.solo = Some(self.pops[i].id);
        f(self);
        self.solo = None;
        swap(self, i);
    }

    /// Open a window for each pane the app moved out, close those whose pane is gone, and keep
    /// their titles on the note they show.
    fn sync_windows(&mut self, el: &ActiveEventLoop) {
        let Some(app) = &self.app else {
            if !self.pops.is_empty() {
                self.pops.clear();
                if let Some(w) = &self.window {
                    w.set_title(&self.title());
                }
            }
            return;
        };
        let titles: Vec<(u32, String)> = app
            .popouts
            .iter()
            .map(|&(id, pane)| {
                let (tabs, active) = app.pane_tabs(pane);
                let note = tabs
                    .get(active)
                    .map_or("MetaMind".to_string(), |t| format!("{} - MetaMind", t.title()));
                (id, note)
            })
            .collect();
        self.pops.retain(|p| titles.iter().any(|t| t.0 == p.id));
        for p in &mut self.pops {
            let Some((_, t)) = titles.iter().find(|t| t.0 == p.id) else { continue };
            if let (true, Some(w)) = (*t != p.title, &p.window) {
                w.set_title(t);
                p.title.clone_from(t);
            }
        }
        for (id, t) in titles {
            if self.pops.iter().any(|p| p.id == id) {
                continue;
            }
            let made = new_window(el, &t, LogicalSize::new(800.0, 700.0), true)
                .and_then(|w| new_surface(&w).map(|s| (w, s)));
            let (window, surface) = match made {
                Ok(ws) => ws,
                Err(e) => {
                    // Without a window the pane goes back to the main one.
                    if let Some(app) = &mut self.app {
                        app.dock_window(id);
                        app.notice = Some(format!("No se pudo abrir una ventana nueva: {e}"));
                    }
                    self.request_redraw();
                    continue;
                }
            };
            window.request_redraw();
            self.pops.push(Pop {
                id,
                title: t,
                window: Some(window),
                surface: Some(surface),
                pixmap: None,
                frame: None,
                mouse: (0.0, 0.0),
                dragging: false,
                pointer: false,
                ime_area: None,
            });
        }
    }

    fn title(&self) -> String {
        match &self.app {
            Some(app) => format!(
                "{} - MetaMind",
                app.store.root.file_name().and_then(|n| n.to_str()).unwrap_or("Vault")
            ),
            None => "MetaMind".to_string(),
        }
    }

    fn stop_watcher(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(t) = self.watcher.take() {
            t.unpark();
        }
    }

    /// Open a vault in this window, replacing the welcome screen or the current vault.
    fn open_vault(&mut self, vault: &Path, note: Option<&str>) -> Result<(), String> {
        if needs_index(vault) {
            self.start_indexing(vault, note);
            return Ok(());
        }
        let app = load(vault, note, self.profile, self.started)?;
        self.install(app);
        Ok(())
    }

    /// Build a vault's first index on a worker thread, keeping the window responsive and showing
    /// progress; `UserEvent::Indexed` installs the vault when it is done.
    fn start_indexing(&mut self, vault: &Path, note: Option<&str>) {
        let progress = Arc::new(Progress::default());
        self.indexing = Some(progress.clone());
        if self.welcome.is_none() {
            self.show_welcome();
        }
        let (proxy, root, note) = (self.proxy.clone(), vault.to_path_buf(), note.map(str::to_string));
        std::thread::spawn(move || {
            let store = VaultStore::open_with_progress(&root, &progress)
                .map(|(store, _)| store)
                .map_err(|e| format!("No se pudo abrir {}: {e}", root.display()));
            let _ = proxy.send_event(UserEvent::Indexed(root, note, store));
        });
    }

    fn indexed(&mut self, root: &Path, note: Option<&str>, store: Result<VaultStore, String>) {
        self.indexing = None;
        let app = store.and_then(|store| {
            App::with_store(root, store).map_err(|e| format!("No se pudo abrir {}: {e}", root.display()))
        });
        match app {
            Ok(app) => self.install(prepare(app, root, note, self.profile, self.started)),
            Err(e) => {
                if let Some(w) = &mut self.welcome {
                    w.notice = None;
                    w.error = Some(e);
                }
            }
        }
        self.request_redraw();
    }

    /// Make a loaded vault the one this window shows, and watch its files.
    fn install(&mut self, app: App) {
        self.stop_watcher();
        // Registering recursive watches walks the whole vault, so it runs off the start-up path.
        let proxy = self.proxy.clone();
        let root = app.store.root.clone();
        let current = self.generation.clone();
        let mine = current.load(Ordering::SeqCst);
        let handle = std::thread::spawn(move || {
            let watcher = VaultWatcher::start(&root, Duration::from_millis(150), move |paths| {
                let _ = proxy.send_event(UserEvent::Changed(mine, paths));
            });
            // Keep the watcher alive while this vault is open; the thread sleeps, it does not poll.
            while watcher.is_ok() && current.load(Ordering::SeqCst) == mine {
                std::thread::park();
            }
        });
        self.watcher = Some(handle.thread().clone());
        if let Ok(mut shown) = self.shown.lock() {
            *shown = Some(app.store.root.clone());
        }
        let mut app = app;
        app.set_system_dark(self.window.as_ref().and_then(|w| w.theme()).map(|t| t == Theme::Dark));
        // The vault index is already built here, but themes, plugins and the search corpus still
        // load over the next frames; the status bar spinner covers that stretch.
        app.start_busy("Cargando bóveda…");
        self.app = Some(app);
        self.welcome = None;
        self.welcome_dom = None;
        self.frame = None;
        self.first_frame_done = false;
        if let Some(w) = &self.window {
            w.set_title(&self.title());
        }
    }

    fn show(&mut self, note: Option<&str>) {
        if let (Some(app), Some(n)) = (&mut self.app, note) {
            app.open_note(n);
        }
        if let Some(w) = &self.window {
            w.set_minimized(false);
            w.focus_window();
        }
        self.request_redraw();
    }

    /// Leave the current vault (if any) and show the vault chooser.
    fn show_welcome(&mut self) {
        if let Ok(mut shown) = self.shown.lock() {
            *shown = None;
        }
        if let Some(app) = &mut self.app {
            app.save_all();
        }
        self.stop_watcher();
        self.app = None;
        self.frame = None;
        let mut welcome = Welcome::new();
        welcome.high_contrast = ospref::high_contrast();
        if !ospref::reduced_motion() {
            welcome.intro();
        }
        self.welcome = Some(welcome);
        if let Some(w) = &self.window {
            w.set_title(&self.title());
        }
    }

    /// A vault was chosen on the welcome screen.
    fn chosen(&mut self, vault: Option<std::path::PathBuf>) {
        let Some(vault) = vault else { return };
        if self.indexing.is_some() {
            return;
        }
        let note = self.welcome.as_mut().and_then(|w| w.open_note.take());
        if let Err(e) = self.open_vault(&vault, note.as_deref()) {
            if let Some(w) = &mut self.welcome {
                w.error = Some(e);
            }
        }
    }

    /// Show a hand over things that can be clicked.
    fn set_pointer(&mut self, pointer: bool) {
        if pointer != self.pointer {
            self.pointer = pointer;
            if let Some(w) = &self.window {
                w.set_cursor(if pointer { CursorIcon::Pointer } else { CursorIcon::Default });
            }
        }
    }

    /// After input reached the app: follow its requests to quit or to switch vaults.
    fn after_input(&mut self, el: &ActiveEventLoop) {
        let Some(app) = &mut self.app else { return };
        if app.quit {
            el.exit();
        } else if app.switch_vault {
            app.switch_vault = false;
            self.show_welcome();
        }
    }

    fn welcome_event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(welcome) = &mut self.welcome else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Ime(Ime::Commit(text)) => welcome.text_input(&text),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let ctrl = self.mods.control_key() || self.mods.super_key();
                match &event.logical_key {
                    WKey::Character(c) if ctrl && c.eq_ignore_ascii_case("q") => return el.exit(),
                    WKey::Named(n) if named(n).is_some_and(|n| n != " ") => {
                        let vault = welcome.key(named(n).unwrap_or_default());
                        self.chosen(vault);
                    }
                    _ if !ctrl && !self.mods.alt_key() => {
                        if let Some(text) = &event.text {
                            welcome.text_input(text);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                if let Some(dom) = &self.welcome_dom {
                    let changed = welcome.hover(dom, self.mouse.0, self.mouse.1);
                    let pointer = welcome.hover_action.is_some();
                    self.set_pointer(pointer);
                    if !changed {
                        return;
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(dom) = &self.welcome_dom {
                    let vault = welcome.click(dom, self.mouse.0, self.mouse.1);
                    self.chosen(vault);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 20.0,
                };
                if let Some(dom) = &self.welcome_dom {
                    welcome.wheel(dom, self.mouse.0, self.mouse.1, lines);
                }
            }
            _ => return,
        }
        self.request_redraw();
    }
}

impl ApplicationHandler<UserEvent> for Win {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The accessibility adapter must exist before the window is first shown.
        let made = new_window(el, &self.title(), LogicalSize::new(1280.0, 800.0), false)
            .and_then(|w| new_surface(&w).map(|s| (w, s)));
        let window = match made {
            Ok((w, s)) => {
                self.surface = Some(s);
                w
            }
            Err(e) => {
                eprintln!("metamind: {e}");
                el.exit();
                return;
            }
        };
        if let Some(app) = &mut self.app {
            app.set_system_dark(window.theme().map(|t| t == Theme::Dark));
        }
        self.access = Some(accesskit_winit::Adapter::with_event_loop_proxy(el, &window, self.proxy.clone()));
        window.set_visible(true);
        self.window = Some(window);
        self.measure_refresh_rate();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        let (generation, paths) = match event {
            UserEvent::Changed(generation, paths) => (generation, paths),
            UserEvent::Indexed(root, note, store) => return self.indexed(&root, note.as_deref(), store),
            UserEvent::Show(note) => return self.show(note.as_deref()),
            UserEvent::Access(e) => {
                match e.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => self.update_access(),
                    accesskit_winit::WindowEvent::ActionRequested(r)
                        if r.action == accesskit::Action::Click =>
                    {
                        self.activate(el, r.target_node)
                    }
                    _ => {}
                }
                return;
            }
        };
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
        }
        if let Some(app) = &mut self.app {
            app.files_changed(&paths);
            self.request_redraw();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let pop = self.pops.iter().position(|p| p.window.as_ref().is_some_and(|w| w.id() == id));
        match pop {
            Some(i) => self.with_pop(i, |w| w.event(el, event)),
            None => self.event(el, event),
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.sync_windows(el);
        self.wait(el);
    }
}

impl Win {
    /// An event for the main window, or for the pop-out window `self.solo`.
    fn event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        if let (Some(access), Some(window), None) = (&mut self.access, &self.window, self.solo) {
            access.process_event(window, &event);
        }
        match event {
            WindowEvent::RedrawRequested => return self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.measure_refresh_rate();
                return self.request_redraw();
            }
            WindowEvent::Moved(_) => return self.measure_refresh_rate(),
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                let (x, y, ctrl) = (self.mouse.0, self.mouse.1, self.mods.control_key());
                let changed = match (&mut self.app, &self.frame) {
                    (Some(app), Some(frame)) => app.hover_link(frame, &mut self.fonts, x, y, ctrl),
                    _ => false,
                };
                if changed {
                    self.request_redraw();
                }
                return;
            }
            WindowEvent::ThemeChanged(t) => {
                if let Some(app) = &mut self.app {
                    app.set_system_dark(Some(t == Theme::Dark));
                    self.request_redraw();
                }
                return;
            }
            _ => {}
        }
        if self.welcome.is_some() {
            return self.welcome_event(el, event);
        }
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
        let Some(app) = &mut self.app else { return };
        match event {
            WindowEvent::CloseRequested if self.solo.is_some() => {
                app.dock_window(self.solo.unwrap_or_default());
                self.request_redraw();
            }
            WindowEvent::CloseRequested => {
                app.save_all();
                el.exit();
            }
            WindowEvent::Focused(false) => app.save_all(),
            // These preferences have no change event on every OS, so they are re-read on focus.
            WindowEvent::Focused(true) => {
                let moved = app.is_popped(app.pane) || self.solo.is_some();
                if moved {
                    app.focus_window(self.solo);
                }
                let (reduced, contrast) = (ospref::reduced_motion(), ospref::high_contrast());
                if moved || (reduced, contrast) != (app.reduced_motion, app.high_contrast) {
                    app.set_reduced_motion(reduced);
                    app.high_contrast = contrast;
                    self.request_redraw();
                }
            }
            WindowEvent::DroppedFile(file) => {
                app.import_files(&[&file]);
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                app.preedit = text;
                app.preedit_cursor = cursor.map(|(_, end)| end);
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                app.preedit.clear();
                app.text_input(&text);
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Disabled) => {
                app.preedit.clear();
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let name = match &event.logical_key {
                    WKey::Named(n) => named(n).map(str::to_string),
                    WKey::Character(c) => Some(c.to_string()),
                    _ => None,
                };
                let mut consumed = false;
                if let Some(name) = name {
                    let mut key = Key::new(&name);
                    key.ctrl = self.mods.control_key() || self.mods.super_key();
                    key.shift = self.mods.shift_key();
                    key.alt = self.mods.alt_key();
                    // Plain characters are text, not shortcuts, unless a modifier is held. The
                    // space bar is the exception: a board pans while it is held, and a note gets
                    // its space because the key is not consumed.
                    let plain_char = name.chars().count() == 1 && name != " " && !key.ctrl && !key.alt;
                    if !plain_char {
                        consumed = app.key(key);
                    }
                    if name == " " {
                        app.exc_space(true);
                    }
                }
                if !consumed && !self.mods.control_key() && !self.mods.alt_key() {
                    if let Some(text) = &event.text {
                        app.text_input(text);
                    }
                }
                self.after_input(el);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Released => {
                // Letting the space bar up stops a board panning with the button still down.
                if matches!(&event.logical_key, WKey::Named(NamedKey::Space)) {
                    app.exc_space(false);
                    self.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                let Some(frame) = &self.frame else { return };
                if self.dragging {
                    app.drag(frame, &mut self.fonts, self.mouse.0, self.mouse.1);
                    self.request_redraw();
                } else {
                    let (x, y, ctrl) = (self.mouse.0, self.mouse.1, self.mods.control_key());
                    let changed = app.hover(frame, x, y) | app.hover_link(frame, &mut self.fonts, x, y, ctrl);
                    let pointer = app.hover_action.is_some();
                    self.set_pointer(pointer);
                    if changed {
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    if let Some(frame) = &self.frame {
                        app.click(frame, &mut self.fonts, self.mouse.0, self.mouse.1, self.mods.shift_key());
                    }
                } else {
                    app.release();
                }
                self.after_input(el);
                self.request_redraw();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                if let Some(frame) = &self.frame {
                    app.context_menu(frame, &mut self.fonts, self.mouse.0, self.mouse.1);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Middle, .. } => {
                if let Some(frame) = &self.frame {
                    if state == ElementState::Pressed {
                        app.middle_click(frame, self.mouse.0, self.mouse.1);
                        // A board grabs the pointer: its movement is a pan, not a click.
                        self.dragging = app.exc_view.as_ref().is_some_and(|v| v.panning());
                    } else {
                        app.release();
                        self.dragging = false;
                    }
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 20.0,
                };
                if let Some(frame) = &self.frame {
                    if self.mods.control_key() || self.mods.super_key() {
                        app.ctrl_wheel(frame, self.mouse.0, self.mouse.1, lines);
                    } else {
                        app.wheel(frame, self.mouse.0, self.mouse.1, lines);
                    }
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn wait(&mut self, el: &ActiveEventLoop) {
        let frame_due = self.last_anim.elapsed() >= self.frame_interval;
        let Some(app) = &mut self.app else {
            // Welcome screen: only its own animations and indexing progress need frames.
            if let (Some(p), Some(w)) = (&self.indexing, &mut self.welcome) {
                let (done, total) = (p.done.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed));
                let text = if total == 0 {
                    "Indexando tu bóveda…".to_string()
                } else {
                    format!("Indexando tu bóveda… {done}/{total}")
                };
                if w.notice.as_ref() != Some(&text) {
                    w.notice = Some(text);
                    self.request_redraw();
                }
            }
            let animating = self.welcome.as_ref().is_some_and(|w| w.anims.active());
            if (animating && frame_due) || (self.animating && !animating) {
                self.last_anim = Instant::now();
                self.request_redraw();
            }
            self.animating = animating;
            return el.set_control_flow(if animating {
                ControlFlow::WaitUntil(self.last_anim + self.frame_interval)
            } else if self.indexing.is_some() {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100))
            } else {
                ControlFlow::Wait
            });
        };
        // Sleep until the next deadline (outline refresh, autosave, notice), or indefinitely.
        let now = now_ms();
        let mut redraw = app.derived_due_in(now) == Some(0);
        if app.autosave_due_in(now) == Some(0) {
            app.save_all();
            redraw = true;
        }
        if app.notice_due_in() == Some(0) {
            app.notice = None;
            redraw = true;
        }
        if let Some(msg) = app.update.poll() {
            app.notice = Some(msg);
            redraw = true;
        }
        // The AI answer arrives piece by piece; the lit waiting dot also needs frames.
        if app.ai.poll() || app.ai.busy() {
            redraw = true;
        }
        let now = now_ms();
        // A background update job is polled until it reports back.
        let update = app.update.busy().then_some(250);
        let ai = app.ai.busy().then_some(60);
        let next = [app.derived_due_in(now), app.autosave_due_in(now), app.notice_due_in(), update, ai]
            .into_iter()
            .flatten()
            .filter(|&ms| ms > 0)
            .min();
        // While the graph layout or an interface animation is moving, redraw at display rate;
        // both stop by themselves.
        let animating = app.graph_animating() || app.anims.active() || app.scrolling() || app.busy.is_some();
        if (animating && frame_due) || (self.animating && !animating) {
            self.last_anim = Instant::now();
            redraw = true;
        }
        self.animating = animating;
        if redraw {
            self.request_redraw();
        }
        let mut wake = next.map(|ms| Instant::now() + Duration::from_millis(ms));
        if animating {
            let frame = self.last_anim + self.frame_interval;
            wake = Some(wake.map_or(frame, |w| w.min(frame)));
        }
        match wake {
            Some(at) => el.set_control_flow(ControlFlow::WaitUntil(at)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// A vault opened for the first time has no persisted index; building one can take a while.
fn needs_index(vault: &Path) -> bool {
    vault.is_dir() && !VaultStore::cache_path(vault).exists()
}

/// Open a vault and prepare it for a window.
fn load(vault: &Path, note: Option<&str>, profile: bool, started: Instant) -> Result<App, String> {
    let app = App::open(vault).map_err(|e| format!("No se pudo abrir {}: {e}", vault.display()))?;
    Ok(prepare(app, vault, note, profile, started))
}

fn prepare(mut app: App, vault: &Path, note: Option<&str>, profile: bool, started: Instant) -> App {
    if profile {
        eprintln!("[profile] vault opened in {:?} ({} files)", started.elapsed(), app.store.index.len());
    }
    app.defer_graph = true;
    app.set_reduced_motion(ospref::reduced_motion());
    app.high_contrast = ospref::high_contrast();
    app.system_clipboard = true;
    if metamind_app::update::auto_check_due(now_ms()) {
        app.update.start_check(false);
    }
    match note {
        Some(n) => app.open_note(n),
        None => app.restore_session(),
    }
    recent::remember(vault);
    app
}

/// Run the window. With no vault given, the most recent vault is reopened; when there is none,
/// the welcome screen asks for one.
pub fn run(vault: Option<&Path>, note: Option<&str>, profile: bool) -> i32 {
    metamind_app::update::clean_old();
    let started = Instant::now();
    // Fonts load on a worker thread while the vault index loads on this one.
    let fonts = std::thread::spawn(Fonts::new);
    std::thread::spawn(metamind_app::spell::load);
    let start = vault.map(Path::to_path_buf).or_else(|| recent::load().into_iter().next());
    // A vault without an index is indexed once the window is up, so its progress can be shown.
    let (initial, pending) = match start {
        Some(v) if needs_index(&v) => (None, Some(v)),
        Some(v) => match load(&v, note.filter(|_| vault.is_some()), profile, started) {
            Ok(app) => (Some(app), None),
            Err(e) if vault.is_some() => {
                eprintln!("metamind: {e}");
                return 1;
            }
            Err(_) => (None, None),
        },
        None => (None, None),
    };
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("metamind: no display available: {e}");
            return 1;
        }
    };
    let Ok(fonts) = fonts.join() else { return 1 };
    if profile {
        eprintln!("[profile] fonts ready at {:?} ({} faces)", started.elapsed(), fonts.font_count());
    }
    let mut win = Win {
        app: None,
        welcome: None,
        welcome_dom: None,
        fonts,
        window: None,
        surface: None,
        pixmap: None,
        frame: None,
        mods: ModifiersState::empty(),
        mouse: (0.0, 0.0),
        dragging: false,
        animating: false,
        pointer: false,
        last_anim: started,
        frame_interval: Duration::from_millis(16),
        started,
        profile,
        first_frame_done: false,
        ime_area: None,
        proxy: event_loop.create_proxy(),
        access: None,
        generation: Arc::new(AtomicU64::new(0)),
        watcher: None,
        indexing: None,
        pops: Vec::new(),
        solo: None,
        shown: Arc::new(Mutex::new(None)),
    };
    let instances = instance::dir();
    if let Some(dir) = &instances {
        match instance::Instance::register(dir) {
            Ok(inst) => {
                let (shown, proxy) = (win.shown.clone(), win.proxy.clone());
                inst.serve(move |vault, note| {
                    let mine = shown.lock().ok().and_then(|s| s.clone());
                    mine.is_some_and(|m| instance::same_vault(&m, vault))
                        && proxy.send_event(UserEvent::Show(note)).is_ok()
                });
            }
            Err(e) => eprintln!("metamind: other launches cannot reach this window: {e}"),
        }
    }
    match (initial, pending) {
        (Some(app), _) => win.install(app),
        (None, Some(v)) => win.start_indexing(&v, note),
        (None, None) => win.show_welcome(),
    }
    let result = event_loop.run_app(&mut win);
    if let Some(dir) = &instances {
        instance::unregister(dir);
    }
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("metamind: {e}");
            1
        }
    }
}
