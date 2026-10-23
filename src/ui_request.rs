//! The request editor (URL bar, params, auth, headers, body, tests, captures, docs),
//! the response viewer and the code snippet panel.

use std::collections::HashSet;

use eframe::egui::{
    self, Color32, FontId, Margin, RichText, Stroke, TextFormat,
    text::{ByteIndex, LayoutJob},
};
use serde_json::Value;

use crate::app::{
    Action, BodyMode, GazaMan, METHODS, Origin, ReqTab, ResTab, Tab, TabKind, ToastKind,
};
use crate::codegen::{self, Lang};
use crate::collection::{
    Assertion, Auth, BodyType, Capture, KeyValue, Op, RawLang, Request, RequestKind, Scope, Source,
};
use crate::http::{self, Outcome};
use crate::rtl::shape;
use crate::tests;
use crate::theme::{self, Palette, pal};

// Highlighting very large bodies every frame gets slow, so we show them as plain text.
const HIGHLIGHT_LIMIT: usize = 200_000;
const TREE_LIMIT: usize = 4_000;

pub fn known_vars(vars: &[KeyValue]) -> HashSet<String> {
    let mut set: HashSet<String> = vars
        .iter()
        .filter(|v| v.enabled)
        .map(|v| v.key.clone())
        .collect();
    set.extend(http::DYNAMIC_VARS.iter().map(|(n, _)| n.to_string()));
    set
}

/// A one-line explanation of a status code, handy when teaching HTTP.
pub fn status_meaning(status: u16) -> &'static str {
    match status {
        200 => "OK: the request worked and the response has the result",
        201 => "Created: a new resource was created (usually after POST)",
        202 => "Accepted: the server accepted the request but hasn't finished it yet",
        204 => "No Content: it worked and there's nothing to send back",
        301 | 308 => "Moved permanently: the resource has a new URL",
        302 | 307 => "Found / temporary redirect: try another URL for now",
        304 => "Not Modified: use the copy you already have",
        400 => "Bad Request: the data you sent is wrong or incomplete",
        401 => "Unauthorized: you need to log in or send a valid token",
        403 => "Forbidden: you're known, but not allowed to do this",
        404 => "Not Found: there's nothing at this URL (or with this id)",
        405 => "Method Not Allowed: this URL doesn't accept this method",
        409 => "Conflict: the request clashes with the current data",
        415 => "Unsupported Media Type: check the Content-Type header",
        422 => "Unprocessable: the data has the right shape but fails validation",
        429 => "Too Many Requests: slow down and try again later",
        500 => "Internal Server Error: something broke on the server",
        502 => "Bad Gateway: a server in between got a bad response",
        503 => "Service Unavailable: the server is down or overloaded",
        504 => "Gateway Timeout: a server in between took too long",
        100..=199 => "Informational: the server is still working on it",
        200..=299 => "Success",
        300..=399 => "Redirect: the resource is somewhere else",
        400..=499 => "Client error: something is wrong with the request",
        _ => "Server error: something went wrong on the server",
    }
}

// ---------------------------------------------------------------------------------------------
// Text highlighting
// ---------------------------------------------------------------------------------------------

/// Colors `{{variables}}` (green when defined, red when not) and `:path` variables.
pub fn highlight_vars(
    text: &str,
    known: &HashSet<String>,
    p: &Palette,
    font: FontId,
    path_vars: bool,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(font.clone(), p.text);
    let var_fmt = |ok: bool| TextFormat {
        font_id: font.clone(),
        color: if ok { p.accent } else { p.red },
        background: if ok {
            p.accent_soft
        } else {
            Color32::from_rgba_unmultiplied(p.red.r(), p.red.g(), p.red.b(), 28)
        },
        ..Default::default()
    };
    let path_fmt = TextFormat {
        font_id: font.clone(),
        color: p.blue,
        ..Default::default()
    };
    let query_start = text.find('?').unwrap_or(text.len());
    let mut rest = text;
    let mut offset = 0;
    while !rest.is_empty() {
        if let Some(start) = rest.find("{{") {
            if let Some(len) = rest[start + 2..].find("}}") {
                push_plain(
                    &mut job,
                    &rest[..start],
                    &plain,
                    &path_fmt,
                    path_vars,
                    offset,
                    query_start,
                );
                let name = rest[start + 2..start + 2 + len].trim();
                job.append(
                    &rest[start..start + 4 + len],
                    0.0,
                    var_fmt(known.contains(name)),
                );
                offset += start + 4 + len;
                rest = &rest[start + 4 + len..];
                continue;
            }
        }
        push_plain(
            &mut job,
            rest,
            &plain,
            &path_fmt,
            path_vars,
            offset,
            query_start,
        );
        break;
    }
    job
}

/// Plain text, with `/:name` path segments colored before the query string.
fn push_plain(
    job: &mut LayoutJob,
    s: &str,
    plain: &TextFormat,
    path_fmt: &TextFormat,
    path_vars: bool,
    offset: usize,
    query_start: usize,
) {
    if !path_vars || !s.contains("/:") {
        job.append(s, 0.0, plain.clone());
        return;
    }
    let mut i = 0;
    let bytes = s.as_bytes();
    let mut last = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && i > 0 && bytes[i - 1] == b'/' && offset + i < query_start {
            let end = s[i + 1..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
                .map(|e| i + 1 + e)
                .unwrap_or(s.len());
            if end > i + 1 {
                job.append(&s[last..i], 0.0, plain.clone());
                job.append(&s[i..end], 0.0, path_fmt.clone());
                last = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    job.append(&s[last..], 0.0, plain.clone());
}

/// Colors a JSON string for display.
pub fn highlight_json(text: &str, p: &Palette, size: f32) -> LayoutJob {
    let font = FontId::monospace(size);
    let (plain, key, string, number, keyword) = if p.dark {
        (
            Color32::from_gray(210),
            Color32::from_rgb(0x8C, 0xB4, 0xFF),
            Color32::from_rgb(0x7F, 0xD6, 0xA4),
            Color32::from_rgb(0xF5, 0xB0, 0x6B),
            Color32::from_rgb(0xD9, 0x8B, 0xF2),
        )
    } else {
        (
            Color32::from_gray(40),
            Color32::from_rgb(0x1F, 0x5F, 0xCC),
            Color32::from_rgb(0x1A, 0x7F, 0x4B),
            Color32::from_rgb(0xB8, 0x5C, 0x00),
            Color32::from_rgb(0x8A, 0x3F, 0xC2),
        )
    };
    let mut job = LayoutJob::default();
    let mut push =
        |s: &str, color: Color32| job.append(s, 0.0, TextFormat::simple(font.clone(), color));

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1 != '"' {
                j += if chars[j].1 == '\\' { 2 } else { 1 };
            }
            j = (j + 1).min(chars.len());
            let mut k = j;
            while k < chars.len() && chars[k].1 == ' ' {
                k += 1;
            }
            let is_key = chars.get(k).map(|c| c.1) == Some(':');
            push(&text[start..end_of(j)], if is_key { key } else { string });
            i = j;
        } else if c == '-' || c.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '0'..='9' | '.' | 'e' | 'E' | '+' | '-') {
                j += 1;
            }
            push(&text[start..end_of(j)], number);
            i = j;
        } else if c.is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1.is_ascii_alphabetic() {
                j += 1;
            }
            push(&text[start..end_of(j)], keyword);
            i = j;
        } else {
            let mut j = i + 1;
            while j < chars.len()
                && !matches!(chars[j].1, '"' | '-' | '0'..='9' | 'a'..='z' | 'A'..='Z')
            {
                j += 1;
            }
            push(&text[start..end_of(j)], plain);
            i = j;
        }
    }
    job
}

const KEYWORDS: [&str; 64] = [
    "const",
    "let",
    "var",
    "await",
    "async",
    "function",
    "return",
    "new",
    "import",
    "from",
    "require",
    "package",
    "func",
    "main",
    "if",
    "else",
    "err",
    "nil",
    "defer",
    "fn",
    "use",
    "pub",
    "mut",
    "Ok",
    "Some",
    "None",
    "True",
    "False",
    "true",
    "false",
    "null",
    "def",
    "print",
    "public",
    "class",
    "static",
    "void",
    "throws",
    "using",
    "val",
    "fun",
    "println",
    "echo",
    "curl",
    "http",
    "wget",
    "puts",
    "end",
    "do",
    "then",
    "catch",
    "try",
    "for",
    "in",
    "of",
    "struct",
    "string",
    "int",
    "String",
    "self",
    "this",
    "type",
    "interface",
    "Invoke-RestMethod",
];

/// A light, language-agnostic highlighter for generated snippets.
pub fn highlight_code(text: &str, p: &Palette, size: f32) -> LayoutJob {
    let font = FontId::monospace(size);
    let (plain, string, comment, number, keyword) = if p.dark {
        (
            Color32::from_gray(212),
            Color32::from_rgb(0x7F, 0xD6, 0xA4),
            Color32::from_gray(110),
            Color32::from_rgb(0xF5, 0xB0, 0x6B),
            Color32::from_rgb(0xD9, 0x8B, 0xF2),
        )
    } else {
        (
            Color32::from_gray(40),
            Color32::from_rgb(0x1A, 0x7F, 0x4B),
            Color32::from_gray(140),
            Color32::from_rgb(0xB8, 0x5C, 0x00),
            Color32::from_rgb(0x8A, 0x3F, 0xC2),
        )
    };
    let mut job = LayoutJob::default();
    let mut push =
        |s: &str, color: Color32| job.append(s, 0.0, TextFormat::simple(font.clone(), color));
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        let (start, c) = chars[i];
        let next = chars.get(i + 1).map(|c| c.1);
        if c == '/' && next == Some('/') || (c == '#' && line_start) {
            let mut j = i;
            while j < chars.len() && chars[j].1 != '\n' {
                j += 1;
            }
            push(&text[start..end_of(j)], comment);
            i = j;
        } else if matches!(c, '"' | '\'' | '`') {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1 != c {
                j += if chars[j].1 == '\\' { 2 } else { 1 };
            }
            j = (j + 1).min(chars.len());
            push(&text[start..end_of(j)], string);
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].1.is_ascii_digit() || chars[j].1 == '.') {
                j += 1;
            }
            push(&text[start..end_of(j)], number);
            i = j;
        } else if c.is_alphabetic()
            || c == '_'
            || c == '$'
            || c == '-' && next.is_some_and(char::is_alphabetic)
        {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].1.is_alphanumeric() || matches!(chars[j].1, '_' | '-'))
            {
                j += 1;
            }
            let word = &text[start..end_of(j)];
            push(
                word,
                if KEYWORDS.contains(&word) || word.starts_with('-') {
                    keyword
                } else {
                    plain
                },
            );
            i = j;
        } else {
            push(&text[start..end_of(i + 1)], plain);
            i += 1;
        }
        line_start = c == '\n' || (line_start && c.is_whitespace());
    }
    job
}

/// Paints a background behind every case-insensitive match of `query`.
fn mark_matches(job: LayoutJob, text: &str, query: &str, color: Color32) -> LayoutJob {
    if query.is_empty() || !text.is_ascii() && !query.is_ascii() {
        return job;
    }
    let hay = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        ranges.push(from + i..from + i + needle.len());
        from += i + needle.len().max(1);
        if ranges.len() > 5000 {
            break;
        }
    }
    if ranges.is_empty() {
        return job;
    }
    let mut out = LayoutJob {
        text: job.text.clone(),
        wrap: job.wrap.clone(),
        halign: job.halign,
        justify: job.justify,
        ..Default::default()
    };
    for s in &job.sections {
        let (mut pos, end) = (s.byte_range.start.0, s.byte_range.end.0);
        while pos < end {
            let inside = ranges.iter().find(|r| r.start <= pos && pos < r.end);
            let next_edge = match inside {
                Some(r) => r.end.min(end),
                None => ranges
                    .iter()
                    .map(|r| r.start)
                    .filter(|st| *st > pos)
                    .min()
                    .unwrap_or(end)
                    .min(end),
            };
            let mut format = s.format.clone();
            if inside.is_some() {
                format.background = color;
            }
            out.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(pos)..ByteIndex(next_edge),
                format,
            });
            pos = next_edge;
        }
    }
    out
}

fn count_matches(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    text.to_ascii_lowercase()
        .matches(&query.to_ascii_lowercase())
        .count()
}

// ---------------------------------------------------------------------------------------------
// Key / value tables
// ---------------------------------------------------------------------------------------------

pub struct KvOptions<'a> {
    pub id: String,
    pub key_hint: &'a str,
    pub files: bool,
    /// Keys come from elsewhere (path variables): no add, remove or toggle.
    pub fixed_keys: bool,
    pub known: &'a HashSet<String>,
}

fn var_edit<'t>(
    text: &'t mut dyn egui::TextBuffer,
    hint: &str,
    known: &HashSet<String>,
    p: &Palette,
    width: f32,
) -> impl egui::Widget + 't {
    let known = known.clone();
    let p = *p;
    let hint = RichText::new(hint.to_string()).color(p.faint);
    move |ui: &mut egui::Ui| {
        let id = ui.auto_id_with("var_edit");
        let editing = ui.memory(|m| m.has_focus(id));
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = highlight_vars(buf.as_str(), &known, &p, FontId::monospace(12.5), false);
            // Arabic reads right only when drawn reordered, which editing can't do.
            if !editing {
                job = crate::rtl::shape_job(job);
            }
            // Single-line fields scroll instead of wrapping.
            let _ = wrap;
            job.wrap.max_width = f32::INFINITY;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let resp = ui.add_sized(
            [width, 28.0],
            egui::TextEdit::singleline(&mut *text)
                .id(id)
                .hint_text(hint)
                .margin(Margin::symmetric(6, 4))
                .layouter(&mut layouter),
        );
        if editing {
            crate::rtl::preview(ui, resp.rect, text.as_str(), id);
        }
        resp
    }
}

fn to_bulk(rows: &[KeyValue]) -> String {
    rows.iter()
        .map(|r| format!("{}{}:{}", if r.enabled { "" } else { "//" }, r.key, r.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn from_bulk(text: &str, old: &[KeyValue]) -> Vec<KeyValue> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (enabled, l) = match l.trim_start().strip_prefix("//") {
                Some(rest) => (false, rest),
                None => (true, l),
            };
            let (k, v) = l.split_once(':').unwrap_or((l, ""));
            let file = old.iter().any(|o| o.key == k.trim() && o.file);
            KeyValue {
                key: k.trim().to_string(),
                value: v.trim().to_string(),
                enabled,
                file,
            }
        })
        .collect()
}

impl GazaMan {
    /// A Postman-style key/value table with an always-present empty row at the bottom.
    pub fn kv_table(
        &mut self,
        ui: &mut egui::Ui,
        rows: &mut Vec<KeyValue>,
        opts: KvOptions,
    ) -> bool {
        let p = pal(ui);
        let mut changed = false;
        if !opts.fixed_keys {
            let bulk = self.bulk_edit.contains_key(&opts.id);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if bulk { "Key-Value Edit" } else { "Bulk Edit" };
                    if ui.link(RichText::new(label).size(12.0)).clicked() {
                        if bulk {
                            self.bulk_edit.remove(&opts.id);
                        } else {
                            self.bulk_edit.insert(opts.id.clone(), to_bulk(rows));
                        }
                    }
                });
            });
            if let Some(buffer) = self.bulk_edit.get_mut(&opts.id) {
                ui.label(
                    RichText::new("One per line as key:value. Start a line with // to disable it.")
                        .size(11.5)
                        .color(p.muted),
                );
                let resp = ui.add(
                    egui::TextEdit::multiline(buffer)
                        .code_editor()
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::same(8)),
                );
                if resp.changed() {
                    *rows = from_bulk(buffer, rows);
                    changed = true;
                }
                return changed;
            }
        }

        let fixed = 22.0 + 30.0 + if opts.files { 72.0 } else { 0.0 } + 3.0 * 6.0;
        let w = ((ui.available_width() - fixed) / 2.0).max(90.0);
        let mut remove = None;
        egui::Grid::new(&opts.id)
            .num_columns(if opts.files { 5 } else { 4 })
            .spacing([6.0, 5.0])
            .min_row_height(28.0)
            .show(ui, |ui| {
                ui.label("");
                ui.label(
                    RichText::new(if opts.fixed_keys { "Variable" } else { "Key" })
                        .size(11.5)
                        .color(p.faint),
                );
                if opts.files {
                    ui.label("");
                }
                ui.label(RichText::new("Value").size(11.5).color(p.faint));
                ui.label("");
                ui.end_row();

                let n = rows.len();
                // One extra, empty row: typing in it adds a new row.
                let mut ghost = KeyValue::default();
                for i in 0..=n {
                    let is_ghost = i == n;
                    if is_ghost && opts.fixed_keys {
                        break;
                    }
                    let row = if is_ghost { &mut ghost } else { &mut rows[i] };
                    if opts.fixed_keys || is_ghost {
                        ui.label("");
                    } else {
                        changed |= ui
                            .add(egui::Checkbox::without_text(&mut row.enabled))
                            .on_hover_text("Include")
                            .changed();
                    }
                    if opts.fixed_keys {
                        ui.add_sized(
                            [w, 26.0],
                            egui::Label::new(
                                RichText::new(format!(":{}", row.key))
                                    .monospace()
                                    .color(p.blue),
                            )
                            .truncate(),
                        );
                    } else {
                        changed |= ui
                            .add(var_edit(&mut row.key, opts.key_hint, opts.known, &p, w))
                            .changed();
                    }
                    if opts.files {
                        let label = if row.file { "File" } else { "Text" };
                        egui::ComboBox::from_id_salt((&opts.id, i, "kind"))
                            .width(62.0)
                            .selected_text(RichText::new(label).size(12.0))
                            .show_ui(ui, |ui| {
                                changed |=
                                    ui.selectable_value(&mut row.file, false, "Text").changed();
                                changed |=
                                    ui.selectable_value(&mut row.file, true, "File").changed();
                            });
                    }
                    if row.file {
                        ui.horizontal(|ui| {
                            let name = if row.value.is_empty() {
                                "Select a file".to_string()
                            } else {
                                http::file_name(&row.value).to_string()
                            };
                            if ui
                                .add_sized(
                                    [w - 4.0, 26.0],
                                    egui::Button::new(
                                        RichText::new(format!("\u{E39A}  {name}")).size(12.0),
                                    ),
                                )
                                .on_hover_text(&row.value)
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new().pick_file() {
                                    row.value = path.display().to_string();
                                    changed = true;
                                }
                            }
                        });
                    } else {
                        changed |= ui
                            .add(var_edit(&mut row.value, "Value", opts.known, &p, w))
                            .changed();
                    }
                    if is_ghost || opts.fixed_keys {
                        ui.label("");
                    } else if theme::icon_button(ui, "×", "Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
                if !ghost.key.is_empty() || !ghost.value.is_empty() || ghost.file {
                    rows.push(ghost);
                    changed = true;
                }
            });
        if let Some(i) = remove {
            rows.remove(i);
            changed = true;
        }
        changed
    }

    // -----------------------------------------------------------------------------------------
    // Request tab
    // -----------------------------------------------------------------------------------------

    pub fn request_tab(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let tab_id = tab.id;
        let TabKind::Request { draft, origin } = &mut tab.kind else {
            return;
        };
        let origin = *origin;
        let coll = origin
            .map(|o| o.coll)
            .filter(|c| self.collection(*c).is_some());
        let vars = self.vars_for(coll);
        let known = known_vars(&vars);
        let p = pal(ui);
        let saved = self.origin_request(origin).is_some();
        let dirty = match self.origin_request(origin) {
            Some(s) => s != draft,
            None => true,
        };
        let busy = self.pending.contains_key(&tab_id);

        if self.ws.show_code && self.ws.features.code {
            let frame = egui::Frame::new()
                .fill(p.sidebar)
                .inner_margin(Margin::same(14));
            egui::Panel::right("code_panel")
                .resizable(true)
                .default_size(400.0)
                .size_range(280.0..=760.0)
                .frame(frame)
                .show(ui, |ui| {
                    self.code_panel(ui, draft, origin);
                });
        }

        egui::Frame::new()
            .inner_margin(Margin {
                left: 20,
                right: 20,
                top: 12,
                bottom: 4,
            })
            .show(ui, |ui| {
                self.request_header(ui, tab_id, draft, origin, dirty, saved);
                ui.add_space(8.0);
                self.url_bar(ui, tab_id, draft, &vars, &known, busy);
                // Without the Docs tab, the description still shows: it's often the explanation.
                if !self.ws.features.docs && !draft.description.trim().is_empty() {
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(shape(draft.description.trim())).size(12.5).color(p.muted)).wrap());
                }
                ui.add_space(6.0);

                let current = self.req_tabs.entry(tab_id).or_insert_with(|| {
                    if draft.body_type != BodyType::None || draft.kind != RequestKind::Http {
                        ReqTab::Body
                    } else {
                        ReqTab::Params
                    }
                });
                let count = |n: usize| (n > 0).then(|| n.to_string());
                let params = draft
                    .params
                    .iter()
                    .filter(|p| p.enabled && !p.key.is_empty())
                    .count()
                    + draft.path_vars.len();
                let headers = draft
                    .headers
                    .iter()
                    .filter(|h| h.enabled && !h.key.is_empty())
                    .count();
                let socket = draft.kind != RequestKind::Http;
                let body_dot = if socket {
                    (!draft.ws_message.trim().is_empty()).then(|| "●".to_string())
                } else {
                    (draft.body_type != BodyType::None).then(|| "●".to_string())
                };
                let auth_dot =
                    (!matches!(draft.auth, Auth::Inherit | Auth::None)).then(|| "●".to_string());
                let docs_dot = (!draft.description.is_empty()).then(|| "●".to_string());
                let tests_n = draft.tests.iter().filter(|t| t.enabled).count();
                let caps_n = draft.captures.iter().filter(|c| c.enabled).count();
                // Hidden features stay visible on a request that already uses them.
                let f = self.ws.features;
                let scripts = scripts_dot(&draft.events);
                let mut items = vec![
                    (ReqTab::Params, "Params", count(params)),
                    (ReqTab::Auth, "Authorization", auth_dot),
                    (ReqTab::Headers, "Headers", count(headers)),
                    (ReqTab::Body, if socket { "Message" } else { "Body" }, body_dot),
                ];
                if f.tests || tests_n > 0 {
                    items.push((ReqTab::Tests, "Tests", count(tests_n)));
                }
                if f.captures || caps_n > 0 {
                    items.push((ReqTab::Capture, "Variables", count(caps_n)));
                }
                if f.scripts || scripts.is_some() {
                    items.push((ReqTab::Scripts, "Scripts", scripts));
                }
                if f.examples || !draft.examples.is_empty() {
                    items.push((ReqTab::Examples, "Examples", count(draft.examples.len())));
                }
                if f.docs {
                    items.push((ReqTab::Docs, "Docs", docs_dot));
                }
                if !items.iter().any(|(t, _, _)| t == current) {
                    *current = ReqTab::Params;
                }
                theme::tab_strip(ui, current, &items);
            });
        let sub = self.req_tabs.get(&tab_id).copied().unwrap_or_default();

        let avail = ui.available_height();
        let frame = egui::Frame::new().fill(p.surface).inner_margin(Margin {
            left: 20,
            right: 20,
            top: 10,
            bottom: 8,
        });
        egui::Panel::bottom("response_panel")
            .resizable(true)
            .default_size((avail * 0.55).max(160.0))
            .size_range(110.0..=(avail - 90.0).max(140.0))
            .frame(frame)
            .show(ui, |ui| {
                // A panel only keeps a dragged size if its content fills it; a short
                // response (an error, or nothing sent yet) would snap it back.
                ui.set_min_height(ui.available_height());
                self.response_panel(ui, tab_id, draft, coll, busy)
            });

        egui::ScrollArea::vertical().id_salt(("req_pane", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 20, right: 20, top: 8, bottom: 12 }).show(ui, |ui| match sub {
                ReqTab::Params => self.params_pane(ui, tab_id, draft, &known),
                ReqTab::Auth => {
                    let inherited = self.inherited_auth(draft, origin);
                    self.auth_editor(ui, &mut draft.auth, Some(inherited), Some(tab_id));
                }
                ReqTab::Headers => self.headers_pane(ui, tab_id, draft, origin, &known),
                ReqTab::Body => self.body_pane(ui, tab_id, draft, &known),
                ReqTab::Tests => self.tests_pane(ui, tab_id, draft),
                ReqTab::Capture => self.capture_pane(ui, tab_id, draft),
                ReqTab::Examples => self.examples_pane(ui, tab_id, draft, coll, &known),
                ReqTab::Scripts => self.scripts_pane(ui, tab_id, &mut draft.events, "request"),
                ReqTab::Docs => {
                    ui.label(RichText::new("Describe what this request does. Shown in exported docs (Markdown, HTML, OpenAPI).").color(p.muted));
                    ui.add_space(4.0);
                    prose_editor(ui, &mut draft.description, 10, "Add a description…", ui.make_persistent_id(("docs", tab_id)));
                }
            });
        });
    }

    /// The auth a request with "Inherit" would use, and where it comes from.
    fn inherited_auth(&self, draft: &Request, origin: Option<Origin>) -> (Auth, String) {
        let Some(c) = origin
            .and_then(|o| self.collection(o.coll))
            .filter(|c| c.find(draft.id).is_some())
        else {
            return (
                Auth::None,
                "this request isn't saved in a collection".into(),
            );
        };
        for f in c.path_to(draft.id).iter().rev() {
            if f.auth != Auth::Inherit {
                return (f.auth.clone(), format!("folder \"{}\"", f.name));
            }
        }
        (c.auth.clone(), format!("collection \"{}\"", c.name))
    }

    fn request_header(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        origin: Option<Origin>,
        dirty: bool,
        saved: bool,
    ) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            if let Some(c) = origin.and_then(|o| self.collection(o.coll)) {
                let mut crumbs = vec![c.name.clone()];
                crumbs.extend(c.path_to(draft.id).iter().map(|f| f.name.clone()));
                ui.label(
                    RichText::new(format!("{}  /", shape(&crumbs.join("  /  ")))).color(p.muted),
                );
            } else {
                ui.label(RichText::new("Unsaved  /").color(p.muted));
            }
            let width = (ui.available_width() - 200.0).max(120.0);
            ui.add(
                egui::TextEdit::singleline(&mut draft.name)
                    .font(FontId::proportional(15.0))
                    .frame(egui::Frame::NONE)
                    .desired_width(width)
                    .text_color(p.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let code = RichText::new("</>")
                    .monospace()
                    .color(if self.ws.show_code { p.accent } else { p.muted });
                if self.ws.features.code
                    && ui
                    .add(
                        egui::Button::new(code)
                            .frame_when_inactive(self.ws.show_code)
                            .min_size(egui::vec2(34.0, 28.0)),
                    )
                    .on_hover_text("Code snippet")
                    .clicked()
                {
                    self.ws.show_code = !self.ws.show_code;
                }
                let save_label = if saved {
                    "\u{E248}  Save"
                } else {
                    "\u{E248}  Save as…"
                };
                let save = egui::Button::new(RichText::new(save_label).color(if dirty {
                    p.text
                } else {
                    p.muted
                }))
                .min_size(egui::vec2(0.0, 28.0));
                if ui
                    .add_enabled(dirty || !saved, save)
                    .on_hover_text(theme::shortcut("Ctrl+S"))
                    .clicked()
                {
                    self.actions.push(Action::Save(tab_id));
                }
            });
        });
    }

    fn url_bar(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        vars: &[KeyValue],
        known: &HashSet<String>,
        busy: bool,
    ) {
        let p = pal(ui);
        let height = 38.0;
        ui.horizontal(|ui| {
            let send_w = 104.0;
            let bar_w = ui.available_width() - send_w - 8.0;
            let frame = egui::Frame::new()
                .fill(p.input)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(4, 2));
            let mut url_resp = None;
            frame.show(ui, |ui| {
                ui.set_width(bar_w - 10.0);
                ui.set_height(height - 6.0);
                ui.horizontal_centered(|ui| {
                    let (label, color) = match draft.kind {
                        RequestKind::Http => (draft.method.as_str(), theme::method_color(&p, &draft.method)),
                        RequestKind::WebSocket => ("WS", p.teal),
                        RequestKind::SocketIo => ("SIO", p.pink),
                    };
                    egui::ComboBox::from_id_salt(("method", tab_id))
                        .width(92.0)
                        .selected_text(
                            RichText::new(label)
                                .monospace()
                                .strong()
                                .size(13.0)
                                .color(color),
                        )
                        .show_ui(ui, |ui| {
                            for m in METHODS {
                                let on = draft.kind == RequestKind::Http && draft.method == m;
                                if ui.selectable_label(on, theme::method_text(&p, m, 13.0)).clicked() {
                                    draft.method = m.to_string();
                                    draft.kind = RequestKind::Http;
                                }
                            }
                            if !self.ws.features.sockets && draft.kind == RequestKind::Http {
                                return;
                            }
                            ui.separator();
                            for (kind, name, color) in [
                                (RequestKind::WebSocket, "WebSocket", p.teal),
                                (RequestKind::SocketIo, "Socket.IO", p.pink),
                            ] {
                                let text = RichText::new(name).monospace().strong().size(13.0).color(color);
                                if ui.selectable_label(draft.kind == kind, text).clicked() {
                                    draft.kind = kind;
                                }
                            }
                        });
                    let (sep, _) =
                        ui.allocate_exact_size(egui::vec2(1.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(sep, 0.0, p.border);
                    let known = known.clone();
                    let url_id = ui.make_persistent_id(("url", tab_id));
                    let editing_url = ui.memory(|m| m.has_focus(url_id));
                    let mut layouter = move |ui: &egui::Ui,
                                             buf: &dyn egui::TextBuffer,
                                             wrap: f32| {
                        let mut job =
                            highlight_vars(buf.as_str(), &known, &p, FontId::monospace(13.5), true);
                        if !editing_url {
                            job = crate::rtl::shape_job(job);
                        }
                        let _ = wrap;
                        job.wrap.max_width = f32::INFINITY;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut draft.url)
                            .id(url_id)
                            .hint_text(
                                RichText::new(match draft.kind {
                                    RequestKind::Http => "Enter URL or paste a cURL command",
                                    RequestKind::WebSocket => "ws://localhost:8080  or  wss://…",
                                    RequestKind::SocketIo => "http://localhost:3000  (add /namespace if you use one)",
                                })
                                .color(p.faint),
                            )
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter),
                    );
                    if editing_url {
                        crate::rtl::preview(ui, resp.rect, &draft.url, url_id);
                    }
                    url_resp = Some(resp);
                });
            });
            if draft.kind != RequestKind::Http {
                // Sockets connect and stay open, instead of sending once.
                let state = self.sockets.get(&tab_id).map(|s| s.state);
                let (button, action) = match state {
                    Some(crate::app::SocketState::Open) => (
                        egui::Button::new(RichText::new("Disconnect").strong()),
                        Action::SocketDisconnect(tab_id),
                    ),
                    Some(crate::app::SocketState::Connecting) => (
                        egui::Button::new(RichText::new("Cancel").strong()),
                        Action::SocketDisconnect(tab_id),
                    ),
                    _ => (theme::primary("Connect"), Action::SocketConnect(tab_id)),
                };
                if ui.add(button.min_size(egui::vec2(send_w, height)).corner_radius(8.0)).clicked() {
                    self.actions.push(action);
                }
            } else {
                let send = if busy {
                    egui::Button::new(RichText::new("Cancel").strong())
                        .min_size(egui::vec2(send_w, height))
                        .corner_radius(8.0)
                } else {
                    theme::primary("Send")
                        .min_size(egui::vec2(send_w, height))
                        .corner_radius(8.0)
                };
                if ui
                    .add(send)
                    .on_hover_text(theme::shortcut("Ctrl+Enter"))
                    .clicked()
                {
                    self.actions.push(if busy {
                        Action::Cancel(tab_id)
                    } else {
                        Action::Send(tab_id)
                    });
                }
            }

            let Some(resp) = url_resp else { return };
            if resp.changed() {
                if draft.url.trim_start().starts_with("curl ") {
                    match crate::import::parse_curl(draft.url.trim()) {
                        Ok(mut r) => {
                            r.id = draft.id;
                            r.tests = std::mem::take(&mut draft.tests);
                            r.captures = std::mem::take(&mut draft.captures);
                            if !draft.name.starts_with("Untitled")
                                && !draft.name.starts_with("New Request")
                            {
                                r.name = draft.name.clone();
                            }
                            *draft = r;
                            self.actions.push(Action::Notify(
                                "Imported the cURL command".into(),
                                ToastKind::Success,
                            ));
                        }
                        Err(e) => self.actions.push(Action::Notify(
                            format!("Couldn't read the cURL command: {e}"),
                            ToastKind::Error,
                        )),
                    }
                } else {
                    draft.sync_from_url();
                }
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !busy {
                let open = self.sockets.get(&tab_id).is_some_and(|s| s.state != crate::app::SocketState::Closed);
                // In a socket's URL, Enter connects (it shouldn't send the message).
                if draft.kind == RequestKind::Http || !open {
                    self.actions.push(Action::Send(tab_id));
                }
            }
            if draft.url.contains("{{") {
                let used = used_vars(&draft.url);
                resp.clone().on_hover_ui(|ui| {
                    ui.set_max_width(420.0);
                    egui::Grid::new("url_vars")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            for name in used {
                                let value = vars
                                    .iter()
                                    .rev()
                                    .find(|v| v.enabled && v.key == name)
                                    .map(|v| v.value.clone());
                                ui.label(
                                    RichText::new(format!("{{{{{name}}}}}")).monospace().color(
                                        if value.is_some() || name.starts_with('$') {
                                            p.accent
                                        } else {
                                            p.red
                                        },
                                    ),
                                );
                                ui.label(
                                    RichText::new(value.unwrap_or_else(|| {
                                        if name.starts_with('$') {
                                            "dynamic".into()
                                        } else {
                                            "not defined".into()
                                        }
                                    }))
                                    .monospace(),
                                );
                                ui.end_row();
                            }
                        });
                });
            }
            // Suggest variables after typing `{{`.
            if resp.has_focus() {
                if let Some(start) = draft
                    .url
                    .rfind("{{")
                    .filter(|s| !draft.url[*s..].contains("}}"))
                {
                    let partial = draft.url[start + 2..].to_lowercase();
                    let mut names: Vec<String> = known
                        .iter()
                        .filter(|n| n.to_lowercase().contains(&partial))
                        .cloned()
                        .collect();
                    names.sort_by_key(|n| (n.starts_with('$'), n.to_lowercase()));
                    names.truncate(10);
                    if !names.is_empty() {
                        egui::Area::new(egui::Id::unique(("var_suggest", tab_id)))
                            .order(egui::Order::Foreground)
                            .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 6.0))
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(260.0);
                                    for name in names {
                                        let value = vars
                                            .iter()
                                            .rev()
                                            .find(|v| v.key == name)
                                            .map(|v| v.value.clone())
                                            .unwrap_or_else(|| {
                                                http::DYNAMIC_VARS
                                                    .iter()
                                                    .find(|(d, _)| *d == name)
                                                    .map(|(_, h)| h.to_string())
                                                    .unwrap_or_default()
                                            });
                                        let text = RichText::new(format!("{name}   ")).monospace();
                                        let b = egui::Button::new(text)
                                            .right_text(
                                                RichText::new(truncate(&value, 28))
                                                    .color(p.muted)
                                                    .size(11.5),
                                            )
                                            .frame_when_inactive(false);
                                        if ui.add_sized([ui.available_width(), 24.0], b).clicked() {
                                            draft.url.truncate(start);
                                            draft.url.push_str(&format!("{{{{{name}}}}}"));
                                            draft.sync_from_url();
                                            resp.request_focus();
                                        }
                                    }
                                });
                            });
                    }
                }
            }
        });
        // The final URL, so students can see what the variables turn into.
        let resolved = http::resolve(&draft.url, vars);
        if resolved != draft.url && !resolved.is_empty() {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("↳  {resolved}"))
                            .monospace()
                            .size(11.5)
                            .color(p.muted),
                    )
                    .truncate(),
                );
            });
        }
    }

    fn params_pane(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        known: &HashSet<String>,
    ) {
        theme::section_title(ui, "Query Params");
        let opts = KvOptions {
            id: format!("params-{tab_id}"),
            key_hint: "Key",
            files: false,
            fixed_keys: false,
            known,
        };
        if self.kv_table(ui, &mut draft.params, opts) {
            draft.sync_to_url();
        }
        if !draft.path_vars.is_empty() {
            ui.add_space(14.0);
            theme::section_title(ui, "Path Variables");
            let opts = KvOptions {
                id: format!("pathvars-{tab_id}"),
                key_hint: "",
                files: false,
                fixed_keys: true,
                known,
            };
            self.kv_table(ui, &mut draft.path_vars, opts);
        } else {
            ui.add_space(10.0);
            ui.label(
                RichText::new("Tip: write /:id in the URL to get a path variable.")
                    .size(12.0)
                    .color(pal(ui).faint),
            );
        }
    }

    fn headers_pane(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        origin: Option<Origin>,
        known: &HashSet<String>,
    ) {
        let p = pal(ui);
        let opts = KvOptions {
            id: format!("headers-{tab_id}"),
            key_hint: "Header",
            files: false,
            fixed_keys: false,
            known,
        };
        self.kv_table(ui, &mut draft.headers, opts);
        ui.add_space(12.0);
        let label = if self.show_auto_headers {
            "\u{E136}  Hide auto-generated headers"
        } else {
            "\u{E13A}  Show auto-generated headers"
        };
        if ui.link(RichText::new(label).size(12.5)).clicked() {
            self.show_auto_headers = !self.show_auto_headers;
        }
        if self.show_auto_headers {
            let prepared = self.prepare(draft, origin);
            let own: HashSet<String> = draft
                .headers
                .iter()
                .filter(|h| h.enabled)
                .map(|h| h.key.to_lowercase())
                .collect();
            let mut auto: Vec<(String, String)> = prepared
                .headers
                .into_iter()
                .filter(|(k, _)| !own.contains(&k.to_lowercase()))
                .collect();
            if self.ws.settings.send_user_agent {
                auto.push(("User-Agent".into(), "GazaMan/1.0".into()));
            }
            auto.push(("Accept".into(), "*/*".into()));
            auto.push(("Accept-Encoding".into(), "gzip".into()));
            if matches!(draft.body_type, BodyType::FormData)
                && !matches!(draft.method.as_str(), "GET" | "HEAD")
            {
                auto.push((
                    "Content-Type".into(),
                    "multipart/form-data; boundary=<calculated>".into(),
                ));
            }
            ui.add_space(4.0);
            egui::Grid::new(("auto_headers", tab_id))
                .num_columns(2)
                .spacing([16.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    for (k, v) in auto {
                        ui.label(RichText::new(k).monospace().color(p.muted));
                        ui.label(RichText::new(truncate(&v, 90)).monospace().color(p.faint));
                        ui.end_row();
                    }
                });
        }
    }

    fn body_pane(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        known: &HashSet<String>,
    ) {
        if draft.kind != RequestKind::Http {
            self.socket_composer(ui, tab_id, draft, known);
            return;
        }
        let p = pal(ui);
        ui.horizontal_wrapped(|ui| {
            for t in BodyType::ALL {
                if t == BodyType::GraphQl && !self.ws.features.graphql && draft.body_type != t {
                    continue;
                }
                ui.radio_value(&mut draft.body_type, t, RichText::new(t.label()).size(12.5));
            }
            if draft.body_type == BodyType::Raw {
                ui.add_space(6.0);
                egui::ComboBox::from_id_salt(("raw_lang", tab_id))
                    .width(110.0)
                    .selected_text(RichText::new(draft.raw_lang.label()).color(p.accent))
                    .show_ui(ui, |ui| {
                        for l in RawLang::ALL {
                            ui.selectable_value(&mut draft.raw_lang, l, l.label());
                        }
                    });
            }
        });
        ui.add_space(6.0);
        if matches!(draft.method.as_str(), "GET" | "HEAD") && draft.body_type != BodyType::None {
            ui.label(
                RichText::new(format!(
                    "\u{E4E0}  {} requests are sent without a body.",
                    draft.method
                ))
                .size(12.0)
                .color(p.amber),
            );
        }
        match draft.body_type {
            BodyType::None => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("This request does not have a body").color(p.muted))
                });
            }
            BodyType::Raw => {
                let is_json = draft.raw_lang == RawLang::Json;
                ui.horizontal(|ui| {
                    if is_json {
                        let check = serde_json::from_str::<Value>(&http::resolve(&draft.raw, &[]))
                            .err()
                            .filter(|_| !draft.raw.trim().is_empty() && !draft.raw.contains("{{"));
                        match check {
                            Some(e) => {
                                ui.label(RichText::new(format!("\u{E4F6}  {e}")).size(12.0).color(p.red))
                            }
                            None if !draft.raw.trim().is_empty() => {
                                ui.label(RichText::new("\u{E182}  Valid JSON").size(12.0).color(p.accent))
                            }
                            None => ui.label(""),
                        };
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if is_json
                            && ui
                                .button(RichText::new("\u{E6B6}  Beautify").size(12.0))
                                .clicked()
                        {
                            match serde_json::from_str::<Value>(&draft.raw) {
                                Ok(v) => {
                                    draft.raw = serde_json::to_string_pretty(&v).unwrap_or_default()
                                }
                                Err(e) => self.actions.push(Action::Notify(
                                    format!("Invalid JSON: {e}"),
                                    ToastKind::Error,
                                )),
                            }
                        }
                        if !self.body_find.is_empty() {
                            let n = count_matches(&draft.raw, &self.body_find);
                            ui.label(RichText::new(format!("{n} found")).size(11.5).color(if n > 0 { p.accent } else { p.faint }));
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.body_find)
                                .hint_text(RichText::new("\u{E30C}  Find in body").color(p.faint))
                                .desired_width(170.0)
                                .margin(Margin::symmetric(8, 3)),
                        );
                    });
                });
                code_editor(ui, &mut draft.raw, is_json, known, &p, 12, &self.body_find);
            }
            BodyType::UrlEncoded => {
                let opts = KvOptions {
                    id: format!("urlenc-{tab_id}"),
                    key_hint: "Key",
                    files: false,
                    fixed_keys: false,
                    known,
                };
                self.kv_table(ui, &mut draft.urlencoded, opts);
            }
            BodyType::FormData => {
                let opts = KvOptions {
                    id: format!("form-{tab_id}"),
                    key_hint: "Key",
                    files: true,
                    fixed_keys: false,
                    known,
                };
                self.kv_table(ui, &mut draft.form, opts);
            }
            BodyType::Binary => {
                ui.horizontal(|ui| {
                    let label = if draft.binary_path.is_empty() {
                        "\u{E39A}  Select file".to_string()
                    } else {
                        format!("\u{E39A}  {}", http::file_name(&draft.binary_path))
                    };
                    if ui.button(label).clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            draft.binary_path = path.display().to_string();
                        }
                    }
                    if !draft.binary_path.is_empty() {
                        ui.label(
                            RichText::new(&draft.binary_path)
                                .monospace()
                                .size(11.5)
                                .color(p.muted),
                        );
                        if theme::icon_button(ui, "×", "Clear").clicked() {
                            draft.binary_path.clear();
                        }
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Sent as-is. Content-Type: {}",
                        http::mime_for(&draft.binary_path)
                    ))
                    .size(12.0)
                    .color(p.faint),
                );
            }
            BodyType::GraphQl => {
                ui.horizontal(|ui| {
                    let fetching = self.fetching_schema.contains(&tab_id);
                    let label = if fetching { "Loading schema…" } else { "\u{E094} Fetch schema" };
                    if ui
                        .add_enabled(!fetching, egui::Button::new(label))
                        .on_hover_text("Ask the server for its schema (introspection) to get suggestions while you type")
                        .clicked()
                    {
                        self.actions.push(Action::FetchSchema(tab_id));
                    }
                    match self.gql_schemas.get(&tab_id) {
                        Some(Ok(schema)) => {
                            ui.label(
                                RichText::new(format!(
                                    "\u{E182} {} types. Suggestions appear as you type; Ctrl+Space shows them all.",
                                    schema.user_type_count()
                                ))
                                .size(12.0)
                                .color(p.accent),
                            );
                        }
                        Some(Err(e)) => {
                            ui.add(egui::Label::new(RichText::new(e).size(12.0).color(p.red)).truncate());
                        }
                        None => {
                            ui.label(RichText::new("Fetch the schema for suggestions and a schema explorer.").size(12.0).color(p.faint));
                        }
                    }
                });
                ui.add_space(4.0);
                let half = (ui.available_width() - 16.0) / 2.0;
                let schema = self.gql_schemas.get(&tab_id).and_then(|r| r.as_ref().ok());
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(half);
                        theme::section_title(ui, "Query");
                        gql_editor(ui, &mut draft.gql_query, schema, known, &p, &self.body_find, tab_id);
                        if let Some(schema) = schema {
                            schema_explorer(ui, schema, &mut draft.gql_query, &p, tab_id);
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(half);
                        theme::section_title(ui, "GraphQL Variables (JSON)");
                        code_editor(ui, &mut draft.gql_variables, true, known, &p, 12, "");
                    });
                });
            }
        }
    }

    fn tests_pane(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
    ) {
        let p = pal(ui);
        ui.label(RichText::new("Checks that run on every response, no code needed. When you export to Postman they become pm.test() scripts.").color(p.muted));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Quick add:").size(12.0).color(p.faint));
            let presets: [(&str, Assertion); 5] = [
                (
                    "Status is 200",
                    Assertion {
                        source: Source::Status,
                        op: Op::Equals,
                        expected: "200".into(),
                        ..Default::default()
                    },
                ),
                (
                    "Status < 400",
                    Assertion {
                        source: Source::Status,
                        op: Op::LessThan,
                        expected: "400".into(),
                        ..Default::default()
                    },
                ),
                (
                    "Time < 500 ms",
                    Assertion {
                        source: Source::Time,
                        op: Op::LessThan,
                        expected: "500".into(),
                        ..Default::default()
                    },
                ),
                (
                    "Body is JSON",
                    Assertion {
                        source: Source::Header,
                        path: "Content-Type".into(),
                        op: Op::Contains,
                        expected: "json".into(),
                        ..Default::default()
                    },
                ),
                (
                    "Field exists",
                    Assertion {
                        source: Source::Json,
                        path: "id".into(),
                        op: Op::Exists,
                        expected: String::new(),
                        ..Default::default()
                    },
                ),
            ];
            for (label, a) in presets {
                if ui
                    .button(RichText::new(format!("+ {label}")).size(12.0))
                    .clicked()
                {
                    draft.tests.push(a);
                }
            }
        });
        ui.add_space(8.0);
        let results = self
            .responses
            .get(&tab_id)
            .map(|r| r.tests.clone())
            .unwrap_or_default();
        let mut remove = None;
        let path_w = ((ui.available_width() - 520.0) / 2.0).clamp(110.0, 260.0);
        egui::Grid::new(("tests_grid", tab_id))
            .num_columns(7)
            .spacing([6.0, 6.0])
            .min_row_height(28.0)
            .show(ui, |ui| {
                let mut result_iter = results.iter();
                for (i, a) in draft.tests.iter_mut().enumerate() {
                    ui.add(egui::Checkbox::without_text(&mut a.enabled));
                    egui::ComboBox::from_id_salt(("t_src", tab_id, i))
                        .width(150.0)
                        .selected_text(a.source.label())
                        .show_ui(ui, |ui| {
                            for s in Source::ALL {
                                ui.selectable_value(&mut a.source, s, s.label());
                            }
                        });
                    if a.source.needs_path() {
                        let hint = if a.source == Source::Json {
                            "data.items[0].id"
                        } else {
                            "Content-Type"
                        };
                        ui.add_sized(
                            [path_w, 28.0],
                            egui::TextEdit::singleline(&mut a.path)
                                .hint_text(hint)
                                .font(egui::TextStyle::Monospace),
                        );
                    } else {
                        ui.label("");
                    }
                    egui::ComboBox::from_id_salt(("t_op", tab_id, i))
                        .width(118.0)
                        .selected_text(a.op.label())
                        .show_ui(ui, |ui| {
                            for o in Op::ALL {
                                ui.selectable_value(&mut a.op, o, o.label());
                            }
                        });
                    if a.op.needs_value() {
                        ui.add_sized(
                            [path_w, 28.0],
                            egui::TextEdit::singleline(&mut a.expected)
                                .hint_text("Expected")
                                .font(egui::TextStyle::Monospace),
                        );
                    } else {
                        ui.label("");
                    }
                    let result = if a.enabled { result_iter.next() } else { None };
                    match result {
                        Some(r) if r.passed => ui
                            .label(RichText::new("\u{E182} PASS").size(11.5).strong().color(p.accent))
                            .on_hover_text(&r.detail),
                        Some(r) => ui
                            .label(RichText::new("\u{E4F6} FAIL").size(11.5).strong().color(p.red))
                            .on_hover_text(&r.detail),
                        None => ui.label(""),
                    };
                    if theme::icon_button(ui, "×", "Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            draft.tests.remove(i);
        }
        if ui.button("+  Add test").clicked() {
            draft.tests.push(Assertion::default());
        }
        if draft.tests.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "JSON paths look like  token,  user.name,  items[0].id  or  items.length",
                )
                .size(12.0)
                .color(p.faint),
            );
        }
        let script = tests::postman_script(&draft.tests, &draft.captures);
        if !script.is_empty() {
            ui.add_space(10.0);
            egui::CollapsingHeader::new(
                RichText::new("Postman script equivalent")
                    .size(12.5)
                    .color(p.muted),
            )
            .id_salt(("script", tab_id))
            .show(ui, |ui| {
                let mut s = script.as_str();
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let mut job = highlight_code(buf.as_str(), &p, 12.0);
                    job.wrap.max_width = wrap;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                egui::Frame::new()
                    .fill(p.input)
                    .corner_radius(8.0)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut s)
                                .code_editor()
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter),
                        );
                    });
                if ui.button("Copy script").clicked() {
                    ui.ctx().copy_text(script.clone());
                }
            });
        }
    }

    fn capture_pane(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
    ) {
        let p = pal(ui);
        ui.label(RichText::new("Save parts of the response into variables, e.g. a login token, then use them as {{name}} in later requests. Runs after every send and in the Runner.").color(p.muted));
        ui.add_space(8.0);
        let captured = self
            .responses
            .get(&tab_id)
            .map(|r| r.captured.clone())
            .unwrap_or_default();
        let mut remove = None;
        egui::Grid::new(("caps_grid", tab_id))
            .num_columns(7)
            .spacing([6.0, 6.0])
            .min_row_height(28.0)
            .show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new("Variable").size(11.5).color(p.faint));
                ui.label(RichText::new("From").size(11.5).color(p.faint));
                ui.label(RichText::new("Path / header").size(11.5).color(p.faint));
                ui.label(RichText::new("Save to").size(11.5).color(p.faint));
                ui.label(RichText::new("Last value").size(11.5).color(p.faint));
                ui.label("");
                ui.end_row();
                for (i, c) in draft.captures.iter_mut().enumerate() {
                    ui.add(egui::Checkbox::without_text(&mut c.enabled));
                    ui.add_sized(
                        [130.0, 28.0],
                        egui::TextEdit::singleline(&mut c.var)
                            .hint_text("token")
                            .font(egui::TextStyle::Monospace),
                    );
                    egui::ComboBox::from_id_salt(("c_src", tab_id, i))
                        .width(120.0)
                        .selected_text(c.source.label())
                        .show_ui(ui, |ui| {
                            for s in Source::CAPTURABLE {
                                ui.selectable_value(&mut c.source, s, s.label());
                            }
                        });
                    if c.source.needs_path() {
                        ui.add_sized(
                            [170.0, 28.0],
                            egui::TextEdit::singleline(&mut c.path)
                                .hint_text(if c.source == Source::Json {
                                    "data.token"
                                } else {
                                    "X-Request-Id"
                                })
                                .font(egui::TextStyle::Monospace),
                        );
                    } else {
                        ui.label("");
                    }
                    egui::ComboBox::from_id_salt(("c_scope", tab_id, i))
                        .width(110.0)
                        .selected_text(c.scope.label())
                        .show_ui(ui, |ui| {
                            for s in Scope::ALL {
                                ui.selectable_value(&mut c.scope, s, s.label());
                            }
                        });
                    match captured.iter().find(|(_, k, _)| *k == c.var.trim()) {
                        Some((_, _, v)) => ui
                            .add(
                                egui::Label::new(
                                    RichText::new(truncate(v, 30))
                                        .monospace()
                                        .size(11.5)
                                        .color(p.accent),
                                )
                                .truncate(),
                            )
                            .on_hover_text(v),
                        None => ui.label(RichText::new("—").color(p.faint)),
                    };
                    if theme::icon_button(ui, "×", "Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            draft.captures.remove(i);
        }
        if ui.button("+  Add variable").clicked() {
            draft.captures.push(Capture::default());
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Tip: in the response's Tree view, right-click any value → \"Save to variable\".",
            )
            .size(12.0)
            .color(p.faint),
        );
        if self.ws.active_env.is_none()
            && draft.captures.iter().any(|c| c.scope == Scope::Environment)
        {
            ui.label(
                RichText::new("No environment is active, so Environment captures go to Globals.")
                    .size(12.0)
                    .color(p.amber),
            );
        }
    }

    fn examples_pane(&mut self, ui: &mut egui::Ui, tab_id: crate::collection::Id, draft: &mut Request, coll: Option<crate::collection::Id>, known: &HashSet<String>) {
        let p = pal(ui);
        ui.label(RichText::new("Saved responses for this request. They appear in exported docs, and a collection's mock server answers with them. Send the request, then use \u{E1FE} → \"Save as example\" on the response.").color(p.muted));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("+  Add example").clicked() {
                draft.examples.push(crate::collection::Example::default());
            }
            if let Some(c) = coll {
                match self.mocks.get(&c) {
                    Some(m) => {
                        ui.label(RichText::new(format!("●  Mock running at {}", m.url())).size(12.0).color(p.accent));
                    }
                    None => {
                        if ui.link(RichText::new("Start a mock server for this collection").size(12.0)).clicked() {
                            self.actions.push(Action::StartMock(c));
                        }
                    }
                }
            }
        });
        ui.add_space(8.0);
        let mut remove = None;
        for (i, e) in draft.examples.iter_mut().enumerate() {
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut e.name).font(FontId::proportional(14.0)).desired_width(260.0));
                    ui.label(RichText::new("Status").size(12.0).color(p.muted));
                    ui.add(egui::DragValue::new(&mut e.status).range(100..=599));
                    theme::soft_badge(ui, http::reason(e.status), theme::status_color(&p, Some(e.status)), 11.5);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, "×", "Delete example").clicked() {
                            remove = Some(i);
                        }
                    });
                });
                egui::CollapsingHeader::new(RichText::new(format!("Headers ({})", e.headers.len())).size(12.5).color(p.muted))
                    .id_salt(("ex_headers", e.id))
                    .show(ui, |ui| {
                        let opts = KvOptions { id: format!("exh-{}", e.id), key_hint: "Header", files: false, fixed_keys: false, known };
                        self.kv_table(ui, &mut e.headers, opts);
                    });
                let json = e.content_type().contains("json");
                code_editor(ui, &mut e.body, json, known, &p, 6, "");
            });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            draft.examples.remove(i);
        }
        let _ = tab_id;
    }

    // -----------------------------------------------------------------------------------------
    // Auth
    // -----------------------------------------------------------------------------------------

    /// `inherited` is the auth "Inherit" resolves to (and where from); `None` hides Inherit.
    pub fn auth_editor(
        &mut self,
        ui: &mut egui::Ui,
        auth: &mut Auth,
        inherited: Option<(Auth, String)>,
        token_tab: Option<crate::collection::Id>,
    ) {
        let p = pal(ui);
        let left_w = 260.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                theme::section_title(ui, "Auth Type");
                egui::ComboBox::from_id_salt("auth_type").width(230.0).selected_text(auth.label()).show_ui(ui, |ui| {
                    let basic = |a: &Auth| matches!(a, Auth::Inherit | Auth::None | Auth::Bearer { .. } | Auth::Basic { .. } | Auth::ApiKey { .. });
                    for opt in Auth::choices() {
                        if inherited.is_none() && opt == Auth::Inherit {
                            continue;
                        }
                        let current = std::mem::discriminant(&opt) == std::mem::discriminant(auth);
                        if !self.ws.features.advanced_auth && !basic(&opt) && !current {
                            continue;
                        }
                        let same = std::mem::discriminant(&opt) == std::mem::discriminant(auth);
                        if ui.selectable_label(same, opt.label()).clicked() && !same {
                            *auth = opt;
                        }
                    }
                });
                ui.add_space(8.0);
                let help = match auth {
                    Auth::Inherit => "Uses the auth of the parent folder or collection.",
                    Auth::None => "No Authorization header is sent.",
                    Auth::Bearer { .. } => "Sends Authorization: Bearer <token>. Most JWT login APIs use this.",
                    Auth::Basic { .. } => "Sends the username and password, base64 encoded.",
                    Auth::ApiKey { .. } => "Sends a key/value pair in a header or the query string.",
                    Auth::OAuth2 { .. } => "Sends an access token. Gaza Man can fetch one using client credentials or a password grant.",
                    Auth::Digest { .. } => "The server answers the first try with a challenge (401); Gaza Man answers it and sends the request again. MD5 and SHA-256.",
                    Auth::Aws { .. } => "Signs the request (URL, headers and body) with your AWS keys, Signature Version 4.",
                    Auth::Ntlm { .. } => "Windows authentication (NTLMv2), used by IIS and other Microsoft servers. Takes a short handshake on one connection.",
                };
                ui.add(egui::Label::new(RichText::new(help).size(12.0).color(p.muted)).wrap());
                ui.add_space(6.0);
                ui.label(RichText::new("Tip: use {{variables}} to keep secrets out of requests.").size(11.5).color(p.faint));
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let field = |ui: &mut egui::Ui, label: &str, value: &mut String, password: bool| {
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                    ui.add(egui::TextEdit::singleline(value).password(password).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).margin(Margin::symmetric(8, 5)));
                    ui.add_space(4.0);
                };
                match auth {
                    Auth::Inherit => {
                        if let Some((a, from)) = &inherited {
                            theme::card(ui, |ui| {
                                ui.label(RichText::new(format!("This request is using {} from {from}.", a.label())).color(p.text));
                            });
                        }
                    }
                    Auth::None => {
                        theme::card(ui, |ui| {
                            ui.label(RichText::new("This request does not use any authorization.").color(p.muted));
                        });
                    }
                    Auth::Bearer { token } => field(ui, "Token", token, false),
                    Auth::Digest { username, password } => {
                        field(ui, "Username", username, false);
                        field(ui, "Password", password, true);
                    }
                    Auth::Aws { access_key, secret_key, region, service, session_token } => {
                        field(ui, "Access Key", access_key, false);
                        field(ui, "Secret Key", secret_key, true);
                        field(ui, "AWS Region", region, false);
                        field(ui, "Service Name (e.g. s3, execute-api)", service, false);
                        field(ui, "Session Token (optional)", session_token, true);
                    }
                    Auth::Ntlm { username, password, domain, workstation } => {
                        field(ui, "Username", username, false);
                        field(ui, "Password", password, true);
                        field(ui, "Domain (optional)", domain, false);
                        field(ui, "Workstation (optional)", workstation, false);
                    }
                    Auth::Basic { username, password } => {
                        field(ui, "Username", username, false);
                        field(ui, "Password", password, true);
                    }
                    Auth::ApiKey { key, value, in_query } => {
                        field(ui, "Key", key, false);
                        field(ui, "Value", value, false);
                        ui.label(RichText::new("Add to").size(12.0).color(p.muted));
                        ui.horizontal(|ui| {
                            ui.radio_value(in_query, false, "Header");
                            ui.radio_value(in_query, true, "Query Params");
                        });
                    }
                    Auth::OAuth2 { access_token, header_prefix, token_url, client_id, client_secret, scope, password_grant, username, password } => {
                        field(ui, "Access Token", access_token, false);
                        field(ui, "Header Prefix", header_prefix, false);
                        ui.add_space(6.0);
                        theme::section_title(ui, "Get new access token");
                        ui.horizontal(|ui| {
                            ui.radio_value(password_grant, false, "Client Credentials");
                            ui.radio_value(password_grant, true, "Password Credentials");
                        });
                        field(ui, "Access Token URL", token_url, false);
                        field(ui, "Client ID", client_id, false);
                        field(ui, "Client Secret", client_secret, true);
                        if *password_grant {
                            field(ui, "Username", username, false);
                            field(ui, "Password", password, true);
                        }
                        field(ui, "Scope", scope, false);
                        if let Some(tab) = token_tab {
                            let busy = self.fetching_token.contains(&tab);
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!busy, theme::primary("Get New Access Token")).clicked() {
                                    self.actions.push(Action::FetchToken(tab));
                                }
                                if busy {
                                    ui.spinner();
                                }
                            });
                        }
                    }
                }
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Response
    // -----------------------------------------------------------------------------------------

    /// The message editor of a WebSocket / Socket.IO request.
    fn socket_composer(&mut self, ui: &mut egui::Ui, tab_id: crate::collection::Id, draft: &mut Request, known: &HashSet<String>) {
        let p = pal(ui);
        let open = self.sockets.get(&tab_id).is_some_and(|s| s.state == crate::app::SocketState::Open);
        let sio = draft.kind == RequestKind::SocketIo;
        if sio {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Event").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut draft.sio_event)
                        .hint_text("message")
                        .font(FontId::monospace(13.0))
                        .desired_width(200.0),
                );
                ui.label(RichText::new("The message below is the event's data: JSON is sent as an object, anything else as text.").size(12.0).color(p.faint));
            });
        } else {
            ui.label(RichText::new("Write a message to send over the open connection. {{variables}} work here too.").size(12.0).color(p.faint));
        }
        ui.add_space(4.0);
        let looks_json = matches!(draft.ws_message.trim_start().chars().next(), Some('{' | '['));
        code_editor(ui, &mut draft.ws_message, looks_json, known, &p, 8, &self.body_find);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if sio { "Emit event" } else { "Send message" };
            let resp = ui
                .add_enabled(open, theme::primary(label).min_size(egui::vec2(130.0, 30.0)))
                .on_hover_text(theme::shortcut("Ctrl+Enter"))
                .on_disabled_hover_text("Connect first, with the button next to the URL");
            if resp.clicked() {
                self.actions.push(Action::SocketSend(tab_id));
            }
            if looks_json && ui.button("Format JSON").clicked() {
                if let Ok(v) = serde_json::from_str::<Value>(&draft.ws_message) {
                    draft.ws_message = serde_json::to_string_pretty(&v).unwrap_or_default();
                }
            }
            if !open {
                ui.label(RichText::new("Not connected").size(12.0).color(p.faint));
            }
        });
    }

    /// The conversation on a WebSocket / Socket.IO connection.
    fn socket_log(&mut self, ui: &mut egui::Ui, tab_id: crate::collection::Id, kind: RequestKind) {
        use crate::app::SocketState;
        use crate::ws::Dir;
        let p = pal(ui);
        let Some(view) = self.sockets.get_mut(&tab_id) else {
            ui.label(RichText::new("Messages").strong());
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.22).max(8.0));
                ui.label(RichText::new("\u{EB56}").size(30.0).color(p.faint));
                ui.add_space(4.0);
                ui.label(RichText::new("Click Connect to open the connection").color(p.muted));
                let how = match kind {
                    RequestKind::SocketIo => "Gaza Man joins the namespace, answers pings, and shows every event the server emits.",
                    _ => "The connection stays open: messages from both sides show up here as they happen.",
                };
                ui.label(RichText::new(how).size(12.0).color(p.faint));
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new("Messages").strong());
            match view.state {
                SocketState::Connecting => {
                    ui.spinner();
                    ui.label(RichText::new("Connecting…").color(p.amber));
                }
                SocketState::Open => {
                    theme::badge(ui, "● Connected", p.accent, p.accent.gamma_multiply(0.18), 11.5);
                }
                SocketState::Closed => {
                    theme::badge(ui, "Disconnected", p.red, p.red.gamma_multiply(0.18), 11.5);
                    if !view.reason.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&view.reason).size(12.0).color(p.muted)).truncate());
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, "\u{E4A6}", "Clear the messages").clicked() {
                    self.actions.push(Action::SocketClear(tab_id));
                }
                ui.add(
                    egui::TextEdit::singleline(&mut view.filter)
                        .hint_text(RichText::new("\u{E30C}  Filter").color(p.faint))
                        .desired_width(170.0)
                        .margin(Margin::symmetric(8, 3)),
                );
                let n = view.log.iter().filter(|m| m.dir != Dir::Info).count();
                ui.label(RichText::new(format!("{n} messages")).size(12.0).color(p.faint));
            });
        });
        if !view.handshake.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("Handshake: {}", view.url)).size(12.0).color(p.muted))
                .id_salt(("ws_handshake", tab_id))
                .show(ui, |ui| {
                    egui::Grid::new(("ws_handshake_grid", tab_id)).num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                        for (k, v) in &view.handshake {
                            ui.label(RichText::new(k).monospace().size(12.0).color(p.muted));
                            ui.label(RichText::new(v).monospace().size(12.0));
                            ui.end_row();
                        }
                    });
                });
        }
        ui.add_space(4.0);

        let filter = view.filter.to_lowercase();
        egui::Frame::new().fill(p.input).corner_radius(8.0).inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::vertical()
                .id_salt(("ws_log", tab_id))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if view.log.is_empty() {
                        ui.label(RichText::new("No messages yet").color(p.faint));
                    }
                    for (i, m) in view.log.iter().enumerate() {
                        if !filter.is_empty()
                            && !m.text.to_lowercase().contains(&filter)
                            && !m.event.as_deref().is_some_and(|e| e.to_lowercase().contains(&filter))
                        {
                            continue;
                        }
                        let (arrow, color, fill) = match m.dir {
                            Dir::Out => ("↑", p.accent, p.accent_soft),
                            Dir::In => ("↓", p.blue, p.surface),
                            Dir::Info => ("•", p.faint, Color32::TRANSPARENT),
                        };
                        egui::Frame::new().fill(fill).corner_radius(6.0).inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(arrow).strong().size(14.0).color(color));
                                ui.label(RichText::new(format!("{:.3}s", m.at_ms as f64 / 1000.0)).monospace().size(11.0).color(p.faint));
                                if let Some(ev) = &m.event {
                                    theme::badge(ui, ev.clone(), p.pink, p.pink.gamma_multiply(0.18), 11.5);
                                }
                                if m.binary {
                                    theme::badge(ui, "binary", p.amber, p.amber.gamma_multiply(0.18), 11.5);
                                }
                                if m.dir == Dir::Info {
                                    ui.label(RichText::new(&m.text).size(12.0).italics().color(p.muted));
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if m.dir != Dir::Info && ui.small_button("\u{E1CA}").on_hover_text("Copy").clicked() {
                                        ui.ctx().copy_text(m.text.clone());
                                    }
                                });
                            });
                            if m.dir != Dir::Info && !m.text.is_empty() {
                                match serde_json::from_str::<Value>(&m.text) {
                                    Ok(v) if v.is_object() || v.is_array() => {
                                        let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
                                        ui.push_id(("ws_msg", i), |ui| json_text(ui, &shape(&pretty), &p, ""));
                                    }
                                    _ => {
                                        ui.add(egui::Label::new(RichText::new(shape(&m.text)).monospace().size(12.5)).wrap().selectable(true));
                                    }
                                }
                            }
                        });
                        ui.add_space(3.0);
                    }
                });
        });
    }

    fn response_panel(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        coll: Option<crate::collection::Id>,
        busy: bool,
    ) {
        if draft.kind != RequestKind::Http {
            self.socket_log(ui, tab_id, draft.kind);
            return;
        }
        let p = pal(ui);
        let Some(view) = self.responses.get(&tab_id) else {
            ui.label(RichText::new("Response").strong());
            if busy {
                let elapsed = self
                    .pending
                    .get(&tab_id)
                    .map(|(_, at)| at.elapsed().as_millis())
                    .unwrap_or(0);
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() * 0.3).max(8.0));
                    ui.spinner();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Sending request…  {elapsed} ms")).color(p.muted),
                    );
                    if ui.button("Cancel").clicked() {
                        self.actions.push(Action::Cancel(tab_id));
                    }
                });
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() * 0.25).max(8.0));
                    ui.label(RichText::new("\u{E398}").size(30.0).color(p.faint));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Enter the URL and click Send to get a response")
                            .color(p.muted),
                    );
                    ui.label(
                        RichText::new(theme::shortcut("or press Ctrl+Enter"))
                            .size(12.0)
                            .color(p.faint),
                    );
                });
            }
            return;
        };

        // Header: tabs on the left, status / time / size on the right.
        let n_headers = match &view.outcome {
            Outcome::Response { headers, .. } => headers.len(),
            Outcome::Error { .. } => 0,
        };
        let passed = view.tests.iter().filter(|t| t.passed).count();
        let tests_label =
            (!view.tests.is_empty()).then(|| format!("{passed}/{}", view.tests.len()));
        let cookies_label = (!view.cookies.is_empty()).then(|| view.cookies.len().to_string());
        let mut clear = false;
        let mut save_example = false;
        let narrow = ui.available_width() < 660.0;
        let mut res_tab = self.res_tab;
        let f = self.ws.features;
        let mut tab_items = vec![(ResTab::Body, "Body", None)];
        if f.cookies || cookies_label.is_some() {
            tab_items.push((ResTab::Cookies, "Cookies", cookies_label));
        }
        tab_items.push((ResTab::Headers, "Headers", (n_headers > 0).then(|| n_headers.to_string())));
        let has_output = view.script.as_ref().is_some_and(|r| !r.logs.is_empty() || !r.errors.is_empty());
        if f.tests || f.scripts || tests_label.is_some() || has_output {
            tab_items.push((ResTab::Tests, "Test Results", tests_label));
        }
        if view.script.as_ref().is_some_and(|r| r.visualizer.is_some()) {
            tab_items.insert(1, (ResTab::Visualize, "Visualize", Some("●".into())));
        }
        if !tab_items.iter().any(|(t, _, _)| *t == res_tab) {
            res_tab = ResTab::Body;
        }
        let actions = &mut self.actions;
        let mut stats = |ui: &mut egui::Ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let more = theme::icon_button(ui, "\u{E1FE}", "More");
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(170.0);
                    if matches!(view.outcome, Outcome::Response { .. }) && ui.button("\u{E248}  Save as example").clicked() {
                        save_example = true;
                    }
                    if ui.button("Save response to file").clicked() {
                        actions.push(Action::SaveResponse(tab_id));
                    }
                    if let Outcome::Response { body, .. } = &view.outcome {
                        if ui.button("Copy response body").clicked() {
                            ui.ctx().copy_text(body.clone());
                            actions.push(Action::Notify(
                                "Copied the response body".into(),
                                ToastKind::Success,
                            ));
                        }
                    }
                    if ui.button("Clear response").clicked() {
                        clear = true;
                    }
                });
                match &view.outcome {
                    Outcome::Response {
                        status,
                        status_text,
                        bytes,
                        time_ms,
                        headers,
                        ..
                    } => {
                        let size = bytes.len();
                        ui.label(
                            RichText::new(theme::format_size(size))
                                .size(12.5)
                                .color(p.accent),
                        )
                        .on_hover_ui(|ui| {
                            let header_bytes: usize =
                                headers.iter().map(|(k, v)| k.len() + v.len() + 4).sum();
                            ui.label(format!(
                                "Body: {}\nHeaders: {}",
                                theme::format_size(size),
                                theme::format_size(header_bytes)
                            ));
                        });
                        ui.label(RichText::new("·").color(p.faint));
                        ui.label(
                            RichText::new(format!("{time_ms} ms"))
                                .size(12.5)
                                .color(p.accent),
                        );
                        ui.label(RichText::new("·").color(p.faint));
                        let color = theme::status_color(&p, Some(*status));
                        theme::soft_badge(ui, format!("{status} {status_text}"), color, 12.5)
                            .on_hover_text(status_meaning(*status));
                    }
                    Outcome::Error { time_ms, .. } => {
                        ui.label(
                            RichText::new(format!("{time_ms} ms"))
                                .size(12.5)
                                .color(p.muted),
                        );
                        theme::soft_badge(ui, "Could not send request", p.red, 12.5);
                    }
                }
            });
        };
        // Postman keeps the status on the tab row; a narrow pane gets it on its own row.
        if narrow {
            ui.horizontal(|ui| stats(ui));
            theme::tab_strip(ui, &mut res_tab, &tab_items);
        } else {
            ui.horizontal(|ui| {
                theme::tab_strip(ui, &mut res_tab, &tab_items);
                stats(ui);
            });
        }
        self.res_tab = res_tab;
        if clear {
            self.responses.remove(&tab_id);
            return;
        }
        if save_example {
            if let Some(Outcome::Response { status, headers, body, .. }) = self.responses.get(&tab_id).map(|v| &v.outcome) {
                // Transport headers would be wrong when the example is served later.
                let skip = ["date", "content-length", "connection", "transfer-encoding", "content-encoding", "keep-alive", "server", "set-cookie"];
                let pretty = serde_json::from_str::<Value>(body).ok().and_then(|v| serde_json::to_string_pretty(&v).ok());
                let example = crate::collection::Example {
                    name: format!("{status} {}", http::reason(*status)),
                    status: *status,
                    headers: headers
                        .iter()
                        .filter(|(k, _)| !skip.contains(&k.to_ascii_lowercase().as_str()))
                        .map(|(k, v)| KeyValue::new(k, v))
                        .collect(),
                    body: pretty.unwrap_or_else(|| body.clone()),
                    ..Default::default()
                };
                // Saved straight into the collection too, like Postman does.
                if let Some(saved) = coll.and_then(|c| self.collection_mut(c)).and_then(|c| c.request_mut(draft.id)) {
                    saved.examples.push(example.clone());
                }
                draft.examples.push(example);
                self.req_tabs.insert(tab_id, ReqTab::Examples);
                self.actions.push(Action::Notify("Saved as an example (the mock server can serve it)".into(), ToastKind::Success));
            }
        }
        let Some(view) = self.responses.get(&tab_id) else {
            return;
        };
        if let Outcome::Response { status, .. } = &view.outcome {
            ui.label(
                RichText::new(status_meaning(*status))
                    .size(11.5)
                    .color(p.faint),
            );
        }
        ui.add_space(4.0);

        if let Outcome::Error { message, .. } = &view.outcome {
            theme::card(ui, |ui| {
                ui.label(
                    RichText::new("The server didn't answer, so there is no status code.")
                        .strong()
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(RichText::new(message).monospace().color(p.red));
                ui.add_space(6.0);
                ui.label(RichText::new("Check that the API is running (for example: npm start), the URL and port are right, and you're online.").color(p.muted));
            });
            return;
        }

        match self.res_tab {
            ResTab::Body => self.response_body(ui, tab_id, draft, coll),
            ResTab::Visualize => {
                let view = &self.responses[&tab_id];
                let Some(v) = view.script.as_ref().and_then(|r| r.visualizer.as_ref()) else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("From pm.visualizer.set(). Shown as a readable page; charts and template scripts run in the browser.").size(12.0).color(p.muted));
                    if ui.button("Open in browser \u{E5DE}").clicked() {
                        if let Err(e) = crate::preview::open_externally(v.page().as_bytes(), "html") {
                            self.actions.push(Action::Notify(e, ToastKind::Error));
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt(("visualize", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
                    let job = crate::preview::html_job(&v.html, &p, f32::INFINITY);
                    ui.add(egui::Label::new(job).wrap().selectable(true));
                });
            }
            ResTab::Headers => {
                let view = &self.responses[&tab_id];
                let Outcome::Response { headers, .. } = &view.outcome else {
                    return;
                };
                egui::ScrollArea::both()
                    .id_salt(("res_headers", tab_id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new(("res_headers_grid", tab_id))
                            .num_columns(2)
                            .striped(true)
                            .spacing([24.0, 7.0])
                            .show(ui, |ui| {
                                for (k, v) in headers {
                                    ui.label(RichText::new(k).monospace().strong().color(p.text));
                                    if ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(v).monospace().color(p.muted),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Click to copy")
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(v.clone());
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
            ResTab::Cookies => {
                let view = &self.responses[&tab_id];
                if view.cookies.is_empty() {
                    theme::empty_state(
                        ui,
                        "\u{E6CA}",
                        "No cookies",
                        "This response didn't set any cookies.",
                    );
                    return;
                }
                egui::ScrollArea::both()
                    .id_salt(("res_cookies", tab_id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new(("cookies_grid", tab_id))
                            .num_columns(6)
                            .striped(true)
                            .spacing([18.0, 7.0])
                            .show(ui, |ui| {
                                for h in ["Name", "Value", "Domain", "Path", "Expires", "Flags"] {
                                    ui.label(RichText::new(h).size(11.5).color(p.faint));
                                }
                                ui.end_row();
                                for c in &view.cookies {
                                    ui.label(RichText::new(&c.name).monospace().strong());
                                    ui.label(
                                        RichText::new(truncate(&c.value, 40))
                                            .monospace()
                                            .color(p.muted),
                                    )
                                    .on_hover_text(&c.value);
                                    ui.label(&c.domain);
                                    ui.label(&c.path);
                                    ui.label(
                                        c.expires
                                            .map(|e| http::iso_time(e, 0)[..19].replace('T', " "))
                                            .unwrap_or_else(|| "Session".into()),
                                    );
                                    let mut flags = vec![];
                                    if c.http_only {
                                        flags.push("HttpOnly");
                                    }
                                    if c.secure {
                                        flags.push("Secure");
                                    }
                                    ui.label(flags.join(", "));
                                    ui.end_row();
                                }
                            });
                    });
            }
            ResTab::Tests => {
                let view = &self.responses[&tab_id];
                let has_output = view.script.as_ref().is_some_and(|r| !r.logs.is_empty() || !r.errors.is_empty());
                if view.tests.is_empty() && !has_output {
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new("No tests for this request").strong());
                        ui.label(
                            RichText::new(
                                "Add checks in the Tests tab (no code), or write pm.test() in Scripts → Post-response.",
                            )
                            .color(p.muted),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Open Tests").clicked() {
                                self.req_tabs.insert(tab_id, ReqTab::Tests);
                            }
                            if ui.button("Open Scripts").clicked() {
                                self.req_tabs.insert(tab_id, ReqTab::Scripts);
                                self.script_phase = crate::script::Phase::PostResponse;
                            }
                        });
                    });
                    return;
                }
                let passed = view.tests.iter().filter(|t| t.passed).count();
                let all = passed == view.tests.len();
                if !view.tests.is_empty() {
                    ui.label(
                        RichText::new(format!("{passed} of {} tests passed", view.tests.len()))
                            .strong()
                            .color(if all { p.accent } else { p.red }),
                    );
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt(("res_tests", tab_id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for t in &view.tests {
                            ui.horizontal(|ui| {
                                if t.passed {
                                    theme::soft_badge(ui, "PASS", p.accent, 11.0);
                                } else {
                                    theme::soft_badge(ui, "FAIL", p.red, 11.0);
                                }
                                ui.label(RichText::new(&t.name).color(p.text));
                                if !t.detail.is_empty() {
                                    ui.label(
                                        RichText::new(format!("— {}", t.detail))
                                            .size(12.0)
                                            .color(if t.passed { p.faint } else { p.red }),
                                    );
                                }
                            });
                            ui.add_space(2.0);
                        }
                        if let Some(report) = &view.script {
                            script_output(ui, &p, report);
                        }
                    });
            }
        }
    }

    fn response_body(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: crate::collection::Id,
        draft: &mut Request,
        coll: Option<crate::collection::Id>,
    ) {
        let p = pal(ui);
        let is_json = self
            .responses
            .get(&tab_id)
            .is_some_and(|v| v.json.is_some());
        let preview_kind = self
            .responses
            .get(&tab_id)
            .map_or(crate::preview::Kind::None, |v| v.preview);
        ui.horizontal(|ui| {
            let mut mode = self.body_mode;
            if preview_kind == crate::preview::Kind::Image {
                // There's nothing useful to show of an image as text.
                mode = BodyMode::Preview;
                theme::segmented(ui, &mut mode, &[(BodyMode::Preview, "Preview")]);
            } else if preview_kind == crate::preview::Kind::Html {
                theme::segmented(
                    ui,
                    &mut mode,
                    &[(BodyMode::Preview, "Preview"), (BodyMode::Pretty, "Pretty"), (BodyMode::Raw, "Raw")],
                );
                if mode == BodyMode::Tree {
                    mode = BodyMode::Preview;
                }
            } else if is_json {
                theme::segmented(
                    ui,
                    &mut mode,
                    &[
                        (BodyMode::Pretty, "Pretty"),
                        (BodyMode::Raw, "Raw"),
                        (BodyMode::Tree, "Tree"),
                    ],
                );
            } else {
                theme::segmented(
                    ui,
                    &mut mode,
                    &[(BodyMode::Pretty, "Pretty"), (BodyMode::Raw, "Raw")],
                );
                if mode == BodyMode::Tree || mode == BodyMode::Preview {
                    mode = BodyMode::Pretty;
                }
            }
            // An image only has a preview; don't make the next JSON response open on it.
            if preview_kind != crate::preview::Kind::Image {
                self.body_mode = mode;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, "\u{E1CA}", "Copy body").clicked() {
                    if let Some(Outcome::Response { body, .. }) =
                        self.responses.get(&tab_id).map(|v| &v.outcome)
                    {
                        ui.ctx().copy_text(body.clone());
                        self.actions
                            .push(Action::Notify("Copied".into(), ToastKind::Success));
                    }
                }
                let hint = if is_json {
                    "Search, or filter with $.path"
                } else {
                    "Search"
                };
                ui.add(
                    egui::TextEdit::singleline(&mut self.res_search)
                        .hint_text(RichText::new(format!("\u{E30C}  {hint}")).color(p.faint))
                        .desired_width(230.0)
                        .margin(Margin::symmetric(8, 3)),
                );
                if !self.res_search.is_empty() && !self.res_search.starts_with("$.") {
                    let n = self
                        .responses
                        .get(&tab_id)
                        .map(|v| count_matches(&v.display, &self.res_search))
                        .unwrap_or(0);
                    ui.label(
                        RichText::new(format!("{n} found"))
                            .size(11.5)
                            .color(if n > 0 { p.accent } else { p.faint }),
                    );
                }
            });
        });
        ui.add_space(4.0);

        let Some(view) = self.responses.get(&tab_id) else {
            return;
        };
        let search = self.res_search.clone();
        let mut new_tests: Vec<Assertion> = vec![];
        let mut new_captures: Vec<Capture> = vec![];
        let frame = egui::Frame::new()
            .fill(p.input)
            .corner_radius(8.0)
            .inner_margin(Margin::same(10));
        frame.show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::both().id_salt(("res_body", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
                // A JSON path in the search box filters the body.
                if let (Some(json), true) = (&view.json, search.starts_with("$.") || search == "$") {
                    match tests::json_path(json, &search) {
                        Some(v) => {
                            let text = serde_json::to_string_pretty(&v).unwrap_or_default();
                            json_text(ui, &text, &p, "");
                        }
                        None => {
                            ui.label(RichText::new(format!("Nothing at {search}")).color(p.muted));
                        }
                    }
                    return;
                }
                if view.display.is_empty() {
                    ui.label(RichText::new("(empty body)").color(p.faint));
                    return;
                }
                let show_preview = view.preview == crate::preview::Kind::Image
                    || (view.preview == crate::preview::Kind::Html && self.body_mode == BodyMode::Preview);
                if show_preview {
                    if let Some(err) = preview_body(ui, &p, tab_id, view) {
                        self.actions.push(Action::Notify(err, ToastKind::Error));
                    }
                    return;
                }
                match (self.body_mode, &view.json) {
                    (BodyMode::Tree, Some(json)) => {
                        if count_nodes(json) > TREE_LIMIT {
                            ui.label(RichText::new("This response is too large for the tree view. Use Pretty, or filter with a $.path first.").color(p.muted));
                        } else {
                            json_tree(ui, &p, json, "", "$", 0, &mut new_tests, &mut new_captures);
                        }
                    }
                    (BodyMode::Pretty, Some(_)) if view.display.len() < HIGHLIGHT_LIMIT => json_text(ui, &view.display, &p, &search),
                    (BodyMode::Raw, _) => {
                        let raw = match &view.outcome {
                            Outcome::Response { body, .. } => body.as_str(),
                            Outcome::Error { .. } => "",
                        };
                        plain_text(ui, raw, &p, &search);
                    }
                    _ => plain_text(ui, &view.display, &p, &search),
                }
            });
        });
        if !new_tests.is_empty() {
            draft.tests.extend(new_tests);
            self.actions.push(Action::Notify(
                "Added a test (see the Tests tab)".into(),
                ToastKind::Success,
            ));
        }
        if !new_captures.is_empty() {
            let names: Vec<String> = new_captures.iter().map(|c| c.var.clone()).collect();
            // Capture the value right away too, so it's usable immediately.
            if let Some(view) = self.responses.get(&tab_id) {
                let now = tests::captures(&new_captures, &view.outcome);
                self.actions.push(Action::Captures(coll, now));
            }
            draft.captures.extend(new_captures);
            self.actions.push(Action::Notify(
                format!("Saved {{{{{}}}}} (see the Variables tab)", names.join(", ")),
                ToastKind::Success,
            ));
        }
    }

    // -----------------------------------------------------------------------------------------
    // Code panel
    // -----------------------------------------------------------------------------------------

    fn code_panel(&mut self, ui: &mut egui::Ui, draft: &Request, origin: Option<Origin>) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Code snippet").size(14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, "×", "Close").clicked() {
                    self.ws.show_code = false;
                }
            });
        });
        ui.add_space(6.0);
        let mut lang = self.ws.code_lang;
        egui::ComboBox::from_id_salt("code_lang")
            .width(ui.available_width() - 8.0)
            .selected_text(lang.label())
            .height(480.0)
            .show_ui(ui, |ui| {
                for l in Lang::ALL {
                    ui.selectable_value(&mut lang, l, l.label());
                }
            });
        self.ws.code_lang = lang;
        let code = codegen::generate(lang, &self.prepare(draft, origin));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("\u{E1CA}  Copy").clicked() {
                ui.ctx().copy_text(code.clone());
                self.actions.push(Action::Notify(
                    format!("Copied {} code", lang.label()),
                    ToastKind::Success,
                ));
            }
            if ui.button("\u{E248}  Save").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("request.{}", lang.extension()))
                    .save_file()
                {
                    let msg = match std::fs::write(&path, &code) {
                        Ok(()) => (format!("Saved to {}", path.display()), ToastKind::Success),
                        Err(e) => (format!("Could not save: {e}"), ToastKind::Error),
                    };
                    self.actions.push(Action::Notify(msg.0, msg.1));
                }
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new("Variables are resolved with the active environment.")
                .size(11.5)
                .color(p.faint),
        );
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(p.input)
            .corner_radius(8.0)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                egui::ScrollArea::both()
                    .id_salt("code_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut s = code.as_str();
                        let mut layouter =
                            |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                                let job = highlight_code(buf.as_str(), &p, 12.0);
                                ui.fonts_mut(|f| f.layout_job(job))
                            };
                        ui.add(
                            egui::TextEdit::multiline(&mut s)
                                .code_editor()
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter),
                        );
                    });
            });
    }
}

fn used_vars(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else {
            break;
        };
        let name = rest[start + 2..start + 2 + len].trim().to_string();
        if !out.contains(&name) {
            out.push(name);
        }
        rest = &rest[start + 4 + len..];
    }
    out
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

/// A multi-line code editor, highlighting JSON or `{{variables}}`.
fn code_editor(
    ui: &mut egui::Ui,
    text: &mut String,
    json: bool,
    known: &HashSet<String>,
    p: &Palette,
    rows: usize,
    find: &str,
) {
    code_editor_ex(ui, text, json, known, p, rows, find, None);
}

/// [`code_editor`] with a fixed id, returning the text edit's output (for the cursor position).
#[allow(clippy::too_many_arguments)]
fn code_editor_ex(
    ui: &mut egui::Ui,
    text: &mut String,
    json: bool,
    known: &HashSet<String>,
    p: &Palette,
    rows: usize,
    find: &str,
    id: Option<egui::Id>,
) -> egui::text_edit::TextEditOutput {
    let known = known.clone();
    let p = *p;
    let find = find.to_string();
    let id = id.unwrap_or_else(|| ui.auto_id_with("code_editor"));
    let editing = ui.memory(|m| m.has_focus(id));
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = code_job(buf.as_str(), json, &known, &p, &find);
        if !editing {
            job = crate::rtl::shape_job(job);
        }
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8.0)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            egui::TextEdit::multiline(text)
                .code_editor()
                .frame(egui::Frame::NONE)
                .desired_rows(rows)
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .layouter(&mut layouter)
                .id(id)
                .show(ui)
        })
        .inner;
    // While editing, the line with the cursor in reading order (multi-line text can't be
    // drawn reordered while it's edited).
    if editing {
        if let Some(c) = output.cursor_range {
            let byte = cursor_byte(text, c.primary.index.0);
            let start = text[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let end = text[byte..].find('\n').map(|i| byte + i).unwrap_or(text.len());
            let line = &text[start..end];
            let row = output.galley.pos_from_cursor(c.primary);
            let rect = egui::Rect::from_min_size(output.galley_pos + row.left_bottom().to_vec2() - egui::vec2(row.left(), 0.0), egui::vec2(output.text_clip_rect.width(), 0.0));
            crate::rtl::preview(ui, rect, line.trim(), id);
        }
    }
    output
}

/// The byte offset of a text cursor. egui 0.37 sometimes reports the cursor inside shaped
/// (Arabic) text as a byte offset instead of a character index, so take whichever fits.
fn cursor_byte(text: &str, at: usize) -> usize {
    match text.char_indices().nth(at) {
        Some((i, _)) => i,
        None if at <= text.chars().count() => text.len(),
        None => {
            let mut b = at.min(text.len());
            while !text.is_char_boundary(b) {
                b -= 1;
            }
            b
        }
    }
}

/// A plain multi-line text field (descriptions) where Arabic reads right when not being edited.
pub fn prose_editor(ui: &mut egui::Ui, text: &mut String, rows: usize, hint: &str, id: egui::Id) -> egui::Response {
    let editing = ui.memory(|m| m.has_focus(id));
    let color = ui.visuals().text_color();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = LayoutJob::simple(buf.as_str().to_string(), FontId::proportional(14.0), color, wrap);
        if !editing {
            job = crate::rtl::shape_job(job);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = egui::TextEdit::multiline(text)
        .id(id)
        .desired_rows(rows)
        .desired_width(f32::INFINITY)
        .hint_text(hint)
        .margin(Margin::same(10))
        .layouter(&mut layouter)
        .show(ui);
    if editing {
        if let Some(c) = output.cursor_range {
            let byte = cursor_byte(text, c.primary.index.0);
            let start = text[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let end = text[byte..].find('\n').map(|i| byte + i).unwrap_or(text.len());
            crate::rtl::preview(ui, output.response.response.rect, text[start..end].trim(), id);
        }
    }
    output.response.response
}

/// The colored layout of a code editor's text.
fn code_job(text: &str, json: bool, known: &HashSet<String>, p: &Palette, find: &str) -> LayoutJob {
    {
        let buf = text;
        let p = *p;
        let mut job = if json && buf.len() < HIGHLIGHT_LIMIT {
            let mut j = highlight_json(buf, &p, 12.5);
            // Show variables inside JSON too.
            if buf.contains("{{") {
                j = mark_vars(j, buf, known, &p);
            }
            j
        } else {
            highlight_vars(buf, known, &p, FontId::monospace(12.5), false)
        };
        if !find.is_empty() {
            job = mark_matches(job, buf, find, search_color(&p));
        }
        job
    }
}

/// Recolors `{{variables}}` inside an already highlighted job.
fn mark_vars(job: LayoutJob, text: &str, known: &HashSet<String>, p: &Palette) -> LayoutJob {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(s) = text[from..].find("{{") {
        let start = from + s;
        let Some(len) = text[start + 2..].find("}}") else {
            break;
        };
        let end = start + 4 + len;
        spans.push((
            start..end,
            known.contains(text[start + 2..start + 2 + len].trim()),
        ));
        from = end;
    }
    let mut out = LayoutJob {
        text: job.text.clone(),
        wrap: job.wrap.clone(),
        ..Default::default()
    };
    for s in &job.sections {
        let (mut pos, end) = (s.byte_range.start.0, s.byte_range.end.0);
        while pos < end {
            let inside = spans.iter().find(|(r, _)| r.start <= pos && pos < r.end);
            let next = match inside {
                Some((r, _)) => r.end.min(end),
                None => spans
                    .iter()
                    .map(|(r, _)| r.start)
                    .filter(|st| *st > pos)
                    .min()
                    .unwrap_or(end)
                    .min(end),
            };
            let mut format = s.format.clone();
            if let Some((_, ok)) = inside {
                format.color = if *ok { p.accent } else { p.red };
                format.background = if *ok {
                    p.accent_soft
                } else {
                    Color32::from_rgba_unmultiplied(p.red.r(), p.red.g(), p.red.b(), 28)
                };
            }
            out.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(pos)..ByteIndex(next),
                format,
            });
            pos = next;
        }
    }
    out
}

fn search_color(p: &Palette) -> Color32 {
    if p.dark {
        Color32::from_rgba_unmultiplied(0xF5, 0xA5, 0x24, 90)
    } else {
        Color32::from_rgba_unmultiplied(0xF5, 0xC5, 0x24, 140)
    }
}

fn json_text(ui: &mut egui::Ui, text: &str, p: &Palette, search: &str) {
    let p = *p;
    let search = search.to_string();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = mark_matches(
            highlight_json(buf.as_str(), &p, 12.5),
            buf.as_str(),
            &search,
            search_color(&p),
        );
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut s = text;
    ui.add(
        egui::TextEdit::multiline(&mut s)
            .code_editor()
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );
}

fn plain_text(ui: &mut egui::Ui, text: &str, p: &Palette, search: &str) {
    let p = *p;
    let search = search.to_string();
    let big = text.len() >= HIGHLIGHT_LIMIT;
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = LayoutJob::simple(
            buf.as_str().to_string(),
            FontId::monospace(12.5),
            p.text,
            wrap,
        );
        if !big {
            job = mark_matches(job, buf.as_str(), &search, search_color(&p));
        }
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut s = text;
    ui.add(
        egui::TextEdit::multiline(&mut s)
            .code_editor()
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );
}

fn count_nodes(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(count_nodes).sum::<usize>(),
        Value::Object(o) => 1 + o.values().map(count_nodes).sum::<usize>(),
        _ => 1,
    }
}

fn child_path(parent: &str, key: &str) -> String {
    let simple = key
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_alphanumeric() || c == '_');
    match (parent.is_empty(), simple) {
        (true, true) => key.to_string(),
        (false, true) => format!("{parent}.{key}"),
        (_, false) => format!("{parent}[\"{key}\"]"),
    }
}

/// A collapsible JSON tree. Right-click a value to copy it or turn it into a test or variable.
#[allow(clippy::too_many_arguments)]
fn json_tree(
    ui: &mut egui::Ui,
    p: &Palette,
    v: &Value,
    path: &str,
    label: &str,
    depth: usize,
    tests_out: &mut Vec<Assertion>,
    caps_out: &mut Vec<Capture>,
) {
    let key_color = if p.dark {
        Color32::from_rgb(0x8C, 0xB4, 0xFF)
    } else {
        Color32::from_rgb(0x1F, 0x5F, 0xCC)
    };
    match v {
        Value::Object(_) | Value::Array(_) => {
            let summary = match v {
                Value::Object(o) => format!("{{ {} }}", o.len()),
                Value::Array(a) => format!("[ {} ]", a.len()),
                _ => String::new(),
            };
            let header = RichText::new(format!("{label}  "))
                .monospace()
                .color(key_color);
            let id = ui.scope_id().with(("json", path));
            let resp = egui::CollapsingHeader::new(egui::WidgetText::from(header))
                .id_salt(id)
                .default_open(depth < 2)
                .show(ui, |ui| match v {
                    Value::Object(o) => {
                        for (k, child) in o {
                            json_tree(
                                ui,
                                p,
                                child,
                                &child_path(path, k),
                                k,
                                depth + 1,
                                tests_out,
                                caps_out,
                            );
                        }
                    }
                    Value::Array(a) => {
                        for (i, child) in a.iter().enumerate().take(500) {
                            json_tree(
                                ui,
                                p,
                                child,
                                &format!("{path}[{i}]"),
                                &format!("[{i}]"),
                                depth + 1,
                                tests_out,
                                caps_out,
                            );
                        }
                        if a.len() > 500 {
                            ui.label(
                                RichText::new(format!("… {} more", a.len() - 500)).color(p.faint),
                            );
                        }
                    }
                    _ => {}
                });
            ui.painter().text(
                resp.header_response.rect.right_center() + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                summary,
                FontId::monospace(11.5),
                p.faint,
            );
            let path_owned = path.to_string();
            resp.header_response.context_menu(|ui| {
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(path_owned.clone());
                }
                if ui.button("Copy JSON").clicked() {
                    ui.ctx()
                        .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
                }
                if ui.button("Test: exists").clicked() {
                    tests_out.push(Assertion {
                        source: Source::Json,
                        path: path_owned.clone(),
                        op: Op::Exists,
                        expected: String::new(),
                        ..Default::default()
                    });
                }
                if let Value::Array(a) = v {
                    if ui.button(format!("Test: length is {}", a.len())).clicked() {
                        tests_out.push(Assertion {
                            source: Source::Json,
                            path: format!("{path_owned}.length"),
                            op: Op::Equals,
                            expected: a.len().to_string(),
                            ..Default::default()
                        });
                    }
                }
            });
        }
        leaf => {
            let (text, color) = match leaf {
                Value::String(s) => (
                    format!("\"{}\"", shape(s)),
                    if p.dark {
                        Color32::from_rgb(0x7F, 0xD6, 0xA4)
                    } else {
                        Color32::from_rgb(0x1A, 0x7F, 0x4B)
                    },
                ),
                Value::Number(n) => (
                    n.to_string(),
                    if p.dark {
                        Color32::from_rgb(0xF5, 0xB0, 0x6B)
                    } else {
                        Color32::from_rgb(0xB8, 0x5C, 0x00)
                    },
                ),
                Value::Bool(b) => (
                    b.to_string(),
                    if p.dark {
                        Color32::from_rgb(0xD9, 0x8B, 0xF2)
                    } else {
                        Color32::from_rgb(0x8A, 0x3F, 0xC2)
                    },
                ),
                _ => ("null".into(), p.faint),
            };
            let resp = ui
                .horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.label(
                        RichText::new(format!("{label}:"))
                            .monospace()
                            .color(key_color),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace().color(color)).truncate(),
                    );
                })
                .response
                .interact(egui::Sense::click());
            let path_owned = path.to_string();
            let plain = tests::plain(leaf);
            let resp = resp.on_hover_text(format!("{path_owned}\nRight-click for actions"));
            resp.context_menu(|ui| {
                ui.set_min_width(220.0);
                if ui.button("Copy value").clicked() {
                    ui.ctx().copy_text(plain.clone());
                }
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(path_owned.clone());
                }
                ui.separator();
                if ui
                    .button(format!("Test: equals {}", truncate(&plain, 24)))
                    .clicked()
                {
                    tests_out.push(Assertion {
                        source: Source::Json,
                        path: path_owned.clone(),
                        op: Op::Equals,
                        expected: plain.clone(),
                        ..Default::default()
                    });
                }
                if ui.button("Test: exists").clicked() {
                    tests_out.push(Assertion {
                        source: Source::Json,
                        path: path_owned.clone(),
                        op: Op::Exists,
                        expected: String::new(),
                        ..Default::default()
                    });
                }
                let var: String = label.trim_matches(|c| c == '[' || c == ']').to_string();
                let var = if var.chars().all(|c| c.is_ascii_digit()) {
                    "value".to_string()
                } else {
                    var
                };
                if ui
                    .button(format!("Save to variable {{{{{var}}}}}"))
                    .clicked()
                {
                    caps_out.push(Capture {
                        var,
                        path: path_owned.clone(),
                        ..Default::default()
                    });
                }
            });
        }
    }
}

/// Shows an image response, or the reader view of an HTML page. Returns an error to report.
fn preview_body(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    tab_id: crate::collection::Id,
    view: &crate::app::ResponseView,
) -> Option<String> {
    use crate::preview;
    let Outcome::Response { bytes, .. } = &view.outcome else { return None };
    let content_type = view.outcome.header("content-type").unwrap_or("");
    let mut error = None;
    let mut open = |ui: &mut egui::Ui, label: &str| {
        if ui.button(label).clicked() {
            if let Err(e) = preview::open_externally(bytes, preview::extension_for(content_type)) {
                error = Some(e);
            }
        }
    };

    match view.preview {
        preview::Kind::Image => match &view.image {
            Some(Ok(img)) => {
                let [w, h] = img.image.size;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} · {w} × {h} px · {}", img.format, theme::format_size(bytes.len())))
                            .color(p.muted),
                    );
                    open(ui, "Open in viewer \u{E5DE}");
                });
                ui.add_space(6.0);
                let texture = view.texture.get_or_init(|| {
                    ui.ctx().load_texture(format!("preview-{tab_id}"), img.image.clone(), Default::default())
                });
                // Fit the visible area; small images (icons) may grow a little so they're visible on a projector.
                let max_scale = if w < 256 { 2.0 } else { 1.0 };
                let visible_h = (ui.clip_rect().bottom() - ui.cursor().top() - 8.0).max(120.0);
                let scale = ((ui.available_width() - 4.0) / w as f32)
                    .min(visible_h / h as f32)
                    .min(max_scale)
                    .max(0.05);
                let size = egui::vec2(w as f32, h as f32) * scale;
                // A checkerboard shows which parts are transparent.
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                let cell = 10.0;
                let (light, dark) = if p.dark { (Color32::from_gray(58), Color32::from_gray(44)) } else { (Color32::from_gray(238), Color32::from_gray(222)) };
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, light);
                let (cols, rows) = ((size.x / cell).ceil() as i32, (size.y / cell).ceil() as i32);
                for y in 0..rows {
                    for x in (y % 2..cols).step_by(2) {
                        let min = rect.min + egui::vec2(x as f32 * cell, y as f32 * cell);
                        painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, dark);
                    }
                }
                painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(p.red));
                open(ui, "Open in viewer \u{E5DE}");
            }
            None => {}
        },
        preview::Kind::Html => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Reader view: the page's text and structure, without its styling.").color(p.muted));
                open(ui, "Open in browser \u{E5DE}");
            });
            ui.separator();
            let job = view.html.get_or_init(|| {
                let body = String::from_utf8_lossy(bytes);
                preview::html_job(&body, p, f32::INFINITY)
            });
            ui.add(egui::Label::new(job.clone()).wrap().selectable(true));
        }
        preview::Kind::None => {}
    }
    error
}

// ---------------------------------------------------------------------------------------------
// GraphQL editor: suggestions from the schema, and a schema explorer
// ---------------------------------------------------------------------------------------------

/// What the suggestion popup remembers between frames.
#[derive(Clone, Default)]
struct GqlPopup {
    open: bool,
    count: usize,
    selected: usize,
    /// Ctrl+Space: show suggestions even with nothing typed.
    forced: bool,
    /// Esc hides the popup until the text changes.
    dismissed_for: Option<String>,
}

fn gql_cursor(ctx: &egui::Context, id: egui::Id) -> Option<usize> {
    egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).map(|r| r.primary.index.0)
}

fn gql_set_cursor(ctx: &egui::Context, id: egui::Id, at: usize) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let c = egui::text::CCursorRange::one(egui::text::CCursor::new(at));
        state.cursor.set_char_range(Some(c));
        egui::TextEdit::store_state(ctx, id, state);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Puts `s` in place of `range` (bytes). Objects get a `{ }` block with the cursor inside.
fn gql_accept(ctx: &egui::Context, id: egui::Id, text: &mut String, range: std::ops::Range<usize>, s: &crate::graphql::Suggestion) {
    let line_start = text[..range.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = text[line_start..].chars().take_while(|c| *c == ' ').collect();
    let (insert, cursor_after) = if s.has_fields {
        let head = format!("{} {{\n{indent}  ", s.name);
        (format!("{head}\n{indent}}}"), head.chars().count())
    } else {
        (s.name.clone(), s.name.chars().count())
    };
    text.replace_range(range.clone(), &insert);
    let at = text[..range.start].chars().count() + cursor_after;
    gql_set_cursor(ctx, id, at);
}

fn gql_editor(
    ui: &mut egui::Ui,
    text: &mut String,
    schema: Option<&crate::graphql::Schema>,
    known: &HashSet<String>,
    p: &Palette,
    find: &str,
    tab_id: crate::collection::Id,
) {
    let id = ui.make_persistent_id(("gql_query", tab_id));
    let popup_id = id.with("popup");
    let ctx = ui.ctx().clone();
    let mut popup: GqlPopup = ctx.data(|d| d.get_temp(popup_id)).unwrap_or_default();
    let focused = ctx.memory(|m| m.has_focus(id));

    // Keys are taken before the text edit sees them, so Enter / Tab pick a suggestion.
    let mut accept = None;
    if focused && schema.is_some() {
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Space) {
                popup.forced = true;
                popup.dismissed_for = None;
            }
            if popup.open && popup.count > 0 {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    popup.selected = (popup.selected + 1) % popup.count;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    popup.selected = (popup.selected + popup.count - 1) % popup.count;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                    accept = Some(popup.selected);
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    popup.dismissed_for = Some(text.clone());
                    popup.forced = false;
                }
            }
        });
    }
    // Enter keeps the line's indentation (and indents one more step after `{`).
    if focused && accept.is_none() && !(popup.open && popup.count > 0) {
        if let Some(cursor) = gql_cursor(&ctx, id) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                let at = text.char_indices().nth(cursor).map(|(i, _)| i).unwrap_or(text.len());
                let line_start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let mut indent: String = text[line_start..at].chars().take_while(|c| *c == ' ').collect();
                if text[..at].trim_end().ends_with('{') {
                    indent.push_str("  ");
                }
                let insert = format!("\n{indent}");
                text.insert_str(at, &insert);
                gql_set_cursor(&ctx, id, cursor + insert.chars().count());
            }
        }
    }
    if let (Some(i), Some(schema), Some(cursor)) = (accept, schema, gql_cursor(&ctx, id)) {
        if let Some((range, list)) = crate::graphql::complete(schema, text, cursor, popup.forced) {
            if let Some(s) = list.get(i) {
                gql_accept(&ctx, id, text, range, s);
            }
        }
        popup = GqlPopup::default();
    }

    let before = text.clone();
    let output = code_editor_ex(ui, text, false, known, p, 12, find, Some(id));
    if *text != before {
        popup.forced = false;
        popup.dismissed_for = None;
        popup.selected = 0;
    }

    // The editor's own focus: `Response::has_focus` is also false while the OS window is inactive.
    let editing = ctx.memory(|m| m.has_focus(id));
    let suggestions = match (schema, output.cursor_range, editing) {
        (Some(schema), Some(range), true) if popup.dismissed_for.as_deref() != Some(text.as_str()) => {
            crate::graphql::complete(schema, text, range.primary.index.0, popup.forced).map(|(r, list)| (r, list, range.primary))
        }
        _ => None,
    };
    popup.open = suggestions.is_some();
    popup.count = suggestions.as_ref().map_or(0, |s| s.1.len());
    popup.selected = popup.selected.min(popup.count.saturating_sub(1));

    if let Some((range, list, cursor)) = suggestions {
        let at = output.galley_pos + output.galley.pos_from_cursor(cursor).left_bottom().to_vec2() + egui::vec2(0.0, 4.0);
        let mut clicked = None;
        egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, s) in list.iter().enumerate() {
                        let selected = i == popup.selected;
                        let row = ui.horizontal(|ui| {
                            ui.set_min_width(300.0);
                            let name = RichText::new(&s.name).monospace().color(if selected { p.text } else { p.muted });
                            ui.label(if selected { name.strong() } else { name });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(&s.detail).monospace().size(11.5).color(p.purple));
                            });
                        });
                        let rect = row.response.rect.expand2(egui::vec2(4.0, 1.0));
                        if selected {
                            ui.painter().rect_filled(rect, 4.0, p.accent_soft);
                            // Paint the row again on top of its highlight.
                            ui.painter().text(rect.left_center() + egui::vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, &s.name, FontId::monospace(12.5), p.text);
                            ui.painter().text(rect.right_center() - egui::vec2(4.0, 0.0), egui::Align2::RIGHT_CENTER, &s.detail, FontId::monospace(11.5), p.purple);
                        }
                        if ui.interact(rect, popup_id.with(i), egui::Sense::click()).clicked() {
                            clicked = Some(i);
                        }
                    }
                    if let Some(s) = list.get(popup.selected).filter(|s| !s.description.is_empty()) {
                        ui.separator();
                        ui.add(egui::Label::new(RichText::new(&s.description).size(12.0).color(p.muted)).wrap());
                    }
                    ui.label(RichText::new("↑↓ choose · Enter / Tab insert · Esc close").size(10.5).color(p.faint));
                });
            });
        if let Some(i) = clicked {
            gql_accept(&ctx, id, text, range, &list[i]);
            popup = GqlPopup::default();
        }
    }
    ctx.data_mut(|d| d.insert_temp(popup_id, popup));
}

/// The schema as a tree: root fields first, each object field opens into its own fields.
fn schema_explorer(ui: &mut egui::Ui, schema: &crate::graphql::Schema, query: &mut String, p: &Palette, tab_id: crate::collection::Id) {
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new("Schema explorer").strong())
        .id_salt(("gql_explorer", tab_id))
        .show(ui, |ui| {
            ui.label(RichText::new("Click a field to add it to the query at the cursor.").size(11.5).color(p.faint));
            egui::ScrollArea::vertical().id_salt(("gql_explorer_scroll", tab_id)).max_height(260.0).show(ui, |ui| {
                for (label, root) in [("Query", &schema.query), ("Mutation", &schema.mutation), ("Subscription", &schema.subscription)] {
                    if let Some(root) = root {
                        ui.label(RichText::new(format!("{label}  ({root})")).color(p.accent).strong());
                        explorer_fields(ui, schema, root, query, p, tab_id, 0, root);
                    }
                }
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn explorer_fields(
    ui: &mut egui::Ui,
    schema: &crate::graphql::Schema,
    type_name: &str,
    query: &mut String,
    p: &Palette,
    tab_id: crate::collection::Id,
    depth: usize,
    path: &str,
) {
    let editor = ui.make_persistent_id(("gql_query", tab_id));
    for f in schema.fields(type_name) {
        let args = if f.args.is_empty() {
            String::new()
        } else {
            format!("({})", f.args.iter().map(|(n, t)| format!("{n}: {t}")).collect::<Vec<_>>().join(", "))
        };
        let text = format!("{}{args}: {}", f.name, f.type_label);
        let insert = |query: &mut String, ui: &egui::Ui| {
            let ctx = ui.ctx().clone();
            let at = gql_cursor(&ctx, editor).unwrap_or(query.chars().count()).min(query.chars().count());
            let byte = query.char_indices().nth(at).map(|(i, _)| i).unwrap_or(query.len());
            let s = crate::graphql::Suggestion {
                name: f.name.clone(),
                detail: f.type_label.clone(),
                description: f.description.clone(),
                has_fields: !schema.fields(&f.base).is_empty(),
            };
            gql_accept(&ctx, editor, query, byte..byte, &s);
        };
        let nested = !schema.fields(&f.base).is_empty() && depth < 4;
        if nested {
            let resp = egui::CollapsingHeader::new(RichText::new(&text).monospace().size(12.0))
                .id_salt(("gql_field", tab_id, path, &f.name))
                .show(ui, |ui| {
                    if ui.small_button(format!("Insert {}", f.name)).clicked() {
                        insert(query, ui);
                    }
                    explorer_fields(ui, schema, &f.base, query, p, tab_id, depth + 1, &format!("{path}.{}", f.name));
                });
            if !f.description.is_empty() {
                resp.header_response.on_hover_text(&f.description);
            }
        } else {
            let r = ui.add(egui::Label::new(RichText::new(format!("   {text}")).monospace().size(12.0).color(p.muted)).sense(egui::Sense::click()));
            let r = if f.description.is_empty() { r } else { r.on_hover_text(&f.description) };
            if r.clicked() {
                insert(query, ui);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Scripts (pre-request / post-response JavaScript)
// ---------------------------------------------------------------------------------------------

/// The dot on the Scripts tab: how many of the two scripts are written.
pub fn scripts_dot(events: &[Value]) -> Option<String> {
    let n = [crate::script::Phase::PreRequest, crate::script::Phase::PostResponse]
        .iter()
        .filter(|ph| !crate::script::code_of(events, ph.listen()).trim().is_empty())
        .count();
    (n > 0).then(|| n.to_string())
}

const PRE_SNIPPETS: [(&str, &str); 8] = [
    ("Set an environment variable", "pm.environment.set(\"name\", \"value\");"),
    ("Set a collection variable", "pm.collectionVariables.set(\"name\", \"value\");"),
    ("Get a variable", "const value = pm.variables.get(\"name\");"),
    ("Add a header", "pm.request.headers.add({ key: \"X-Request-Id\", value: pm.variables.replaceIn(\"{{$guid}}\") });"),
    ("Save a timestamp", "pm.environment.set(\"timestamp\", Date.now());"),
    ("Get a token first (sendRequest)", "pm.sendRequest({\n    url: pm.variables.replaceIn(\"{{baseUrl}}/login\"),\n    method: \"POST\",\n    header: { \"Content-Type\": \"application/json\" },\n    body: { mode: \"raw\", raw: JSON.stringify({ username: \"admin\", password: \"secret\" }) }\n}, (err, res) => {\n    pm.environment.set(\"token\", res.json().token);\n});"),
    ("Sign the body (HMAC SHA256)", "const signature = CryptoJS.HmacSHA256(pm.request.body.raw, \"secret\").toString();\npm.request.headers.upsert({ key: \"X-Signature\", value: signature });"),
    ("Log to the console", "console.log(pm.request.method, pm.request.url.toString());"),
];

const POST_SNIPPETS: [(&str, &str); 9] = [
    ("Status code is 200", "pm.test(\"Status code is 200\", () => {\n    pm.response.to.have.status(200);\n});"),
    ("Response time is below 500 ms", "pm.test(\"Response time is below 500 ms\", () => {\n    pm.expect(pm.response.responseTime).to.be.below(500);\n});"),
    ("Body is JSON", "pm.test(\"Body is JSON\", () => {\n    pm.response.to.be.json;\n});"),
    ("Check a JSON value", "pm.test(\"Check a value\", () => {\n    const json = pm.response.json();\n    pm.expect(json.name).to.eql(\"value\");\n});"),
    ("Body has a property", "pm.test(\"Has an id\", () => {\n    pm.expect(pm.response.json()).to.have.property(\"id\");\n});"),
    ("Header is present", "pm.test(\"Content-Type is present\", () => {\n    pm.response.to.have.header(\"Content-Type\");\n});"),
    ("Save a value from the response", "pm.environment.set(\"id\", pm.response.json().id);"),
    ("Log the response", "console.log(pm.response.json());"),
    ("Next request in the Runner", "pm.execution.setNextRequest(\"Request name\");"),
];

/// A JavaScript editor with syntax colors.
fn script_editor(ui: &mut egui::Ui, text: &mut String, p: &Palette, find: &str, id: egui::Id) -> egui::Response {
    let p = *p;
    let find = find.to_string();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight_code(buf.as_str(), &p, 12.5);
        if !find.is_empty() {
            job = mark_matches(job, buf.as_str(), &find, search_color(&p));
        }
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8.0)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id(id)
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .desired_rows(16)
                    .desired_width(f32::INFINITY)
                    .lock_focus(true)
                    .hint_text(RichText::new("// JavaScript. Click a snippet on the right to start.").color(p.faint))
                    .layouter(&mut layouter),
            )
        })
        .inner
}

impl GazaMan {
    /// The pre-request and post-response scripts of a request, folder or collection.
    pub fn scripts_pane(&mut self, ui: &mut egui::Ui, owner: crate::collection::Id, events: &mut Vec<Value>, level: &str) {
        use crate::script::{Phase, code_of, set_code};
        let p = pal(ui);
        let has = |ph: Phase| !code_of(events, ph.listen()).trim().is_empty();
        ui.horizontal(|ui| {
            let mut phase = self.script_phase;
            let label = |ph: Phase| if has(ph) { format!("{}  ●", ph.label()) } else { ph.label().to_string() };
            let (pre, post) = (label(Phase::PreRequest), label(Phase::PostResponse));
            theme::segmented(ui, &mut phase, &[(Phase::PreRequest, pre.as_str()), (Phase::PostResponse, post.as_str())]);
            self.script_phase = phase;
        });
        let phase = self.script_phase;
        let when = match (phase, level) {
            (Phase::PreRequest, "request") => "Runs before the request is sent: set variables, add headers, or fetch a token first.",
            (Phase::PostResponse, "request") => "Runs after the response arrives: write pm.test() checks and save values for later requests.",
            (Phase::PreRequest, _) => "Runs before every request inside, before the request's own pre-request script.",
            (Phase::PostResponse, _) => "Runs after every request inside, before the request's own post-response script.",
        };
        ui.label(RichText::new(when).size(12.0).color(p.muted));
        ui.add_space(6.0);

        let mut code = code_of(events, phase.listen());
        let before = code.clone();
        let snippets: &[(&str, &str)] = match phase {
            Phase::PreRequest => &PRE_SNIPPETS,
            Phase::PostResponse => &POST_SNIPPETS,
        };
        let id = ui.make_persistent_id(("script_editor", owner, phase.listen()));
        let side = 240.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - side - 12.0).max(260.0));
                script_editor(ui, &mut code, &p, &self.body_find, id);
            });
            ui.vertical(|ui| {
                ui.set_width(side);
                theme::section_title(ui, "Snippets");
                for (name, snippet) in snippets {
                    let r = ui.add(egui::Label::new(RichText::new(*name).size(12.5).color(p.blue)).sense(egui::Sense::click()));
                    if r.on_hover_text(*snippet).clicked() {
                        if !code.is_empty() && !code.ends_with('\n') {
                            code.push('\n');
                        }
                        code.push_str(snippet);
                        code.push('\n');
                    }
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new("pm.* works like in Postman: variables, pm.test / pm.expect, pm.request, pm.response, pm.sendRequest, console.log and CryptoJS. Output shows in Test Results and the Console.")
                        .size(11.5)
                        .color(p.faint),
                );
            });
        });
        if code != before {
            set_code(events, phase.listen(), &code);
        }
    }
}

/// What scripts printed and the errors they hit, under the test results.
fn script_output(ui: &mut egui::Ui, p: &Palette, report: &crate::script::Report) {
    if report.logs.is_empty() && report.errors.is_empty() {
        return;
    }
    ui.add_space(10.0);
    theme::section_title(ui, "Script output");
    for e in &report.errors {
        ui.horizontal_wrapped(|ui| {
            theme::soft_badge(ui, "ERROR", p.red, 11.0);
            ui.label(RichText::new(format!("{} · {}", e.phase.label(), e.script)).size(12.0).color(p.muted));
            ui.label(RichText::new(&e.message).monospace().size(12.0).color(p.red));
        });
    }
    for l in &report.logs {
        let color = match l.level.as_str() {
            "error" => p.red,
            "warn" => p.amber,
            "info" => p.blue,
            _ => p.text,
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(match l.phase { crate::script::Phase::PreRequest => "pre ›", _ => "post ›" }).monospace().size(11.0).color(p.faint));
            ui.add(egui::Label::new(RichText::new(shape(&l.text)).monospace().size(12.0).color(color)).wrap().selectable(true))
                .on_hover_text(&l.script);
        });
    }
}
