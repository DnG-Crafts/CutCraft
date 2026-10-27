//! Guided screens, as in Cricut Design Space: the image upload wizard (cut
//! image / print-then-cut image / flat graphic, with background removal and a
//! preview of the cut line), the Home gallery of recent projects, and the
//! tool-change prompt shown between parts of a job.

use crate::app::App;
use crate::chrome::ACCENT;
use cutcraft_core::doc::{Shape, ShapeKind, Style};
use cutcraft_core::kurbo::{Affine, BezPath, Point};
use cutcraft_core::trace::{self, TraceMode, TraceParams};
use cutcraft_core::{geom, io, raster};
use egui::{Color32, RichText, Stroke};
use std::path::PathBuf;

// ------------------------------------------------------------------ upload wizard

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UploadKind {
    /// The outer silhouette only, as a cut line.
    Cut,
    /// The picture printed, with a cut line round it.
    PrintThenCut,
    /// The picture printed, nothing cut.
    Flat,
}

pub struct UploadWizard {
    pub name: String,
    /// The imported picture shape (as `io::import` made it).
    pub shape: Shape,
    /// The picture as uploaded, before background removal.
    pub original: Vec<u8>,
    pub kind: UploadKind,
    pub remove_bg: bool,
    pub tolerance: f64,
    /// The picture after background removal, and its outer cut line (mm).
    pub png: Vec<u8>,
    pub outline: BezPath,
    tex: Option<egui::TextureHandle>,
    dirty: bool,
}

impl UploadWizard {
    pub fn new(path: &std::path::Path, shape: Shape) -> Option<Self> {
        let ShapeKind::Image(img) = &shape.kind else { return None };
        let original = img.png.to_vec();
        Some(UploadWizard {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            png: original.clone(),
            original,
            shape,
            kind: UploadKind::PrintThenCut,
            remove_bg: false,
            tolerance: 15.0,
            outline: BezPath::new(),
            tex: None,
            dirty: true,
        })
    }

    fn size_mm(&self) -> (f64, f64) {
        match &self.shape.kind {
            ShapeKind::Image(i) => (i.width_mm, i.height_mm),
            _ => (1.0, 1.0),
        }
    }

    /// Re-runs background removal and the cut-line trace.
    fn refresh(&mut self) {
        let (w, h) = self.size_mm();
        let mut png = self.original.clone();
        if self.remove_bg {
            // clear the background from each corner inwards
            for (x, y) in [(0.01, 0.01), (w - 0.01, 0.01), (0.01, h - 0.01), (w - 0.01, h - 0.01)] {
                if let Ok((p, _)) = raster::select_erase(&png, (x, y), self.tolerance, false, w, h) {
                    png = p;
                }
            }
        }
        self.outline = BezPath::new();
        if let Ok(img) = image::load_from_memory(&png) {
            let img = img.to_rgba8();
            // trace what's left: opaque pixels black on white
            let mut mask = image::RgbaImage::from_pixel(img.width(), img.height(), image::Rgba([255, 255, 255, 255]));
            for (x, y, p) in img.enumerate_pixels() {
                if p.0[3] > 127 {
                    mask.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
            let params = TraceParams { mode: TraceMode::Threshold, threshold: 128, despeckle: 64, outer_only: true, tolerance: 1.0, smooth: true, ..TraceParams::default() };
            let px = trace::trace(&mask, &params);
            self.outline = Affine::scale_non_uniform(w / img.width().max(1) as f64, h / img.height().max(1) as f64) * px;
        }
        self.png = png;
        self.tex = None;
        self.dirty = false;
    }
}

/// Opens the wizard for an image file (instead of placing it directly).
pub fn start_upload(app: &mut App, path: PathBuf) -> bool {
    if io::kind_of(&path) != io::FileKind::Image {
        return false;
    }
    match io::import(&path, None) {
        Ok(mut shapes) if shapes.len() == 1 => match UploadWizard::new(&path, shapes.remove(0)) {
            Some(w) => {
                app.upload = Some(w);
                true
            }
            None => false,
        },
        _ => false,
    }
}

pub fn upload_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut wiz) = app.upload.take() else { return };
    if wiz.dirty {
        wiz.refresh();
    }
    let mut open = true;
    let mut done = false;
    let shown = app.place.place(&app.prefs, ctx, "upload", egui::Window::new(format!("Upload image - {}", wiz.name)).collapsible(false).resizable(false).open(&mut open)).show(ctx, |ui| {
        ui.horizontal_top(|ui| {
            // preview: the picture (or just its shape) with the cut line in red
            let (w, h) = wiz.size_mm();
            let k = (360.0 / w.max(h)) as f32;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w as f32 * k + 20.0, h as f32 * k + 20.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 6.0, Color32::from_gray(235));
            // checkerboard shows what's transparent
            let inner = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 10.0), egui::vec2(w as f32 * k, h as f32 * k));
            let cell = 10.0;
            let (nx, ny) = ((inner.width() / cell).ceil() as i32, (inner.height() / cell).ceil() as i32);
            for j in 0..ny {
                for i in 0..nx {
                    if (i + j) % 2 == 0 {
                        let c = egui::Rect::from_min_size(inner.min + egui::vec2(i as f32 * cell, j as f32 * cell), egui::vec2(cell, cell)).intersect(inner);
                        ui.painter().rect_filled(c, 0.0, Color32::from_gray(212));
                    }
                }
            }
            if wiz.kind != UploadKind::Cut {
                if wiz.tex.is_none() {
                    if let Ok(img) = image::load_from_memory(&wiz.png) {
                        let img = img.to_rgba8();
                        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                        wiz.tex = Some(ctx.load_texture("upload-preview", ci, egui::TextureOptions::LINEAR));
                    }
                }
                if let Some(t) = &wiz.tex {
                    ui.painter().image(t.id(), inner, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
            if wiz.kind != UploadKind::Flat {
                for pl in geom::flatten(&wiz.outline, 0.2) {
                    let pts: Vec<egui::Pos2> = pl.pts.iter().map(|p| inner.min + egui::vec2(p.x as f32 * k, p.y as f32 * k)).collect();
                    ui.painter().add(crate::render::closed_outline(pts, Stroke::new(2.0, Color32::from_rgb(220, 30, 30))));
                }
            }

            ui.vertical(|ui| {
                ui.set_width(260.0);
                ui.label(RichText::new("How will you use it?").strong());
                for (k, title, sub) in [
                    (UploadKind::PrintThenCut, "Print Then Cut image", "Print the picture, then cut round it (red line)"),
                    (UploadKind::Cut, "Cut image", "Just the outer shape, cut from material"),
                    (UploadKind::Flat, "Flat graphic", "Print the picture; nothing is cut"),
                ] {
                    if ui.radio(wiz.kind == k, RichText::new(title).strong()).clicked() {
                        wiz.kind = k;
                    }
                    ui.label(RichText::new(sub).small().weak());
                }
                ui.separator();
                if ui.checkbox(&mut wiz.remove_bg, "Remove background").on_hover_text("Clears the colour around the edges (e.g. a white or plain backdrop)").changed() {
                    wiz.dirty = true;
                }
                if wiz.remove_bg {
                    ui.horizontal(|ui| {
                        ui.label("Tolerance");
                        if ui.add(egui::Slider::new(&mut wiz.tolerance, 0.0..=60.0)).drag_stopped() {
                            wiz.dirty = true;
                        }
                    });
                    ui.small("Use the image tool's Select & erase or Erase afterwards for finer clean-up.");
                }
                ui.add_space(8.0);
                ui.small(format!("{:.0} × {:.0} mm - resize after placing", w, h));
                ui.add_space(8.0);
                let add = egui::Button::new(RichText::new("Add to design").strong().color(Color32::WHITE)).fill(ACCENT).min_size(egui::vec2(160.0, 30.0)).corner_radius(egui::CornerRadius::same(15));
                if ui.add(add).clicked() {
                    done = true;
                }
            });
        });
    });
    app.place.moved(&mut app.prefs, "upload", &shown);
    if done {
        place_upload(app, &wiz);
    } else if open {
        app.upload = Some(wiz);
    }
}

/// Adds the uploaded image as chosen, keeping it on the page.
fn place_upload(app: &mut App, wiz: &UploadWizard) {
    app.checkpoint_named("Upload image");
    let layer = app.current_layer();
    let mut pic = wiz.shape.clone();
    if let ShapeKind::Image(i) = &mut pic.kind {
        i.png = std::sync::Arc::new(wiz.png.clone());
    }
    let at = Affine::translate((10.0, 10.0));
    pic.transform = at;
    let cut_style = Style { stroke: Some(app.layer_colour()), ..Style::default() };
    let outline = at * wiz.outline.clone();
    let has_outline = !outline.elements().is_empty();
    let ids = match wiz.kind {
        UploadKind::Flat => {
            app.doc.reid(&mut pic);
            vec![app.doc.add_shape(pic, layer)]
        }
        UploadKind::Cut if has_outline => vec![app.doc.add_path(outline, cut_style, layer)],
        UploadKind::Cut => vec![],
        UploadKind::PrintThenCut => {
            app.doc.reid(&mut pic);
            let p = app.doc.add_shape(pic, layer);
            if has_outline {
                let c = app.doc.add_path(outline, Style { no_print: true, ..cut_style }, layer);
                cutcraft_core::ops::group(&mut app.doc, &[p, c])
            } else {
                vec![p]
            }
        }
    };
    app.status = match (wiz.kind, ids.is_empty()) {
        (_, true) => "Nothing to cut - try removing the background".into(),
        (UploadKind::PrintThenCut, _) => format!("Added {} - print it with Print & Cut marks, then cut", wiz.name),
        (UploadKind::Cut, _) => format!("Added the shape of {}", wiz.name),
        (UploadKind::Flat, _) => format!("Added {} (prints, not cut)", wiz.name),
    };
    app.sel = ids;
}

// ------------------------------------------------------------------ home gallery

pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub modified: String,
    /// The page outline and its shapes, scaled into 0..1 (with the aspect).
    pub thumb: Option<(f32, Color32, Vec<(Vec<Point>, Color32)>)>,
}

fn load_project(path: PathBuf) -> Project {
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d %b %Y").to_string())
        .unwrap_or_default();
    let thumb = io::load_native(&path).ok().map(|d| {
        let page = d.page();
        let s = page.width.max(page.height).max(1.0);
        let mut lines = vec![];
        for sh in page.shapes.iter().filter(|x| page.is_visible(x)) {
            for leaf in sh.leaves() {
                if leaf.hidden {
                    continue;
                }
                let col = leaf.style.fill.or(leaf.style.stroke).map(crate::render::c32).unwrap_or(Color32::GRAY);
                for pl in geom::flatten_all(&leaf.world_paths(), 0.5) {
                    let mut pts: Vec<Point> = pl.pts.iter().map(|p| Point::new(p.x / s, p.y / s)).collect();
                    if pl.closed && !pts.is_empty() {
                        pts.push(pts[0]);
                    }
                    lines.push((pts, col));
                }
            }
        }
        let m = page.media_color;
        ((page.width / page.height.max(1.0)) as f32, Color32::from_rgb(m.0, m.1, m.2), lines)
    });
    Project { path, name, modified, thumb }
}

pub struct Home {
    pub projects: Vec<Project>,
    pub folder: Option<PathBuf>,
}

impl Home {
    pub fn new(app: &App) -> Self {
        let projects = app.prefs.recent_files.iter().map(PathBuf::from).filter(|p| p.exists()).map(load_project).collect();
        Home { projects, folder: None }
    }
}

/// The Home screen in place of the workspace. Returns false to close it.
pub fn home_screen(app: &mut App, ui: &mut egui::Ui) -> bool {
    let Some(mut home) = app.home.take() else { return false };
    let mut keep = true;
    let mut open: Option<PathBuf> = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("CutCraft").size(26.0).strong());
            ui.add_space(20.0);
            let new = egui::Button::new(RichText::new("+  New design").strong().size(15.0).color(Color32::WHITE)).fill(ACCENT).corner_radius(egui::CornerRadius::same(16)).min_size(egui::vec2(150.0, 32.0));
            if ui.add(new).clicked() {
                keep = false;
            }
            if ui.button(RichText::new("Open…").size(15.0)).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("CutCraft", &[io::NATIVE_EXT]).pick_file() {
                    open = Some(p);
                }
            }
            if ui.button(RichText::new("Browse a folder…").size(15.0)).on_hover_text("Show every CutCraft design in a folder").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| io::kind_of(p) == io::FileKind::Native).collect()).unwrap_or_default();
                    files.sort();
                    home.projects = files.into_iter().take(200).map(load_project).collect();
                    home.folder = Some(dir);
                }
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(match &home.folder {
            Some(d) => format!("Designs in {}", d.display()),
            None => "Recent designs".into(),
        }).strong());
        ui.add_space(6.0);
        if home.projects.is_empty() {
            ui.weak("Nothing here yet - start a new design, or open one.");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(14.0, 14.0);
                for p in &home.projects {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(200.0, 230.0), egui::Sense::click());
                    let v = ui.visuals();
                    ui.painter().rect_filled(rect, 8.0, if resp.hovered() { v.widgets.hovered.weak_bg_fill } else { v.faint_bg_color });
                    let img = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 10.0), egui::vec2(180.0, 170.0));
                    match &p.thumb {
                        Some((aspect, media, lines)) => {
                            let (w, h) = if *aspect >= 1.0 { (170.0, 170.0 / aspect) } else { (170.0 * aspect, 170.0) };
                            let page = egui::Rect::from_center_size(img.center(), egui::vec2(w, h));
                            ui.painter().rect_filled(page, 2.0, *media);
                            let k = 170.0f32;
                            for (pts, c) in lines {
                                let sp: Vec<egui::Pos2> = pts.iter().map(|q| page.min + egui::vec2(q.x as f32 * k, q.y as f32 * k)).collect();
                                ui.painter().add(egui::Shape::line(sp, Stroke::new(1.0, *c)));
                            }
                        }
                        None => {
                            ui.painter().text(img.center(), egui::Align2::CENTER_CENTER, "can't preview", egui::FontId::proportional(12.0), v.weak_text_color());
                        }
                    }
                    ui.painter().text(rect.left_bottom() + egui::vec2(12.0, -30.0), egui::Align2::LEFT_BOTTOM, &p.name, egui::FontId::proportional(14.0), v.strong_text_color());
                    ui.painter().text(rect.left_bottom() + egui::vec2(12.0, -12.0), egui::Align2::LEFT_BOTTOM, &p.modified, egui::FontId::proportional(11.0), v.weak_text_color());
                    if resp.on_hover_text(p.path.display().to_string()).clicked() {
                        open = Some(p.path.clone());
                    }
                }
            });
        });
    });
    if let Some(p) = open {
        app.open_path(p);
        keep = false;
    }
    if keep {
        app.home = Some(home);
    }
    keep
}

// ------------------------------------------------------------------ tool-change prompt

/// Between parts of a job that need a different tool: asks for the swap, then
/// sends the next part.
pub fn tool_change_window(app: &mut App, ctx: &egui::Context) {
    if app.sending || app.tool_queue.is_empty() {
        return;
    }
    let failed = app.progress.lock().map(|p| p.error.is_some()).unwrap_or(false);
    let (_, tool, slot, _) = &app.tool_queue[0];
    let (tool, slot) = (*tool, *slot);
    let mut go = false;
    let mut stop = false;
    let shown = app.place.place(&app.prefs, ctx, "tool-change", crate::dialogs::prompt_window(ctx, "Change tool")).show(ctx, |ui| {
        use crate::dialogs::Choice;
        let mut lines = vec![];
        if failed {
            lines.push("The last part didn't finish.".to_string());
        }
        lines.push("Don't unload the mat - the next part cuts in the same place.".to_string());
        lines.push(format!("{} part{} left.", app.tool_queue.len(), if app.tool_queue.len() == 1 { "" } else { "s" }));
        match crate::dialogs::prompt(ui, &format!("Load the {} in carriage {slot}", tool.label()), &lines, &[Choice::main("Continue"), Choice::other("Stop here")]) {
            Some(0) => go = true,
            Some(_) => stop = true,
            None => {}
        }
    });
    app.place.moved(&mut app.prefs, "tool-change", &shown);
    if stop {
        app.tool_queue.clear();
        app.status = "Stopped before the tool change".into();
    } else if go {
        let (job, _, _, last) = app.tool_queue.remove(0);
        crate::panels::send_part(app, job, false, last);
    }
}

/// Splits pen/blade/… parts of `job` so the user is asked to swap tools in
/// between; returns the first part to send now, and whether it's the last.
pub fn queue_tool_changes(app: &mut App, job: cutcraft_core::cutjob::CutJob) -> (cutcraft_core::cutjob::CutJob, bool) {
    let slots = cutcraft_core::machines::by_id(&app.doc.machine).tool_slots;
    let mut parts = cutcraft_core::cutjob::split_at_tool_changes(&job, slots);
    if parts.len() <= 1 {
        return (job, true);
    }
    let n = parts.len();
    let first = parts.remove(0).0;
    app.tool_queue = parts.into_iter().enumerate().map(|(i, (j, t))| {
        let (tool, slot) = t.unwrap_or((cutcraft_core::materials::Tool::AutoBlade, 1));
        (j, tool, slot, i + 2 == n)
    }).collect();
    (first, false)
}
