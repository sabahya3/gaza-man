//! The window chrome: top bar, the icon rail, the sidebar (collections, environments,
//! history) and the status bar.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke};

use crate::app::{Action, DropAt, GazaMan, SidebarView, TabKind, Target, ToastKind};
use crate::collection::{Collection, Id, Node};
use crate::export::Format;
use crate::rtl::shape;
use crate::theme::{self, Palette, pal};

const ROW_HEIGHT: f32 = 28.0;

/// What a dragged tab carries.
#[derive(Clone, Copy)]
struct TabDrag(Id);

/// What a dragged sidebar row carries.
#[derive(Clone, Copy)]
struct DragNode {
    coll: Id,
    node: Id,
}

enum RowIcon<'a> {
    Collection,
    Folder(bool),
    Method(&'a str),
}

struct RowOut {
    response: egui::Response,
    rect: egui::Rect,
}

/// One row of the tree: indent, caret, icon, text and an optional right-side badge.
fn tree_row(
    ui: &mut egui::Ui,
    p: &Palette,
    depth: usize,
    icon: RowIcon,
    label: &str,
    selected: bool,
    open: Option<bool>,
    trailing: Option<(String, Color32)>,
) -> RowOut {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let hovered = response.hovered() || response.context_menu_opened();
    if selected {
        painter.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6, p.accent_soft);
        painter.rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(4.0, 6.0),
                egui::vec2(2.5, rect.height() - 12.0),
            ),
            2,
            p.accent,
        );
    } else if hovered {
        painter.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6, p.hover);
    }

    let mut x = rect.left() + 12.0 + depth as f32 * 14.0;
    let cy = rect.center().y;
    if let Some(open) = open {
        let c = egui::pos2(x + 4.0, cy);
        let pts = if open {
            vec![
                c + egui::vec2(-3.5, -2.0),
                c + egui::vec2(3.5, -2.0),
                c + egui::vec2(0.0, 2.5),
            ]
        } else {
            vec![
                c + egui::vec2(-2.0, -3.5),
                c + egui::vec2(2.5, 0.0),
                c + egui::vec2(-2.0, 3.5),
            ]
        };
        painter.add(egui::Shape::convex_polygon(pts, p.muted, Stroke::NONE));
    }
    x += 14.0;
    match icon {
        RowIcon::Collection => {
            let r = egui::Rect::from_center_size(egui::pos2(x + 8.0, cy), egui::vec2(16.0, 13.0));
            painter.rect_filled(r, 3, if selected { p.accent } else { p.faint });
            painter.rect_filled(
                egui::Rect::from_min_size(r.min + egui::vec2(3.0, 3.0), egui::vec2(10.0, 1.5)),
                1,
                p.sidebar,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(r.min + egui::vec2(3.0, 6.5), egui::vec2(7.0, 1.5)),
                1,
                p.sidebar,
            );
            x += 24.0;
        }
        RowIcon::Folder(open) => {
            let r =
                egui::Rect::from_center_size(egui::pos2(x + 8.0, cy + 1.0), egui::vec2(15.0, 11.0));
            let tab = egui::Rect::from_min_size(r.min - egui::vec2(0.0, 2.5), egui::vec2(6.5, 4.0));
            let color = if open { p.amber } else { p.muted };
            painter.rect_filled(tab, 1, color);
            painter.rect_filled(r, 2, color);
            x += 24.0;
        }
        RowIcon::Method(m) => {
            let galley = painter.layout_no_wrap(
                theme::method_short(m).to_string(),
                FontId::monospace(10.0),
                theme::method_color(p, m),
            );
            painter.galley(
                egui::pos2(x, cy - galley.size().y / 2.0),
                galley,
                Color32::WHITE,
            );
            x += 40.0;
        }
    }

    let right_space = if trailing.is_some() { 48.0 } else { 8.0 };
    let max_w = (rect.right() - right_space - x).max(10.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_string(),
        FontId::proportional(13.0),
        if selected {
            p.text
        } else {
            p.text.gamma_multiply(0.92)
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, p.text);

    if let Some((text, color)) = trailing {
        let galley = painter.layout_no_wrap(text, FontId::proportional(10.5), color);
        let r = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 12.0 - galley.size().x - 10.0, cy - 8.0),
            egui::vec2(galley.size().x + 10.0, 16.0),
        );
        painter.rect_filled(
            r,
            4,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 30),
        );
        painter.galley(
            r.min + egui::vec2(5.0, 8.0 - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    RowOut { response, rect }
}

/// The drop indicator line, and where a drop on this row would land.
fn drop_target(
    ui: &egui::Ui,
    p: &Palette,
    out: &RowOut,
    is_folder: bool,
    folder_id: Option<Id>,
    node_id: Id,
) -> Option<DropAt> {
    let pointer = ui.ctx().pointer_hover_pos()?;
    if !out.rect.contains(pointer) {
        return None;
    }
    let painter = ui.painter();
    if is_folder && pointer.y > out.rect.top() + out.rect.height() * 0.3 {
        painter.rect_stroke(
            out.rect.shrink2(egui::vec2(4.0, 1.0)),
            6,
            Stroke::new(1.5, p.accent),
            egui::StrokeKind::Inside,
        );
        Some(DropAt::Into(folder_id))
    } else {
        let y = out.rect.top();
        painter.line_segment(
            [
                egui::pos2(out.rect.left() + 8.0, y),
                egui::pos2(out.rect.right() - 8.0, y),
            ],
            Stroke::new(2.0, p.accent),
        );
        Some(DropAt::Before(node_id))
    }
}

fn matches(query: &str, text: &str) -> bool {
    text.to_lowercase().contains(query)
}

fn node_matches(n: &Node, q: &str) -> bool {
    match n {
        Node::Request(r) => {
            matches(q, &r.name) || matches(q, &r.url) || matches(q, &r.method.to_lowercase())
        }
        Node::Folder(f) => matches(q, &f.name) || f.items.iter().any(|c| node_matches(c, q)),
    }
}

impl GazaMan {
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let ctx = ui.ctx().clone();
        let frame = egui::Frame::new().fill(p.sidebar).inner_margin(Margin {
            left: 14,
            right: 12,
            top: 7,
            bottom: 7,
        });
        egui::Panel::top("top")
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::draw_flag(ui, egui::vec2(24.0, 16.0));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Gaza Man").size(15.5).strong().color(p.text));
                    ui.add_space(14.0);

                    let f = self.ws.features;
                    ui.menu_button(RichText::new("+  New").color(p.text), |ui| {
                        ui.set_min_width(250.0);
                        // Each kind of request, with what it's for, like Postman's "New" dialog.
                        // Always all four, so they're easy to find. Picking one that's hidden in
                        // the current mode turns it on.
                        let kinds = [
                            (crate::app::NewKind::Http, "HTTP", "GET, POST, PUT… to a REST API", p.accent),
                            (crate::app::NewKind::GraphQl, "GraphQL", "A query with schema suggestions", p.pink),
                            (crate::app::NewKind::WebSocket, "WebSocket", "A live two-way connection (ws://)", p.teal),
                            (crate::app::NewKind::SocketIo, "Socket.IO", "Events over a Socket.IO server", p.purple),
                        ];
                        for (kind, name, what, color) in kinds {
                            let mut job = egui::text::LayoutJob::default();
                            job.append(name, 0.0, egui::TextFormat::simple(FontId::proportional(13.5), color));
                            job.append(&format!("\n{what}"), 0.0, egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
                            let r = ui.add(
                                egui::Button::new(egui::WidgetText::from(job))
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(240.0, 0.0)),
                            );
                            if r.clicked() {
                                self.actions.push(Action::NewTyped(kind));
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("\u{E466}  Collection").clicked() {
                            self.actions.push(Action::NewCollection);
                        }
                        if f.environments && ui.button("\u{E288}  Environment").clicked() {
                            self.actions.push(Action::NewEnvironment);
                        }
                        if ui.button("\u{E79E}  Demo collection").on_hover_text("Examples on real public APIs, numbered like a lesson").clicked() {
                            self.actions.push(Action::AddDemo);
                        }
                    });
                    if theme::ghost(ui, RichText::new("\u{E20C}  Import").color(p.text))
                        .on_hover_text(theme::shortcut("Import  (Ctrl+O)"))
                        .clicked()
                    {
                        self.actions.push(Action::Import);
                    }
                    if f.runner
                        && theme::ghost(ui, RichText::new("\u{E3D0}  Runner").color(p.text))
                            .on_hover_text(theme::shortcut("Collection Runner  (Ctrl+Shift+R)"))
                            .clicked()
                    {
                        if self.runner.coll.is_none() {
                            self.runner.coll = self.ws.collections.first().map(|c| c.id);
                        }
                        self.runner.open = true;
                    }
                    ui.add_space(6.0);
                    let undo = ui.add_enabled(self.can_undo(), egui::Button::new(RichText::new("\u{E08A}").size(15.0)).frame_when_inactive(false).min_size(egui::vec2(26.0, 26.0)));
                    if undo.on_hover_text(theme::shortcut("Undo  (Ctrl+Z)")).clicked() {
                        self.actions.push(Action::Undo);
                    }
                    let redo = ui.add_enabled(self.can_redo(), egui::Button::new(RichText::new("\u{E08C}").size(15.0)).frame_when_inactive(false).min_size(egui::vec2(26.0, 26.0)));
                    if redo.on_hover_text(theme::shortcut("Redo  (Ctrl+Shift+Z)")).clicked() {
                        self.actions.push(Action::Redo);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, "\u{E270}", &theme::shortcut("Settings  (Ctrl+,)"))
                            .clicked()
                        {
                            self.show_settings = true;
                        }
                        let theme_icon = if self.ws.dark { "\u{E472}" } else { "\u{E330}" };
                        if theme::icon_button(ui, theme_icon, "Switch light / dark theme").clicked()
                        {
                            self.ws.dark = !self.ws.dark;
                        }
                        self.mode_switch(ui);
                        if !f.teaching {
                            ui.add_space(6.0);
                            self.environment_picker(ui);
                            return;
                        }
                        let lens = if self.lens.enabled {
                            RichText::new("\u{E30C}").size(14.0).color(p.accent)
                        } else {
                            RichText::new("\u{E30C}").size(14.0).color(p.muted)
                        };
                        if ui
                            .add(
                                egui::Button::new(lens)
                                    .frame_when_inactive(self.lens.enabled)
                                    .min_size(egui::vec2(26.0, 26.0)),
                            )
                            .on_hover_text(theme::shortcut(
                                "Magnifying lens for teaching  (Ctrl+L)",
                            ))
                            .clicked()
                        {
                            self.lens.toggle();
                        }
                        let zoom = ctx.zoom_factor();
                        if theme::icon_button(ui, "+", &theme::shortcut("Zoom in  (Ctrl+=)"))
                            .clicked()
                        {
                            ctx.set_zoom_factor((zoom + 0.1).min(3.0));
                        }
                        if theme::ghost(
                            ui,
                            RichText::new(format!("{:.0}%", zoom * 100.0))
                                .size(12.0)
                                .color(p.muted),
                        )
                        .on_hover_text(theme::shortcut("Reset zoom  (Ctrl+0)"))
                        .clicked()
                        {
                            ctx.set_zoom_factor(1.0);
                        }
                        if theme::icon_button(ui, "\u{E32A}", &theme::shortcut("Zoom out  (Ctrl+-)"))
                            .clicked()
                        {
                            ctx.set_zoom_factor((zoom - 0.1).max(0.5));
                        }
                        ui.add_space(6.0);
                        self.environment_picker(ui);
                    });
                });
            });
        // A hairline under the bar.
        let rect = ui.max_rect();
        let _ = rect;
    }

    /// The variables quick look (always), and the environment picker (when Environments is on).
    fn environment_picker(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let show_envs = self.ws.features.environments;
        // The collection of the request being looked at, for its variables like {{baseUrl}}.
        let coll = self.ws.active_tab.and_then(|t| self.tab(t)).and_then(|t| match &t.kind {
            TabKind::Request { origin, .. } => origin.map(|o| o.coll),
            TabKind::Collection { coll } | TabKind::Folder { coll, .. } => Some(*coll),
            _ => None,
        });
        let coll = coll.and_then(|c| self.collection(c)).map(|c| (c.id, c.name.clone(), c.variables.clone()));
        // Quick look at the variables in scope.
        let eye = theme::icon_button(ui, "\u{E220}", "Quick look at variables");
        egui::Popup::menu(&eye).show(|ui| {
            ui.set_min_width(340.0);
            ui.set_max_width(460.0);
            if let Some((id, name, vars)) = &coll {
                theme::section_title(ui, &format!("Collection · {}", shape(name)));
                if vars.is_empty() {
                    ui.weak("No collection variables");
                } else {
                    vars_grid(ui, &p, "coll_quick", vars);
                }
                if ui.button("Edit collection variables").clicked() {
                    self.actions.push(Action::OpenTab(TabKind::Collection { coll: *id }));
                    ui.close();
                }
                ui.add_space(6.0);
            }
            let env = self.active_env().cloned();
            theme::section_title(
                ui,
                &format!(
                    "Environment · {}",
                    env.as_ref().map(|e| e.name.as_str()).unwrap_or("none")
                ),
            );
            match &env {
                Some(e) if !e.values.is_empty() => vars_grid(ui, &p, "env_quick", &e.values),
                Some(_) => {
                    ui.weak("No variables yet");
                }
                None => {
                    ui.weak("No environment selected");
                }
            }
            ui.add_space(6.0);
            theme::section_title(ui, "Globals");
            if self.ws.globals.is_empty() {
                ui.weak("No global variables");
            } else {
                vars_grid(ui, &p, "globals_quick", &self.ws.globals);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(e) = &env {
                    if ui.button("Edit environment").clicked() {
                        self.actions
                            .push(Action::OpenTab(TabKind::Environment { env: e.id }));
                        ui.close();
                    }
                }
                if ui.button("Edit globals").clicked() {
                    self.actions.push(Action::OpenTab(TabKind::Globals));
                    ui.close();
                }
            });
        });

        if !show_envs {
            return;
        }
        let current = self
            .active_env()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "No Environment".into());
        let dot = if self.ws.active_env.is_some() {
            p.accent
        } else {
            p.faint
        };
        egui::ComboBox::from_id_salt("env_picker")
            .width(190.0)
            .selected_text(RichText::new(format!("●  {current}")).color(
                if self.ws.active_env.is_some() {
                    p.text
                } else {
                    p.muted
                },
            ))
            .show_ui(ui, |ui| {
                let mut active = self.ws.active_env;
                ui.selectable_value(&mut active, None, "No Environment");
                for e in &self.ws.environments {
                    ui.selectable_value(&mut active, Some(e.id), shape(&e.name));
                }
                ui.separator();
                if ui.button("+  New environment").clicked() {
                    self.actions.push(Action::NewEnvironment);
                }
                if ui.button("Manage environments").clicked() {
                    self.ws.sidebar_view = SidebarView::Environments;
                    self.ws.show_sidebar = true;
                }
                if active != self.ws.active_env {
                    self.actions.push(Action::SetActiveEnv(active));
                }
            });
        let _ = dot;
    }

    pub fn rail(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let frame = egui::Frame::new()
            .fill(p.sidebar)
            .inner_margin(Margin::symmetric(6, 10))
            .stroke(Stroke::new(1.0, p.border));
        egui::Panel::left("rail")
            .exact_size(68.0)
            .resizable(false)
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    let f = self.ws.features;
                    let shown = |v: SidebarView| match v {
                        SidebarView::Environments => f.environments,
                        SidebarView::History => f.history,
                        _ => true,
                    };
                    if !shown(self.ws.sidebar_view) {
                        self.ws.sidebar_view = SidebarView::Collections;
                    }
                    for (view, icon, label) in [
                        (SidebarView::Collections, "\u{E466}", "Collections"),
                        (SidebarView::Environments, "\u{E288}", "Environments"),
                        (SidebarView::History, "\u{E1A0}", "History"),
                    ] {
                        if !shown(view) {
                            continue;
                        }
                        let selected = self.ws.sidebar_view == view && self.ws.show_sidebar;
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(56.0, 52.0), Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if selected {
                            ui.painter().rect_filled(rect, 8, p.accent_soft);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 8, p.hover);
                        }
                        let color = if selected { p.accent } else { p.muted };
                        ui.painter().text(
                            rect.center() - egui::vec2(0.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            icon,
                            FontId::proportional(18.0),
                            color,
                        );
                        ui.painter().text(
                            rect.center() + egui::vec2(0.0, 14.0),
                            egui::Align2::CENTER_CENTER,
                            label,
                            FontId::proportional(9.5),
                            color,
                        );
                        if resp.clicked() {
                            if self.ws.sidebar_view == view {
                                self.ws.show_sidebar = !self.ws.show_sidebar;
                            } else {
                                self.ws.sidebar_view = view;
                                self.ws.show_sidebar = true;
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
            });
    }

    pub fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let frame = egui::Frame::new()
            .fill(p.sidebar)
            .inner_margin(Margin {
                left: 6,
                right: 6,
                top: 10,
                bottom: 6,
            })
            .stroke(Stroke::new(1.0, p.border));
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(290.0)
            .size_range(220.0..=520.0)
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                let (title, add_hint) = match self.ws.sidebar_view {
                    SidebarView::Collections => ("Collections", "New collection"),
                    SidebarView::Environments => ("Environments", "New environment"),
                    SidebarView::History => ("History", ""),
                };
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).size(14.0).strong().color(p.text));
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| match self.ws.sidebar_view {
                            SidebarView::History => {
                                if theme::ghost(
                                    ui,
                                    RichText::new("Clear").size(12.0).color(p.muted),
                                )
                                .clicked()
                                    && !self.ws.history.is_empty()
                                {
                                    self.dialog = Some(crate::app::Dialog::Confirm {
                                        title: "Clear history?".into(),
                                        message: "All requests in your history will be removed."
                                            .into(),
                                        kind: crate::app::ConfirmKind::ClearHistory,
                                    });
                                }
                            }
                            view => {
                                if theme::icon_button(ui, "\u{E20C}", "Import").clicked() {
                                    self.actions.push(Action::Import);
                                }
                                if theme::icon_button(ui, "+", add_hint).clicked() {
                                    self.actions.push(if view == SidebarView::Collections {
                                        Action::NewCollection
                                    } else {
                                        Action::NewEnvironment
                                    });
                                }
                            }
                        },
                    );
                });
                ui.add_space(4.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.sidebar_search)
                        .hint_text(
                            RichText::new(theme::shortcut("\u{E30C}  Search  (Ctrl+K)")).color(p.faint),
                        )
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(8, 5)),
                );
                if self.focus_search {
                    search.request_focus();
                    self.focus_search = false;
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        match self.ws.sidebar_view {
                            SidebarView::Collections => self.collections_list(ui),
                            SidebarView::Environments => self.environments_list(ui),
                            SidebarView::History => self.history_list(ui),
                        }
                    });
            });
    }

    fn collections_list(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        if self.ws.collections.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No collections yet").strong());
                ui.add(egui::Label::new(RichText::new("Collections group your requests. Create one, or import from Postman, OpenAPI or cURL.").color(p.muted)).wrap());
                ui.add_space(8.0);
                if ui.add(theme::primary("Create collection")).clicked() {
                    self.actions.push(Action::NewCollection);
                }
                if ui.button("Import").clicked() {
                    self.actions.push(Action::Import);
                }
            });
            return;
        }
        let query = self.sidebar_search.trim().to_lowercase();
        let active_req = self
            .ws
            .active_tab
            .and_then(|t| self.tab(t))
            .and_then(|t| match &t.kind {
                TabKind::Request {
                    origin: Some(o), ..
                } => Some(o.req),
                TabKind::Folder { folder, .. } => Some(*folder),
                _ => None,
            });
        let active_coll =
            self.ws
                .active_tab
                .and_then(|t| self.tab(t))
                .and_then(|t| match &t.kind {
                    TabKind::Collection { coll } => Some(*coll),
                    _ => None,
                });
        // Clone so drawing can push actions and toggle folders while reading the tree.
        let collections = self.ws.collections.clone();
        for c in &collections {
            if !query.is_empty()
                && !matches(&query, &c.name)
                && !c.items.iter().any(|n| node_matches(n, &query))
            {
                continue;
            }
            let open = self.ws.open_nodes.contains(&c.id) || !query.is_empty();
            let count = c.count_requests();
            let out = tree_row(
                ui,
                &p,
                0,
                RowIcon::Collection,
                &shape(&c.name),
                active_coll == Some(c.id),
                Some(open),
                self.mocks.get(&c.id).map(|m| (format!("MOCK :{}", m.port), p.accent)),
            );
            let resp = out
                .response
                .clone()
                .on_hover_text(format!("{count} requests"));
            if resp.clicked() {
                if self.ws.open_nodes.contains(&c.id) {
                    self.ws.open_nodes.remove(&c.id);
                } else {
                    self.ws.open_nodes.insert(c.id);
                }
            }
            if resp.double_clicked() {
                self.actions
                    .push(Action::OpenTab(TabKind::Collection { coll: c.id }));
            }
            if let Some(drag) = resp.dnd_hover_payload::<DragNode>() {
                let _ = drag;
                ui.painter().rect_stroke(
                    out.rect.shrink2(egui::vec2(4.0, 1.0)),
                    6,
                    Stroke::new(1.5, p.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(drag) = resp.dnd_release_payload::<DragNode>() {
                self.actions.push(Action::Move {
                    from: drag.coll,
                    node: drag.node,
                    to: c.id,
                    drop: DropAt::Into(None),
                });
            }
            resp.context_menu(|ui| self.collection_menu(ui, c));
            if open {
                if c.items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(52.0);
                        ui.label(
                            RichText::new("This collection is empty.")
                                .size(12.0)
                                .color(p.muted),
                        );
                        if ui.link(RichText::new("Add a request").size(12.0)).clicked() {
                            self.actions.push(Action::AddRequest {
                                coll: c.id,
                                folder: None,
                            });
                        }
                    });
                    ui.add_space(4.0);
                }
                self.tree_nodes(ui, &p, c, &c.items, 1, &query, active_req);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_nodes(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        c: &Collection,
        items: &[Node],
        depth: usize,
        query: &str,
        active: Option<Id>,
    ) {
        for n in items {
            if !query.is_empty() && !node_matches(n, query) {
                continue;
            }
            match n {
                Node::Folder(f) => {
                    let open = self.ws.open_nodes.contains(&f.id) || !query.is_empty();
                    let out = tree_row(
                        ui,
                        p,
                        depth,
                        RowIcon::Folder(open),
                        &shape(&f.name),
                        active == Some(f.id),
                        Some(open),
                        None,
                    );
                    let resp = out.response.clone();
                    if resp.clicked() {
                        if self.ws.open_nodes.contains(&f.id) {
                            self.ws.open_nodes.remove(&f.id);
                        } else {
                            self.ws.open_nodes.insert(f.id);
                        }
                    }
                    if resp.double_clicked() {
                        self.actions.push(Action::OpenTab(TabKind::Folder {
                            coll: c.id,
                            folder: f.id,
                        }));
                    }
                    resp.dnd_set_drag_payload(DragNode {
                        coll: c.id,
                        node: f.id,
                    });
                    if resp.dnd_hover_payload::<DragNode>().is_some() {
                        drop_target(ui, p, &out, true, Some(f.id), f.id);
                    }
                    if let Some(drag) = resp.dnd_release_payload::<DragNode>() {
                        if let Some(at) = drop_target(ui, p, &out, true, Some(f.id), f.id) {
                            self.actions.push(Action::Move {
                                from: drag.coll,
                                node: drag.node,
                                to: c.id,
                                drop: at,
                            });
                        }
                    }
                    resp.context_menu(|ui| self.folder_menu(ui, c.id, f.id));
                    if open {
                        self.tree_nodes(ui, p, c, &f.items, depth + 1, query, active);
                    }
                }
                Node::Request(r) => {
                    let status = self.last_status.get(&r.id).copied();
                    let pending = self.ws.tabs.iter().any(|t| {
                        matches!(&t.kind, TabKind::Request { origin: Some(o), .. } if o.req == r.id)
                            && self.pending.contains_key(&t.id)
                    });
                    let trailing = if pending {
                        Some(("···".to_string(), p.muted))
                    } else {
                        status.map(|s| {
                            (
                                s.map(|s| s.to_string()).unwrap_or_else(|| "ERR".into()),
                                theme::status_color(p, s),
                            )
                        })
                    };
                    let out = tree_row(
                        ui,
                        p,
                        depth,
                        RowIcon::Method(theme::request_label(r)),
                        &shape(&r.name),
                        active == Some(r.id),
                        None,
                        trailing,
                    );
                    let resp = out.response.clone().on_hover_text(if r.url.is_empty() {
                        "No URL yet".to_string()
                    } else {
                        r.url.clone()
                    });
                    if resp.clicked() || resp.double_clicked() {
                        self.actions.push(Action::OpenRequest {
                            coll: c.id,
                            req: r.id,
                        });
                    }
                    resp.dnd_set_drag_payload(DragNode {
                        coll: c.id,
                        node: r.id,
                    });
                    if resp.dnd_hover_payload::<DragNode>().is_some() {
                        drop_target(ui, p, &out, false, None, r.id);
                    }
                    if let Some(drag) = resp.dnd_release_payload::<DragNode>() {
                        if drag.node != r.id {
                            self.actions.push(Action::Move {
                                from: drag.coll,
                                node: drag.node,
                                to: c.id,
                                drop: DropAt::Before(r.id),
                            });
                        }
                    }
                    resp.context_menu(|ui| self.request_menu(ui, c, r.id));
                }
            }
        }
    }

    fn collection_menu(&mut self, ui: &mut egui::Ui, c: &Collection) {
        ui.set_min_width(200.0);
        let id = c.id;
        if ui.button("Add request").clicked() {
            self.actions.push(Action::AddRequest {
                coll: id,
                folder: None,
            });
        }
        if ui.button("Add folder").clicked() {
            self.actions.push(Action::AddFolder {
                coll: id,
                parent: None,
            });
        }
        if self.mocks.contains_key(&id) {
            if ui.button("\u{E46C}  Stop mock server").clicked() {
                self.actions.push(Action::StopMock(id));
            }
        } else if ui.button("\u{E2A0}  Start mock server").clicked() {
            self.actions.push(Action::StartMock(id));
        }
        if self.ws.features.runner && ui.button("\u{E3D0}  Run collection").clicked() {
            self.actions.push(Action::Run {
                coll: id,
                folder: None,
            });
        }
        ui.separator();
        if ui.button("Edit (auth, variables, docs)").clicked() {
            self.actions
                .push(Action::OpenTab(TabKind::Collection { coll: id }));
        }
        if ui.button("Rename").clicked() {
            self.actions.push(Action::Rename(Target::Collection(id)));
        }
        if ui.button("Duplicate").clicked() {
            self.actions.push(Action::Duplicate(Target::Collection(id)));
        }
        ui.menu_button("Export", |ui| {
            ui.set_min_width(260.0);
            for f in Format::ALL {
                if ui.button(f.label()).clicked() {
                    self.actions.push(Action::Export(id, f));
                }
            }
        });
        ui.separator();
        if ui
            .button(RichText::new("Delete").color(pal(ui).red))
            .clicked()
        {
            self.actions.push(Action::Delete(Target::Collection(id)));
        }
    }

    fn folder_menu(&mut self, ui: &mut egui::Ui, coll: Id, folder: Id) {
        ui.set_min_width(200.0);
        if ui.button("Add request").clicked() {
            self.actions.push(Action::AddRequest {
                coll,
                folder: Some(folder),
            });
        }
        if ui.button("Add folder").clicked() {
            self.actions.push(Action::AddFolder {
                coll,
                parent: Some(folder),
            });
        }
        if self.ws.features.runner && ui.button("\u{E3D0}  Run folder").clicked() {
            self.actions.push(Action::Run {
                coll,
                folder: Some(folder),
            });
        }
        ui.separator();
        if ui.button("Edit (auth, docs)").clicked() {
            self.actions
                .push(Action::OpenTab(TabKind::Folder { coll, folder }));
        }
        if ui.button("Rename").clicked() {
            self.actions
                .push(Action::Rename(Target::Node(coll, folder)));
        }
        if ui.button("Duplicate").clicked() {
            self.actions
                .push(Action::Duplicate(Target::Node(coll, folder)));
        }
        if ui.button("Move up").clicked() {
            self.actions.push(Action::Shift {
                coll,
                node: folder,
                delta: -1,
            });
        }
        if ui.button("Move down").clicked() {
            self.actions.push(Action::Shift {
                coll,
                node: folder,
                delta: 1,
            });
        }
        ui.separator();
        if ui
            .button(RichText::new("Delete").color(pal(ui).red))
            .clicked()
        {
            self.actions
                .push(Action::Delete(Target::Node(coll, folder)));
        }
    }

    fn request_menu(&mut self, ui: &mut egui::Ui, c: &Collection, req: Id) {
        ui.set_min_width(200.0);
        let coll = c.id;
        if ui.button("Open").clicked() {
            self.actions.push(Action::OpenRequest { coll, req });
        }
        if ui.button("Copy as cURL").clicked() {
            if let Some(r) = c.request(req) {
                let p = crate::http::prepare(
                    r,
                    &c.effective_auth(req, &r.auth),
                    &self.vars_for(Some(coll)),
                );
                ui.ctx().copy_text(crate::codegen::curl(&p));
                self.actions
                    .push(Action::Notify("Copied cURL".into(), ToastKind::Success));
            }
        }
        ui.separator();
        if ui.button("Rename").clicked() {
            self.actions.push(Action::Rename(Target::Node(coll, req)));
        }
        if ui.button("Duplicate").clicked() {
            self.actions
                .push(Action::Duplicate(Target::Node(coll, req)));
        }
        if ui.button("Move up").clicked() {
            self.actions.push(Action::Shift {
                coll,
                node: req,
                delta: -1,
            });
        }
        if ui.button("Move down").clicked() {
            self.actions.push(Action::Shift {
                coll,
                node: req,
                delta: 1,
            });
        }
        ui.separator();
        if ui
            .button(RichText::new("Delete").color(pal(ui).red))
            .clicked()
        {
            self.actions.push(Action::Delete(Target::Node(coll, req)));
        }
    }

    fn environments_list(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let query = self.sidebar_search.trim().to_lowercase();
        let globals_open = matches!(
            self.ws
                .active_tab
                .and_then(|t| self.tab(t))
                .map(|t| &t.kind),
            Some(TabKind::Globals)
        );
        let out = tree_row(
            ui,
            &p,
            0,
            RowIcon::Collection,
            "Globals",
            globals_open,
            None,
            Some((self.ws.globals.len().to_string(), p.muted)),
        );
        if out.response.clicked() {
            self.actions.push(Action::OpenTab(TabKind::Globals));
        }
        ui.add_space(6.0);
        if self.ws.environments.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No environments yet").strong());
                ui.add(egui::Label::new(RichText::new("An environment is a set of variables, like baseUrl for local vs. production.").color(p.muted)).wrap());
                ui.add_space(8.0);
                if ui.add(theme::primary("Create environment")).clicked() {
                    self.actions.push(Action::NewEnvironment);
                }
            });
            return;
        }
        let open_env = self
            .ws
            .active_tab
            .and_then(|t| self.tab(t))
            .and_then(|t| match &t.kind {
                TabKind::Environment { env } => Some(*env),
                _ => None,
            });
        let envs: Vec<(Id, String, usize)> = self
            .ws
            .environments
            .iter()
            .map(|e| (e.id, e.name.clone(), e.values.len()))
            .collect();
        for (id, name, count) in envs {
            if !query.is_empty() && !matches(&query, &name) {
                continue;
            }
            let active = self.ws.active_env == Some(id);
            let width = ui.available_width();
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
            let painter = ui.painter_at(rect);
            if open_env == Some(id) {
                painter.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6, p.accent_soft);
            } else if resp.hovered() || resp.context_menu_opened() {
                painter.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6, p.hover);
            }
            // The radio dot sets the active environment.
            let dot = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 22.0, rect.center().y),
                egui::vec2(18.0, 18.0),
            );
            let dot_resp = ui
                .interact(dot, ui.scope_id().with(("env_dot", id)), Sense::click())
                .on_hover_text(if active {
                    "Active environment"
                } else {
                    "Set as active"
                });
            painter.circle_stroke(
                dot.center(),
                6.0,
                Stroke::new(1.5, if active { p.accent } else { p.faint }),
            );
            if active {
                painter.circle_filled(dot.center(), 3.5, p.accent);
            }
            painter.text(
                egui::pos2(rect.left() + 40.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                shape(&name),
                FontId::proportional(13.0),
                p.text,
            );
            painter.text(
                egui::pos2(rect.right() - 14.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                count.to_string(),
                FontId::proportional(11.0),
                p.faint,
            );
            if dot_resp.clicked() {
                self.actions
                    .push(Action::SetActiveEnv(if active { None } else { Some(id) }));
            } else if resp.clicked() {
                self.actions
                    .push(Action::OpenTab(TabKind::Environment { env: id }));
            }
            resp.context_menu(|ui| {
                ui.set_min_width(180.0);
                if ui
                    .button(if active {
                        "Deactivate"
                    } else {
                        "Set as active"
                    })
                    .clicked()
                {
                    self.actions
                        .push(Action::SetActiveEnv(if active { None } else { Some(id) }));
                }
                if ui.button("Edit").clicked() {
                    self.actions
                        .push(Action::OpenTab(TabKind::Environment { env: id }));
                }
                if ui.button("Rename").clicked() {
                    self.actions.push(Action::Rename(Target::Environment(id)));
                }
                if ui.button("Duplicate").clicked() {
                    self.actions
                        .push(Action::Duplicate(Target::Environment(id)));
                }
                if ui.button("Export").clicked() {
                    self.actions.push(Action::ExportEnvironment(id));
                }
                ui.separator();
                if ui.button(RichText::new("Delete").color(p.red)).clicked() {
                    self.actions.push(Action::Delete(Target::Environment(id)));
                }
            });
        }
    }

    fn history_list(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        if self.ws.history.is_empty() {
            theme::empty_state(
                ui,
                "\u{E1A0}",
                "No history yet",
                "Requests you send show up here.",
            );
            return;
        }
        let query = self.sidebar_search.trim().to_lowercase();
        let today = crate::http::now_secs() / 86_400;
        let mut last_day = None;
        let entries: Vec<_> = self
            .ws
            .history
            .iter()
            .map(|h| {
                (
                    h.id,
                    h.at,
                    h.request.method.clone(),
                    h.request.url.clone(),
                    h.request.name.clone(),
                    h.status,
                    h.time_ms,
                )
            })
            .collect();
        for (id, at, method, url, name, status, time_ms) in entries {
            if !query.is_empty()
                && !matches(&query, &url)
                && !matches(&query, &name)
                && !matches(&query, &method.to_lowercase())
            {
                continue;
            }
            let day = at / 86_400;
            if last_day != Some(day) {
                last_day = Some(day);
                let label = match today.saturating_sub(day) {
                    0 => "Today".to_string(),
                    1 => "Yesterday".to_string(),
                    _ => crate::http::iso_time(at, 0)[..10].to_string(),
                };
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    theme::section_title(ui, &label);
                });
                ui.add_space(2.0);
            }
            let label = if url.is_empty() {
                name.clone()
            } else {
                url.clone()
            };
            let trailing = Some((
                status
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "ERR".into()),
                theme::status_color(&p, status),
            ));
            let out = tree_row(
                ui,
                &p,
                0,
                RowIcon::Method(&method),
                &label,
                false,
                None,
                trailing,
            );
            let resp = out.response.on_hover_text(format!(
                "{name}\n{} · {time_ms} ms",
                crate::http::iso_time(at, 0)
                    .replace('T', " ")
                    .trim_end_matches(".000Z")
            ));
            if resp.clicked() {
                self.actions.push(Action::OpenHistory(id));
            }
            resp.context_menu(|ui| {
                if ui.button("Open in new tab").clicked() {
                    self.actions.push(Action::OpenHistory(id));
                }
                if ui.button(RichText::new("Delete").color(p.red)).clicked() {
                    self.actions.push(Action::Delete(Target::History(id)));
                }
            });
        }
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let frame = egui::Frame::new()
            .fill(p.sidebar)
            .inner_margin(Margin::symmetric(10, 3))
            .stroke(Stroke::new(1.0, p.border));
        egui::Panel::bottom("status")
            .frame(frame)
            .show_separator_line(false)
            .exact_size(28.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let sidebar_label = if self.ws.show_sidebar {
                        "\u{EC24}  Hide sidebar"
                    } else {
                        "\u{EC24}  Show sidebar"
                    };
                    if theme::ghost(ui, RichText::new(sidebar_label).size(11.5).color(p.muted))
                        .on_hover_text(theme::shortcut("Ctrl+B"))
                        .clicked()
                    {
                        self.ws.show_sidebar = !self.ws.show_sidebar;
                    }
                    let console = RichText::new(format!(">_  Console ({})", self.console.len()))
                        .size(11.5)
                        .color(if self.show_console { p.accent } else { p.muted });
                    if !self.ws.features.console {
                        self.show_console = false;
                    }
                    if self.ws.features.console
                        && theme::ghost(ui, console)
                            .on_hover_text(theme::shortcut("Ctrl+Alt+C"))
                            .clicked()
                    {
                        self.show_console = !self.show_console;
                    }
                    let n_cookies = self.cookies.lock().map(|j| j.cookies.len()).unwrap_or(0);
                    if self.ws.features.cookies && theme::ghost(
                        ui,
                        RichText::new(format!("\u{E6CA}  Cookies ({n_cookies})"))
                            .size(11.5)
                            .color(p.muted),
                    )
                    .clicked()
                    {
                        self.show_cookies = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Gaza Man 2.0").size(11.0).color(p.faint));
                        ui.add_space(10.0);
                        let reqs: usize = self
                            .ws
                            .collections
                            .iter()
                            .map(Collection::count_requests)
                            .sum();
                        ui.label(
                            RichText::new(format!(
                                "{} collections · {reqs} requests",
                                self.ws.collections.len()
                            ))
                            .size(11.0)
                            .color(p.faint),
                        );
                    });
                });
            });
    }

    pub fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let height = 38.0;
        let (bar, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(bar, 0, p.sidebar);
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0, p.border),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        egui::ScrollArea::horizontal()
            .id_salt("tabs_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let tabs: Vec<(Id, String, bool, Option<String>, bool)> = self
                    .ws
                    .tabs
                    .iter()
                    .map(|t| {
                        let (method, is_req) = match &t.kind {
                            TabKind::Request { draft, .. } => (Some(theme::request_label(draft).to_string()), true),
                            _ => (None, false),
                        };
                        (t.id, self.tab_title(t), self.is_dirty(t), method, is_req)
                    })
                    .collect();
                for (id, title, dirty, method, _) in tabs {
                    let active = self.ws.active_tab == Some(id);
                    let width = 190.0;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
                    // Drag a tab onto another to reorder.
                    resp.dnd_set_drag_payload(TabDrag(id));
                    if resp.dnd_hover_payload::<TabDrag>().is_some_and(|d| d.0 != id) {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.5, rect.height())),
                            0,
                            p.accent,
                        );
                    }
                    if let Some(d) = resp.dnd_release_payload::<TabDrag>() {
                        self.actions.push(Action::MoveTab { tab: d.0, before: id });
                    }
                    let painter = ui.painter_at(rect);
                    if active {
                        painter.rect_filled(rect, CornerRadius::ZERO, p.surface);
                        painter.rect_filled(
                            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 2.0)),
                            0,
                            p.accent,
                        );
                    } else if resp.hovered() {
                        painter.rect_filled(rect, 0, p.hover);
                    }
                    painter.line_segment(
                        [
                            rect.right_top() + egui::vec2(0.0, 8.0),
                            rect.right_bottom() - egui::vec2(0.0, 8.0),
                        ],
                        Stroke::new(1.0, p.border),
                    );
                    let mut x = rect.left() + 12.0;
                    let cy = rect.center().y;
                    match &method {
                        Some(m) => {
                            let g = painter.layout_no_wrap(
                                theme::method_short(m).to_string(),
                                FontId::monospace(10.0),
                                theme::method_color(&p, m),
                            );
                            painter.galley(
                                egui::pos2(x, cy - g.size().y / 2.0),
                                g.clone(),
                                Color32::WHITE,
                            );
                            x += g.size().x + 8.0;
                        }
                        None => {
                            painter.text(
                                egui::pos2(x, cy),
                                egui::Align2::LEFT_CENTER,
                                "\u{E1EC}",
                                FontId::proportional(11.0),
                                p.accent,
                            );
                            x += 18.0;
                        }
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        shape(&title),
                        FontId::proportional(12.5),
                        if active { p.text } else { p.muted },
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.right() - 30.0 - x);
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, p.text);

                    let close_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 16.0, cy),
                        egui::vec2(18.0, 18.0),
                    );
                    let close = ui.interact(
                        close_rect,
                        ui.scope_id().with(("close_tab", id)),
                        Sense::click(),
                    );
                    if close.hovered() {
                        painter.rect_filled(close_rect, 4, p.hover);
                    }
                    if dirty && !close.hovered() && !resp.hovered() {
                        painter.circle_filled(close_rect.center(), 3.5, p.amber);
                    } else if resp.hovered() || close.hovered() || active {
                        painter.text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "×",
                            FontId::proportional(15.0),
                            p.muted,
                        );
                    }
                    if close.clicked() || resp.middle_clicked() {
                        self.actions.push(Action::CloseTab(id));
                    } else if resp.clicked() {
                        self.actions.push(Action::FocusTab(id));
                    }
                    resp.on_hover_text(&title).context_menu(|ui| {
                        ui.set_min_width(170.0);
                        if ui.button("Duplicate tab").clicked() {
                            self.actions.push(Action::DuplicateTab(id));
                        }
                        if ui.button("Close tab").clicked() {
                            self.actions.push(Action::CloseTab(id));
                        }
                        if ui.button("Close other tabs").clicked() {
                            self.actions.push(Action::CloseOtherTabs(id));
                        }
                    });
                }
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(38.0, height), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect.shrink(6.0), 6, p.hover);
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "+",
                    FontId::proportional(15.0),
                    p.muted,
                );
                if resp
                    .on_hover_text(theme::shortcut("New request  (Ctrl+T)"))
                    .clicked()
                {
                    self.actions.push(Action::NewRequestTab);
                }
            });
    }

    pub fn welcome(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.22).max(20.0));
            theme::draw_flag(ui, egui::vec2(84.0, 56.0));
            ui.add_space(14.0);
            ui.label(RichText::new("Gaza Man").size(30.0).strong().color(p.text));
            ui.label(
                RichText::new("Build, test and document APIs. Made with love for Palestine.")
                    .color(p.muted),
            );
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                let w = 3.0 * 150.0 + 2.0 * 8.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if ui
                    .add(theme::primary("+  New request").min_size(egui::vec2(150.0, 34.0)))
                    .clicked()
                {
                    self.actions.push(Action::NewRequestTab);
                }
                if ui
                    .add(egui::Button::new("\u{E20C}  Import").min_size(egui::vec2(150.0, 34.0)))
                    .clicked()
                {
                    self.actions.push(Action::Import);
                }
                if ui
                    .add(egui::Button::new("\u{E466}  New collection").min_size(egui::vec2(150.0, 34.0)))
                    .clicked()
                {
                    self.actions.push(Action::NewCollection);
                }
            });
            ui.add_space(26.0);
            for (keys, what) in [
                ("Ctrl+T", "New request"),
                ("Ctrl+Enter", "Send"),
                ("Ctrl+S", "Save"),
                ("Ctrl+O", "Import"),
                ("Ctrl+K", "Search"),
                ("Ctrl+L", "Teaching lens"),
            ] {
                ui.label(
                    RichText::new(format!("{}   {what}", theme::shortcut(keys)))
                        .color(p.faint)
                        .size(12.0),
                );
            }
        });
    }
}

fn vars_grid(ui: &mut egui::Ui, p: &Palette, id: &str, vars: &[crate::collection::KeyValue]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for v in vars {
                let color = if v.enabled { p.text } else { p.faint };
                ui.label(RichText::new(&v.key).monospace().color(color));
                let value = if v.value.chars().count() > 48 {
                    format!("{}…", v.value.chars().take(48).collect::<String>())
                } else {
                    v.value.clone()
                };
                ui.label(RichText::new(value).monospace().color(p.muted));
                ui.end_row();
            }
        });
}

impl GazaMan {
    /// One button: it shows the mode (Simple, Full or Custom) and opens the list of features.
    fn mode_switch(&mut self, ui: &mut egui::Ui) {
        use crate::features::Features;
        let p = pal(ui);
        let f = self.ws.features;
        let label = if f.is_simple() {
            "Simple"
        } else if f.is_full() {
            "Full"
        } else {
            "Custom"
        };
        let text = RichText::new(format!("\u{E434}  {label}  \u{E136}"))
            .size(12.0)
            .color(if f.is_simple() { p.muted } else { p.accent });
        let button = ui
            .add(egui::Button::new(text).frame_when_inactive(true))
            .on_hover_text({
                let (on, total) = f.count();
                format!("Choose what's shown ({on} of {total} features)")
            });
        egui::Popup::menu(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(340.0);
                let mut mode = if f.is_simple() { 0 } else if f.is_full() { 1 } else { 2 };
                let before = mode;
                ui.horizontal(|ui| {
                    theme::segmented(ui, &mut mode, &[(0, "Simple"), (1, "Full"), (2, "Custom")]);
                });
                if mode != before {
                    match mode {
                        0 => self.ws.features = Features::SIMPLE,
                        1 => self.ws.features = Features::FULL,
                        _ => {}
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Hiding a feature only hides it: nothing is deleted, and a request that already uses it still shows it.")
                        .size(11.5)
                        .color(p.muted),
                );
                let mut group = "";
                for item in Features::items() {
                    if item.group != group {
                        group = item.group;
                        ui.add_space(6.0);
                        theme::section_title(ui, group);
                    }
                    let value = (item.get)(&mut self.ws.features);
                    theme::toggle(ui, value, item.label, item.help);
                }
            });
    }
}
