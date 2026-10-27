//! The Print & Cut calibration wizard: make the test sheet, print and cut
//! it, then measure it from a photo (or by eye off its rulers) and save the
//! correction for this machine. See `cutcraft_core::calibration`.

use crate::app::{Action, App};
use cutcraft_core::calibration::{self as cal, Calibration};
use cutcraft_core::doc::Page;
use cutcraft_core::kurbo::{Point, Vec2};
use cutcraft_core::machines;
use egui::RichText;

pub struct Wizard {
    /// How far each square landed from its dot (mm, right and down +), in
    /// `cal::TARGET_NAMES` order.
    offsets: [[f64; 2]; 5],
    /// Which squares count (one that couldn't be read can be left out).
    used: [bool; 5],
    /// What the last photo showed.
    note: String,
}

pub fn open(app: &mut App) {
    app.calib = Some(Wizard { offsets: [[0.0; 2]; 5], used: [true; 5], note: String::new() });
}

/// The calibration sheet's page (its targets), or the current page.
fn sheet(app: &App) -> &Page {
    app.doc.pages.iter().find(|p| cal::is_sheet(p)).unwrap_or(app.doc.page())
}

/// The correction the current numbers give.
fn result(app: &App, wz: &Wizard) -> Option<Calibration> {
    let want = cal::targets(sheet(app));
    let (mut e, mut a) = (vec![], vec![]);
    for i in 0..5 {
        if wz.used[i] {
            e.push(want[i]);
            a.push(want[i] + Vec2::new(wz.offsets[i][0], wz.offsets[i][1]));
        }
    }
    cal::fit(&e, &a)
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(mut wz) = app.calib.take() else { return };
    let mut open = true;
    let mut act = None;
    let machine = machines::by_id(&app.doc.machine);
    let shown = app
        .place
        .place(&app.prefs, ctx, "calibrate", egui::Window::new("Print & Cut calibration").open(&mut open).collapsible(false).resizable(false).default_width(440.0))
        .show(ctx, |ui| {
            ui.label(format!("Lines up {}'s cuts with your printer's print. Do it once per printer and machine, and again if cuts drift.", machine.name));
            ui.add_space(6.0);

            ui.label(RichText::new("1. Make the test sheet").strong());
            if cal::is_sheet(app.doc.page()) {
                ui.label("✓ The test sheet is the current page.");
            } else if ui.button("Add the test sheet").on_hover_text("Adds a page with registration marks, five dots and a square to cut around each").clicked() {
                app.checkpoint_named("Add calibration sheet");
                cal::add_sheet(&mut app.doc);
                app.rev += 1;
                app.dirty = true;
            }
            ui.add_space(6.0);

            ui.label(RichText::new("2. Print it and cut it").strong());
            ui.horizontal(|ui| {
                if ui.button("Export print PDF…").clicked() {
                    act = Some(Action::ExportPdf);
                }
                ui.label("then print it at 100% scale.");
            });
            ui.label("Load it and send the page with registration marks (Make it, or Send). To measure from a photo, set the cut lines' action to a pen (Sketch) so the squares show in ink.");
            ui.add_space(6.0);

            ui.label(RichText::new("3. Measure").strong());
            if ui.button("Load a photo or scan…").on_hover_text("A photo straight above the sheet, the whole sheet in view, or a flatbed scan").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png", "bmp", "tif", "tiff", "webp"]).pick_file() {
                    wz.note = match image::open(&path) {
                        Err(e) => format!("Couldn't open the picture: {e}"),
                        Ok(img) => match cal::measure_photo(&img.to_rgba8(), sheet(app)) {
                            Err(e) => e,
                            Ok(found) => {
                                let want = cal::targets(sheet(app));
                                for i in 0..5 {
                                    wz.used[i] = found[i].is_some();
                                    if let Some(p) = found[i] {
                                        wz.offsets[i] = [p.x - want[i].x, p.y - want[i].y];
                                    }
                                }
                                let n = found.iter().flatten().count();
                                format!("Measured {n} of 5 squares - check the numbers below, then save.")
                            }
                        },
                    };
                }
            }
            if !wz.note.is_empty() {
                ui.label(&wz.note);
            }
            ui.small("Or by eye: where each square's left edge crosses its ruler gives Right (+ to the right), where its top edge crosses the other gives Down (+ lower). Ticks are 0.5 mm apart.");
            egui::Grid::new("cal-off").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label("");
                ui.label("Right");
                ui.label("Down");
                ui.end_row();
                for i in 0..5 {
                    ui.checkbox(&mut wz.used[i], cal::TARGET_NAMES[i]);
                    for v in &mut wz.offsets[i] {
                        ui.add_enabled(wz.used[i], egui::DragValue::new(v).speed(0.05).range(-10.0..=10.0).fixed_decimals(2).suffix(" mm"));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);

            let res = result(app, &wz);
            match &res {
                Some(c) => {
                    ui.label(format!("Correction: {}.", c.describe(sheet(app))));
                    if wz.offsets.iter().zip(wz.used).any(|(o, u)| u && Point::new(o[0], o[1]).to_vec2().hypot() > 5.0) {
                        ui.colored_label(ui.visuals().warn_fg_color, "That's a lot - check the sheet was printed at 100% scale.");
                    }
                }
                None => {
                    ui.label("Tick at least one square.");
                }
            }
            let saved = app.prefs.print_cut_calibration.get(&app.doc.machine).copied();
            ui.horizontal(|ui| {
                if ui.add_enabled(res.is_some(), egui::Button::new(format!("Save for {}", machine.name))).clicked() {
                    if let Some(c) = res {
                        app.prefs.print_cut_calibration.insert(app.doc.machine.clone(), c);
                        let _ = app.prefs.save();
                        app.status = format!("Print & Cut calibration saved for {} - it's used for every Print & Cut job on it", machine.name);
                    }
                }
                if saved.is_some() && ui.button("Remove saved calibration").clicked() {
                    app.prefs.print_cut_calibration.remove(&app.doc.machine);
                    let _ = app.prefs.save();
                    app.status = format!("Print & Cut calibration removed for {}", machine.name);
                }
            });
            if let Some(c) = saved {
                ui.small(format!("Saved now: {}.", c.describe(sheet(app))));
            }
            ui.small("The test sheet is cut without any saved correction, so you can redo this any time.");
        });
    app.place.moved(&mut app.prefs, "calibrate", &shown);
    if let Some(a) = act {
        app.do_action(a, ctx);
    }
    if open {
        app.calib = Some(wz);
    }
}
