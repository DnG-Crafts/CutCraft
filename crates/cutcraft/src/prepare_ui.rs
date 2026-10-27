//! The Prepare screen ("Make it"), as in Cricut Design Space: the design split
//! onto mats by material, each previewed large, shapes movable / hideable /
//! movable to another mat, copies laid out onto as many mats as they need, and
//! each mat cut (or, for a Brother machine, saved) on its own.
//!
//! PixMat: a photo of the loaded mat, straightened, shown as the mat so
//! shapes can be dragged onto the material actually on it (scraps, a print).

use crate::app::App;
use crate::chrome::ACCENT;
use cutcraft_core::cutjob;
use cutcraft_core::doc::{Page, Rgba};
use cutcraft_core::kurbo::{Point, Rect, Vec2};
use cutcraft_core::materials::{self, Action as CutAction, Tool as CutTool};
use cutcraft_core::prepare::{self, Prepared};
use cutcraft_core::geom;
use egui::{Color32, Pos2, RichText, Stroke, Ui};

/// The open Prepare screen.
pub struct PrepareView {
    pub prep: Prepared,
    /// The mat shown large.
    pub mat: usize,
    /// The selected item on it.
    pub sel: Vec<usize>,
    /// Where a selection box started (screen), and where it is now.
    band: Option<(Pos2, Pos2)>,
    /// Mats already cut (or saved).
    pub done: Vec<bool>,
    /// Laid out tightly rather than as drawn.
    pub packed: bool,
    /// PixMat photos of the mats, straightened to the mat's grid, by mat.
    pub photos: std::collections::HashMap<usize, egui::TextureHandle>,
    /// Show the photos (they can be hidden to see the plain mat).
    pub show_photos: bool,
    /// A photo being lined up with its mat.
    pick: Option<Pick>,
}

/// Lining up a PixMat photo: where the mat grid's corners are in it.
struct Pick {
    mat: usize,
    photo: image::RgbaImage,
    tex: egui::TextureHandle,
    /// Photo px, top left, top right, bottom right, bottom left.
    corners: [Point; 4],
    drag: Option<usize>,
}

impl PrepareView {
    pub fn new(app: &mut App, pack: bool) -> Self {
        prepare::sync_plan_all(&mut app.plan, &app.doc);
        let prep = prepare::prepare(&app.doc, &app.plan, pack);
        let n = prep.mats.len();
        PrepareView { prep, mat: 0, sel: vec![], band: None, done: vec![false; n], packed: pack, photos: Default::default(), show_photos: true, pick: None }
    }
}

/// Maps the media (plus its mat) into a screen rectangle.
struct Fit {
    origin: Pos2,
    k: f32,
    /// Flip across the media (mirrored mats are shown as they'll be cut).
    mirror: Option<f64>,
}

impl Fit {
    fn new(page: &Page, into: egui::Rect, mirror: bool) -> Self {
        let r = mat_rect(page);
        let k = ((into.width() - 16.0) / r.width() as f32).min((into.height() - 16.0) / r.height() as f32).max(0.01);
        let size = egui::vec2(r.width() as f32 * k, r.height() as f32 * k);
        let min = into.center() - size / 2.0;
        Fit { origin: min - egui::vec2(r.x0 as f32 * k, r.y0 as f32 * k), k, mirror: mirror.then_some(page.width) }
    }
    fn to(&self, p: Point) -> Pos2 {
        let x = match self.mirror {
            Some(w) => w - p.x,
            None => p.x,
        };
        self.origin + egui::vec2(x as f32 * self.k, p.y as f32 * self.k)
    }
    fn rect(&self, r: Rect) -> egui::Rect {
        egui::Rect::from_two_pos(self.to(Point::new(r.x0, r.y0)), self.to(Point::new(r.x1, r.y1)))
    }
}

/// The mat (if any) around the media, in mm.
fn mat_rect(page: &Page) -> Rect {
    match page.mat.size_mm() {
        Some((w, h)) => Rect::new(-12.0, -12.0, w + 12.0, h + 12.0).union(page.rect()),
        None => page.rect().inflate(6.0, 6.0),
    }
}

/// Draws one mat: the cutting mat, the media, and every item's outline.
/// The page whose media and cutting mat mat `mi` uses.
fn media<'a>(app: &'a App, view: &PrepareView, mi: usize) -> &'a Page {
    view.prep.mats.get(mi).and_then(|m| app.doc.pages.get(m.page)).unwrap_or(app.doc.page())
}

fn draw_mat(app: &App, view: &PrepareView, mi: usize, painter: &egui::Painter, fit: &Fit, sel: &[usize]) {
    let page = media(app, view, mi);
    if let Some((mw, mh)) = page.mat.size_mm() {
        painter.rect_filled(fit.rect(Rect::new(-12.0, -12.0, mw + 12.0, mh + 12.0)), 6.0, Color32::from_rgb(170, 180, 190));
        painter.rect_filled(fit.rect(Rect::new(0.0, 0.0, mw, mh)), 0.0, Color32::from_rgb(200, 208, 214));
    }
    let mc = page.media_color;
    let photo = view.photos.get(&mi).filter(|_| view.show_photos);
    match photo {
        // the real mat, as it is: never mirrored
        Some(tex) => {
            let r = grid_area(page);
            let px = egui::Rect::from_min_max(fit.origin + egui::vec2(r.x0 as f32 * fit.k, r.y0 as f32 * fit.k), fit.origin + egui::vec2(r.x1 as f32 * fit.k, r.y1 as f32 * fit.k));
            painter.image(tex.id(), px, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(fit.rect(page.rect()), 0.0, Color32::from_rgb(mc.0, mc.1, mc.2));
        }
    }
    draw_grid(page, painter, fit);
    let mat = &view.prep.mats[mi];
    let g = app.plan.groups.get(mat.group);
    for (ii, it) in mat.items.iter().enumerate() {
        let Some(s) = prepare::find_shape(&app.doc, it.shape) else { continue };
        let a = cutcraft_core::kurbo::Affine::translate(it.offset);
        for leaf in s.leaves() {
            if leaf.hidden || leaf.is_image() || g.is_none_or(|g| !cutjob::leaf_in_group(&app.plan, g, s.layer, &leaf)) {
                continue;
            }
            let col = leaf.style.stroke.map(crate::render::c32).unwrap_or(Color32::DARK_GRAY);
            let st = if it.hidden { Stroke::new(1.0, col.gamma_multiply(0.25)) } else { Stroke::new(1.3, col) };
            let paths: Vec<_> = leaf.cut_paths().into_iter().map(|p| a * p).collect();
            if let Some(f) = crate::render::fill_colour(&leaf) {
                let (v, idx) = crate::render::tessellate(&paths, 0.1);
                if !idx.is_empty() {
                    // see-through over a photo, so the material shows
                    let k = if it.hidden { 0.25 } else if photo.is_some() { 0.45 } else { 1.0 };
                    let fill = crate::render::c32(f).gamma_multiply(k);
                    let mut mesh = egui::Mesh::default();
                    for p in v {
                        mesh.colored_vertex(fit.to(Point::new(p[0] as f64, p[1] as f64)), fill);
                    }
                    mesh.indices = idx;
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
            for pl in geom::flatten_all(&paths, 0.3) {
                let pts: Vec<Pos2> = pl.pts.iter().map(|p| fit.to(*p)).collect();
                painter.add(if pl.closed { crate::render::closed_outline(pts, st) } else { egui::Shape::line(pts, st) });
            }
        }
        if sel.contains(&ii) {
            if let Some(b) = prepare::item_bounds(&app.doc, &app.plan, mat, it) {
                let r = fit.rect(b).expand(3.0);
                let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.5, ACCENT), 5.0, 3.0));
            }
        }
    }
}

/// The mat's grid area (mm), or the media when there's no mat.
fn grid_area(page: &Page) -> Rect {
    match page.mat.size_mm() {
        Some((w, h)) => Rect::new(0.0, 0.0, w, h),
        None => page.rect(),
    }
}

/// The cutting mat's grid (the page grid's spacing, an inch by default),
/// across the mat and the media on it.
fn draw_grid(page: &Page, painter: &egui::Painter, fit: &Fit) {
    let area = grid_area(page);
    let major = if page.grid.spacing > 0.0 { page.grid.spacing } else { 25.4 };
    let minor = major / page.grid.subdivisions.max(1) as f64;
    // minor lines only where they're far enough apart to read
    let step = if minor * fit.k as f64 > 6.0 { minor } else { major };
    if step * (fit.k as f64) < 3.0 {
        return;
    }
    let line = |a: Point, b: Point, major: bool| {
        let c = if major { Color32::from_rgba_unmultiplied(80, 120, 200, 70) } else { Color32::from_rgba_unmultiplied(80, 120, 200, 28) };
        painter.line_segment([fit.to(a), fit.to(b)], Stroke::new(1.0, c));
    };
    let is_major = |v: f64| (v / major - (v / major).round()).abs() < 1e-6;
    let mut i = 0;
    while i as f64 * step <= area.width() + 1e-6 {
        let x = area.x0 + i as f64 * step;
        line(Point::new(x, area.y0), Point::new(x, area.y1), is_major(x - area.x0));
        i += 1;
    }
    let mut i = 0;
    while i as f64 * step <= area.height() + 1e-6 {
        let y = area.y0 + i as f64 * step;
        line(Point::new(area.x0, y), Point::new(area.x1, y), is_major(y - area.y0));
        i += 1;
    }
}

fn group_label(app: &App, gi: usize) -> String {
    app.plan.groups.get(gi).map(|g| g.label.clone()).unwrap_or_default()
}

fn swatch(app: &App, gi: usize) -> Option<Color32> {
    app.plan.groups.get(gi).and_then(|g| Rgba::from_hex(&g.key)).map(crate::render::c32)
}

/// The whole Prepare screen, in place of the canvas. Returns false when it
/// should close.
pub fn show(app: &mut App, ctx: &egui::Context, ui: &mut Ui) -> bool {
    let Some(mut view) = app.prepare.take() else { return false };
    let mut keep = true;
    let mut rebuild: Option<bool> = None;

    // ---- top bar
    egui::Panel::top("prep-top").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("← Back to design").clicked() {
                keep = false;
            }
            ui.separator();
            ui.heading("Prepare to cut");
            ui.separator();
            ui.label("Mats by");
            let mut mode = app.plan.mode;
            ui.selectable_value(&mut mode, cutjob::GroupMode::Simple, "One material");
            ui.selectable_value(&mut mode, cutjob::GroupMode::ByColor, "Line colour");
            ui.selectable_value(&mut mode, cutjob::GroupMode::ByFill, "Fill colour");
            ui.selectable_value(&mut mode, cutjob::GroupMode::ByLayer, "Layer");
            if mode != app.plan.mode {
                app.plan.mode = mode;
                rebuild = Some(view.packed);
            }
            ui.separator();
            ui.label("Copies");
            let mut copies = app.plan.copies;
            if ui.add(egui::DragValue::new(&mut copies).range(1..=200)).on_hover_text("Lay out this many copies of the design onto the mats (resets moves you've made)").changed() {
                app.plan.copies = copies;
                rebuild = Some(true);
            }
            if ui.button("Pack mats").on_hover_text("Arrange every shape tightly, using as few mats as possible (resets moves you've made)").clicked() {
                rebuild = Some(true);
            }
            if view.packed && app.plan.copies <= 1 && ui.button("Back to design layout").on_hover_text("Put shapes where they are in the design again").clicked() {
                rebuild = Some(false);
            }
        });
        ui.add_space(4.0);
    });

    if view.prep.mats.is_empty() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| ui.label("Nothing to cut - give shapes a cut style (Style ▸ Fill & Line) and enable a group in Make ▸ Send."));
        });
        // honour "Back to design" (and a changed grouping) here too
        if let Some(pack) = rebuild {
            view = PrepareView::new(app, pack);
        }
        if keep {
            app.prepare = Some(view);
        }
        return keep;
    }
    view.mat = view.mat.min(view.prep.mats.len() - 1);

    // ---- left: the mats
    // a mat right-clicked: (which, pack it?)
    let mut one_mat: Option<(usize, bool)> = None;
    egui::Panel::left("prep-mats").resizable(false).exact_size(210.0).show(ui, |ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{} mat{}", view.prep.mats.len(), if view.prep.mats.len() == 1 { "" } else { "s" })).strong());
        egui::ScrollArea::vertical().show(ui, |ui| {
            for mi in 0..view.prep.mats.len() {
                let gi = view.prep.mats[mi].group;
                let selected = mi == view.mat;
                let frame = egui::Frame::group(ui.style()).fill(if selected { ui.visuals().widgets.active.weak_bg_fill } else { ui.visuals().panel_fill }).stroke(Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { ACCENT } else { ui.visuals().widgets.noninteractive.bg_stroke.color }));
                let resp = frame
                    .show(ui, |ui| {
                        ui.set_width(180.0);
                        ui.horizontal(|ui| {
                            if let Some(c) = swatch(app, gi) {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 3.0, c);
                            }
                            ui.label(RichText::new(format!("Mat {}", mi + 1)).strong());
                            if app.doc.pages.len() > 1 {
                                let pi = view.prep.mats[mi].page;
                                ui.label(RichText::new(app.doc.pages.get(pi).map(|p| p.name.as_str()).unwrap_or("")).weak());
                            }
                            if view.done.get(mi).copied().unwrap_or(false) {
                                ui.label(RichText::new("✔ done").color(Color32::from_rgb(60, 180, 90)));
                            }
                        });
                        let (r, _) = ui.allocate_exact_size(egui::vec2(180.0, 150.0), egui::Sense::hover());
                        let mirror = app.plan.groups.get(gi).is_some_and(|g| g.mirror);
                        let fit = Fit::new(media(app, &view, mi), r, mirror);
                        draw_mat(app, &view, mi, ui.painter(), &fit, &[]);
                        ui.small(format!("{} · {}", group_label(app, gi), app.plan.groups.get(gi).map(|g| g.material.as_str()).unwrap_or("")));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.clicked() && !selected {
                    view.mat = mi;
                    view.sel.clear();
                }
                resp.on_hover_text("Right-click to pack just this mat or put it back to the design layout").context_menu(|ui| {
                    if ui.button("Pack this mat").on_hover_text("Arrange this mat's shapes tightly; anything that doesn't fit goes on a new mat after it").clicked() {
                        one_mat = Some((mi, true));
                        ui.close();
                    }
                    if ui.button("Back to design layout").on_hover_text("Put this mat's shapes where they are in the design").clicked() {
                        one_mat = Some((mi, false));
                        ui.close();
                    }
                });
                ui.add_space(4.0);
            }
        });
    });

    if let Some((mi, pack)) = one_mat {
        if pack {
            let added = prepare::pack_mat(&app.doc, &app.plan, &mut view.prep, mi);
            // the new mats come right after it: move later mats' marks along
            for _ in 0..added {
                view.done.insert((mi + 1).min(view.done.len()), false);
            }
            if added > 0 {
                view.photos = std::mem::take(&mut view.photos).into_iter().map(|(k, v)| (if k > mi { k + added } else { k }, v)).collect();
            }
            app.status = if added > 0 { format!("Mat {} packed - {added} more mat{} for what didn't fit", mi + 1, if added == 1 { "" } else { "s" }) } else { format!("Mat {} packed", mi + 1) };
        } else {
            prepare::design_layout_mat(&mut view.prep, mi);
            app.status = format!("Mat {} is back to the design layout", mi + 1);
        }
        view.mat = mi;
        view.sel.clear();
    }

    // ---- right: this mat's settings and the cut button
    let mi = view.mat;
    let gi = view.prep.mats[mi].group;
    egui::Panel::right("prep-settings").resizable(false).exact_size(320.0).show(ui, |ui| {
        ui.add_space(6.0);
        ui.heading(format!("Mat {} of {}", mi + 1, view.prep.mats.len()));
        if app.doc.pages.len() > 1 {
            let pi = view.prep.mats[mi].page;
            ui.label(RichText::new(format!("From {}", app.doc.pages.get(pi).map(|p| p.name.as_str()).unwrap_or(""))).weak());
        }
        ui.label(RichText::new(group_label(app, gi)).weak());
        ui.add_space(4.0);
        let mut mats = materials::builtin();
        mats.extend(app.prefs.user_materials.clone());
        if let Some(g) = app.plan.groups.get_mut(gi) {
            egui::Grid::new("prep-mat").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Material");
                egui::ComboBox::from_id_salt("prep-material").selected_text(&g.material).width(190.0).show_ui(ui, |ui| {
                    for mat in &mats {
                        if ui.selectable_label(g.material == mat.name, format!("{}  ({})", mat.name, mat.category)).clicked() {
                            g.material = mat.name.clone();
                            g.settings = mat.settings.clone();
                            g.mirror = crate::panels::is_iron_on(mat);
                        }
                    }
                });
                ui.end_row();
                ui.label("Action");
                egui::ComboBox::from_id_salt("prep-action").selected_text(g.action.label()).show_ui(ui, |ui| {
                    for a in CutAction::ALL {
                        ui.selectable_value(&mut g.action, a, a.label());
                    }
                });
                ui.end_row();
                ui.label("Tool");
                egui::ComboBox::from_id_salt("prep-tool").selected_text(g.settings.tool.label()).show_ui(ui, |ui| {
                    for t in CutTool::ALL {
                        ui.selectable_value(&mut g.settings.tool, t, t.label());
                    }
                });
                ui.end_row();
                ui.label("Mirror");
                ui.checkbox(&mut g.mirror, "Cut mirror-image").on_hover_text("For iron-on / heat-transfer vinyl, cut from the back");
                ui.end_row();
            });
        }
        ui.small("Force, speed and passes are in Make ▸ Send.");
        ui.separator();

        // PixMat
        ui.label(RichText::new("PixMat").strong());
        ui.horizontal_wrapped(|ui| {
            let has = view.photos.contains_key(&mi);
            if ui.button(if has { "📷 New photo…" } else { "📷 Photo of this mat…" }).on_hover_text("Photograph the loaded mat from above, line it up, then drag shapes onto your material").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png", "bmp", "tif", "tiff", "webp"]).pick_file() {
                    match image::open(&path) {
                        Ok(img) => view.pick = Some(Pick::new(ctx, mi, img.to_rgba8())),
                        Err(e) => app.status = format!("Couldn't open the photo: {e}"),
                    }
                }
            }
            if has {
                ui.checkbox(&mut view.show_photos, "Show");
                if ui.button("Remove").clicked() {
                    view.photos.remove(&mi);
                }
            }
        });
        if !view.photos.contains_key(&mi) {
            ui.small("See your material on the mat: photograph the loaded mat from straight above, then place shapes where the material is.");
        }
        ui.separator();

        // the selected shapes
        let n_items = view.prep.mats[mi].items.len();
        let sel: Vec<usize> = view.sel.iter().copied().filter(|s| *s < n_items).collect();
        if sel.is_empty() {
            ui.weak("Drag shapes to move them on the mat; click one (or drag a box round several) to hide them or move them to another mat.");
        } else {
            let title = if sel.len() == 1 {
                prepare::find_shape(&app.doc, view.prep.mats[mi].items[sel[0]].shape).map(|s| s.display_name()).unwrap_or_default()
            } else {
                format!("{} shapes selected", sel.len())
            };
            ui.label(RichText::new(title).strong());
            ui.horizontal_wrapped(|ui| {
                let all_hidden = sel.iter().all(|&s| view.prep.mats[mi].items[s].hidden);
                if ui.button(if all_hidden { "Show" } else { "Hide" }).on_hover_text("Hidden shapes aren't cut on this mat").clicked() {
                    for &s in &sel {
                        view.prep.mats[mi].items[s].hidden = !all_hidden;
                    }
                }
                let others: Vec<usize> = (0..view.prep.mats.len()).filter(|&m| m != mi && view.prep.mats[m].group == gi).collect();
                if !others.is_empty() {
                    let mut to = None;
                    egui::ComboBox::from_id_salt("prep-move").selected_text("Move to mat…").show_ui(ui, |ui| {
                        for m in others {
                            if ui.selectable_label(false, format!("Mat {}", m + 1)).clicked() {
                                to = Some(m);
                            }
                        }
                    });
                    if let Some(m) = to {
                        prepare::move_items(&mut view.prep, mi, &sel, m);
                        view.sel.clear();
                    }
                }
            });
        }
        ui.separator();

        // cut / save this mat
        let brother = crate::panels::is_brother(app);
        let prog = app.progress.lock().map(|p| p.clone()).unwrap_or_default();
        if app.sending && prog.done {
            app.sending = false;
            app.status = prog.message.clone();
        }
        let label = if brother { "✂  Send this mat to ScanNCut" } else { "✂  Cut this mat" };
        let go = egui::Button::new(RichText::new(label).strong().size(16.0).color(Color32::WHITE)).fill(ACCENT).min_size(egui::vec2(ui.available_width(), 36.0)).corner_radius(egui::CornerRadius::same(18));
        if ui.add_enabled(!app.sending, go).clicked() {
            let page = prepare::mat_page(&app.doc, &view.prep.mats[mi]);
            let plan = prepare::mat_plan(&app.plan, &view.prep.mats[mi]);
            if brother {
                app.send_fcm_usb(&page, &plan);
            } else {
                crate::panels::send_job(app, cutjob::generate(&page, &plan), false);
            }
            if let Some(d) = view.done.get_mut(mi) {
                *d = true;
            }
        }
        if app.sending || prog.done {
            let frac = if prog.total > 0 { prog.sent as f32 / prog.total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            ui.label(&prog.message);
            if let Some(e) = &prog.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        }
        if brother {
            ui.horizontal(|ui| {
                ui.small("Put the ScanNCut on its home screen first.");
                if ui.add_enabled(!app.sending, egui::Button::new("💾 Save .fcm…").small()).on_hover_text("Save this mat as a file for CanvasWorkspace or a USB stick").clicked() {
                    let page = prepare::mat_page(&app.doc, &view.prep.mats[mi]);
                    let plan = prepare::mat_plan(&app.plan, &view.prep.mats[mi]);
                    app.save_fcm(&page, &plan, &format!("mat-{}.fcm", mi + 1));
                }
            });
        } else {
            ui.small("Uses the connection set up in Make ▸ Send.");
        }
        if mi + 1 < view.prep.mats.len() && ui.button("Next mat →").clicked() {
            view.mat += 1;
            view.sel.clear();
        }
    });

    // ---- centre: the mat, large, with its shapes draggable
    egui::CentralPanel::default().show(ui, |ui| {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let mirror = app.plan.groups.get(gi).is_some_and(|g| g.mirror);
        // leave room under the mat for its caption
        let media_w = media(app, &view, mi).width;
        let fit = Fit::new(media(app, &view, mi), egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 24.0)), mirror);
        // pointer → mm (undoing the mirror)
        let to_mm = |p: Pos2| {
            let x = ((p.x - fit.origin.x) / fit.k) as f64;
            let y = ((p.y - fit.origin.y) / fit.k) as f64;
            Point::new(if mirror { media_w - x } else { x }, y)
        };
        let hit = |p: Pos2| {
            let q = to_mm(p);
            view.prep.mats[mi].items.iter().enumerate().rev().find(|(_, it)| prepare::item_bounds(&app.doc, &app.plan, &view.prep.mats[mi], it).is_some_and(|b| b.contains(q))).map(|(i, _)| i)
        };
        let shift = ctx.input(|i| i.modifiers.shift);
        if resp.clicked() {
            match resp.interact_pointer_pos().and_then(hit) {
                Some(i) if shift => {
                    if let Some(k) = view.sel.iter().position(|&s| s == i) {
                        view.sel.remove(k);
                    } else {
                        view.sel.push(i);
                    }
                }
                Some(i) => view.sel = vec![i],
                None if !shift => view.sel.clear(),
                None => {}
            }
        }
        if resp.drag_started() {
            let start = ctx.input(|i| i.pointer.press_origin());
            match start.and_then(hit) {
                // dragging a shape moves it, with the rest of the selection if it's in it
                Some(i) => {
                    if !view.sel.contains(&i) {
                        if !shift {
                            view.sel.clear();
                        }
                        view.sel.push(i);
                    }
                    view.band = None;
                }
                // dragging empty mat draws a selection box
                None => view.band = start.map(|s| (s, s)),
            }
        }
        if resp.dragged() {
            match &mut view.band {
                Some((_, now)) => {
                    if let Some(p) = resp.interact_pointer_pos() {
                        *now = p;
                    }
                }
                None => {
                    let d = resp.drag_delta();
                    let dx = (d.x / fit.k) as f64 * if mirror { -1.0 } else { 1.0 };
                    let n = view.prep.mats[mi].items.len();
                    for &si in view.sel.iter().filter(|&&s| s < n) {
                        view.prep.mats[mi].items[si].offset += Vec2::new(dx, (d.y / fit.k) as f64);
                    }
                }
            }
        }
        if resp.drag_stopped() {
            if let Some((a, b)) = view.band.take() {
                // left to right: shapes wholly inside; right to left: anything the box touches
                let boxed = egui::Rect::from_two_pos(a, b);
                let inside = b.x >= a.x;
                let mat = &view.prep.mats[mi];
                let hits: Vec<usize> = mat
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| {
                        prepare::item_bounds(&app.doc, &app.plan, mat, it).is_some_and(|r| {
                            let r = fit.rect(r);
                            if inside { boxed.contains_rect(r) } else { boxed.intersects(r) }
                        })
                    })
                    .map(|(i, _)| i)
                    .collect();
                if !shift {
                    view.sel.clear();
                }
                for h in hits {
                    if !view.sel.contains(&h) {
                        view.sel.push(h);
                    }
                }
            }
        }
        draw_mat(app, &view, mi, ui.painter(), &fit, &view.sel);
        if let Some((a, b)) = view.band {
            let r = egui::Rect::from_two_pos(a, b);
            ui.painter().rect_filled(r, 0.0, ACCENT.gamma_multiply(0.08));
            // dashed when it picks anything it touches, solid when only what's inside
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            if b.x >= a.x {
                ui.painter().add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, ACCENT)));
            } else {
                ui.painter().add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, ACCENT), 5.0, 3.0));
            }
        }
        let caption = if mirror { "Mirrored for cutting from the back" } else { "As it will be cut" };
        ui.painter().text(rect.left_bottom() + egui::vec2(10.0, -8.0), egui::Align2::LEFT_BOTTOM, caption, egui::FontId::proportional(13.0), ui.visuals().weak_text_color());
    });

    pick_window(ctx, &mut view, app);

    if let Some(pack) = rebuild {
        let photos = std::mem::take(&mut view.photos);
        view = PrepareView::new(app, pack);
        view.photos = photos;
    }
    if keep {
        app.prepare = Some(view);
    }
    keep
}

impl Pick {
    fn new(ctx: &egui::Context, mat: usize, mut photo: image::RgbaImage) -> Pick {
        // keep phone photos to a sensible size
        let big = photo.width().max(photo.height());
        if big > 4000 {
            let k = 4000.0 / big as f64;
            photo = image::imageops::resize(&photo, (photo.width() as f64 * k) as u32, (photo.height() as f64 * k) as u32, image::imageops::FilterType::Triangle);
        }
        let preview = image::imageops::resize(&photo, (photo.width() / 2).max(1), (photo.height() / 2).max(1), image::imageops::FilterType::Triangle);
        let ci = egui::ColorImage::from_rgba_unmultiplied([preview.width() as usize, preview.height() as usize], preview.as_raw());
        let tex = ctx.load_texture("pixmat-pick", ci, egui::TextureOptions::LINEAR);
        let (w, h) = (photo.width() as f64, photo.height() as f64);
        let corners = [Point::new(w * 0.1, h * 0.1), Point::new(w * 0.9, h * 0.1), Point::new(w * 0.9, h * 0.9), Point::new(w * 0.1, h * 0.9)];
        Pick { mat, photo, tex, corners, drag: None }
    }
}

/// Lining up a PixMat photo: drag the four corners onto the mat grid's
/// corners, then it's straightened to the mat.
fn pick_window(ctx: &egui::Context, view: &mut PrepareView, app: &mut App) {
    let Some(pick) = &mut view.pick else { return };
    let mut open = true;
    let mut done = false;
    let mut cancel = false;
    let (pw, ph) = (pick.photo.width() as f32, pick.photo.height() as f32);
    let shown = app.place.place(&app.prefs, ctx, "pixmat", egui::Window::new("PixMat - line up the mat").open(&mut open).collapsible(false).default_size([780.0, 640.0])).show(ctx, |ui| {
        ui.label("Drag the numbered corners onto the corners of the mat's grid (the cutting area): 1 top left, 2 top right, 3 bottom right, 4 bottom left - the top is the end that goes into the machine.");
        let avail = ui.available_size() - egui::vec2(0.0, 40.0);
        let k = (avail.x / pw).min(avail.y / ph).max(0.02);
        let (resp, painter) = ui.allocate_painter(egui::vec2(pw * k, ph * k), egui::Sense::click_and_drag());
        let r = resp.rect;
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(pick.tex.id(), r, full, Color32::WHITE);
        let to_s = |p: Point| r.min + egui::vec2(p.x as f32 * k, p.y as f32 * k);
        let sp: Vec<Pos2> = pick.corners.iter().map(|p| to_s(*p)).collect();
        painter.add(egui::Shape::closed_line(sp.clone(), Stroke::new(2.0, Color32::from_rgb(255, 160, 0))));
        for (i, p) in sp.iter().enumerate() {
            painter.circle_stroke(*p, 9.0, Stroke::new(2.0, Color32::from_rgb(255, 60, 0)));
            painter.text(*p + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, format!("{}", i + 1), egui::FontId::proportional(16.0), Color32::YELLOW);
        }
        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                pick.drag = sp.iter().enumerate().map(|(i, p)| (i, p.distance(pos))).filter(|(_, d)| *d < 24.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
            }
        }
        if resp.dragged() {
            if let (Some(i), Some(pos)) = (pick.drag, resp.interact_pointer_pos()) {
                let v = (pos - r.min) / k;
                pick.corners[i] = Point::new(v.x.clamp(0.0, pw) as f64, v.y.clamp(0.0, ph) as f64);
                // a magnified look at the spot, for placing it exactly
                let c = to_s(pick.corners[i]);
                let rel = egui::pos2((c.x - r.min.x) / r.width(), (c.y - r.min.y) / r.height());
                let span = egui::vec2(0.06 * ph / pw, 0.06);
                let uv = egui::Rect::from_center_size(rel, span);
                let lens = egui::Rect::from_min_size(r.min + egui::vec2(8.0, 8.0), egui::vec2(160.0, 160.0));
                painter.rect_filled(lens.expand(2.0), 4.0, Color32::BLACK);
                painter.image(pick.tex.id(), lens, uv, Color32::WHITE);
                painter.line_segment([lens.center_top(), lens.center_bottom()], Stroke::new(1.0, Color32::from_rgb(255, 60, 0)));
                painter.line_segment([lens.left_center(), lens.right_center()], Stroke::new(1.0, Color32::from_rgb(255, 60, 0)));
            }
        }
        if resp.drag_stopped() {
            pick.drag = None;
        }
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Use this photo").strong()).clicked() {
                done = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    app.place.moved(&mut app.prefs, "pixmat", &shown);
    if done {
        let pick = view.pick.take().unwrap();
        let area = grid_area(media(app, view, pick.mat));
        // about 4 px per mm: sharp enough to place shapes by
        match cutcraft_core::pixscan::rectify(&pick.photo, &pick.corners, area.width(), area.height(), 4.0) {
            Some(out) => {
                let ci = egui::ColorImage::from_rgba_unmultiplied([out.width() as usize, out.height() as usize], out.as_raw());
                view.photos.insert(pick.mat, ctx.load_texture(format!("pixmat-{}", pick.mat), ci, egui::TextureOptions::LINEAR));
                view.show_photos = true;
                app.status = "PixMat photo placed - drag shapes onto your material".into();
            }
            None => app.status = "Those corners don't make a usable shape - try again".into(),
        }
    } else if !open || cancel {
        view.pick = None;
    }
}
