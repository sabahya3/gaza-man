//! A magnifying lens that follows the mouse, for showing any part of the app to a class.
//!
//! It works on what egui already drew this frame: we copy every shape, scale it around the
//! cursor and draw the copy, clipped to the lens, on the top layer.

use eframe::egui::{self, Color32, Key, Modifiers, Order, Rect, Shape, Stroke, emath::TSTransform};

const MIN_ZOOM: f32 = 1.25;
const MAX_ZOOM: f32 = 8.0;
const MIN_SIZE: egui::Vec2 = egui::vec2(200.0, 130.0);
const MAX_SIZE: egui::Vec2 = egui::vec2(1100.0, 720.0);

pub struct Lens {
    pub enabled: bool,
    pub zoom: f32,
    pub size: egui::Vec2,
}

impl Default for Lens {
    fn default() -> Self {
        Self {
            enabled: false,
            zoom: 2.5,
            size: egui::vec2(480.0, 300.0),
        }
    }
}

impl Lens {
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Shortcuts: ⌘/Ctrl+L toggles, Esc closes, Alt+scroll zooms, Alt+Shift+scroll resizes,
    /// and `[` / `]` zoom when nothing is being typed. Call before laying out the UI so the
    /// scroll we use doesn't also scroll the panel under the lens.
    pub fn handle_input(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::COMMAND, Key::L) {
                self.enabled = !self.enabled;
            }
            if !self.enabled {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                self.enabled = false;
            }
            if !typing {
                if i.consume_key(Modifiers::NONE, Key::CloseBracket) {
                    self.zoom = (self.zoom + 0.5).clamp(MIN_ZOOM, MAX_ZOOM);
                }
                if i.consume_key(Modifiers::NONE, Key::OpenBracket) {
                    self.zoom = (self.zoom - 0.5).clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }
            if i.modifiers.alt {
                // egui turns Shift+scroll into horizontal scroll, so read whichever axis moved.
                let d = i.smooth_scroll_delta;
                let amount = if d.y != 0.0 { d.y } else { d.x };
                if amount != 0.0 {
                    if i.modifiers.shift {
                        self.size = (self.size * (1.0 + amount * 0.004)).clamp(MIN_SIZE, MAX_SIZE);
                    } else {
                        self.zoom = (self.zoom * (1.0 + amount * 0.004)).clamp(MIN_ZOOM, MAX_ZOOM);
                    }
                    i.smooth_scroll_delta = egui::Vec2::ZERO;
                }
            }
        });
        let zoom = self.zoom;
        self.set_zoom(zoom);
    }

    /// Draws the lens over everything painted so far. Call at the very end of the frame.
    pub fn paint(&self, ctx: &egui::Context, accent: Color32) {
        if !self.enabled {
            return;
        }
        let Some(pointer) = ctx.pointer_hover_pos() else {
            return;
        };
        let screen = ctx.content_rect();
        // Centered on the cursor, but pushed back inside the window near the edges.
        let size = self.size.min(screen.size() - egui::vec2(8.0, 36.0));
        let mut lens = Rect::from_center_size(pointer, size);
        let margin = 4.0;
        lens = lens.translate(egui::vec2(
            (screen.left() + margin - lens.left()).max(0.0)
                - (lens.right() - screen.right() + margin).max(0.0),
            (screen.top() + margin - lens.top()).max(0.0)
                - (lens.bottom() - screen.bottom() + 32.0).max(0.0),
        ));

        // Copy this frame's shapes in paint order (back to front).
        let order: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
        let shapes = ctx
            .graphics(|g| g.clone())
            .drain(&order, &ahash::HashMap::default());

        // Scale around the cursor, so the point under the mouse stays under the mouse.
        let transform = TSTransform::new(pointer.to_vec2() * (1.0 - self.zoom), self.zoom);

        let layer = egui::LayerId::new(Order::Debug, egui::Id::unique("gaza_man_lens"));
        let background = ctx.global_style().visuals.panel_fill;
        let shadow = egui::epaint::Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 2,
            color: Color32::from_black_alpha(110),
        };

        ctx.graphics_mut(|g| {
            let list = g.entry(layer);
            list.add(screen, shadow.as_shape(lens, 10.0).into());
            list.add(lens, Shape::rect_filled(lens, 0.0, background));
            for egui::epaint::ClippedShape {
                clip_rect,
                mut shape,
                transform_after_tessellation: after,
            } in shapes
            {
                let clip = (transform * clip_rect).intersect(lens);
                if !clip.is_positive() {
                    continue;
                }
                if after == TSTransform::IDENTITY {
                    // Scaling the shape itself keeps lines and text sharper.
                    shape.transform(transform);
                    list.add(clip, shape);
                } else {
                    // Something mid-animation (e.g. a popup): magnify after its own transform.
                    let idx = list.add(clip, shape);
                    list.mutate_shape(idx, |s| s.transform_after_tessellation = transform * after);
                }
            }
        });

        let painter = ctx.layer_painter(layer);
        painter.rect_stroke(
            lens,
            10.0,
            Stroke::new(3.0, accent),
            egui::StrokeKind::Outside,
        );

        // A small badge with the zoom level and the controls.
        let label = format!(
            "{:.1}×   ⌥ scroll: zoom · ⌥⇧ scroll: size · Esc: close",
            self.zoom
        );
        let galley =
            painter.layout_no_wrap(label, egui::FontId::proportional(12.0), Color32::WHITE);
        let badge = Rect::from_min_size(
            lens.left_bottom() + egui::vec2(0.0, 6.0),
            galley.size() + egui::vec2(16.0, 8.0),
        );
        painter.rect_filled(badge, 6.0, accent);
        painter.galley(badge.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
    }
}
