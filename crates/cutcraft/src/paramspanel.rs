//! The Parameters panel (named values and locked sizes/distances), keeping the
//! design to its constraints, and drawing them on the canvas. Ported from
//! BurnCraft's `params.rs`.

use crate::app::{Action, App};
use cutcraft_core::doc::ShapeId;
use cutcraft_core::kurbo::Point;
use cutcraft_core::params::{self, Anchor, Axis, Constraint, ConstraintKind, Param};
use egui::{Color32, Pos2, Stroke, Ui};

enum Change {
    /// A text field was clicked into: record undo before it's typed in.
    Checkpoint,
    AddParam,
    RemoveParam(usize),
    Add(ConstraintKind, f64),
    RemoveConstraint(usize),
}

fn anchor_combo(ui: &mut Ui, id: egui::Id, anchor: &mut Anchor, axis: Axis) {
    egui::ComboBox::from_id_salt(id).width(64.0).selected_text(anchor.label(axis)).show_ui(ui, |ui| {
        for a in Anchor::ALL {
            ui.selectable_value(anchor, a, a.label(axis));
        }
    });
}

/// Moves and resizes shapes so the design meets its constraints (every frame;
/// it only changes anything when something is off).
pub fn enforce(app: &mut App) {
    if app.doc.params.constraints.is_empty() {
        app.constraint_errors.clear();
        return;
    }
    let (changed, errors) = params::solve(&mut app.doc);
    if changed {
        app.rev += 1;
        app.dirty = true;
    }
    app.constraint_errors = errors;
}

/// The selected top-level shapes, largest first (the first is the reference
/// for a distance, the second moves).
fn selected_by_size(app: &App) -> Vec<ShapeId> {
    let page = app.doc.page();
    let mut v: Vec<(ShapeId, f64)> = app.sel.iter().filter_map(|id| page.shape(*id).and_then(|s| s.bbox()).map(|b| (*id, b.area()))).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.into_iter().map(|(id, _)| id).collect()
}

pub fn params_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let mut changes: Vec<Change> = Vec::new();
    let units = selected_by_size(app);
    let measure = |doc: &cutcraft_core::doc::Document, kind: &ConstraintKind| Constraint { kind: kind.clone(), value: String::new(), enabled: true }.measure(doc).unwrap_or(0.0);
    let (values, _) = app.doc.params.values();

    ui.heading("Parameters");
    ui.label(egui::RichText::new("Named values to use in the constraints below, e.g. thickness = 3. A value can be a formula using the parameters above it: thickness * 2 + 0.2").small().weak());
    egui::Grid::new("params").num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
        for (i, p) in app.doc.params.params.iter_mut().enumerate() {
            if ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(80.0)).gained_focus() {
                changes.push(Change::Checkpoint);
            }
            if ui.add(egui::TextEdit::singleline(&mut p.value).desired_width(110.0)).gained_focus() {
                changes.push(Change::Checkpoint);
            }
            match values.get(p.name.trim()) {
                Some(v) => ui.label(format!("= {v:.3}")),
                None => ui.colored_label(ui.visuals().error_fg_color, "?"),
            };
            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                changes.push(Change::RemoveParam(i));
            }
            ui.end_row();
        }
    });
    if ui.button("Add parameter").clicked() {
        changes.push(Change::AddParam);
    }

    ui.separator();
    ui.heading("Constraints");
    ui.label(
        egui::RichText::new("Locked sizes and distances, in mm (or write 2in, 5cm). The design keeps to them as you edit; type a number or a formula. A distance moves its second shape.")
            .small()
            .weak(),
    );
    let snapshot = app.doc.clone();
    for (i, c) in app.doc.params.constraints.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            // wrapped rows, so a long description never widens the side panel
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut c.enabled, "").on_hover_text("On / off");
                ui.label(c.describe(&snapshot));
                if ui.small_button("✕").on_hover_text("Remove this constraint").clicked() {
                    changes.push(Change::RemoveConstraint(i));
                }
            });
            ui.horizontal_wrapped(|ui| {
                match &mut c.kind {
                    ConstraintKind::Size { axis, anchor, .. } => {
                        ui.weak("keep");
                        anchor_combo(ui, egui::Id::new(("size-anchor", i)), anchor, *axis);
                        ui.weak("still");
                    }
                    ConstraintKind::Distance { axis, from_anchor, to_anchor, .. } => {
                        ui.weak("from");
                        anchor_combo(ui, egui::Id::new(("from-anchor", i)), from_anchor, *axis);
                        ui.weak("to");
                        anchor_combo(ui, egui::Id::new(("to-anchor", i)), to_anchor, *axis);
                    }
                }
                if ui.add(egui::TextEdit::singleline(&mut c.value).desired_width(80.0)).gained_focus() {
                    changes.push(Change::Checkpoint);
                }
                match params::eval(&c.value, &values) {
                    Ok(v) => ui.weak(format!("= {v:.2} mm")),
                    Err(e) => ui.colored_label(ui.visuals().error_fg_color, e),
                };
            });
        });
    }
    if app.doc.params.constraints.is_empty() {
        ui.weak("None yet - select one shape to lock its size, or two to lock the distance between them.");
    }
    ui.horizontal_wrapped(|ui| {
        let one = units.len() == 1;
        let two = units.len() == 2;
        for (label, axis) in [("Lock width", Axis::X), ("Lock height", Axis::Y)] {
            if ui.add_enabled(one, egui::Button::new(label)).on_disabled_hover_text("Select one shape or group").clicked() {
                let kind = ConstraintKind::Size { shape: units[0], axis, anchor: Anchor::Start };
                changes.push(Change::Add(kind.clone(), measure(&app.doc, &kind)));
            }
        }
        for (label, axis) in [("Lock distance across", Axis::X), ("Lock distance down", Axis::Y)] {
            let tip = "Select two shapes or groups: the larger is the reference, the smaller moves";
            if ui.add_enabled(two, egui::Button::new(label)).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                let kind = ConstraintKind::Distance { axis, from: units[0], from_anchor: Anchor::Start, to: units[1], to_anchor: Anchor::Start };
                changes.push(Change::Add(kind.clone(), measure(&app.doc, &kind)));
            }
        }
    });
    for e in &app.constraint_errors {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    ui.add_space(4.0);
    ui.checkbox(&mut app.prefs.show_constraints, "Show constraints on the canvas");

    if !changes.is_empty() {
        app.checkpoint_named("Edit parameters");
    }
    for change in changes {
        match change {
            Change::Checkpoint => {}
            Change::AddParam => {
                let name = app.doc.params.new_name();
                app.doc.params.params.push(Param { name, value: "0".into() });
            }
            Change::RemoveParam(i) => {
                app.doc.params.params.remove(i);
            }
            Change::Add(kind, now) => app.doc.params.constraints.push(Constraint { kind, value: format!("{now:.2}"), enabled: true }),
            Change::RemoveConstraint(i) => {
                app.doc.params.constraints.remove(i);
            }
        }
    }
    None
}

/// Locked sizes and distances on the active page, as blue dimension lines
/// with their values.
pub fn draw_constraints(app: &App, painter: &egui::Painter) {
    if !app.prefs.show_constraints {
        return;
    }
    let page = app.doc.page();
    let ink = Color32::from_rgb(40, 120, 230);
    let stroke = Stroke::new(1.2, ink);
    let to = |p: Point| app.view.to_screen(p);
    let gap = 14.0 / app.view.zoom as f64;
    let bounds = |id: ShapeId| page.shape(id).and_then(|s| s.bbox());
    let label = |at: Pos2, text: String| {
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), ink);
        let r = egui::Rect::from_center_size(at, galley.size() + egui::vec2(6.0, 2.0));
        painter.rect_filled(r, 3.0, Color32::from_white_alpha(225));
        painter.galley(r.min + egui::vec2(3.0, 1.0), galley, ink);
    };
    // a dimension from a to b with short end ticks
    let dim = |a: Point, b: Point, text: String, enabled: bool| {
        let s = if enabled { stroke } else { Stroke::new(1.0, ink.gamma_multiply(0.4)) };
        let (sa, sb) = (to(a), to(b));
        painter.line_segment([sa, sb], s);
        let d = (sb - sa).normalized();
        let n = egui::vec2(-d.y, d.x) * 4.0;
        painter.line_segment([sa - n, sa + n], s);
        painter.line_segment([sb - n, sb + n], s);
        label(sa + (sb - sa) / 2.0, text);
    };
    for c in &app.doc.params.constraints {
        let value = c.measure(&app.doc).map_or_else(|| "?".into(), |v| format!("{v:.2}"));
        let text = if c.value.trim().parse::<f64>().is_ok() { value } else { format!("{} = {value}", c.value.trim()) };
        match c.kind {
            ConstraintKind::Size { shape, axis, .. } => {
                let Some(r) = bounds(shape) else { continue };
                match axis {
                    Axis::X => dim(Point::new(r.x0, r.y0 - gap), Point::new(r.x1, r.y0 - gap), text, c.enabled),
                    Axis::Y => dim(Point::new(r.x0 - gap, r.y0), Point::new(r.x0 - gap, r.y1), text, c.enabled),
                }
            }
            ConstraintKind::Distance { axis, from, from_anchor, to: target, to_anchor } => {
                let (Some(a), Some(b)) = (bounds(from), bounds(target)) else { continue };
                match axis {
                    Axis::X => {
                        let y = (a.center().y + b.center().y) / 2.0;
                        dim(Point::new(from_anchor.of(a, axis), y), Point::new(to_anchor.of(b, axis), y), text, c.enabled);
                    }
                    Axis::Y => {
                        let x = (a.center().x + b.center().x) / 2.0;
                        dim(Point::new(x, from_anchor.of(a, axis)), Point::new(x, to_anchor.of(b, axis)), text, c.enabled);
                    }
                }
            }
        }
    }
}
