//! The numeric edit bar above the canvas: position, size (and %), rotation and
//! anchor point of the selection, plus font settings when text is selected.
//! Ported from BurnCraft's `editbar.rs`; edits made by dragging or typing in a
//! field merge into one undo step (`App::checkpoint_merged`).

use crate::app::{App, Tool};
use crate::panels::ImageMode;
use cutcraft_core::doc::ShapeKind;
use cutcraft_core::kurbo::{Affine, Point, Rect, Vec2};
use cutcraft_core::ops;
use egui::{DragValue, RichText, Ui};

/// The selection-relative point an anchor index (0..9, row-major from the
/// top-left) refers to.
pub fn anchor_point(b: Rect, anchor: usize) -> Point {
    let f = [0.0, 0.5, 1.0];
    Point::new(b.x0 + b.width() * f[anchor % 3], b.y0 + b.height() * f[(anchor / 3).min(2)])
}

fn about(p: Point, t: Affine) -> Affine {
    Affine::translate(p.to_vec2()) * t * Affine::translate(-p.to_vec2())
}

/// A length field shown in the user's units, stored in mm.
fn len(ui: &mut Ui, v: &mut f64, units: cutcraft_core::prefs::Units, width: f32) -> bool {
    let f = units.factor();
    let mut x = *v * f;
    let r = ui.add_sized([width, 20.0], DragValue::new(&mut x).speed(0.1 * f).max_decimals(3).suffix(format!(" {}", units.suffix())));
    if r.changed() {
        *v = x / f;
        true
    } else {
        false
    }
}

pub fn edit_bar(app: &mut App, ui: &mut Ui) {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    let bounds = ops::selection_bbox(&app.doc, &sel);
    let enabled = bounds.is_some();
    let b = bounds.unwrap_or(Rect::ZERO);
    let anchor = anchor_point(b, app.prefs.anchor);
    let now = ui.input(|i| i.time);
    const W: f32 = 82.0;

    // Wraps onto a second line on narrow windows instead of clipping.
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            // ---- position of the anchor point
            let (mut x, mut y) = (anchor.x, anchor.y);
            ui.label("X");
            let mx = len(ui, &mut x, units, W);
            ui.label("Y");
            let my = len(ui, &mut y, units, W);
            if mx || my {
                app.checkpoint_merged("Move", now);
                ops::move_by(&mut app.doc, &sel, Vec2::new(x - anchor.x, y - anchor.y));
            }
            ui.separator();

            // ---- size, in mm and %
            let (mut w, mut h) = (b.width(), b.height());
            let (mut pw, mut ph) = (100.0f64, 100.0f64);
            ui.label("W");
            let wc = len(ui, &mut w, units, W);
            let pwc = ui.add_sized([58.0, 20.0], DragValue::new(&mut pw).range(1.0..=10000.0).speed(0.5).max_decimals(1).suffix(" %")).changed();
            let lock = if app.ps.keep_aspect { crate::icons::Icon::Lock } else { crate::icons::Icon::Unlocked };
            let tip = if app.ps.keep_aspect { "Aspect ratio locked - click to unlock" } else { "Aspect ratio free - click to lock" };
            if crate::icons::button(ui, lock, 22.0, app.ps.keep_aspect, true, tip).clicked() {
                app.ps.keep_aspect = !app.ps.keep_aspect;
            }
            ui.label("H");
            let hc = len(ui, &mut h, units, W);
            let phc = ui.add_sized([58.0, 20.0], DragValue::new(&mut ph).range(1.0..=10000.0).speed(0.5).max_decimals(1).suffix(" %")).changed();
            let ka = app.ps.keep_aspect;
            let (mut sx, mut sy) = (1.0, 1.0);
            if wc && b.width() > 1e-9 {
                sx = w.max(0.01) / b.width();
                sy = if ka { sx } else { 1.0 };
            }
            if hc && b.height() > 1e-9 {
                sy = h.max(0.01) / b.height();
                sx = if ka { sy } else { 1.0 };
            }
            if pwc {
                sx = pw / 100.0;
                sy = if ka { sx } else { 1.0 };
            }
            if phc {
                sy = ph / 100.0;
                sx = if ka { sy } else { 1.0 };
            }
            if (sx - 1.0).abs() > 1e-12 || (sy - 1.0).abs() > 1e-12 {
                app.checkpoint_merged("Resize", now);
                ops::transform(&mut app.doc, &sel, about(anchor, Affine::scale_non_uniform(sx, sy)));
            }
            ui.separator();

            // ---- rotation: type an angle and press Enter (turns by that much)
            ui.label("↻");
            let r = ui.add_sized([62.0, 20.0], DragValue::new(&mut app.ps.bar_rot).speed(1.0).max_decimals(2).suffix("°"));
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (entered || r.drag_stopped()) && app.ps.bar_rot.abs() > 1e-9 {
                let a = app.ps.bar_rot;
                app.checkpoint_named("Rotate");
                ops::transform(&mut app.doc, &sel, about(anchor, Affine::rotate(a.to_radians())));
                app.ps.bar_rot = 0.0;
            }
            r.on_hover_text("Type an angle and press Enter to rotate the selection about the anchor point");
        });

        // ---- anchor point (3 × 3 dots)
        ui.separator();
        const CELL: f32 = 9.0;
        const GAP: f32 = 3.0;
        let side = 3.0 * CELL + 2.0 * GAP;
        let (block, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let ink = ui.visuals().strong_text_color();
        for i in 0..9 {
            let (row, col) = ((i / 3) as f32, (i % 3) as f32);
            let cell = egui::Rect::from_min_size(block.min + egui::vec2(col * (CELL + GAP), row * (CELL + GAP)), egui::vec2(CELL, CELL));
            let r = ui.interact(cell, ui.id().with(("anchor-dot", i)), egui::Sense::click());
            if i == app.prefs.anchor {
                ui.painter().circle_filled(cell.center(), 3.5, ui.visuals().selection.stroke.color);
            } else {
                let c = if r.hovered() { ink } else { ink.gamma_multiply(0.6) };
                ui.painter().circle_stroke(cell.center(), 3.0, egui::Stroke::new(1.0, c));
            }
            if r.on_hover_text("X/Y, resizing and rotation use this point of the selection").clicked() {
                app.prefs.anchor = i;
            }
        }

        // ---- the image tool's modes, while it's the tool in use
        if app.tool == Tool::ImageEraser {
            ui.separator();
            image_tool_bar(app, ui);
        }

        // ---- text settings when exactly one text shape is selected
        let text_id = match sel.as_slice() {
            [id] => app.doc.page().shape(*id).filter(|s| matches!(s.kind, ShapeKind::Text { .. })).map(|_| *id),
            _ => None,
        };
        if let Some(id) = text_id {
            ui.separator();
            text_settings(app, ui, id, now);
        }
    });
}

fn image_tool_bar(app: &mut App, ui: &mut Ui) {
    let m = &mut app.ps.image_mode;
    ui.selectable_value(m, ImageMode::Erase, "Erase").on_hover_text("Brush parts of the picture away");
    ui.selectable_value(m, ImageMode::Restore, "Restore").on_hover_text("Brush erased parts back");
    ui.selectable_value(m, ImageMode::SelectErase, "Select & erase").on_hover_text("Click an area of one colour to clear it (e.g. a white background)");
    ui.selectable_value(m, ImageMode::Crop, "Crop").on_hover_text("Drag a box over the picture to keep only that part");
    match app.ps.image_mode {
        ImageMode::Erase | ImageMode::Restore => {
            ui.label("Size");
            len(ui, &mut app.ps.eraser_radius, app.prefs.units, 72.0);
        }
        ImageMode::SelectErase => {
            ui.label("Tolerance");
            ui.add(egui::Slider::new(&mut app.ps.select_tolerance, 0.0..=100.0).show_value(true)).on_hover_text("How different a colour may be and still be cleared");
            ui.checkbox(&mut app.ps.select_everywhere, "Everywhere").on_hover_text("Clear that colour across the whole picture, not just the area you click");
        }
        ImageMode::Crop => {
            ui.weak("drag a box over the picture");
        }
    }
}

fn text_settings(app: &mut App, ui: &mut Ui, id: cutcraft_core::doc::ShapeId, now: f64) {
    let Some(ShapeKind::Text { data, .. }) = app.doc.page().shape(id).map(|s| &s.kind) else { return };
    let mut d = data.clone();
    let before = d.clone();
    egui::ComboBox::from_id_salt("bar-font").selected_text(RichText::new(&d.family).small()).width(150.0).show_ui(ui, |ui| {
        for f in app.fonts.families() {
            ui.selectable_value(&mut d.family, f.clone(), f);
        }
    });
    len(ui, &mut d.size, app.prefs.units, 72.0);
    ui.toggle_value(&mut d.bold, RichText::new("B").strong());
    ui.toggle_value(&mut d.italic, RichText::new("I").italics());
    if d.family != cutcraft_core::singleline::FAMILY {
        ui.toggle_value(&mut d.weld, "Weld").on_hover_text("Fuse overlapping letters into one outline");
    }
    if d.on_path.is_none() && !d.vertical {
        ui.label("Curve");
        ui.add_sized([62.0, 20.0], DragValue::new(&mut d.curve).range(-360.0..=360.0).speed(1.0).max_decimals(0).suffix("°"))
            .on_hover_text("Bend the text round a circle (positive arches up, negative curves down)");
    }
    if d != before {
        app.checkpoint_merged("Text", now);
        if let Some(s) = app.doc.page_mut().shape_mut(id) {
            if let ShapeKind::Text { data, .. } = &mut s.kind {
                *data = d;
            }
        }
        app.relayout_text(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_points_cover_the_box() {
        let b = Rect::new(10.0, 20.0, 50.0, 80.0);
        assert_eq!(anchor_point(b, 0), Point::new(10.0, 20.0));
        assert_eq!(anchor_point(b, 4), Point::new(30.0, 50.0));
        assert_eq!(anchor_point(b, 8), Point::new(50.0, 80.0));
    }

    #[test]
    fn scaling_about_the_anchor_keeps_it_fixed() {
        let p = Point::new(30.0, 50.0);
        assert!((about(p, Affine::scale(2.0)) * p - p).hypot() < 1e-12);
    }
}
