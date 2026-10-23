//! Screenshot mode, for checking the UI from a script without touching the saved workspace:
//!
//! `GAZA_SHOT=out.png GAZA_SHOT_STEPS="send,res=Tree,code" cargo run`
//!
//! Steps are separated by commas, so commas can't appear inside a step's text.
//!
//! Each step runs once the app is idle, then the window is captured to a PNG and closed.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

use crate::app::{Action, BodyMode, GazaMan, ReqTab, ResTab, SidebarView, TabKind};

pub struct Shot {
    path: PathBuf,
    steps: Vec<String>,
    next: usize,
    idle_frames: u32,
    requested: bool,
    done: Arc<AtomicBool>,
    /// Fake input, one batch per frame (for `drag=` steps).
    pub input: VecDeque<Vec<egui::Event>>,
}

impl Shot {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("GAZA_SHOT")?;
        let steps = std::env::var("GAZA_SHOT_STEPS").unwrap_or_default();
        Some(Self {
            path: PathBuf::from(path),
            steps: steps
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            next: 0,
            idle_frames: 0,
            requested: false,
            done: Arc::new(AtomicBool::new(false)),
            input: VecDeque::new(),
        })
    }

    /// Call once per frame. Returns the next step when it's time to run it.
    pub fn tick(&mut self, ctx: &egui::Context, busy: bool) -> Option<String> {
        ctx.request_repaint();
        if self.done.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return None;
        }
        if busy || !self.input.is_empty() {
            self.idle_frames = 0;
            return None;
        }
        self.idle_frames += 1;
        if self.next < self.steps.len() {
            if self.idle_frames >= 4 {
                self.idle_frames = 0;
                self.next += 1;
                return Some(self.steps[self.next - 1].clone());
            }
            return None;
        }
        if self.idle_frames >= 12 && !self.requested {
            self.requested = true;
            let (path, done) = (self.path.clone(), self.done.clone());
            ctx.request_screenshot(move |image| {
                if let Err(e) = write_png(&path, &image) {
                    eprintln!("screenshot failed: {e}");
                }
                done.store(true, Ordering::Relaxed);
            });
        }
        None
    }
}

fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(image.as_raw())
        .map_err(|e| e.to_string())
}

pub fn apply(app: &mut GazaMan, ctx: &egui::Context, step: &str) {
    let active = app.ws.active_tab;
    let (name, arg) = step.split_once('=').unwrap_or((step, ""));
    match name {
        "send" => {
            if let Some(t) = active {
                app.actions.push(Action::Send(t));
            }
        }
        "code" => app.ws.show_code = true,
        "light" => app.ws.dark = false,
        "full" => app.ws.features = crate::features::Features::FULL,
        "settings" => app.show_settings = true,
        "cookies" => app.show_cookies = true,
        "console" => app.show_console = true,
        "import" => app.import_dialog(),
        "env" => app.ws.sidebar_view = SidebarView::Environments,
        "history" => app.ws.sidebar_view = SidebarView::History,
        "new" => app.new_request_tab(),
        "collection" => {
            if let Some(c) = app.ws.collections.first().map(|c| c.id) {
                app.open_tab(TabKind::Collection { coll: c });
            }
        }
        "envtab" => {
            if let Some(e) = app.ws.environments.first().map(|e| e.id) {
                app.open_tab(TabKind::Environment { env: e });
            }
        }
        "open" => {
            let n: usize = arg.parse().unwrap_or(0);
            let target = app
                .ws
                .collections
                .first()
                .and_then(|c| c.requests().get(n).map(|r| (c.id, r.id)));
            if let Some((c, r)) = target {
                app.open_request(c, r);
            }
        }
        "runner" => {
            app.runner.coll = app.ws.collections.first().map(|c| c.id);
            app.runner.open = true;
            if arg != "open" {
                app.start_runner(ctx);
            }
        }
        "tab" => {
            let t = match arg {
                "Auth" => ReqTab::Auth,
                "Headers" => ReqTab::Headers,
                "Body" => ReqTab::Body,
                "Tests" => ReqTab::Tests,
                "Capture" => ReqTab::Capture,
                "Docs" => ReqTab::Docs,
                "Scripts" => ReqTab::Scripts,
                _ => ReqTab::Params,
            };
            if let Some(id) = active {
                app.req_tabs.insert(id, t);
            }
        }
        "res" => match arg {
            "Tree" => app.body_mode = BodyMode::Tree,
            "Raw" => app.body_mode = BodyMode::Raw,
            "Preview" => app.body_mode = BodyMode::Preview,
            "Visualize" => app.res_tab = ResTab::Visualize,
            "Headers" => app.res_tab = ResTab::Headers,
            "Tests" => app.res_tab = ResTab::Tests,
            "Cookies" => app.res_tab = ResTab::Cookies,
            _ => app.res_tab = ResTab::Body,
        },
        "search" => app.res_search = arg.to_string(),
        "url" => {
            if let Some(i) = active.and_then(|t| app.tab_index(t)) {
                if let TabKind::Request { draft, .. } = &mut app.ws.tabs[i].kind {
                    draft.url = arg.to_string();
                    draft.sync_from_url();
                }
            }
        }
        // A mouse drag: `drag=x1;y1;x2;y2` in points.
        "drag" => {
            let v: Vec<f32> = arg.split(';').filter_map(|n| n.parse().ok()).collect();
            if let (Some(shot), [x1, y1, x2, y2]) = (app.shot_mut(), v.as_slice()) {
                let (a, b) = (egui::pos2(*x1, *y1), egui::pos2(*x2, *y2));
                let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
                shot.input.push_back(vec![egui::Event::PointerMoved(a)]);
                shot.input.push_back(vec![button(a, true)]);
                for i in 1..=10 {
                    shot.input.push_back(vec![egui::Event::PointerMoved(a.lerp(b, i as f32 / 10.0))]);
                }
                shot.input.push_back(vec![button(b, false)]);
            }
        }
        // Sockets and GraphQL: `kind=ws|sio|http`, `msg=…`, `event=…`, `gql=…`, then the actions.
        "body" | "desc" => {
            if let Some(i) = active.and_then(|t| app.tab_index(t)) {
                if let TabKind::Request { draft, .. } = &mut app.ws.tabs[i].kind {
                    if name == "body" {
                        draft.body_type = crate::collection::BodyType::Raw;
                        draft.raw = arg.replace("\\n", "\n");
                    } else {
                        draft.description = arg.to_string();
                    }
                }
            }
        }
        "kind" | "msg" | "event" | "gql" => {
            if let Some(i) = active.and_then(|t| app.tab_index(t)) {
                if let TabKind::Request { draft, .. } = &mut app.ws.tabs[i].kind {
                    match name {
                        "kind" => {
                            draft.kind = match arg {
                                "ws" => crate::collection::RequestKind::WebSocket,
                                "sio" => crate::collection::RequestKind::SocketIo,
                                _ => crate::collection::RequestKind::Http,
                            }
                        }
                        "msg" => draft.ws_message = arg.to_string(),
                        "event" => draft.sio_event = arg.to_string(),
                        _ => {
                            draft.method = "POST".into();
                            draft.body_type = crate::collection::BodyType::GraphQl;
                            draft.gql_query = arg.replace("\\n", "\n");
                        }
                    }
                }
            }
        }
        // Scripts come from GAZA_SHOT_PRE / GAZA_SHOT_POST, since steps can't hold commas.
        "scripts" => {
            if let Some(i) = active.and_then(|t| app.tab_index(t)) {
                if let TabKind::Request { draft, .. } = &mut app.ws.tabs[i].kind {
                    for (var, listen) in [("GAZA_SHOT_PRE", "prerequest"), ("GAZA_SHOT_POST", "test")] {
                        if let Ok(code) = std::env::var(var) {
                            crate::script::set_code(&mut draft.events, listen, &code);
                        }
                    }
                }
            }
            if arg == "post" {
                app.script_phase = crate::script::Phase::PostResponse;
            }
        }
        // Collection-level scripts, from GAZA_SHOT_COLL_PRE / GAZA_SHOT_COLL_POST.
        "collscripts" => {
            if let Some(c) = app.ws.collections.first_mut() {
                for (var, listen) in [("GAZA_SHOT_COLL_PRE", "prerequest"), ("GAZA_SHOT_COLL_POST", "test")] {
                    if let Ok(code) = std::env::var(var) {
                        crate::script::set_code(&mut c.events, listen, &code);
                    }
                }
            }
        }
        "wsconnect" | "wssend" | "schema" => {
            if let Some(t) = active {
                app.actions.push(match name {
                    "wsconnect" => Action::SocketConnect(t),
                    "wssend" => Action::SocketSend(t),
                    _ => Action::FetchSchema(t),
                });
            }
        }
        // Waits, so background work (connections, schema fetches) can finish.
        "sleep" => std::thread::sleep(std::time::Duration::from_millis(arg.parse().unwrap_or(500))),
        // A click and typed text: `click=x;y`, `type=text`, `key=ctrl+space`.
        "click" | "type" | "key" => {
            if let Some(shot) = app.shot_mut() {
                match name {
                    "click" => {
                        let v: Vec<f32> = arg.split(';').filter_map(|n| n.parse().ok()).collect();
                        if let [x, y] = v.as_slice() {
                            let pos = egui::pos2(*x, *y);
                            let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
                            // A real click also focuses the window; scripted runs may start unfocused.
                            shot.input.push_back(vec![egui::Event::WindowFocused(true), egui::Event::PointerMoved(pos)]);
                            shot.input.push_back(vec![button(true)]);
                            shot.input.push_back(vec![button(false)]);
                        }
                    }
                    "type" => {
                        for c in arg.chars() {
                            shot.input.push_back(vec![egui::Event::Text(c.to_string())]);
                        }
                    }
                    _ => {
                        let (modifiers, key) = match arg {
                            "ctrl+space" => (egui::Modifiers::CTRL, egui::Key::Space),
                            "down" => (egui::Modifiers::NONE, egui::Key::ArrowDown),
                            "enter" => (egui::Modifiers::NONE, egui::Key::Enter),
                            "end" => (egui::Modifiers::COMMAND, egui::Key::End),
                            _ => (egui::Modifiers::NONE, egui::Key::Escape),
                        };
                        let ev = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
                        shot.input.push_back(vec![ev(true), ev(false)]);
                    }
                }
            }
        }
        // Lists which of these characters the fonts can't draw.
        "glyphs" => {
            let missing: String = arg
                .chars()
                .filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(13.0), *c)))
                .collect();
            eprintln!("missing glyphs: {missing:?}");
        }
        _ => eprintln!("unknown screenshot step: {step}"),
    }
}
