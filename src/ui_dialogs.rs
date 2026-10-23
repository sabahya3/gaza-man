//! Collection / folder / environment tabs, modal dialogs, the runner, settings, cookies and
//! the console.

use std::collections::HashSet;

use eframe::egui::{self, Color32, Margin, RichText, Stroke};
use serde_json::json;

use crate::app::{
    Action, ConfirmKind, Dialog, GazaMan, PromptKind, RunFilter, Tab, TabKind, ToastKind,
};
use crate::collection::{Auth, Collection, Environment, Folder, Node};
use crate::export::Format;
use crate::http::Outcome;
use crate::rtl::shape;
use crate::theme::{self, pal};
use crate::ui_request::{KvOptions, known_vars, truncate};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum CollSection {
    #[default]
    Overview,
    Auth,
    Variables,
    Scripts,
    Mock,
}

fn modal_frame(p: &theme::Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14.0)
        .inner_margin(Margin::same(22))
        .shadow(egui::epaint::Shadow {
            offset: [0, 16],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

fn window_frame(ctx: &egui::Context) -> egui::Frame {
    let p = theme::palette(ctx.global_style().visuals.dark_mode);
    egui::Frame::window(&ctx.global_style())
        .fill(p.surface)
        .inner_margin(Margin::same(18))
        .corner_radius(14.0)
}

impl GazaMan {
    // -----------------------------------------------------------------------------------------
    // Collection and folder tabs
    // -----------------------------------------------------------------------------------------

    pub fn collection_tab(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let p = pal(ui);
        let (coll_id, folder_id) = match &tab.kind {
            TabKind::Collection { coll } => (*coll, None),
            TabKind::Folder { coll, folder } => (*coll, Some(*folder)),
            _ => return,
        };
        let Some(c) = self.collection(coll_id).cloned() else {
            theme::empty_state(
                ui,
                "\u{E466}",
                "This collection was deleted",
                "Close this tab to continue.",
            );
            return;
        };
        let section_id = egui::Id::unique(("coll_section", tab.id));
        let mut section: CollSection = ui
            .ctx()
            .data_mut(|d| d.get_temp(section_id).unwrap_or_default());
        let vars = self.vars_for(Some(coll_id));
        let known = known_vars(&vars);

        egui::ScrollArea::vertical().id_salt(("coll_scroll", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(28, 20)).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(980.0));
                // Header
                ui.horizontal(|ui| {
                    let (icon, kind) = if folder_id.is_some() { ("\u{E24A}", "Folder") } else { ("\u{E466}", "Collection") };
                    ui.label(RichText::new(icon).size(22.0));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(kind.to_uppercase()).size(10.5).color(p.faint));
                        let mut name = match folder_id {
                            Some(f) => c.find(f).map(|n| n.name().to_string()).unwrap_or_default(),
                            None => c.name.clone(),
                        };
                        let resp = ui.add(egui::TextEdit::singleline(&mut name).font(egui::FontId::proportional(20.0)).frame(egui::Frame::NONE).desired_width(420.0));
                        if resp.changed() && !name.trim().is_empty() {
                            match folder_id {
                                Some(f) => {
                                    if let Some(n) = self.collection_mut(coll_id).and_then(|c| c.find_mut(f)) {
                                        *n.name_mut() = name;
                                    }
                                }
                                None => {
                                    if let Some(c) = self.collection_mut(coll_id) {
                                        c.name = name;
                                    }
                                }
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if folder_id.is_none() {
                            let export = ui.button("\u{EAF0}  Export");
                            egui::Popup::menu(&export).show(|ui| {
                                ui.set_min_width(260.0);
                                for f in Format::ALL {
                                    if ui.button(f.label()).clicked() {
                                        self.actions.push(Action::Export(coll_id, f));
                                    }
                                }
                            });
                        }
                        if self.ws.features.runner && ui.add(theme::primary("\u{E3D0}  Run")).clicked() {
                            self.actions.push(Action::Run { coll: coll_id, folder: folder_id });
                        }
                        if ui.button("+  Add request").clicked() {
                            self.actions.push(Action::AddRequest { coll: coll_id, folder: folder_id });
                        }
                    });
                });
                ui.add_space(12.0);
                let mut items = vec![(CollSection::Overview, "Overview", None), (CollSection::Auth, "Authorization", None)];
                let own_events = match folder_id {
                    Some(f) => match c.find(f) {
                        Some(Node::Folder(folder)) => folder.events.clone(),
                        _ => vec![],
                    },
                    None => c.events.clone(),
                };
                let f = self.ws.features;
                let scripts_dot = crate::ui_request::scripts_dot(&own_events);
                if f.scripts || scripts_dot.is_some() {
                    items.push((CollSection::Scripts, "Scripts", scripts_dot));
                }
                if folder_id.is_none() {
                    items.push((CollSection::Variables, "Variables", (!c.variables.is_empty()).then(|| c.variables.len().to_string())));
                    if f.mock || self.mocks.contains_key(&coll_id) {
                        items.push((CollSection::Mock, "Mock Server", self.mocks.contains_key(&coll_id).then(|| "●".to_string())));
                    }
                }
                if !items.iter().any(|(t, _, _)| *t == section) {
                    section = CollSection::Overview;
                }
                theme::tab_strip(ui, &mut section, &items);
                ui.add_space(12.0);

                match section {
                    CollSection::Overview => {
                        let (desc, requests, folders) = match folder_id {
                            Some(f) => match c.find(f) {
                                Some(Node::Folder(folder)) => (folder.description.clone(), c.requests_in(Some(f)).len(), count_folders(&folder.items)),
                                _ => (String::new(), 0, 0),
                            },
                            None => (c.description.clone(), c.count_requests(), count_folders(&c.items)),
                        };
                        ui.horizontal(|ui| {
                            for (n, label) in [(requests, "requests"), (folders, "folders")] {
                                theme::card(ui, |ui| {
                                    ui.set_min_width(130.0);
                                    ui.label(RichText::new(n.to_string()).size(22.0).strong().color(p.text));
                                    ui.label(RichText::new(label).color(p.muted));
                                });
                            }
                            if folder_id.is_none() {
                                theme::card(ui, |ui| {
                                    ui.set_min_width(130.0);
                                    ui.label(RichText::new(c.variables.len().to_string()).size(22.0).strong().color(p.text));
                                    ui.label(RichText::new("variables").color(p.muted));
                                });
                            }
                        });
                        ui.add_space(14.0);
                        theme::section_title(ui, "Description");
                        let mut d = desc;
                        let resp = crate::ui_request::prose_editor(ui, &mut d, 8, "Describe this collection: what the API does, how to log in…", ui.make_persistent_id(("coll_desc", tab.id)));
                        if resp.changed() {
                            match folder_id {
                                Some(f) => {
                                    if let Some(folder) = self.collection_mut(coll_id).and_then(|c| c.folder_mut(f)) {
                                        folder.description = d;
                                    }
                                }
                                None => {
                                    if let Some(c) = self.collection_mut(coll_id) {
                                        c.description = d;
                                    }
                                }
                            }
                        }
                    }
                    CollSection::Auth => {
                        ui.label(RichText::new("Requests set to \"Inherit auth from parent\" use this.").color(p.muted));
                        ui.add_space(8.0);
                        let mut auth = match folder_id {
                            Some(f) => match c.find(f) {
                                Some(Node::Folder(folder)) => folder.auth.clone(),
                                _ => Auth::Inherit,
                            },
                            None => c.auth.clone(),
                        };
                        let inherited = folder_id.map(|f| {
                            let parent = c.path_to(f).iter().rev().map(|x| &x.auth).find(|a| **a != Auth::Inherit).cloned();
                            (parent.unwrap_or_else(|| c.auth.clone()), format!("collection \"{}\"", c.name))
                        });
                        let before = auth.clone();
                        self.auth_editor(ui, &mut auth, inherited, None);
                        if auth != before {
                            match folder_id {
                                Some(f) => {
                                    if let Some(folder) = self.collection_mut(coll_id).and_then(|c| c.folder_mut(f)) {
                                        folder.auth = auth;
                                    }
                                }
                                None => {
                                    if let Some(c) = self.collection_mut(coll_id) {
                                        c.auth = auth;
                                    }
                                }
                            }
                        }
                    }
                    CollSection::Variables => {
                        ui.label(RichText::new("Collection variables are shared by every request in this collection. The active environment overrides them.").color(p.muted));
                        ui.add_space(8.0);
                        let mut list = c.variables.clone();
                        let opts = KvOptions { id: format!("collvars-{coll_id}"), key_hint: "Variable", files: false, fixed_keys: false, known: &known };
                        if self.kv_table(ui, &mut list, opts) {
                            if let Some(c) = self.collection_mut(coll_id) {
                                c.variables = list;
                            }
                        }
                    }
                    CollSection::Mock => self.mock_section(ui, &c),
                    CollSection::Scripts => {
                        let mut events = own_events.clone();
                        self.scripts_pane(ui, folder_id.unwrap_or(coll_id), &mut events, if folder_id.is_some() { "folder" } else { "collection" });
                        if events != own_events {
                            match folder_id {
                                Some(f) => {
                                    if let Some(folder) = self.collection_mut(coll_id).and_then(|c| c.folder_mut(f)) {
                                        folder.events = events;
                                    }
                                }
                                None => {
                                    if let Some(c) = self.collection_mut(coll_id) {
                                        c.events = events;
                                    }
                                }
                            }
                        }
                    }
                }
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(section_id, section));
    }

    fn mock_section(&mut self, ui: &mut egui::Ui, c: &Collection) {
        let p = pal(ui);
        ui.label(RichText::new("A local server that answers like your API, using the examples saved on each request. Point your app or a front end at it before the real API is ready.").color(p.muted));
        ui.add_space(10.0);
        match self.mocks.get(&c.id) {
            Some(m) => {
                let url = m.url();
                ui.horizontal(|ui| {
                    theme::soft_badge(ui, "●  Running", p.accent, 12.0);
                    ui.label(RichText::new(&url).monospace().size(14.0).color(p.text));
                    if ui.button("\u{E1CA}  Copy URL").clicked() {
                        ui.ctx().copy_text(url.clone());
                        self.actions.push(Action::Notify("Copied the mock URL".into(), ToastKind::Success));
                    }
                    if ui.button("Use as {{baseUrl}} in a new environment").clicked() {
                        let e = Environment { name: format!("{} (mock)", c.name), values: vec![crate::collection::KeyValue::new("baseUrl", &url)], ..Default::default() };
                        let id = e.id;
                        self.ws.environments.push(e);
                        self.actions.push(Action::SetActiveEnv(Some(id)));
                        self.actions.push(Action::Notify("Created and activated the mock environment".into(), ToastKind::Success));
                    }
                    if ui.button(RichText::new("\u{E46C}  Stop").color(p.red)).clicked() {
                        self.actions.push(Action::StopMock(c.id));
                    }
                });
                if m.public {
                    ui.label(RichText::new("Other devices on your network can reach it at this computer's IP address and the same port.").size(12.0).color(p.amber));
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Pick a response with the headers  x-mock-response-name: <example name>  or  x-mock-response-code: 404.").size(12.0).color(p.faint));
            }
            None => {
                ui.horizontal(|ui| {
                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut self.mock_port).range(1024..=65535));
                    ui.checkbox(&mut self.mock_public, "Let other devices on the network connect");
                    if ui.add(theme::primary("\u{E2A0}  Start mock server")).clicked() {
                        self.actions.push(Action::StartMock(c.id));
                    }
                });
            }
        }
        ui.add_space(12.0);
        theme::section_title(ui, "Routes");
        let vars = self.vars_for(Some(c.id));
        egui::Grid::new(("mock_routes", c.id)).num_columns(4).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
            for r in c.requests() {
                let path = crate::collection::url_base(&crate::http::resolve(&r.url, &vars)).to_string();
                let path = path.split_once("://").and_then(|(_, rest)| rest.find('/').map(|i| rest[i..].to_string())).unwrap_or(path);
                ui.label(theme::method_text(&p, &r.method, 11.5));
                ui.label(RichText::new(path).monospace().size(12.0));
                ui.label(shape(&r.name));
                if r.examples.is_empty() {
                    ui.label(RichText::new("no example yet").size(11.5).color(p.amber));
                } else {
                    ui.label(RichText::new(format!("{} example(s)", r.examples.len())).size(11.5).color(p.accent));
                }
                ui.end_row();
            }
        });
        if let Some(m) = self.mocks.get(&c.id) {
            ui.add_space(12.0);
            theme::section_title(ui, "Calls");
            let log = m.log.lock().map(|l| l.clone()).unwrap_or_default();
            if log.is_empty() {
                ui.label(RichText::new("No calls yet. Try it: send a request to the URL above.").color(p.faint));
            }
            egui::Grid::new(("mock_log", c.id)).num_columns(4).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                for e in log.iter().rev().take(50) {
                    ui.label(RichText::new(crate::http::iso_time(e.at, 0)[11..19].to_string()).monospace().size(11.5).color(p.faint));
                    ui.label(RichText::new(format!("{} {}", e.method, e.path)).monospace().size(12.0));
                    theme::soft_badge(ui, e.status.to_string(), theme::status_color(&p, Some(e.status)), 11.0);
                    ui.label(RichText::new(e.matched.clone().unwrap_or_else(|| "no match".into())).size(11.5).color(p.muted));
                    ui.end_row();
                }
            });
        }
    }

    pub fn environment_tab(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let p = pal(ui);
        let env_id = match &tab.kind {
            TabKind::Environment { env } => Some(*env),
            _ => None,
        };
        let env: Option<Environment> =
            env_id.and_then(|id| self.ws.environments.iter().find(|e| e.id == id).cloned());
        if env_id.is_some() && env.is_none() {
            theme::empty_state(
                ui,
                "\u{E288}",
                "This environment was deleted",
                "Close this tab to continue.",
            );
            return;
        }
        let known = known_vars(&self.vars_for(None));
        egui::ScrollArea::vertical().id_salt(("env_scroll", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(28, 20)).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(980.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if env.is_some() { "\u{E288}" } else { "\u{E28C}" }).size(22.0));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(if env.is_some() { "ENVIRONMENT" } else { "GLOBALS" }).size(10.5).color(p.faint));
                        match &env {
                            Some(e) => {
                                let mut name = e.name.clone();
                                if ui.add(egui::TextEdit::singleline(&mut name).font(egui::FontId::proportional(20.0)).frame(egui::Frame::NONE).desired_width(420.0)).changed() {
                                    if let Some(e) = self.ws.environments.iter_mut().find(|x| Some(x.id) == env_id) {
                                        e.name = name;
                                    }
                                }
                            }
                            None => {
                                ui.label(RichText::new("Globals").size(20.0));
                            }
                        }
                    });
                    if let Some(e) = &env {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("\u{EAF0}  Export").clicked() {
                                self.actions.push(Action::ExportEnvironment(e.id));
                            }
                            let active = self.ws.active_env == Some(e.id);
                            if active {
                                theme::soft_badge(ui, "●  Active", p.accent, 12.0);
                            } else if ui.add(theme::primary("Set active")).clicked() {
                                self.actions.push(Action::SetActiveEnv(Some(e.id)));
                            }
                        });
                    }
                });
                ui.add_space(8.0);
                let help = if env.is_some() {
                    "Use these as {{name}} in URLs, headers, bodies and auth. Switch environments from the top bar to point the same requests at another server."
                } else {
                    "Global variables are available everywhere, in every collection. Environments and collection variables override them."
                };
                ui.label(RichText::new(help).color(p.muted));
                ui.add_space(12.0);
                let mut values = match &env {
                    Some(e) => e.values.clone(),
                    None => self.ws.globals.clone(),
                };
                let id = format!("envvars-{}", env_id.unwrap_or(0));
                let opts = KvOptions { id, key_hint: "Variable", files: false, fixed_keys: false, known: &known };
                if self.kv_table(ui, &mut values, opts) {
                    match env_id {
                        Some(id) => {
                            if let Some(e) = self.ws.environments.iter_mut().find(|x| x.id == id) {
                                e.values = values;
                            }
                        }
                        None => self.ws.globals = values,
                    }
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Dialogs
    // -----------------------------------------------------------------------------------------

    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let p = theme::palette(ctx.global_style().visuals.dark_mode);
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("dialog"))
            .frame(modal_frame(&p))
            .backdrop_color(Color32::from_black_alpha(140));
        let response = modal.show(ctx, |ui| {
            match &mut dialog {
                Dialog::Prompt { title, text, kind } => {
                    ui.set_width(380.0);
                    ui.label(RichText::new(title.as_str()).size(17.0).strong());
                    ui.add_space(10.0);
                    let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
                    if !resp.has_focus() && ui.memory(|m| m.focused().is_none()) {
                        resp.request_focus();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(14.0);
                    let mut ok = enter;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ok |= ui.add(theme::primary(if matches!(kind, PromptKind::Rename(_)) { "Rename" } else { "Create" })).clicked();
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if ok && !text.trim().is_empty() {
                        let name = text.trim().to_string();
                        match kind {
                            PromptKind::NewCollection => {
                                let c = Collection::new(&name);
                                let id = c.id;
                                self.ws.collections.push(c);
                                self.ws.open_nodes.insert(id);
                                self.ws.sidebar_view = crate::app::SidebarView::Collections;
                                self.actions.push(Action::OpenTab(TabKind::Collection { coll: id }));
                            }
                            PromptKind::NewFolder { coll, parent } => {
                                let f = Folder { name, ..Default::default() };
                                let (coll, parent) = (*coll, *parent);
                                let fid = f.id;
                                if let Some(c) = self.collection_mut(coll) {
                                    match c.items_mut(parent) {
                                        Some(items) => items.push(Node::Folder(f)),
                                        None => c.items.push(Node::Folder(f)),
                                    }
                                }
                                self.ws.open_nodes.extend([coll, fid]);
                                if let Some(pid) = parent {
                                    self.ws.open_nodes.insert(pid);
                                }
                            }
                            PromptKind::NewEnvironment => {
                                let e = Environment { name, ..Default::default() };
                                let id = e.id;
                                self.ws.environments.push(e);
                                self.ws.sidebar_view = crate::app::SidebarView::Environments;
                                if self.ws.active_env.is_none() {
                                    self.ws.active_env = Some(id);
                                }
                                self.actions.push(Action::OpenTab(TabKind::Environment { env: id }));
                            }
                            PromptKind::Rename(t) => self.rename(*t, name),
                        }
                        close = true;
                    }
                }
                Dialog::Confirm { title, message, kind } => {
                    ui.set_width(400.0);
                    ui.label(RichText::new(title.as_str()).size(17.0).strong());
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(message.as_str()).color(p.muted)).wrap());
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let danger = egui::Button::new(RichText::new("Delete").strong().color(Color32::WHITE)).fill(p.red).corner_radius(7.0);
                        let label_ok = matches!(kind, ConfirmKind::Delete(_));
                        let b = if label_ok { danger } else { egui::Button::new(RichText::new("Clear").strong().color(Color32::WHITE)).fill(p.red).corner_radius(7.0) };
                        if ui.add(b).clicked() {
                            match kind {
                                ConfirmKind::Delete(t) => {
                                    let name = title.trim_start_matches("Delete ").trim_end_matches('?').to_string();
                                    self.delete(*t);
                                    self.notify_undo(format!("Deleted {name}"));
                                }
                                ConfirmKind::ClearHistory => self.ws.history.clear(),
                            }
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::SaveAs { tab, name, coll, folder, close_after } => {
                    ui.set_width(440.0);
                    ui.label(RichText::new("Save request").size(17.0).strong());
                    ui.add_space(10.0);
                    ui.label(RichText::new("Request name").size(12.0).color(p.muted));
                    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
                    ui.add_space(8.0);
                    if self.ws.collections.is_empty() {
                        ui.label(RichText::new("You have no collections yet. One will be created for you.").color(p.muted));
                    } else {
                        ui.label(RichText::new("Save to").size(12.0).color(p.muted));
                        let current = coll.and_then(|id| self.collection(id)).map(|c| c.name.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt("save_coll").width(400.0).selected_text(shape(&current)).show_ui(ui, |ui| {
                            for c in &self.ws.collections {
                                if ui.selectable_label(*coll == Some(c.id), shape(&c.name)).clicked() {
                                    *coll = Some(c.id);
                                    *folder = None;
                                }
                            }
                        });
                        if let Some(c) = coll.and_then(|id| self.collection(id)) {
                            let folders = c.folders();
                            if !folders.is_empty() {
                                let label = folder.and_then(|f| folders.iter().find(|(id, _)| *id == f)).map(|(_, l)| l.clone()).unwrap_or_else(|| "(collection root)".into());
                                ui.add_space(6.0);
                                egui::ComboBox::from_id_salt("save_folder").width(400.0).selected_text(shape(&label)).show_ui(ui, |ui| {
                                    ui.selectable_value(folder, None, "(collection root)");
                                    for (id, l) in &folders {
                                        ui.selectable_value(folder, Some(*id), shape(l));
                                    }
                                });
                            }
                        }
                    }
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::primary("Save")).clicked() && !name.trim().is_empty() {
                            let target = match *coll {
                                Some(c) => c,
                                None => {
                                    let c = Collection::new("My Collection");
                                    let id = c.id;
                                    self.ws.collections.push(c);
                                    id
                                }
                            };
                            self.save_as(*tab, name.trim().to_string(), target, *folder, *close_after);
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::CloseDirty { tab } => {
                    ui.set_width(420.0);
                    let title = self.tab(*tab).map(|t| self.tab_title(t)).unwrap_or_default();
                    ui.label(RichText::new("Unsaved changes").size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("\"{}\" has changes you haven't saved. Save them before closing?", shape(&title))).color(p.muted));
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::primary("Save")).clicked() {
                            let t = *tab;
                            close = true;
                            self.save_tab(t, true);
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.button(RichText::new("Don't save").color(p.red)).clicked() {
                            self.actions.push(Action::ForceCloseTab(*tab));
                            close = true;
                        }
                    });
                }
                Dialog::Import { text, error } => {
                    ui.set_width(560.0);
                    ui.label(RichText::new("Import").size(17.0).strong());
                    ui.add_space(4.0);
                    ui.label(RichText::new("Postman collections & environments, OpenAPI 3 / Swagger 2 (JSON), Insomnia exports, or a cURL command.").color(p.muted));
                    ui.add_space(12.0);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), egui::Sense::click());
                    let hovered = resp.hovered();
                    ui.painter().rect_filled(rect, 10.0, if hovered { p.hover } else { p.input });
                    ui.painter().rect_stroke(rect, 10.0, Stroke::new(1.5, if hovered { p.accent } else { p.border }), egui::StrokeKind::Inside);
                    ui.painter().text(rect.center() - egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "\u{E256}  Choose files", egui::FontId::proportional(15.0), p.text);
                    ui.painter().text(rect.center() + egui::vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, "or drop them anywhere on the window", egui::FontId::proportional(12.0), p.muted);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.pick_import_files();
                        close = true;
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("Or paste raw text").size(12.0).color(p.muted));
                    ui.add(egui::TextEdit::multiline(text).code_editor().desired_rows(7).desired_width(f32::INFINITY).hint_text("curl https://api.example.com/users -H 'Accept: application/json'").margin(Margin::same(10)));
                    if let Some(e) = error.as_ref() {
                        ui.label(RichText::new(e.as_str()).color(p.red));
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!text.trim().is_empty(), theme::primary("Import")).clicked() {
                            match self.import_text(text) {
                                Ok(()) => close = true,
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if response.should_close() {
            close = true;
        }
        if !close && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Runner
    // -----------------------------------------------------------------------------------------

    pub fn runner_window(&mut self, ctx: &egui::Context) {
        if !self.runner.open {
            return;
        }
        let p = theme::palette(ctx.global_style().visuals.dark_mode);
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Window::new(RichText::new("Collection Runner").strong())
            .open(&mut open)
            .frame(window_frame(ctx))
            .default_size([980.0_f32.min(screen.width() - 80.0), 620.0_f32.min(screen.height() - 120.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    // Left: configuration
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        theme::section_title(ui, "Run");
                        let coll_name = self.runner.coll.and_then(|id| self.collection(id)).map(|c| c.name.clone()).unwrap_or_else(|| "Choose a collection".into());
                        let mut coll = self.runner.coll;
                        egui::ComboBox::from_id_salt("run_coll").width(240.0).selected_text(shape(&coll_name)).show_ui(ui, |ui| {
                            for c in &self.ws.collections {
                                ui.selectable_value(&mut coll, Some(c.id), shape(&c.name));
                            }
                        });
                        if coll != self.runner.coll {
                            self.runner.coll = coll;
                            self.runner.folder = None;
                        }
                        if let Some(c) = self.runner.coll.and_then(|id| self.collection(id)).cloned() {
                            let folders = c.folders();
                            if !folders.is_empty() {
                                ui.add_space(4.0);
                                let label = self.runner.folder.and_then(|f| folders.iter().find(|(id, _)| *id == f)).map(|(_, l)| l.clone()).unwrap_or_else(|| "All requests".into());
                                let mut folder = self.runner.folder;
                                egui::ComboBox::from_id_salt("run_folder").width(240.0).selected_text(shape(&label)).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut folder, None, "All requests");
                                    for (id, l) in &folders {
                                        ui.selectable_value(&mut folder, Some(*id), shape(l));
                                    }
                                });
                                self.runner.folder = folder;
                            }
                            let reqs = c.requests_in(self.runner.folder);
                            let live = reqs.iter().filter(|r| r.kind != crate::collection::RequestKind::Http).count();
                            let n = reqs.len() - live;
                            let extra = if live > 0 { format!(" (skips {live} live connection{})", if live == 1 { "" } else { "s" }) } else { String::new() };
                            ui.label(RichText::new(format!("{n} requests, in order{extra}")).size(12.0).color(p.muted));
                        }
                        ui.add_space(10.0);
                        egui::Grid::new("run_opts").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                            ui.label("Iterations");
                            if self.runner.data.is_empty() {
                                ui.add(egui::DragValue::new(&mut self.runner.iterations).range(1..=1000));
                            } else {
                                ui.label(RichText::new(format!("{} (one per data row)", self.runner.data.len())).color(p.muted));
                            }
                            ui.end_row();
                            ui.label("Data file");
                            ui.horizontal(|ui| {
                                let label = if self.runner.data_path.is_empty() { "Select CSV / JSON".to_string() } else { format!("\u{E23A}  {}", crate::http::file_name(&self.runner.data_path)) };
                                if ui.button(label).on_hover_text("Each row is one iteration; its columns become {{variables}}").clicked() {
                                    if let Some(path) = rfd::FileDialog::new().add_filter("Data", &["csv", "json"]).pick_file() {
                                        let path = path.display().to_string();
                                        match crate::data::load(&path) {
                                            Ok(rows) => {
                                                self.runner.data = rows;
                                                self.runner.data_path = path;
                                            }
                                            Err(e) => self.actions.push(Action::Notify(e, ToastKind::Error)),
                                        }
                                    }
                                }
                                if !self.runner.data_path.is_empty() && theme::icon_button(ui, "×", "Remove data file").clicked() {
                                    self.runner.data.clear();
                                    self.runner.data_path.clear();
                                }
                            });
                            ui.end_row();
                            ui.label("Delay");
                            ui.add(egui::DragValue::new(&mut self.runner.delay_ms).range(0..=60_000).suffix(" ms").speed(10));
                            ui.end_row();
                        });
                        if let Some(first) = self.runner.data.first() {
                            let cols: Vec<String> = first.iter().map(|(k, _)| format!("{{{{{k}}}}}")).collect();
                            ui.add(egui::Label::new(RichText::new(format!("Variables from the file: {}", cols.join(" "))).size(11.5).color(p.accent)).wrap());
                        }
                        ui.checkbox(&mut self.runner.stop_on_fail, "Stop on first failure");
                        ui.add_space(12.0);
                        if self.runner.running {
                            if ui.add(egui::Button::new(RichText::new("\u{E46C}  Stop").strong()).min_size(egui::vec2(240.0, 34.0))).clicked() {
                                self.runner.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        } else {
                            let name = self.runner.coll.and_then(|id| self.collection(id)).map(|c| c.name.clone()).unwrap_or_default();
                            let label = format!("\u{E3D0}  Run {}", truncate(&name, 22));
                            if ui.add_enabled(self.runner.coll.is_some(), theme::primary(&label).min_size(egui::vec2(240.0, 34.0))).clicked() {
                                self.start_runner(ctx);
                            }
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("Requests run one after another. Captured variables (like a login token) feed the next requests. {{iteration}} holds the current iteration number.").size(11.5).color(p.faint));
                    });
                    ui.separator();
                    // Right: results
                    ui.vertical(|ui| self.runner_results(ui, &p));
                });
            });
        self.runner.open = open;
    }

    fn runner_results(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let r = &self.runner;
        if r.results.is_empty() && !r.running {
            theme::empty_state(
                ui,
                "\u{E3D0}",
                "Run a collection",
                "Pick a collection on the left and press Run. Every request is sent and its tests are checked.",
            );
            return;
        }
        let done = r.results.len();
        let passed_req = r.results.iter().filter(|x| x.passed()).count();
        let tests_total: usize = r.results.iter().map(|x| x.tests.len()).sum();
        let tests_passed: usize = r
            .results
            .iter()
            .map(|x| x.tests.iter().filter(|t| t.passed).count())
            .sum();
        let avg = if done > 0 {
            r.results.iter().map(|x| x.time_ms).sum::<u128>() / done as u128
        } else {
            0
        };
        let elapsed = if r.running {
            r.started.map(|s| s.elapsed()).unwrap_or_default()
        } else {
            r.elapsed
        };
        ui.horizontal(|ui| {
            let stat = |ui: &mut egui::Ui, value: String, label: &str, color: Color32| {
                theme::card(ui, |ui| {
                    ui.set_min_width(104.0);
                    ui.label(RichText::new(value).size(20.0).strong().color(color));
                    ui.label(RichText::new(label).size(11.5).color(p.muted));
                });
            };
            stat(
                ui,
                format!("{passed_req}/{done}"),
                "requests passed",
                if passed_req == done {
                    p.accent
                } else {
                    p.amber
                },
            );
            stat(
                ui,
                format!("{tests_passed}/{tests_total}"),
                "tests passed",
                if tests_passed == tests_total {
                    p.accent
                } else {
                    p.red
                },
            );
            stat(ui, format!("{avg} ms"), "avg. response", p.text);
            stat(
                ui,
                format!("{:.1} s", elapsed.as_secs_f32()),
                "duration",
                p.text,
            );
        });
        ui.add_space(8.0);
        let progress = if r.total > 0 {
            done as f32 / r.total as f32
        } else {
            0.0
        };
        let all_good = passed_req == done;
        ui.add(
            egui::ProgressBar::new(progress)
                .desired_height(6.0)
                .fill(if all_good { p.accent } else { p.amber }),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let mut f = self.runner.filter;
            theme::segmented(ui, &mut f, &[(RunFilter::All, "All"), (RunFilter::Passed, "Passed"), (RunFilter::Failed, "Failed")]);
            self.runner.filter = f;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.runner.running && ui.button("\u{EAF0}  Export results").clicked() {
                    let results: Vec<_> = self
                        .runner
                        .results
                        .iter()
                        .map(|x| {
                            json!({
                                "iteration": x.iteration, "name": x.name, "method": x.method, "url": x.url,
                                "status": x.status, "time_ms": x.time_ms as u64, "size": x.size, "passed": x.passed(), "error": x.error,
                                "tests": x.tests.iter().map(|t| json!({ "name": t.name, "passed": t.passed, "detail": t.detail })).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    let content = serde_json::to_string_pretty(&json!({ "results": results })).unwrap_or_default();
                    if let Some(path) = rfd::FileDialog::new().set_file_name("run-results.json").save_file() {
                        let msg = match std::fs::write(&path, content) {
                            Ok(()) => ("Saved the run results".to_string(), ToastKind::Success),
                            Err(e) => (format!("Could not save: {e}"), ToastKind::Error),
                        };
                        self.actions.push(Action::Notify(msg.0, msg.1));
                    }
                }
            });
        });
        ui.add_space(6.0);
        let filter = self.runner.filter;
        let iterations = self.runner.iterations;
        egui::ScrollArea::vertical()
            .id_salt("run_results")
            .auto_shrink([false, false])
            .stick_to_bottom(self.runner.running)
            .show(ui, |ui| {
                let mut last_iteration = 0;
                for (i, res) in self.runner.results.iter().enumerate() {
                    let passed = res.passed();
                    if (filter == RunFilter::Passed && !passed)
                        || (filter == RunFilter::Failed && passed)
                    {
                        continue;
                    }
                    if iterations > 1 && res.iteration != last_iteration {
                        last_iteration = res.iteration;
                        ui.add_space(4.0);
                        theme::section_title(ui, &format!("Iteration {}", res.iteration));
                    }
                    let expanded = self.runner.expanded.contains(&i);
                    let row = ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if passed { "\u{E182}" } else { "\u{E4F6}" }).color(if passed {
                                p.accent
                            } else {
                                p.red
                            }),
                        );
                        ui.add_sized(
                            [58.0, 20.0],
                            egui::Label::new(theme::method_text(p, &res.method, 11.0)),
                        );
                        ui.label(RichText::new(shape(&res.name)).color(p.text));
                        ui.add(
                            egui::Label::new(
                                RichText::new(truncate(&res.url, 60))
                                    .size(11.5)
                                    .color(p.faint),
                            )
                            .truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Fixed-width columns, so rows line up.
                            let tp = res.tests.iter().filter(|t| t.passed).count();
                            let tests = if res.tests.is_empty() {
                                String::new()
                            } else {
                                format!("{tp}/{} tests", res.tests.len())
                            };
                            let color = if tp == res.tests.len() {
                                p.accent
                            } else {
                                p.red
                            };
                            ui.add_sized(
                                [62.0, 20.0],
                                egui::Label::new(RichText::new(tests).size(11.5).color(color)),
                            );
                            ui.add_sized(
                                [58.0, 20.0],
                                egui::Label::new(
                                    RichText::new(format!("{} ms", res.time_ms))
                                        .size(11.5)
                                        .color(p.muted),
                                ),
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(44.0, 20.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| match res.status {
                                    Some(s) => theme::soft_badge(
                                        ui,
                                        s.to_string(),
                                        theme::status_color(p, Some(s)),
                                        11.0,
                                    ),
                                    None => theme::soft_badge(ui, "ERR", p.red, 11.0),
                                },
                            );
                        });
                    });
                    let resp = row.response.interact(egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(
                            resp.rect.expand(2.0),
                            6.0,
                            Color32::from_white_alpha(if p.dark { 6 } else { 0 }),
                        );
                    }
                    if resp.clicked() {
                        if expanded {
                            self.runner.expanded.remove(&i);
                        } else {
                            self.runner.expanded.insert(i);
                        }
                    }
                    if expanded {
                        egui::Frame::new()
                            .fill(p.input)
                            .corner_radius(8.0)
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                if let Some(e) = &res.error {
                                    ui.label(RichText::new(e).monospace().color(p.red));
                                }
                                if res.tests.is_empty() {
                                    ui.label(RichText::new("No tests").color(p.faint));
                                }
                                for t in &res.tests {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(if t.passed { "PASS" } else { "FAIL" })
                                                .size(11.0)
                                                .strong()
                                                .color(if t.passed { p.accent } else { p.red }),
                                        );
                                        ui.label(&t.name);
                                        ui.label(
                                            RichText::new(&t.detail).size(11.5).color(p.faint),
                                        );
                                    });
                                }
                            });
                    }
                    ui.separator();
                }
                if self.runner.running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(format!(
                                "Running {} of {}…",
                                done + 1,
                                self.runner.total
                            ))
                            .color(p.muted),
                        );
                    });
                }
            });
    }

    // -----------------------------------------------------------------------------------------
    // Settings, cookies, console
    // -----------------------------------------------------------------------------------------

    pub fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let p = theme::palette(ctx.global_style().visuals.dark_mode);
        let mut open = true;
        let section_id = egui::Id::unique("settings_section");
        let mut section: SettingsSection = ctx.data_mut(|d| d.get_temp(section_id).unwrap_or_default());
        egui::Window::new(RichText::new("Settings").strong())
            .open(&mut open)
            .frame(window_frame(ctx))
            .default_width(600.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(600.0);
                theme::tab_strip(
                    ui,
                    &mut section,
                    &[
                        (SettingsSection::General, "General", None),
                        (SettingsSection::Proxy, "Proxy", None),
                        (SettingsSection::Certificates, "Certificates", None),
                        (SettingsSection::Vault, "Vault", (!self.ws.vault.is_empty()).then(|| self.ws.vault.len().to_string())),
                        (SettingsSection::Shortcuts, "Shortcuts", None),
                    ],
                );
                ui.add_space(10.0);
                egui::ScrollArea::vertical().max_height(470.0).show(ui, |ui| match section {
                    SettingsSection::General => self.settings_general(ui, &p),
                    SettingsSection::Proxy => self.settings_proxy(ui, &p),
                    SettingsSection::Certificates => self.settings_certificates(ui, &p),
                    SettingsSection::Shortcuts => settings_shortcuts(ui, &p),
                    SettingsSection::Vault => {
                        ui.label(RichText::new("Secrets like API keys and passwords. They stay on this computer and are never part of an exported collection.").color(p.muted));
                        ui.label(RichText::new("Use them as {{vault:name}} in a request, or with await pm.vault.get(\"name\") in a script.").size(12.0).color(p.faint));
                        ui.add_space(6.0);
                        let mut list = self.ws.vault.clone();
                        let known = HashSet::new();
                        let opts = KvOptions { id: "vault".into(), key_hint: "Name", files: false, fixed_keys: false, known: &known };
                        if self.kv_table(ui, &mut list, opts) {
                            self.ws.vault = list;
                        }
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::draw_flag(ui, egui::vec2(21.0, 14.0));
                    ui.label(RichText::new("Gaza Man 2.0 · Free and open, made for students and teachers.").size(12.0).color(p.muted));
                });
            });
        ctx.data_mut(|d| d.insert_temp(section_id, section));
        self.show_settings = open;
    }

    fn settings_general(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        theme::section_title(ui, "Appearance");
        ui.horizontal(|ui| {
            ui.label("Theme");
            ui.selectable_value(&mut self.ws.dark, true, "\u{E330}  Dark");
            ui.selectable_value(&mut self.ws.dark, false, "\u{E472}  Light");
        });
        ui.add_space(10.0);
        theme::section_title(ui, "Requests");
        let s = &mut self.ws.settings;
        egui::Grid::new("settings_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Request timeout");
            ui.add(egui::DragValue::new(&mut s.timeout_secs).range(1..=600).suffix(" s"));
            ui.end_row();
            ui.label("Follow redirects");
            ui.checkbox(&mut s.follow_redirects, "");
            ui.end_row();
            ui.label("Max redirects");
            ui.add_enabled(s.follow_redirects, egui::DragValue::new(&mut s.max_redirects).range(1..=50));
            ui.end_row();
            ui.label("SSL certificate verification");
            ui.checkbox(&mut s.verify_ssl, "").on_hover_text("Turn off for local servers with self-signed certificates");
            ui.end_row();
            ui.label("Send cookies automatically");
            ui.checkbox(&mut s.use_cookies, "");
            ui.end_row();
            ui.label("Send User-Agent header");
            ui.checkbox(&mut s.send_user_agent, "");
            ui.end_row();
            ui.label("History size");
            ui.add(egui::DragValue::new(&mut s.history_limit).range(10..=2000));
            ui.end_row();
        });
        if !s.verify_ssl {
            ui.label(RichText::new("\u{E4E0}  SSL verification is off: only use this with servers you trust.").size(12.0).color(p.amber));
        }
    }

    fn settings_proxy(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let s = &mut self.ws.settings;
        ui.label(RichText::new("Send requests through a proxy, e.g. a company proxy or a debugging tool like Charles or mitmproxy.").color(p.muted));
        ui.add_space(8.0);
        ui.checkbox(&mut s.proxy_enabled, "Use a custom proxy");
        ui.add_enabled_ui(s.proxy_enabled, |ui| {
            ui.label(RichText::new("Proxy URL").size(12.0).color(p.muted));
            ui.add(
                egui::TextEdit::singleline(&mut s.proxy_url)
                    .hint_text("http://user:pass@proxy.local:8080   or   socks5://127.0.0.1:1080")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(8, 5)),
            );
        });
        ui.add_space(8.0);
        ui.checkbox(&mut s.use_system_proxy, "Otherwise use the system proxy (HTTP_PROXY / HTTPS_PROXY / ALL_PROXY)");
        ui.add_space(8.0);
        ui.label(RichText::new("Don't use a proxy for (comma-separated, *.domain works)").size(12.0).color(p.muted));
        ui.add(egui::TextEdit::singleline(&mut s.proxy_bypass).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).margin(Margin::symmetric(8, 5)));
        if s.proxy_enabled && !s.proxy_url.trim().is_empty() {
            if let Err(e) = ureq::Proxy::new(s.proxy_url.trim()) {
                ui.label(RichText::new(format!("\u{E4F6}  {e}")).size(12.0).color(p.red));
            }
        }
    }

    fn settings_certificates(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let pick = |current: &mut String, title: &str| {
            if let Some(path) = rfd::FileDialog::new().set_title(title).add_filter("PEM", &["pem", "crt", "cer", "key"]).pick_file() {
                *current = path.display().to_string();
            }
        };
        let file_label = |path: &str| if path.is_empty() { "Choose file…".to_string() } else { format!("\u{E39A}  {}", crate::http::file_name(path)) };
        let s = &mut self.ws.settings;
        theme::section_title(ui, "CA certificate");
        ui.label(RichText::new("Trust servers signed by your own certificate authority (PEM), on top of the public ones. Handy for company or local HTTPS servers.").color(p.muted));
        ui.horizontal(|ui| {
            if ui.button(file_label(&s.ca_cert)).on_hover_text(&s.ca_cert).clicked() {
                pick(&mut s.ca_cert, "CA certificate");
            }
            if !s.ca_cert.is_empty() && theme::icon_button(ui, "×", "Remove").clicked() {
                s.ca_cert.clear();
            }
        });
        ui.add_space(14.0);
        theme::section_title(ui, "Client certificates (mutual TLS)");
        ui.label(RichText::new("Sent to servers that ask who you are. One certificate per host.").color(p.muted));
        ui.add_space(4.0);
        let mut remove = None;
        for (i, c) in s.client_certs.iter_mut().enumerate() {
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut c.enabled, "");
                    ui.label(RichText::new("Host").size(12.0).color(p.muted));
                    ui.add(egui::TextEdit::singleline(&mut c.host).hint_text("api.example.com  or  *.example.com:8443").desired_width(300.0).font(egui::TextStyle::Monospace));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, "×", "Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Certificate").size(12.0).color(p.muted));
                    if ui.button(file_label(&c.cert)).on_hover_text(&c.cert).clicked() {
                        pick(&mut c.cert, "Client certificate (PEM)");
                    }
                    ui.label(RichText::new("Key").size(12.0).color(p.muted));
                    if ui.button(file_label(&c.key)).on_hover_text(&c.key).clicked() {
                        pick(&mut c.key, "Private key (PEM)");
                    }
                });
            });
            ui.add_space(4.0);
        }
        if let Some(i) = remove {
            s.client_certs.remove(i);
        }
        if ui.button("+  Add certificate").clicked() {
            s.client_certs.push(crate::http::ClientCertEntry { enabled: true, ..Default::default() });
        }
    }

    pub fn cookies_window(&mut self, ctx: &egui::Context) {
        if !self.show_cookies {
            return;
        }
        let p = theme::palette(ctx.global_style().visuals.dark_mode);
        let mut open = true;
        let mut cookies = self
            .cookies
            .lock()
            .map(|j| j.cookies.clone())
            .unwrap_or_default();
        let before = cookies.len();
        egui::Window::new(RichText::new("Cookies").strong())
            .open(&mut open)
            .frame(window_frame(ctx))
            .default_size([640.0, 420.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("Cookies set by servers are stored here and sent back automatically, like a browser does.").color(p.muted));
                ui.add_space(8.0);
                if cookies.is_empty() {
                    theme::empty_state(ui, "\u{E6CA}", "No cookies yet", "When a response sets a cookie, it shows up here.");
                    return;
                }
                let domains: Vec<String> = {
                    let mut d: Vec<String> = cookies.iter().map(|c| c.domain.clone()).collect();
                    d.sort();
                    d.dedup();
                    d
                };
                let mut remove: HashSet<(String, String, String)> = HashSet::new();
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(320.0).show(ui, |ui| {
                    for domain in &domains {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(domain).strong());
                            if ui.small_button("Remove all").clicked() {
                                for c in cookies.iter().filter(|c| &c.domain == domain) {
                                    remove.insert((c.domain.clone(), c.path.clone(), c.name.clone()));
                                }
                            }
                        });
                        for c in cookies.iter().filter(|c| &c.domain == domain) {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(RichText::new(&c.name).monospace().color(p.accent));
                                ui.label(RichText::new(truncate(&c.value, 50)).monospace().color(p.muted)).on_hover_text(&c.value);
                                ui.label(RichText::new(&c.path).size(11.5).color(p.faint));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if theme::icon_button(ui, "×", "Delete cookie").clicked() {
                                        remove.insert((c.domain.clone(), c.path.clone(), c.name.clone()));
                                    }
                                });
                            });
                        }
                        ui.add_space(6.0);
                    }
                });
                cookies.retain(|c| !remove.contains(&(c.domain.clone(), c.path.clone(), c.name.clone())));
                ui.add_space(8.0);
                if ui.button(RichText::new("Clear all cookies").color(p.red)).clicked() {
                    cookies.clear();
                }
            });
        if cookies.len() != before {
            if let Ok(mut j) = self.cookies.lock() {
                j.cookies = cookies;
            }
        }
        self.show_cookies = open;
    }

    pub fn console_panel(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let frame = egui::Frame::new()
            .fill(p.sidebar)
            .inner_margin(Margin::symmetric(14, 8));
        egui::Panel::bottom("console")
            .resizable(true)
            .default_size(220.0)
            .size_range(100.0..=600.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(">_  Console").strong());
                    ui.label(
                        RichText::new("Every request exactly as it was sent, and what came back.")
                            .size(12.0)
                            .color(p.faint),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, "×", "Close").clicked() {
                            self.show_console = false;
                        }
                        if theme::ghost(ui, RichText::new("Clear").size(12.0).color(p.muted))
                            .clicked()
                        {
                            self.console.clear();
                        }
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("console_scroll")
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.console.is_empty() {
                            ui.label(
                                RichText::new("Nothing logged yet. Send a request.").color(p.faint),
                            );
                        }
                        for (i, e) in self.console.iter().enumerate() {
                            let (status, color) = match &e.outcome {
                                Outcome::Response { status, .. } => {
                                    (status.to_string(), theme::status_color(&p, Some(*status)))
                                }
                                Outcome::Error { .. } => ("ERR".to_string(), p.red),
                            };
                            let time = crate::http::iso_time(e.at, 0)[11..19].to_string();
                            let header = RichText::new(format!(
                                "{time}   {}   {}   {status}   {} ms",
                                e.sent.method,
                                truncate(&e.sent.url, 90),
                                e.outcome.time_ms()
                            ))
                            .monospace()
                            .size(12.0)
                            .color(color);
                            // Script output around its request, like Postman's console:
                            // pre-request lines above it, post-response lines below.
                            let log_lines = |ui: &mut egui::Ui, phase: crate::script::Phase| {
                                for l in e.logs.iter().filter(|l| l.phase == phase) {
                                    let color = match l.level.as_str() {
                                        "error" => p.red,
                                        "warn" => p.amber,
                                        _ => p.text,
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("   › {}", crate::rtl::shape(&l.text)))
                                                .monospace()
                                                .size(11.5)
                                                .color(color),
                                        )
                                        .wrap(),
                                    )
                                    .on_hover_text(&l.script);
                                }
                            };
                            log_lines(ui, crate::script::Phase::PreRequest);
                            egui::CollapsingHeader::new(header)
                                .id_salt(("console", i, e.at))
                                .show(ui, |ui| {
                                    let mono = |ui: &mut egui::Ui, s: &str, c: Color32| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(s).monospace().size(11.5).color(c),
                                            )
                                            .wrap(),
                                        );
                                    };
                                    theme::section_title(ui, "Request headers");
                                    for (k, v) in &e.sent.headers {
                                        mono(ui, &format!("{k}: {v}"), p.muted);
                                    }
                                    if let Some(body) = &e.sent.body {
                                        theme::section_title(ui, "Request body");
                                        let text = match body {
                                            crate::http::PreparedBody::Raw(s) => s.clone(),
                                            crate::http::PreparedBody::UrlEncoded(f) => {
                                                crate::http::encode_urlencoded(f)
                                            }
                                            crate::http::PreparedBody::Multipart(parts) => parts
                                                .iter()
                                                .map(|p| {
                                                    format!(
                                                        "{}={}{}",
                                                        p.name,
                                                        if p.file { "@" } else { "" },
                                                        p.value
                                                    )
                                                })
                                                .collect::<Vec<_>>()
                                                .join("\n"),
                                            crate::http::PreparedBody::File(path) => {
                                                format!("<file {path}>")
                                            }
                                        };
                                        mono(ui, &truncate(&text, 4000), p.muted);
                                    }
                                    match &e.outcome {
                                        Outcome::Response { headers, body, .. } => {
                                            theme::section_title(ui, "Response headers");
                                            for (k, v) in headers {
                                                mono(ui, &format!("{k}: {v}"), p.muted);
                                            }
                                            theme::section_title(ui, "Response body");
                                            mono(ui, &truncate(body, 4000), p.text);
                                        }
                                        Outcome::Error { message, .. } => mono(ui, message, p.red),
                                    }
                                });
                            log_lines(ui, crate::script::Phase::PostResponse);
                        }
                    });
            });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsSection {
    #[default]
    General,
    Proxy,
    Certificates,
    Vault,
    Shortcuts,
}

fn settings_shortcuts(ui: &mut egui::Ui, p: &theme::Palette) {
    egui::Grid::new("keys_grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for (k, what) in [
            ("Ctrl+Enter", "Send request"),
            ("Ctrl+S", "Save request"),
            ("Ctrl+T", "New request tab"),
            ("Ctrl+W", "Close tab"),
            ("Ctrl+Shift+] / [", "Next / previous tab"),
            ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo changes to collections"),
            ("Ctrl+O", "Import"),
            ("Ctrl+K", "Search the sidebar"),
            ("Ctrl+B", "Show / hide sidebar"),
            ("Ctrl+Shift+R", "Collection runner"),
            ("Ctrl+Alt+C", "Console"),
            ("Ctrl+,", "Settings"),
            ("Ctrl+L", "Teaching lens"),
            ("Ctrl+= / Ctrl+-", "Zoom"),
        ] {
            ui.label(RichText::new(theme::shortcut(k)).monospace().color(p.accent));
            ui.label(what);
            ui.end_row();
        }
    });
}

fn count_folders(items: &[Node]) -> usize {
    items
        .iter()
        .map(|n| match n {
            Node::Folder(f) => 1 + count_folders(&f.items),
            Node::Request(_) => 0,
        })
        .sum()
}
