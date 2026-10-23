//! egui shapes each Arabic word (joined letters, right-to-left inside the word) but lays the
//! words out left-to-right. We reorder the words of right-to-left runs before display.

fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

fn shape_line(line: &str) -> String {
    let info = unicode_bidi::BidiInfo::new(line, None);
    let Some(para) = info.paragraphs.first() else {
        return line.to_string();
    };
    let (levels, runs) = info.visual_runs(para, para.range.clone());
    runs.into_iter()
        .map(|run| {
            let text = &line[run.clone()];
            if levels[run.start].is_rtl() { reverse_rtl_run(text) } else { text.to_string() }
        })
        .collect()
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        c => c,
    }
}

/// A right-to-left run in visual order: the pieces (Arabic words, and the spaces and
/// punctuation between them) in reverse. egui already draws each Arabic word right to left,
/// so a word stays as it is; the other pieces are reversed and mirrored, as bidi wants.
fn reverse_rtl_run(text: &str) -> String {
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_arabic = None;
    for c in text.chars() {
        let arabic = is_arabic(c);
        if current_arabic.is_some_and(|a| a != arabic) {
            pieces.push(std::mem::take(&mut current));
        }
        current.push(c);
        current_arabic = Some(arabic);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
        .into_iter()
        .rev()
        .map(|p| if p.chars().next().is_some_and(is_arabic) { p } else { p.chars().rev().map(mirror).collect() })
        .collect()
}

/// Returns text ready to be shown in a label. Only for display, never for editing.
pub fn shape(s: &str) -> String {
    if !s.chars().any(is_arabic) {
        return s.to_string();
    }
    s.lines().map(shape_line).collect::<Vec<_>>().join("\n")
}

pub fn has_arabic(s: &str) -> bool {
    s.chars().any(is_arabic)
}

/// Like [`shape`], but keeps every newline (so the text keeps its exact length and lines).
fn shape_keep_lines(s: &str) -> String {
    s.split('\n').map(shape_line).collect::<Vec<_>>().join("\n")
}

/// Puts the Arabic of an already colored layout in reading order, section by section.
///
/// egui text fields can't edit right-to-left text, so this is only for a field that isn't
/// being edited: the text stays the same, only how it's drawn changes.
pub fn shape_job(job: eframe::egui::text::LayoutJob) -> eframe::egui::text::LayoutJob {
    if !has_arabic(&job.text) {
        return job;
    }
    let mut out = eframe::egui::text::LayoutJob {
        wrap: job.wrap.clone(),
        halign: job.halign,
        justify: job.justify,
        break_on_newline: job.break_on_newline,
        first_row_min_height: job.first_row_min_height,
        ..Default::default()
    };
    for section in &job.sections {
        let text = &job.text[section.byte_range.start.0..section.byte_range.end.0];
        let text = if has_arabic(text) { shape_keep_lines(text) } else { text.to_string() };
        out.append(&text, section.leading_space, section.format.clone());
    }
    out
}

/// While a field with Arabic is being edited, shows the line under the cursor (or all of a
/// short field) in reading order just below the field.
pub fn preview(ui: &eframe::egui::Ui, field: eframe::egui::Rect, text: &str, id: eframe::egui::Id) {
    use eframe::egui;
    if !has_arabic(text) {
        return;
    }
    let p = crate::theme::pal(ui);
    egui::Area::new(id.with("rtl_preview"))
        .order(egui::Order::Foreground)
        .fixed_pos(field.left_bottom() + egui::vec2(0.0, 3.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.set_max_width(field.width().max(240.0));
                ui.label(egui::RichText::new("Reads as:").size(10.5).color(p.faint));
                ui.label(egui::RichText::new(shape(text)).size(13.5).color(p.text));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

    #[test]
    fn reorders_inside_sections_only() {
        let mut job = LayoutJob::default();
        let f = |c| TextFormat::simple(FontId::monospace(12.0), c);
        job.append("{\"city\": ", 0.0, f(Color32::WHITE));
        job.append("\"مدينة غزة\"", 0.0, f(Color32::GREEN));
        job.append(",\n\"n\": 1}", 0.0, f(Color32::WHITE));
        let out = shape_job(job.clone());
        assert_eq!(out.sections.len(), 3, "colors stay where they were");
        assert_eq!(out.text.chars().count(), job.text.chars().count(), "same length, so nothing shifts");
        assert!(out.text.contains("غزة مدينة"), "{}", out.text);
        assert!(out.text.ends_with(",\n\"n\": 1}"), "other lines untouched");
        // No Arabic: the very same job.
        let mut plain = LayoutJob::default();
        plain.append("hello", 0.0, f(Color32::WHITE));
        assert_eq!(shape_job(plain.clone()).text, plain.text);
    }
}
