//! Panels for warp, PixScan, barcodes, mail merge, font creation and spelling.

use crate::app::{Action, App};
use cutcraft_core::barcode::{self, Symbology};
use cutcraft_core::doc::{CutStyle, Page, Rgba, Shape, ShapeKind, Style, TextData};
use cutcraft_core::kurbo::{Affine, Point, Rect, Vec2};
use cutcraft_core::mailmerge::{self, Table};
use cutcraft_core::pixscan::{self, MarkerSheet};
use cutcraft_core::prefs::Units;
use cutcraft_core::spell::{self, Speller};
use cutcraft_core::userfont::UserFont;
use cutcraft_core::warp::{ConeParams, Preset, WarpMode};
use cutcraft_core::fx::{self, Warp as FxWarp};
use cutcraft_core::{ble, effects, geom, io, ops, pathtools};
use cutcraft_core::doc::ShapeId;
use cutcraft_core::kurbo::BezPath;
use egui::{Color32, DragValue, RichText, Ui};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct Extra {
    // warp
    pub warp_rows: usize,
    pub warp_cols: usize,
    pub warp_k: f64,
    pub cone: ConeParams,
    // pixscan
    pub pix_photo: Option<image::RgbaImage>,
    pub pix_tex: Option<egui::TextureHandle>,
    pub pix_corners: [Point; 4],
    pub pix_target: [f64; 4],
    pub pix_dpmm: f64,
    pub pix_window: bool,
    pub pix_drag: Option<usize>,
    pub pix_sheet: MarkerSheet,
    // barcode
    pub bc_sym: Symbology,
    pub bc_text: String,
    pub bc_module: f64,
    pub bc_height: f64,
    pub bc_label: bool,
    pub bc_cut: bool,
    // mail merge
    pub mm_table: Option<Table>,
    pub mm_file: String,
    pub mm_gap: f64,
    pub mm_margin: f64,
    pub mm_count: usize,
    pub mm_serial: i64,
    pub mm_inc: i64,
    // fonts
    pub font_sel: Option<usize>,
    pub font_new_name: String,
    pub font_char: String,
    pub font_seq: String,
    pub font_em: f64,
    pub font_bearing: f64,
    pub font_baseline_bottom: bool,
    // spelling
    pub dicts: Option<Vec<PathBuf>>,
    pub speller: Option<Speller>,
    pub spell_word: Option<(String, Vec<String>)>,
    // bluetooth
    pub ble_scan: Option<Arc<Mutex<Option<Result<Vec<ble::BleDevice>, String>>>>>,
    pub ble_devices: Vec<ble::BleDevice>,
    pub ble_idx: usize,
    pub ble_chunk: usize,
    pub ble_write_uuid: String,
    pub ble_notify_uuid: String,
    // distort (fx)
    pub fx_warp: usize,
    pub fx_amount: f64,
    pub fx_k2: f64,
    pub shadow_dx: f64,
    pub shadow_dy: f64,
    pub cast_skew: f64,
    pub cast_squash: f64,
    pub extrude_dx: f64,
    pub extrude_dy: f64,
    pub round_radius: f64,
    // path tools
    pub lattice_cell: f64,
    pub lattice_bar: f64,
    pub knockout_gap: f64,
    pub dedup_tol: f64,
    pub bridge_w: f64,
    pub bridge_n: usize,
    pub linefill_spacing: f64,
    pub linefill_angle: f64,
    pub join_tol: f64,
    pub oop_count: usize,
    pub oop_rotate: bool,
    pub oop_keep: bool,
    // dieline
    pub die_kind: usize,
    pub die: cutcraft_core::dieline::DielineParams,
    // cost estimate
    pub cost: cutcraft_core::cutjob::CostParams,
    // batch fill
    pub batch_rotate: bool,
    pub batch_per: usize,
    // blend
    pub blend_steps: usize,
    // script
    pub script_src: String,
    pub script_msg: String,
    // image tools (ported from BurnCraft)
    pub img_adj: cutcraft_core::raster::Adjustments,
    pub img_lines: cutcraft_core::raster::LineOptions,
    pub img_plates: usize,
    pub img_border: f64,
    pub img_smooth: f64,
    /// Preview cache: (image id, base thumbnail) and (id, adjustments, texture).
    pub img_base: Option<(ShapeId, cutcraft_core::raster::Gray)>,
    pub img_prev: Option<(ShapeId, cutcraft_core::raster::Adjustments, egui::TextureHandle)>,
    // art panel (ported from BurnCraft)
    pub art_size: f64,
    pub border_len: f64,
    pub border_h: f64,
    pub sign_shape: usize,
    pub sign_w: f64,
    pub sign_h: f64,
    pub sign_corner: f64,
    pub sign_holes: u32,
    pub sign_hole_d: f64,
    pub sign_inset: f64,
    pub mono: [String; 3],
    pub mono_family: String,
    pub mono_size: f64,
    pub mono_frame: usize,
}

impl Default for Extra {
    fn default() -> Self {
        Extra {
            warp_rows: 4,
            warp_cols: 4,
            warp_k: 0.25,
            cone: ConeParams::default(),
            pix_photo: None,
            pix_tex: None,
            pix_corners: [Point::ZERO; 4],
            pix_target: [0.0; 4],
            pix_dpmm: 6.0,
            pix_window: false,
            pix_drag: None,
            pix_sheet: MarkerSheet::default(),
            bc_sym: Symbology::Qr,
            bc_text: "https://example.com".into(),
            bc_module: 1.0,
            bc_height: 15.0,
            bc_label: true,
            bc_cut: false,
            mm_table: None,
            mm_file: String::new(),
            mm_gap: 5.0,
            mm_margin: 10.0,
            mm_count: 10,
            mm_serial: 1,
            mm_inc: 1,
            font_sel: None,
            font_new_name: "My Handwriting".into(),
            font_char: "A".into(),
            font_seq: "ABCDEFGHIJKLMNOPQRSTUVWXYZ".into(),
            font_em: 40.0,
            font_bearing: 1.0,
            font_baseline_bottom: true,
            dicts: None,
            speller: None,
            spell_word: None,
            ble_scan: None,
            ble_devices: vec![],
            ble_idx: 0,
            ble_chunk: 20,
            ble_write_uuid: String::new(),
            ble_notify_uuid: String::new(),
            fx_warp: 0,
            fx_amount: 0.3,
            fx_k2: 1.0,
            shadow_dx: 3.0,
            shadow_dy: 3.0,
            cast_skew: 1.2,
            cast_squash: 0.35,
            extrude_dx: 8.0,
            extrude_dy: -8.0,
            round_radius: 3.0,
            lattice_cell: 12.0,
            lattice_bar: 3.0,
            knockout_gap: 1.0,
            dedup_tol: 0.1,
            bridge_w: 2.0,
            bridge_n: 3,
            linefill_spacing: 2.0,
            linefill_angle: 45.0,
            join_tol: 0.2,
            oop_count: 8,
            oop_rotate: true,
            oop_keep: false,
            die_kind: 0,
            die: cutcraft_core::dieline::DielineParams::default(),
            cost: cutcraft_core::cutjob::CostParams::default(),
            batch_rotate: false,
            blend_steps: 5,
            batch_per: 0,
            script_src: DEFAULT_SCRIPT.into(),
            script_msg: String::new(),
            img_adj: Default::default(),
            img_lines: Default::default(),
            img_plates: 4,
            img_border: 10.0,
            img_smooth: 1.5,
            img_base: None,
            img_prev: None,
            art_size: 60.0,
            border_len: 200.0,
            border_h: 25.0,
            sign_shape: 3,
            sign_w: 200.0,
            sign_h: 80.0,
            sign_corner: 12.0,
            sign_holes: 2,
            sign_hole_d: 5.0,
            sign_inset: 8.0,
            mono: ["J".into(), "S".into(), "A".into()],
            mono_family: "Times New Roman".into(),
            mono_size: 40.0,
            mono_frame: 2,
        }
    }
}

/// Starter script shown in the Script panel.
const DEFAULT_SCRIPT: &str = r#"// Petals around a circle. Numbers are millimetres.
let cx = page_w() / 2.0;
let cy = page_h() / 2.0;
let n = 12;
for i in 0..n {
    let a = i * 2.0 * pi() / n;
    circle(cx + cos(a) * 35.0, cy + sin(a) * 35.0, 12.0);
}
circle(cx, cy, 15.0);

// Builders: rect(x,y,w,h) circle(cx,cy,r) ellipse(cx,cy,rx,ry)
//           line(x1,y1,x2,y2) polygon(cx,cy,r,sides) star(cx,cy,ro,ri,pts)
//           poly([x0,y0,...]) polyline([...])  helpers: page_w() page_h() pi() rad(deg)
"#;

pub(crate) fn len_field(ui: &mut Ui, v: &mut f64, units: Units) -> egui::Response {
    let f = units.factor();
    let mut x = *v * f;
    let r = ui.add(DragValue::new(&mut x).speed(0.1 * f).max_decimals(3).suffix(format!(" {}", units.suffix())));
    if r.changed() {
        *v = x / f;
    }
    r
}

// ------------------------------------------------------------------ warp

pub fn warp_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    let warp_id = sel.iter().copied().find(|id| matches!(app.doc.page().shape(*id).map(|s| &s.kind), Some(ShapeKind::Warp(_))));
    let mut act = None;
    ui.heading("Warp");
    if sel.is_empty() {
        ui.label("Select artwork to warp. Text, groups and images' outlines all work.");
    }
    ui.horizontal(|ui| {
        ui.label("Grid");
        ui.add(DragValue::new(&mut app.ps.x.warp_rows).range(1..=12).prefix("rows "));
        ui.add(DragValue::new(&mut app.ps.x.warp_cols).range(1..=12).prefix("cols "));
    });
    ui.horizontal(|ui| {
        ui.label("Strength");
        ui.add(egui::Slider::new(&mut app.ps.x.warp_k, -0.8..=0.8));
    });
    let (rows, cols, k) = (app.ps.x.warp_rows, app.ps.x.warp_cols, app.ps.x.warp_k);
    if warp_id.is_none() && !sel.is_empty() && ui.button("Apply envelope warp").clicked() {
        app.edit(|d, s| Some(ops::make_warp(d, s, WarpMode::Mesh { rows, cols, pts: vec![] })));
        app.tool = crate::app::Tool::Node;
        app.status = "Drag the grid points with the ◇ tool, or pick a preset".into();
    }
    if let Some(id) = warp_id {
        let mesh = matches!(app.doc.page().shape(id).map(|s| &s.kind), Some(ShapeKind::Warp(w)) if matches!(w.mode, WarpMode::Mesh{..}));
        if mesh {
            ui.label("Presets:");
            ui.horizontal_wrapped(|ui| {
                for (p, name) in Preset::ALL {
                    if ui.button(name).clicked() {
                        app.edit(|d, _| {
                            if let Some(ShapeKind::Warp(w)) = d.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                                w.set_preset(p, k, rows, cols);
                            }
                            None
                        });
                    }
                }
            });
            ui.small("Edit the grid freely with the ◇ point tool.");
        }
    }
    ui.separator();
    ui.heading("Conical warp (tumblers & cups)");
    let c = &mut app.ps.x.cone;
    let before = c.clone();
    egui::Grid::new("cone").num_columns(2).show(ui, |ui| {
        ui.label("Top diameter");
        len_field(ui, &mut c.top_d, units);
        ui.end_row();
        ui.label("Bottom diameter");
        len_field(ui, &mut c.bottom_d, units);
        ui.end_row();
        ui.label("Slant height");
        len_field(ui, &mut c.slant, units);
        ui.end_row();
        ui.label("Design below rim");
        len_field(ui, &mut c.offset_from_top, units);
        ui.end_row();
    });
    let cone = c.clone();
    if let Some(id) = warp_id {
        let conical = matches!(app.doc.page().shape(id).map(|s| &s.kind), Some(ShapeKind::Warp(w)) if matches!(w.mode, WarpMode::Conical(_)));
        if conical && cone != before {
            app.checkpoint();
            if let Some(ShapeKind::Warp(w)) = app.doc.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                w.mode = WarpMode::Conical(cone.clone());
            }
        }
    } else if !sel.is_empty() && ui.button("Apply conical warp").clicked() {
        let p = cone.clone();
        app.edit(|d, s| Some(ops::make_warp(d, s, WarpMode::Conical(p))));
    }
    if ui.add_enabled(!sel.is_empty(), egui::Button::new("Add full cup wrap template")).clicked() {
        if let Some(bb) = ops::selection_bbox(&app.doc, &sel) {
            if let Some(t) = cutcraft_core::warp::cone_template(&cone, bb) {
                app.checkpoint();
                let st = Style { stroke: Some(Rgba(120, 120, 120, 255)), ..Style::default() };
                let l = app.current_layer();
                let id = app.doc.add_path(t, st, l);
                app.sel = vec![id];
            } else {
                app.status = "Top and bottom are equal - a cylinder needs no conical warp".into();
            }
        }
    }
    if warp_id.is_some() {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Release (convert to paths)").clicked() {
                act = Some(Action::ConvertToPath);
            }
            if ui.button("Revert to original").clicked() {
                app.edit(|d, s| Some(ops::warp_revert(d, s)));
            }
        });
    }
    act
}

/// Grid handles of a selected mesh warp: (shape id, points in page space, rows, cols).
pub fn warp_handles(app: &App) -> Option<(u64, Vec<Point>, usize, usize)> {
    if app.sel.len() != 1 {
        return None;
    }
    let s = app.doc.page().shape(app.sel[0])?;
    if let ShapeKind::Warp(w) = &s.kind {
        if let WarpMode::Mesh { rows, cols, pts } = &w.mode {
            return Some((s.id, pts.iter().map(|p| s.transform * Point::new(p.0, p.1)).collect(), *rows, *cols));
        }
    }
    None
}

pub fn set_warp_point(app: &mut App, id: u64, idx: usize, page_pt: Point) {
    if let Some(s) = app.doc.page_mut().shape_mut(id) {
        let inv = s.transform.inverse();
        if let ShapeKind::Warp(w) = &mut s.kind {
            if let WarpMode::Mesh { pts, .. } = &mut w.mode {
                let l = inv * page_pt;
                if let Some(p) = pts.get_mut(idx) {
                    *p = (l.x, l.y);
                }
            }
        }
    }
    app.touch();
}

// ------------------------------------------------------------------ pixscan

pub fn pixscan_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("PixScan - place a photo at true size");
    ui.label("1. Print the marker sheet for this page size (or use any mat whose corner positions you know).");
    ui.label("2. Put your item on it and take a photo from above.");
    ui.label("3. Load the photo; markers are found automatically (drag the corners to adjust).");
    ui.horizontal(|ui| {
        ui.label("Marker size");
        len_field(ui, &mut app.ps.x.pix_sheet.size, units);
        ui.label("inset");
        len_field(ui, &mut app.ps.x.pix_sheet.inset, units);
    });
    if ui.button("Export marker sheet PDF…").clicked() {
        let p = app.doc.page();
        let mut sheet = Page::new("PixScan sheet", p.width, p.height);
        for m in app.ps.x.pix_sheet.markers(p.width, p.height) {
            sheet.shapes.push(Shape::new_path(0, m, Style { fill: Some(Rgba::BLACK), stroke: None, ..Style::default() }));
        }
        if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name("pixscan_sheet.pdf").save_file() {
            app.status = match io::export_print_pdf(&sheet, &path) {
                Ok(()) => format!("Marker sheet written - print at 100%: {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            };
        }
    }
    ui.separator();
    if ui.button("Load photo…").clicked() {
        if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["jpg", "jpeg", "png", "bmp", "tif", "tiff", "webp"]).pick_file() {
            match image::open(&path) {
                Ok(img) => {
                    let mut rgba = img.to_rgba8();
                    // keep memory sane for 48 MP phone photos
                    if rgba.width().max(rgba.height()) > 4000 {
                        let k = 4000.0 / rgba.width().max(rgba.height()) as f64;
                        rgba = image::imageops::resize(&rgba, (rgba.width() as f64 * k) as u32, (rgba.height() as f64 * k) as u32, image::imageops::FilterType::Triangle);
                    }
                    let (w, h) = (rgba.width() as f64, rgba.height() as f64);
                    let page = app.doc.page();
                    let c = app.ps.x.pix_sheet.centres(page.width, page.height);
                    app.ps.x.pix_target = [c[0].x, c[0].y, c[2].x, c[2].y];
                    app.ps.x.pix_corners = match pixscan::detect_markers(&rgba) {
                        Some(m) => {
                            app.status = "Markers found".into();
                            m
                        }
                        None => {
                            app.status = "Markers not found - drag the four corners onto known points".into();
                            [Point::new(w * 0.1, h * 0.1), Point::new(w * 0.9, h * 0.1), Point::new(w * 0.9, h * 0.9), Point::new(w * 0.1, h * 0.9)]
                        }
                    };
                    let preview = image::imageops::resize(&rgba, (rgba.width() / 4).max(1), (rgba.height() / 4).max(1), image::imageops::FilterType::Triangle);
                    let ci = egui::ColorImage::from_rgba_unmultiplied([preview.width() as usize, preview.height() as usize], preview.as_raw());
                    app.ps.x.pix_tex = Some(ui.ctx().load_texture("pixscan", ci, egui::TextureOptions::LINEAR));
                    app.ps.x.pix_photo = Some(rgba);
                    app.ps.x.pix_window = true;
                }
                Err(e) => app.status = format!("Can't open photo: {e}"),
            }
        }
    }
    if app.ps.x.pix_photo.is_some() {
        ui.label("The corners map to this rectangle on the page:");
        let t = &mut app.ps.x.pix_target;
        egui::Grid::new("pxt").num_columns(4).show(ui, |ui| {
            ui.label("Left");
            len_field(ui, &mut t[0], units);
            ui.label("Top");
            len_field(ui, &mut t[1], units);
            ui.end_row();
            ui.label("Right");
            len_field(ui, &mut t[2], units);
            ui.label("Bottom");
            len_field(ui, &mut t[3], units);
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.label("Resolution");
            ui.add(DragValue::new(&mut app.ps.x.pix_dpmm).range(1.0..=20.0).suffix(" px/mm"));
        });
        ui.horizontal(|ui| {
            if ui.button("Show alignment window").clicked() {
                app.ps.x.pix_window = true;
            }
            if ui.button(RichText::new("Place on page").strong()).clicked() {
                place_pixscan(app);
            }
        });
    }
    None
}

fn place_pixscan(app: &mut App) {
    let Some(photo) = &app.ps.x.pix_photo else { return };
    let t = app.ps.x.pix_target;
    let (w, h) = (t[2] - t[0], t[3] - t[1]);
    if w <= 1.0 || h <= 1.0 {
        app.status = "Target rectangle is too small".into();
        return;
    }
    let Some(out) = pixscan::rectify(photo, &app.ps.x.pix_corners, w, h, app.ps.x.pix_dpmm) else {
        app.status = "Corners don't form a usable quadrilateral".into();
        return;
    };
    let mut png = vec![];
    if image::DynamicImage::ImageRgba8(out.clone()).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
        return;
    }
    let s = Shape {
        id: 0,
        name: "PixScan image".into(),
        layer: 0,
        transform: Affine::translate((t[0], t[1])),
        style: Style { fill: None, stroke: None, stroke_width: 0.0, cut: CutStyle::NoCut, no_print: true },
        kind: ShapeKind::Image(cutcraft_core::doc::ImageData { png: Arc::new(png), px_w: out.width(), px_h: out.height(), width_mm: w, height_mm: h, original: None }),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    };
    app.checkpoint();
    let id = app.add_shape(s);
    // send it to the back so tracing results sit on top
    ops::arrange(&mut app.doc, &[id], ops::Arrange::Back);
    app.ps.x.pix_window = false;
    app.status = "PixScan image placed - trace it or draw cut lines around it".into();
}

/// Floating window to adjust the four photo corners.
pub fn pixscan_window(app: &mut App, ctx: &egui::Context) {
    if !app.ps.x.pix_window {
        return;
    }
    let (Some(tex), Some(photo)) = (app.ps.x.pix_tex.clone(), app.ps.x.pix_photo.as_ref()) else { return };
    let (pw, ph) = (photo.width() as f32, photo.height() as f32);
    let mut open = true;
    let mut place = false;
    let shown = app.place.place(&app.prefs, ctx, "pixscan", egui::Window::new("PixScan alignment").open(&mut open).default_size([760.0, 600.0])).show(ctx, |ui| {
        ui.label("Drag the numbered corners onto the marker centres: 1 top-left, 2 top-right, 3 bottom-right, 4 bottom-left.");
        let avail = ui.available_size() - egui::vec2(0.0, 40.0);
        let k = (avail.x / pw).min(avail.y / ph).max(0.05);
        let (resp, painter) = ui.allocate_painter(egui::vec2(pw * k, ph * k), egui::Sense::click_and_drag());
        let r = resp.rect;
        painter.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        let to_s = |p: Point| r.min + egui::vec2(p.x as f32 * k, p.y as f32 * k);
        let corners = app.ps.x.pix_corners;
        let sp: Vec<egui::Pos2> = corners.iter().map(|p| to_s(*p)).collect();
        painter.add(egui::Shape::closed_line(sp.clone(), egui::Stroke::new(2.0, Color32::from_rgb(255, 160, 0))));
        for (i, p) in sp.iter().enumerate() {
            painter.circle_stroke(*p, 9.0, egui::Stroke::new(2.0, Color32::from_rgb(255, 60, 0)));
            painter.text(*p + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, format!("{}", i + 1), egui::FontId::proportional(16.0), Color32::YELLOW);
        }
        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                app.ps.x.pix_drag = sp.iter().enumerate().map(|(i, p)| (i, p.distance(pos))).filter(|(_, d)| *d < 20.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
            }
        }
        if resp.dragged() {
            if let (Some(i), Some(pos)) = (app.ps.x.pix_drag, resp.interact_pointer_pos()) {
                let v = (pos - r.min) / k;
                app.ps.x.pix_corners[i] = Point::new(v.x.clamp(0.0, pw) as f64, v.y.clamp(0.0, ph) as f64);
            }
        }
        if resp.drag_stopped() {
            app.ps.x.pix_drag = None;
        }
        ui.horizontal(|ui| {
            if ui.button("Auto-detect markers").clicked() {
                if let Some(m) = app.ps.x.pix_photo.as_ref().and_then(pixscan::detect_markers) {
                    app.ps.x.pix_corners = m;
                }
            }
            if ui.button(RichText::new("Place on page").strong()).clicked() {
                place = true;
            }
        });
    });
    app.place.moved(&mut app.prefs, "pixscan", &shown);
    if place {
        place_pixscan(app);
    }
    if !open {
        app.ps.x.pix_window = false;
    }
}

// ------------------------------------------------------------------ barcode

pub fn barcode_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let x = &mut app.ps.x;
    ui.heading("Barcodes & QR codes");
    ui.horizontal(|ui| {
        for (s, n) in Symbology::ALL {
            ui.selectable_value(&mut x.bc_sym, s, n);
        }
    });
    ui.add(egui::TextEdit::singleline(&mut x.bc_text).hint_text("text / URL").desired_width(f32::INFINITY));
    egui::Grid::new("bc").num_columns(2).show(ui, |ui| {
        ui.label(if x.bc_sym == Symbology::Qr { "Module size" } else { "Narrow bar" });
        len_field(ui, &mut x.bc_module, units);
        ui.end_row();
        if x.bc_sym != Symbology::Qr {
            ui.label("Bar height");
            len_field(ui, &mut x.bc_height, units);
            ui.end_row();
        }
    });
    if x.bc_sym != Symbology::Qr {
        ui.checkbox(&mut x.bc_label, "Add human-readable text");
    }
    ui.checkbox(&mut x.bc_cut, "Cut the bars (stencil) instead of printing them");
    if x.bc_module < 0.5 {
        ui.colored_label(Color32::YELLOW, "Bars thinner than 0.5 mm are hard to cut cleanly.");
    }
    if ui.button("Add to page").clicked() {
        let (sym, text, m, h, label, cut) = (x.bc_sym, x.bc_text.clone(), x.bc_module.max(0.05), x.bc_height, x.bc_label, x.bc_cut);
        let page = app.doc.page();
        let (cx, cy) = (page.width / 2.0, page.height / 2.0);
        match barcode::generate(sym, &text, cx, cy, m, h) {
            Ok(path) => {
                app.checkpoint();
                let bb = geom::paths_bbox(&[path.clone()]).unwrap_or(Rect::new(cx, cy, cx, cy));
                let mut path = path;
                path.apply_affine(Affine::translate((-bb.width() / 2.0, -bb.height() / 2.0)));
                let style = Style {
                    fill: Some(Rgba::BLACK),
                    stroke: if cut { Some(Rgba::RED) } else { None },
                    cut: if cut { CutStyle::Cut } else { CutStyle::NoCut },
                    ..Style::default()
                };
                let layer = app.current_layer();
                let mut ids = vec![app.doc.add_path(path, style, layer)];
                if label && sym != Symbology::Qr {
                    let data = TextData { text: text.to_uppercase(), size: (h * 0.25).max(3.0), align: cutcraft_core::doc::Align::Center, ..app.ps.text.clone() };
                    let outline = app.fonts.layout(&data);
                    let tb = geom::paths_bbox(&[outline.clone()]).unwrap_or(Rect::ZERO);
                    let s = Shape {
                        id: 0,
                        name: String::new(),
                        layer,
                        transform: Affine::translate((cx - tb.width() / 2.0 - tb.x0, cy + h / 2.0 + 1.0)),
                        style: Style { fill: Some(Rgba::BLACK), stroke: None, cut: CutStyle::NoCut, ..Style::default() },
                        kind: ShapeKind::Text { data, outline },
                        locked: false,
                        hidden: false,
                        effects: Vec::new(),
                    };
                    ids.push(app.doc.add_shape(s, layer));
                }
                app.sel = ids;
            }
            Err(e) => app.status = e.to_string(),
        }
    }
    None
}

// ------------------------------------------------------------------ mail merge

/// Built-in merge fields with today's date and the current time.
fn merge_fields(app: &App) -> mailmerge::Fields {
    let now = chrono::Local::now();
    mailmerge::Fields {
        serial_start: app.ps.x.mm_serial,
        increment: app.ps.x.mm_inc,
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H:%M").to_string(),
    }
}

fn is_builtin_field(name: &str) -> bool {
    let key = name.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    mailmerge::BUILTIN_FIELDS.contains(&key.as_str())
}

pub fn mailmerge_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Mail merge");
    ui.label("Put placeholders like {{Name}} in text, select the template, load a CSV whose header row names the fields, then Merge.");
    egui::CollapsingHeader::new("Built-in fields").show(ui, |ui| {
        egui::Grid::new("mmfields").num_columns(2).striped(true).show(ui, |ui| {
            for (f, d) in [
                ("{{serial}}", "serial number (start + step per copy)"),
                ("{{serial:0000}}", "serial number zero-padded to 4 digits"),
                ("{{row}}", "copy number, 1-based"),
                ("{{date}}", "today's date"),
                ("{{time}}", "the current time"),
            ] {
                ui.monospace(f);
                ui.label(d);
                ui.end_row();
            }
        });
    });
    if ui.button("Load CSV…").clicked() {
        if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv", "txt", "tsv"]).pick_file() {
            match std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).to_string()).map_err(anyhow::Error::from).and_then(|s| mailmerge::parse_csv(&s)) {
                Ok(t) => {
                    app.ps.x.mm_file = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    app.status = format!("{} rows loaded", t.rows.len());
                    app.ps.x.mm_table = Some(t);
                }
                Err(e) => app.status = format!("CSV error: {e}"),
            }
        }
    }
    let sel_shapes: Vec<Shape> = app.doc.page().shapes.iter().filter(|s| app.sel.contains(&s.id)).cloned().collect();
    let used = mailmerge::placeholders(&sel_shapes);
    if let Some(t) = &app.ps.x.mm_table {
        ui.label(RichText::new(format!("{} - {} rows", app.ps.x.mm_file, t.rows.len())).strong());
        egui::ScrollArea::horizontal().id_salt("mmgrid").show(ui, |ui| {
            egui::Grid::new("mm").striped(true).show(ui, |ui| {
                for h in &t.headers {
                    let ok = used.iter().any(|u| u.eq_ignore_ascii_case(h));
                    ui.label(RichText::new(h).strong().color(if ok { Color32::LIGHT_GREEN } else { ui.visuals().text_color() }));
                }
                ui.end_row();
                for r in t.rows.iter().take(5) {
                    for c in r {
                        ui.label(c);
                    }
                    ui.end_row();
                }
            });
        });
        let missing: Vec<&String> =
            used.iter().filter(|u| !is_builtin_field(u) && !t.headers.iter().any(|h| h.eq_ignore_ascii_case(u))).collect();
        if !missing.is_empty() {
            ui.colored_label(Color32::YELLOW, format!("Not in CSV: {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }
    ui.label(if used.is_empty() { "Selection has no {{placeholders}}.".to_string() } else { format!("Placeholders in selection: {}", used.join(", ")) });
    egui::Grid::new("mmlayout").num_columns(2).show(ui, |ui| {
        ui.label("Spacing");
        len_field(ui, &mut app.ps.x.mm_gap, units);
        ui.end_row();
        ui.label("Page margin");
        len_field(ui, &mut app.ps.x.mm_margin, units);
        ui.end_row();
        ui.label("First serial");
        ui.add(egui::DragValue::new(&mut app.ps.x.mm_serial));
        ui.end_row();
        ui.label("Serial step");
        ui.add(egui::DragValue::new(&mut app.ps.x.mm_inc));
        ui.end_row();
    });
    let can = app.ps.x.mm_table.is_some() && !app.sel.is_empty();
    if ui.add_enabled(can, egui::Button::new(RichText::new("Merge").strong())).clicked() {
        let t = app.ps.x.mm_table.clone().unwrap();
        let (g, m) = (app.ps.x.mm_gap, app.ps.x.mm_margin);
        app.checkpoint();
        let sel = app.sel.clone();
        let fields = merge_fields(app);
        match mailmerge::merge(&mut app.doc, &sel, &t, &fields, &app.fonts, g, m) {
            Ok(n) => {
                app.status = format!("Created {n} copies on {} page(s)", app.doc.pages.len());
                app.sel.clear();
            }
            Err(e) => app.status = e.to_string(),
        }
    }

    ui.separator();
    ui.label(RichText::new("Numbered copies (no CSV needed)").strong());
    ui.small("E.g. tags reading \"No. {{serial:000}}\": makes that many copies with the built-in fields filled in.");
    ui.horizontal(|ui| {
        ui.label("Copies");
        ui.add(egui::DragValue::new(&mut app.ps.x.mm_count).range(1..=10_000));
        let can = !app.sel.is_empty();
        if ui.add_enabled(can, egui::Button::new("Make copies")).clicked() {
            let (n, g, m) = (app.ps.x.mm_count, app.ps.x.mm_gap, app.ps.x.mm_margin);
            let fields = merge_fields(app);
            app.checkpoint();
            let sel = app.sel.clone();
            match mailmerge::merge_count(&mut app.doc, &sel, n, &fields, &app.fonts, g, m) {
                Ok(n) => {
                    app.status = format!("Created {n} numbered copies on {} page(s)", app.doc.pages.len());
                    app.sel.clear();
                }
                Err(e) => app.status = e.to_string(),
            }
        }
    });
    None
}

// ------------------------------------------------------------------ fonts

pub fn fonts_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Font creation");
    ui.label("Draw or trace letters, select one, and assign it to a key. Fonts appear in the Text font list and can be exported as .ttf.");
    let names: Vec<String> = app.fonts.user.iter().map(|f| f.name.clone()).collect();
    for (i, n) in names.iter().enumerate() {
        let count = app.fonts.user[i].glyphs.len();
        if ui.selectable_label(app.ps.x.font_sel == Some(i), format!("{n}  ({count} glyphs)")).clicked() {
            app.ps.x.font_sel = Some(i);
        }
    }
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.ps.x.font_new_name);
        if ui.button("New font").clicked() {
            let name = app.ps.x.font_new_name.trim().to_string();
            if name.is_empty() || app.fonts.user.iter().any(|f| f.name == name) || app.fonts.families().contains(&name) {
                app.status = "Choose a unique font name".into();
            } else {
                let f = UserFont::new(&name);
                if let Err(e) = f.save() {
                    app.status = format!("Save failed: {e}");
                }
                app.fonts.reload_user_fonts();
                app.ps.x.font_sel = app.fonts.user.iter().position(|u| u.name == name);
            }
        }
    });
    let Some(fi) = app.ps.x.font_sel.filter(|i| *i < app.fonts.user.len()) else { return None };
    ui.separator();
    ui.label(RichText::new(&app.fonts.user[fi].name).strong());
    egui::Grid::new("fontcfg").num_columns(2).show(ui, |ui| {
        ui.label("Drawn at em size");
        len_field(ui, &mut app.ps.x.font_em, units).on_hover_text("Capital letters are about 0.7 × this tall");
        ui.end_row();
        ui.label("Side bearing");
        len_field(ui, &mut app.ps.x.font_bearing, units);
        ui.end_row();
    });
    ui.checkbox(&mut app.ps.x.font_baseline_bottom, "Baseline at the bottom of each letter")
        .on_hover_text("Untick for letters with descenders (g, j, p, q, y): the baseline is then the bottom of the letter's guide box - draw letters on a shared baseline and assign them together");
    let sel_paths: Vec<(u64, Vec<cutcraft_core::kurbo::BezPath>, Rect)> = app
        .sel
        .iter()
        .filter_map(|id| app.doc.page().shape(*id).map(|s| (s.id, s.world_paths(), s.bbox().unwrap_or(Rect::ZERO))))
        .collect();
    let (em, sb, base_bottom) = (app.ps.x.font_em, app.ps.x.font_bearing, app.ps.x.font_baseline_bottom);
    // common baseline of the selection when not per-letter
    let shared_base = sel_paths.iter().map(|s| s.2.y1).fold(f64::NEG_INFINITY, f64::max);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Key");
        ui.add(egui::TextEdit::singleline(&mut app.ps.x.font_char).desired_width(30.0).char_limit(1));
        if ui.add_enabled(!sel_paths.is_empty(), egui::Button::new("Assign selection")).clicked() {
            if let Some(ch) = app.ps.x.font_char.chars().next() {
                let paths: Vec<_> = sel_paths.iter().flat_map(|s| s.1.clone()).collect();
                let base = if base_bottom { None } else { Some(shared_base) };
                match app.fonts.user[fi].set_glyph(ch, &paths, em, base, sb) {
                    Ok(()) => changed = true,
                    Err(e) => app.status = e.to_string(),
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Left-to-right as");
        ui.text_edit_singleline(&mut app.ps.x.font_seq);
    });
    if ui.add_enabled(sel_paths.len() > 1, egui::Button::new("Assign selected shapes in order")).clicked() {
        let mut items = sel_paths.clone();
        // reading order: rows (by top) then left to right
        items.sort_by(|a, b| {
            let row = |r: &Rect| (r.y0 / (em * 0.8)).floor();
            row(&a.2).total_cmp(&row(&b.2)).then(a.2.x0.total_cmp(&b.2.x0))
        });
        let chars: Vec<char> = app.ps.x.font_seq.chars().filter(|c| !c.is_whitespace()).collect();
        let mut n = 0;
        for (it, ch) in items.iter().zip(chars) {
            let base = if base_bottom { None } else { Some(it.2.y1) };
            if app.fonts.user[fi].set_glyph(ch, &it.1, em, base, sb).is_ok() {
                n += 1;
            }
        }
        app.status = format!("Assigned {n} glyphs");
        changed = true;
    }
    // glyph overview
    let glyph_list: Vec<char> = app.fonts.user[fi].glyphs.keys().copied().collect();
    ui.label(format!("Glyphs: {}", glyph_list.iter().collect::<String>()));
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for c in &glyph_list {
            if ui.small_button(format!("{c} ✖")).on_hover_text("Remove glyph").clicked() {
                remove = Some(*c);
            }
        }
    });
    if let Some(c) = remove {
        app.fonts.user[fi].glyphs.remove(&c);
        changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("Space width");
        ui.add(DragValue::new(&mut app.fonts.user[fi].space_advance).range(50.0..=1000.0).suffix(" /1000 em"));
    });
    if changed {
        let f = app.fonts.user[fi].clone();
        if let Err(e) = f.save() {
            app.status = format!("Save failed: {e}");
        }
        let name = f.name.clone();
        app.fonts.reload_user_fonts();
        app.ps.x.font_sel = app.fonts.user.iter().position(|u| u.name == name);
        relayout_family(app, &name);
    }
    ui.horizontal(|ui| {
        if ui.button("Type with this font").clicked() {
            app.ps.text.family = app.fonts.user[fi].name.clone();
            app.panel = crate::panels::Panel::Text;
        }
        if ui.button("Export .ttf…").clicked() {
            let f = &app.fonts.user[fi];
            if let Some(p) = rfd::FileDialog::new().add_filter("TrueType font", &["ttf"]).set_file_name(format!("{}.ttf", f.name)).save_file() {
                app.status = match f.export_ttf(&p) {
                    Ok(()) => format!("Font exported to {} - double-click it to install", p.display()),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
        }
        if ui.button("Delete font").clicked() && crate::app::confirm("Delete this font?") {
            let _ = app.fonts.user[fi].delete();
            app.fonts.reload_user_fonts();
            app.ps.x.font_sel = None;
        }
    });
    None
}

fn relayout_family(app: &mut App, family: &str) {
    let ids: Vec<u64> = app
        .doc
        .page()
        .shapes
        .iter()
        .filter(|s| matches!(&s.kind, ShapeKind::Text { data, .. } if data.family == family))
        .map(|s| s.id)
        .collect();
    for id in ids {
        app.relayout_text(id);
    }
}

// ------------------------------------------------------------------ spelling

/// Spelling section shown inside the Text panel for the selected text shape.
pub fn spelling_section(app: &mut App, ui: &mut Ui, text_id: u64) {
    egui::CollapsingHeader::new("Spelling").default_open(true).show(ui, |ui| {
        if app.ps.x.dicts.is_none() {
            app.ps.x.dicts = Some(spell::find_dictionaries());
            if app.ps.x.speller.is_none() {
                let pref = app.prefs.spell_dictionary.clone();
                let pick = app.ps.x.dicts.as_ref().unwrap().iter().find(|p| p.to_string_lossy() == pref).cloned().or_else(|| {
                    app.ps.x.dicts.as_ref().unwrap().iter().find(|p| p.file_stem().map(|s| s.to_string_lossy().starts_with("en_US")).unwrap_or(false)).cloned()
                });
                if let Some(p) = pick {
                    load_dict(app, &p);
                }
            }
        }
        let current = app.ps.x.speller.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "none".into());
        let dicts = app.ps.x.dicts.clone().unwrap_or_default();
        let mut choose = None;
        egui::ComboBox::from_label("Dictionary").selected_text(&current).show_ui(ui, |ui| {
            for d in &dicts {
                let n = d.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if ui.selectable_label(n == current, &n).on_hover_text(d.display().to_string()).clicked() {
                    choose = Some(d.clone());
                }
            }
            if ui.button("Browse for .aff file…").clicked() {
                choose = rfd::FileDialog::new().add_filter("Hunspell", &["aff"]).pick_file();
            }
        });
        if let Some(p) = choose {
            load_dict(app, &p);
        }
        let Some(sp) = &app.ps.x.speller else {
            ui.small(format!(
                "No dictionary found. Install LibreOffice, or copy a Hunspell .aff/.dic pair into {}",
                cutcraft_core::prefs::config_dir().join("dictionaries").display()
            ));
            return;
        };
        let Some(ShapeKind::Text { data, .. }) = app.doc.page().shape(text_id).map(|s| &s.kind) else { return };
        let text = data.text.clone();
        let errs = sp.check_text(&text);
        if errs.is_empty() {
            ui.colored_label(Color32::LIGHT_GREEN, "No spelling mistakes");
            return;
        }
        let mut replace: Option<(usize, usize, String)> = None;
        let mut add_word = None;
        for e in &errs {
            ui.horizontal_wrapped(|ui| {
                if ui.link(RichText::new(&e.word).color(Color32::LIGHT_RED)).on_hover_text("Show suggestions").clicked() {
                    app.ps.x.spell_word = Some((e.word.clone(), sp.suggest(&e.word)));
                }
                if let Some((w, sugg)) = &app.ps.x.spell_word {
                    if *w == e.word {
                        for s in sugg {
                            if ui.small_button(s).clicked() {
                                replace = Some((e.start, e.end, s.clone()));
                            }
                        }
                        if sugg.is_empty() {
                            ui.small("no suggestions");
                        }
                    }
                }
                if ui.small_button("+ dictionary").clicked() {
                    add_word = Some(e.word.clone());
                }
            });
        }
        if let Some((a, b, s)) = replace {
            app.checkpoint();
            if let Some(ShapeKind::Text { data, .. }) = app.doc.page_mut().shape_mut(text_id).map(|s| &mut s.kind) {
                data.text.replace_range(a..b, &s);
            }
            app.relayout_text(text_id);
            app.ps.x.spell_word = None;
        }
        if let Some(w) = add_word {
            let lw = w.to_lowercase();
            if let Some(sp) = app.ps.x.speller.as_mut() {
                sp.custom.insert(lw.clone());
            }
            if !app.prefs.custom_words.contains(&lw) {
                app.prefs.custom_words.push(lw);
                let _ = app.prefs.save();
            }
        }
    });
}

fn load_dict(app: &mut App, p: &std::path::Path) {
    match Speller::load(p) {
        Ok(mut s) => {
            s.custom.extend(app.prefs.custom_words.iter().cloned());
            app.prefs.spell_dictionary = p.to_string_lossy().to_string();
            let _ = app.prefs.save();
            app.ps.x.speller = Some(s);
        }
        Err(e) => app.status = format!("Dictionary error: {e}"),
    }
}

// ------------------------------------------------------------------ bluetooth

/// Bluetooth controls inside the Send panel.
pub fn bluetooth_section(app: &mut App, ui: &mut Ui) {
    let x = &mut app.ps.x;
    let scanning = x.ble_scan.is_some();
    let finished = x.ble_scan.as_ref().and_then(|slot| slot.lock().unwrap().take());
    if x.ble_scan.is_some() {
        if let Some(res) = finished {
            match res {
                Ok(d) => {
                    app.ps.device_info = format!("{} Bluetooth device(s) found", d.len());
                    x.ble_devices = d;
                }
                Err(e) => app.ps.device_info = format!("Bluetooth: {e}"),
            }
            x.ble_scan = None;
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(!scanning, egui::Button::new(if scanning { "Scanning…" } else { "⟳ Scan (5 s)" })).clicked() {
            let slot = Arc::new(Mutex::new(None));
            let s2 = slot.clone();
            std::thread::spawn(move || {
                let r = ble::scan(5).map_err(|e| e.to_string());
                *s2.lock().unwrap() = Some(r);
            });
            x.ble_scan = Some(slot);
        }
    });
    if !app.ps.device_info.is_empty() {
        ui.add(egui::Label::new(&app.ps.device_info).wrap());
    }
    if !x.ble_devices.is_empty() {
        let idx = x.ble_idx.min(x.ble_devices.len() - 1);
        let label = |d: &ble::BleDevice| format!("{}{}{}", d.name, if d.likely_cutter { " ✂" } else { "" }, d.rssi.map(|r| format!("  ({r} dBm)")).unwrap_or_default());
        egui::ComboBox::from_id_salt("ble").selected_text(label(&x.ble_devices[idx])).width(260.0).show_ui(ui, |ui| {
            for (i, d) in x.ble_devices.iter().enumerate() {
                ui.selectable_value(&mut x.ble_idx, i, label(d));
            }
        });
    }
    egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Packet size");
            ui.add(DragValue::new(&mut x.ble_chunk).range(20..=512).suffix(" bytes"));
        });
        ui.add(egui::TextEdit::singleline(&mut x.ble_write_uuid).hint_text("write characteristic UUID (auto)"));
        ui.add(egui::TextEdit::singleline(&mut x.ble_notify_uuid).hint_text("notify characteristic UUID (auto)"));
    });
}

/// Builder for a Bluetooth transport that connects on the worker thread.
pub fn bluetooth_opener(app: &App) -> anyhow::Result<Box<dyn FnOnce() -> anyhow::Result<Box<dyn cutcraft_core::device::Transport>> + Send>> {
    let x = &app.ps.x;
    let dev = x.ble_devices.get(x.ble_idx).cloned().ok_or_else(|| anyhow::anyhow!("Scan for Bluetooth devices and pick your cutter"))?;
    let parse = |s: &str| -> anyhow::Result<Option<uuid::Uuid>> { if s.trim().is_empty() { Ok(None) } else { Ok(Some(uuid::Uuid::parse_str(s.trim())?)) } };
    let (w, n, chunk) = (parse(&x.ble_write_uuid)?, parse(&x.ble_notify_uuid)?, x.ble_chunk);
    Ok(Box::new(move || {
        let t = ble::BleTransport::connect(&dev, w, n, chunk)?;
        Ok(Box::new(t) as Box<dyn cutcraft_core::device::Transport>)
    }))
}

// ------------------------------------------------------------------ shared helpers

fn sel_world(app: &App, sel: &[ShapeId]) -> Vec<BezPath> {
    sel.iter().filter_map(|id| app.doc.page().shape(*id)).flat_map(|s| s.world_paths()).collect()
}

/// Replace the whole selection with one new path shape (keeps the first shape's style/layer).
fn replace_sel(app: &mut App, result: BezPath) {
    if result.elements().is_empty() {
        app.status = "The operation produced nothing".into();
        return;
    }
    app.edit(move |doc, sel| {
        let (style, layer) =
            sel.first().and_then(|id| doc.page().shape(*id)).map(|s| (s.style.clone(), s.layer)).unwrap_or_default();
        doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
        let id = doc.add_path(result, style, layer);
        Some(vec![id])
    });
}

/// Add a new shape directly behind the current selection (for shadows), keeping the selection.
fn add_behind(app: &mut App, path: BezPath, style: Style, name: &str) {
    if path.elements().is_empty() {
        return;
    }
    let name = name.to_string();
    app.edit(move |doc, sel| {
        let layer = sel.first().and_then(|id| doc.page().shape(*id)).map(|s| s.layer).unwrap_or(0);
        let idx = doc.page().shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
        let id = doc.alloc_id();
        let mut sh = Shape::new_path(id, path, style);
        sh.layer = layer;
        sh.name = name;
        doc.page_mut().shapes.insert(idx, sh);
        Some(sel.to_vec())
    });
}

/// Add a new shape on top and select it (for hull, line fill, pierce).
fn add_new(app: &mut App, path: BezPath, style: Style, name: &str) {
    if path.elements().is_empty() {
        app.status = "The operation produced nothing".into();
        return;
    }
    let name = name.to_string();
    app.edit(move |doc, sel| {
        let layer = sel.first().and_then(|id| doc.page().shape(*id)).map(|s| s.layer).unwrap_or(0);
        let id = doc.alloc_id();
        let mut sh = Shape::new_path(id, path, style);
        sh.layer = layer;
        sh.name = name;
        doc.page_mut().shapes.push(sh);
        Some(vec![id])
    });
}

// ------------------------------------------------------------------ image tools

/// The selected image's id, if exactly one image is part of the selection.
fn selected_image(app: &App) -> Option<ShapeId> {
    app.sel.iter().copied().find(|id| app.doc.page().shape(*id).map(|s| s.is_image()).unwrap_or(false))
}

/// The image decoded to gray (long side ≤ `max` px) with the panel's adjustments
/// applied, plus its size in mm and world transform.
fn adjusted_gray(app: &App, id: ShapeId, max: usize) -> Option<(cutcraft_core::raster::Gray, f64, f64, Affine)> {
    let s = app.doc.page().shape(id)?;
    let ShapeKind::Image(img) = &s.kind else { return None };
    let mut g = cutcraft_core::raster::Gray::decode(&img.png).ok()?.fit(max);
    g.adjust(&app.ps.x.img_adj);
    Some((g, img.width_mm, img.height_mm, s.transform))
}

pub fn image_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    use cutcraft_core::raster::{self, Effect, LineStyle};
    let units = app.prefs.units;
    ui.heading("Image");
    let Some(id) = selected_image(app) else {
        ui.label("Select an image (File ▸ Import or drag one onto the window).");
        return None;
    };

    // ---- adjustments (non-destructive: they feed the tools below)
    egui::CollapsingHeader::new("Adjust").default_open(true).show(ui, |ui| {
        let a = &mut app.ps.x.img_adj;
        egui::Grid::new("imgadj").num_columns(2).show(ui, |ui| {
            ui.label("Brightness");
            ui.add(egui::Slider::new(&mut a.brightness, -1.0..=1.0));
            ui.end_row();
            ui.label("Contrast");
            ui.add(egui::Slider::new(&mut a.contrast, -1.0..=1.0));
            ui.end_row();
            ui.label("Gamma");
            ui.add(egui::Slider::new(&mut a.gamma, 0.2..=3.0));
            ui.end_row();
            ui.label("Sharpen");
            ui.add(egui::Slider::new(&mut a.sharpen, 0.0..=2.0));
            ui.end_row();
            ui.label("Remove background");
            ui.add(egui::Slider::new(&mut a.remove_background, 0.0..=0.5));
            ui.end_row();
            ui.label("Effect");
            egui::ComboBox::from_id_salt("imgfx")
                .selected_text(Effect::ALL.iter().find(|(e, _)| *e == a.effect).map(|(_, n)| *n).unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (e, n) in Effect::ALL {
                        ui.selectable_value(&mut a.effect, e, n);
                    }
                });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut a.auto_level, "Auto levels");
            ui.checkbox(&mut a.invert, "Invert");
            if ui.button("Reset").clicked() {
                *a = Default::default();
            }
        });

        // Live preview from a cached small thumbnail.
        if app.ps.x.img_base.as_ref().map(|(i, _)| *i != id).unwrap_or(true) {
            app.ps.x.img_base = app.doc.page().shape(id).and_then(|s| match &s.kind {
                ShapeKind::Image(img) => raster::Gray::decode(&img.png).ok().map(|g| (id, g.fit(240))),
                _ => None,
            });
            app.ps.x.img_prev = None;
        }
        let adj = app.ps.x.img_adj;
        let stale = app.ps.x.img_prev.as_ref().map(|(i, a, _)| *i != id || *a != adj).unwrap_or(true);
        if stale {
            if let Some((_, base)) = &app.ps.x.img_base {
                let mut g = base.clone();
                g.adjust(&adj);
                let rgba = g.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                app.ps.x.img_prev = Some((id, adj, ui.ctx().load_texture("img-adjust", ci, egui::TextureOptions::LINEAR)));
            }
        }
        if let Some((_, _, tex)) = &app.ps.x.img_prev {
            let s = tex.size_vec2();
            let k = (220.0 / s.x.max(s.y)).min(1.0);
            ui.image((tex.id(), s * k));
        }
        ui.small("Adjustments feed Photo → lines and Shadow box below (the image itself is untouched).");
        if ui.button("Bake into image (grayscale)").on_hover_text("Rewrite the picture with these adjustments, e.g. before Trace. Undoable.").clicked() {
            if let Some((g, ..)) = adjusted_gray(app, id, 4000) {
                let mut png = std::io::Cursor::new(Vec::new());
                if g.to_rgba8().write_to(&mut png, image::ImageFormat::Png).is_ok() {
                    app.checkpoint();
                    if let Some(s) = app.doc.page_mut().shape_mut(id) {
                        if let ShapeKind::Image(img) = &mut s.kind {
                            img.png = Arc::new(png.into_inner());
                            img.px_w = g.width as u32;
                            img.px_h = g.height as u32;
                        }
                    }
                    app.ps.x.img_adj = Default::default();
                    app.ps.x.img_base = None;
                    app.status = "Adjustments baked into the image".into();
                }
            }
        }
    });

    // ---- photo to lines (for sketch pens)
    egui::CollapsingHeader::new("Photo → lines (for pens)").show(ui, |ui| {
        let o = &mut app.ps.x.img_lines;
        egui::ComboBox::from_label("Style")
            .selected_text(LineStyle::ALL.iter().find(|(s, _)| *s == o.style).map(|(_, n)| *n).unwrap_or(""))
            .show_ui(ui, |ui| {
                for (s, n) in LineStyle::ALL {
                    ui.selectable_value(&mut o.style, s, n);
                }
            });
        egui::Grid::new("imglines").num_columns(2).show(ui, |ui| {
            ui.label("Spacing");
            len_field(ui, &mut o.spacing, units);
            ui.end_row();
            ui.label("Strength");
            ui.add(egui::Slider::new(&mut o.strength, 0.0..=1.0));
            ui.end_row();
            if o.style == LineStyle::Stipple {
                ui.label("Dot size");
                len_field(ui, &mut o.dot, units);
                ui.end_row();
            }
        });
        if o.style == LineStyle::Stipple && ui.button("Shuffle dots").clicked() {
            o.seed = o.seed.wrapping_add(1);
        }
        if ui.button("Create lines").clicked() {
            let opts = app.ps.x.img_lines;
            if let Some((g, w, h, tr)) = adjusted_gray(app, id, 1600) {
                let mut p = raster::lines_to_path(&raster::photo_lines(&g, w, h, &opts));
                if p.elements().is_empty() {
                    app.status = "The picture is too light to draw - try more contrast".into();
                } else {
                    p.apply_affine(tr);
                    let layer = app.current_layer();
                    let style = Style { fill: None, stroke: Some(Rgba(30, 30, 160, 255)), ..Style::default() };
                    app.edit(move |doc, _| Some(vec![doc.add_path(p, style, layer)]));
                    app.status = "Line art created - set its colour to a Sketch/pen action in Send".into();
                }
            }
        }
    });

    // ---- shadow box (layered papercut)
    egui::CollapsingHeader::new("Shadow box (layered papercut)").show(ui, |ui| {
        ui.small("Splits the picture into stacked plates by brightness: the front has the biggest opening, the back is solid. Cut from card, stack with spacers and light from behind.");
        egui::Grid::new("imgsbox").num_columns(2).show(ui, |ui| {
            ui.label("Plates");
            ui.add(egui::DragValue::new(&mut app.ps.x.img_plates).range(2..=8));
            ui.end_row();
            ui.label("Frame border");
            len_field(ui, &mut app.ps.x.img_border, units);
            ui.end_row();
            ui.label("Smoothing");
            ui.add(egui::Slider::new(&mut app.ps.x.img_smooth, 0.3..=5.0));
            ui.end_row();
        });
        if ui.button("Create shadow box").clicked() {
            let (n, border, smooth) = (app.ps.x.img_plates, app.ps.x.img_border, app.ps.x.img_smooth);
            let bb = app.doc.page().shape(id).and_then(|s| s.bbox());
            if let (Some((g, w, h, tr)), Some(bb)) = (adjusted_gray(app, id, 1600), bb) {
                let plates = raster::shadow_box_plates(&g, w, h, n, border, smooth);
                let step = bb.width() + 2.0 * border + 10.0;
                let layer = app.current_layer();
                app.edit(move |doc, _| {
                    let ids: Vec<ShapeId> = plates
                        .into_iter()
                        .enumerate()
                        .map(|(k, mut p)| {
                            p.apply_affine(tr);
                            p.apply_affine(Affine::translate((step * (k + 1) as f64, 0.0)));
                            doc.add_path(p, Style::default(), layer)
                        })
                        .collect();
                    Some(ops::group(doc, &ids))
                });
                app.status = format!("{n} shadow-box plates created beside the picture, front to back");
            }
        }
    });
    None
}

// ------------------------------------------------------------------ art

const THUMB: f32 = 64.0;

/// A clickable preview tile (outline thumbnail + label). Ported from BurnCraft.
pub(crate) fn art_tile(ui: &mut Ui, label: &str, path: &BezPath) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(THUMB, THUMB + 14.0), egui::Sense::click());
    let vis = ui.visuals();
    let bg = if response.hovered() { vis.widgets.hovered.weak_bg_fill } else { vis.extreme_bg_color };
    let image = egui::Rect::from_min_size(rect.min, egui::vec2(THUMB, THUMB));
    ui.painter().rect_filled(image, 4.0, bg);
    let ink = vis.text_color();
    let inner = image.shrink(7.0);
    for l in cutcraft_core::library::thumbnail(path) {
        let pts: Vec<egui::Pos2> = l.iter().map(|p| inner.min + egui::vec2(p.x as f32, p.y as f32) * inner.width()).collect();
        ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.2, ink)));
    }
    let name: String = if label.chars().count() > 11 { label.chars().take(10).chain(['…']).collect() } else { label.to_string() };
    ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 6.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(10.0), ink);
    response.on_hover_text(label)
}

/// Add a path centred on the page and select it.
const TEMPLATE_LAYER: &str = "Template";

/// Puts a product outline (and its dashed design area) on a locked
/// "Template" layer at the page centre — a guide, never cut or printed.
fn add_product(app: &mut App, name: &'static str, (outline, area): (BezPath, Rect)) {
    app.checkpoint_named("Product template");
    let page = app.doc.page_mut();
    let layer = match page.layers.iter().find(|l| l.name == TEMPLATE_LAYER) {
        Some(l) => l.id,
        None => {
            let id = page.layers.iter().map(|l| l.id).max().unwrap_or(0) + 1;
            // at the bottom, so artwork draws over it
            page.layers.insert(0, cutcraft_core::doc::Layer { id, name: TEMPLATE_LAYER.into(), visible: true, locked: true, color: Rgba(150, 150, 150, 255) });
            id
        }
    };
    let b = cutcraft_core::kurbo::Shape::bounding_box(&outline);
    let c = page.rect().center();
    let at = Affine::translate((c.x - b.center().x, c.y - b.center().y));
    let guide = Style { fill: None, stroke: Some(Rgba(150, 150, 150, 255)), stroke_width: 0.5, cut: CutStyle::NoCut, no_print: true };
    let soft = Style { stroke: Some(Rgba(120, 170, 230, 255)), stroke_width: 0.3, ..guide.clone() };
    let id = app.doc.add_path(at * outline, guide, layer);
    let dashes = geom::dash(&geom::flatten_all(&[geom::rect_path(area)], 0.1), 4.0, 3.0);
    let dash_path = geom::merge_paths(&dashes.iter().map(|pl| geom::polyline_path(&pl.pts, false)).collect::<Vec<_>>());
    let did = app.doc.add_path(at * dash_path, soft, layer);
    // templates sit underneath everything
    let page = app.doc.page_mut();
    for (k, sid) in [id, did].into_iter().enumerate() {
        if let Some(pos) = page.shapes.iter().position(|s| s.id == sid) {
            let s = page.shapes.remove(pos);
            page.shapes.insert(k, s);
        }
    }
    if let Some(s) = page.shape_mut(id) {
        s.name = format!("{name} template");
    }
    app.status = format!("{name} template added on the locked “Template” layer - the dashed box is the usual design area");
}

fn add_centred(app: &mut App, path: BezPath, name: impl Into<String>) {
    let name: String = name.into();
    let c = app.doc.page().rect().center();
    let mut p = path;
    p.apply_affine(Affine::translate(c.to_vec2()));
    let layer = app.current_layer();
    app.edit(move |doc, _| {
        let id = doc.add_path(p, Style::default(), layer);
        if let Some(s) = doc.page_mut().shape_mut(id) {
            s.name = name;
        }
        Some(vec![id])
    });
}

pub fn art_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    use cutcraft_core::library::{self, MonogramFrame, SignShape, TEMPLATES};
    let units = app.prefs.units;
    ui.heading("Art");

    egui::CollapsingHeader::new("Shapes").default_open(true).show(ui, |ui| {
        use cutcraft_core::shapelib;
        ui.horizontal(|ui| {
            ui.label("Size");
            len_field(ui, &mut app.ps.x.art_size, units);
        })
        .response
        .on_hover_text("The longer side of the shape you add");
        let size = app.ps.x.art_size.max(1.0);
        let mut picked = None;
        for cat in shapelib::CATEGORIES {
            egui::CollapsingHeader::new(cat).id_salt(("shape-cat", cat)).default_open(cat == "Square").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for s in shapelib::shapes().iter().filter(|s| s.category == cat) {
                        if art_tile(ui, s.name, &s.unit).clicked() {
                            picked = Some(s);
                        }
                    }
                });
            });
        }
        if let Some(s) = picked {
            let name = if s.name == s.category || s.category == "Others" { s.name.to_string() } else { format!("{} {}", s.name, s.category.to_lowercase()) };
            add_centred(app, s.path(size), name);
        }
    });

    egui::CollapsingHeader::new("Borders").show(ui, |ui| {
        use cutcraft_core::shapelib::{self, BORDERS};
        ui.horizontal(|ui| {
            ui.label("Length");
            len_field(ui, &mut app.ps.x.border_len, units);
            ui.label("Height");
            len_field(ui, &mut app.ps.x.border_h, units);
        });
        ui.small("Strips for card edges, frames and shelf trims - the pattern repeats to fill the length.");
        let (l, h) = (app.ps.x.border_len.max(5.0), app.ps.x.border_h.max(2.0));
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for b in &BORDERS {
                if art_tile(ui, b.name, shapelib::border_preview(b)).clicked() {
                    picked = Some(b);
                }
            }
        });
        if let Some(b) = picked {
            let p = Affine::translate((-l / 2.0, -h / 2.0)) * (b.build)(l, h);
            add_centred(app, p, format!("{} border", b.name));
        }
    });

    egui::CollapsingHeader::new("Templates").show(ui, |ui| {
        ui.small("Ready-made blanks at true size, added at the centre of the page.");
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in TEMPLATES.iter().enumerate() {
                if art_tile(ui, t.name, &(t.build)()).clicked() {
                    picked = Some(i);
                }
            }
        });
        if let Some(i) = picked {
            add_centred(app, (TEMPLATES[i].build)(), TEMPLATES[i].name);
        }
    });

    egui::CollapsingHeader::new("Product templates").show(ui, |ui| {
        use cutcraft_core::library::PRODUCTS;
        ui.small("A product outline to size your design on - shown on a locked “Template” layer, never cut or printed.");
        let prod = app.ps.product.min(PRODUCTS.len() - 1);
        egui::ComboBox::from_label("Product").selected_text(PRODUCTS[prod].name).show_ui(ui, |ui| {
            for (i, p) in PRODUCTS.iter().enumerate() {
                if ui.selectable_value(&mut app.ps.product, i, p.name).clicked() {
                    app.ps.product_size = 0;
                }
            }
        });
        let prod = app.ps.product.min(PRODUCTS.len() - 1);
        let sizes = PRODUCTS[prod].sizes;
        let si = app.ps.product_size.min(sizes.len() - 1);
        egui::ComboBox::from_label("Size").selected_text(format!("{}  ({:.0} × {:.0} mm)", sizes[si].0, sizes[si].1, sizes[si].2)).show_ui(ui, |ui| {
            for (i, (n, w, h)) in sizes.iter().enumerate() {
                ui.selectable_value(&mut app.ps.product_size, i, format!("{n}  ({w:.0} × {h:.0} mm)"));
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Add template").clicked() {
                let (_, w, h) = sizes[si];
                add_product(app, PRODUCTS[prod].name, (PRODUCTS[prod].build)(w, h));
            }
            if ui.button("Remove templates").clicked() {
                app.checkpoint_named("Remove templates");
                let p = app.doc.page_mut();
                let ids: Vec<u32> = p.layers.iter().filter(|l| l.name == TEMPLATE_LAYER).map(|l| l.id).collect();
                p.shapes.retain(|s| !ids.contains(&s.layer));
                if p.layers.len() > ids.len() {
                    p.layers.retain(|l| !ids.contains(&l.id));
                }
            }
        });
    });

    egui::CollapsingHeader::new("Sign / plaque").show(ui, |ui| {
        let x = &mut app.ps.x;
        egui::ComboBox::from_label("Shape").selected_text(SignShape::ALL[x.sign_shape.min(3)].1).show_ui(ui, |ui| {
            for (i, (_, n)) in SignShape::ALL.iter().enumerate() {
                ui.selectable_value(&mut x.sign_shape, i, *n);
            }
        });
        egui::Grid::new("sign").num_columns(2).show(ui, |ui| {
            ui.label("Width");
            len_field(ui, &mut x.sign_w, units);
            ui.end_row();
            ui.label("Height");
            len_field(ui, &mut x.sign_h, units);
            ui.end_row();
            ui.label("Corner");
            len_field(ui, &mut x.sign_corner, units);
            ui.end_row();
            ui.label("Mounting holes");
            ui.add(egui::DragValue::new(&mut x.sign_holes).range(0..=4));
            ui.end_row();
            if x.sign_holes > 0 {
                ui.label("Hole diameter");
                len_field(ui, &mut x.sign_hole_d, units);
                ui.end_row();
                ui.label("Hole inset");
                len_field(ui, &mut x.sign_inset, units);
                ui.end_row();
            }
        });
        if ui.button("Add sign").clicked() {
            let shape = SignShape::ALL[x.sign_shape.min(3)].0;
            let mut p = library::sign_outline(shape, x.sign_w, x.sign_h, x.sign_corner);
            p.extend(library::mounting_holes(x.sign_holes, x.sign_w, x.sign_h, x.sign_hole_d, x.sign_inset, shape == SignShape::Oval).elements().iter().copied());
            add_centred(app, p, "Sign");
        }
    });

    egui::CollapsingHeader::new("Monogram").show(ui, |ui| {
        ui.small("Large surname initial in the middle, first and middle initials either side.");
        egui::Grid::new("mono").num_columns(2).show(ui, |ui| {
            for (i, l) in ["First initial", "Surname initial", "Middle initial"].into_iter().enumerate() {
                ui.label(l);
                ui.add(egui::TextEdit::singleline(&mut app.ps.x.mono[i]).desired_width(40.0).char_limit(2));
                ui.end_row();
            }
            ui.label("Font");
            egui::ComboBox::from_id_salt("monofont").selected_text(&app.ps.x.mono_family).width(170.0).show_ui(ui, |ui| {
                for f in app.fonts.families() {
                    ui.selectable_value(&mut app.ps.x.mono_family, f.clone(), f);
                }
            });
            ui.end_row();
            ui.label("Size");
            len_field(ui, &mut app.ps.x.mono_size, units);
            ui.end_row();
            ui.label("Frame");
            egui::ComboBox::from_id_salt("monoframe").selected_text(MonogramFrame::ALL[app.ps.x.mono_frame.min(3)].1).show_ui(ui, |ui| {
                for (i, (_, n)) in MonogramFrame::ALL.iter().enumerate() {
                    ui.selectable_value(&mut app.ps.x.mono_frame, i, *n);
                }
            });
            ui.end_row();
        });
        if ui.button("Add monogram").clicked() {
            let m = &app.ps.x.mono;
            let frame = MonogramFrame::ALL[app.ps.x.mono_frame.min(3)].0;
            match library::monogram(&app.fonts, [&m[0], &m[1], &m[2]], &app.ps.x.mono_family, app.ps.x.mono_size, frame) {
                Some(shapes) => {
                    let c = app.doc.page().rect().center();
                    let layer = app.current_layer();
                    app.edit(move |doc, _| {
                        let ids: Vec<ShapeId> = shapes
                            .into_iter()
                            .map(|mut s| {
                                s.transform = Affine::translate(c.to_vec2()) * s.transform;
                                doc.add_shape(s, layer)
                            })
                            .collect();
                        Some(ops::group(doc, &ids))
                    });
                    app.status = "Monogram added at the centre of the page".into();
                }
                None => app.status = "Enter at least the surname initial".into(),
            }
        }
    });
    None
}

// ------------------------------------------------------------------ script

pub fn script_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Script");
    ui.small("Generate artwork with a Rhai script - loops, maths and geometry builders.");
    ui.add(
        egui::TextEdit::multiline(&mut app.ps.x.script_src)
            .code_editor()
            .desired_rows(16)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        if ui.button("▶ Run").clicked() {
            let src = app.ps.x.script_src.clone();
            app.checkpoint();
            let layer = app.current_layer();
            match cutcraft_core::script::run(&mut app.doc, &src, layer) {
                Ok(n) => {
                    app.ps.x.script_msg = format!("Created {n} shape(s).");
                    app.status = app.ps.x.script_msg.clone();
                }
                Err(e) => {
                    app.undo.pop(); // run() didn't modify the doc; drop the redundant checkpoint
                    app.ps.x.script_msg = e.to_string();
                }
            }
        }
        if ui.button("Reset example").clicked() {
            app.ps.x.script_src = DEFAULT_SCRIPT.into();
            app.ps.x.script_msg.clear();
        }
    });
    if !app.ps.x.script_msg.is_empty() {
        ui.label(&app.ps.x.script_msg);
    }
    None
}

// ------------------------------------------------------------------ dieline

pub fn dieline_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    use cutcraft_core::dieline::{self, DielineKind};
    let units = app.prefs.units;
    ui.heading("Dieline / packaging");
    ui.small("Builds a foldable net: red cut lines + blue fold/score lines.");
    let ki = app.ps.x.die_kind.min(DielineKind::ALL.len() - 1);
    egui::ComboBox::from_label("Type").selected_text(DielineKind::ALL[ki].1).show_ui(ui, |ui| {
        for (i, (_, n)) in DielineKind::ALL.iter().enumerate() {
            ui.selectable_value(&mut app.ps.x.die_kind, i, *n);
        }
    });
    let kind = DielineKind::ALL[app.ps.x.die_kind.min(DielineKind::ALL.len() - 1)].0;
    let d = &mut app.ps.x.die;
    egui::Grid::new("diegrid").num_columns(2).show(ui, |ui| {
        let (wl, hl) = match kind {
            DielineKind::Envelope => ("Pocket width", "Pocket height"),
            DielineKind::PillowBox => ("Width", "Length"),
            DielineKind::TuckBox => ("Width", "Height"),
        };
        ui.label(wl);
        len_field(ui, &mut d.w, units);
        ui.end_row();
        ui.label(hl);
        len_field(ui, &mut d.h, units);
        ui.end_row();
        if matches!(kind, DielineKind::TuckBox | DielineKind::PillowBox) {
            ui.label("Depth");
            len_field(ui, &mut d.d, units);
            ui.end_row();
            ui.label("Glue tab");
            len_field(ui, &mut d.glue, units);
            ui.end_row();
        }
        if !matches!(kind, DielineKind::PillowBox) {
            ui.label(if matches!(kind, DielineKind::Envelope) { "Seal flap" } else { "Tuck flap" });
            len_field(ui, &mut d.tuck, units);
            ui.end_row();
        }
    });
    if ui.button("Create dieline").clicked() {
        let g = dieline::generate(kind, &app.ps.x.die);
        let (pw, ph) = (app.doc.page().width, app.doc.page().height);
        let off = Vec2::new(((pw - g.size.0) / 2.0).max(5.0), ((ph - g.size.1) / 2.0).max(5.0));
        let tr = Affine::translate(off);
        let mut cut = g.cut.clone();
        cut.apply_affine(tr);
        let mut score = g.score.clone();
        score.apply_affine(tr);
        app.checkpoint();
        let layer = app.current_layer();
        let cut_style = Style { fill: None, stroke: Some(Rgba::RED), cut: CutStyle::Cut, ..Style::default() };
        let score_style =
            Style { fill: None, stroke: Some(Rgba(30, 90, 220, 255)), cut: CutStyle::Perforate { on: 2.0, off: 2.0 }, ..Style::default() };
        app.doc.add_path(cut, cut_style, layer);
        app.doc.add_path(score, score_style, layer);
        app.status = format!("Created {} dieline", DielineKind::ALL[app.ps.x.die_kind.min(DielineKind::ALL.len() - 1)].1);
    }
    None
}

// ------------------------------------------------------------------ live effect stack

/// A re-editable, non-destructive stack of path effects on a single shape.
fn effectstack_section(app: &mut App, ui: &mut Ui, id: ShapeId) {
    use cutcraft_core::effectstack::Effect;
    use cutcraft_core::fx::Warp;

    let Some(before) = app.doc.page().shape(id).map(|s| s.effects.clone()) else { return };
    let mut effects = before.clone();

    egui::CollapsingHeader::new(format!("Live effects ({})", effects.len())).default_open(true).show(ui, |ui| {
        ui.small("Non-destructive - tweak, reorder or remove any time.");
        let (mut remove, mut up, mut down) = (None, None, None);
        let mut started = false; // a drag started this frame → snapshot for undo
        for (i, e) in effects.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                match e {
                    Effect::Offset { dist, round } => {
                        ui.label("Offset");
                        started |= ui.add(DragValue::new(dist).speed(0.1).suffix(" mm")).drag_started();
                        ui.checkbox(round, "round");
                    }
                    Effect::RoundCorners { radius } => {
                        ui.label("Round");
                        started |= ui.add(DragValue::new(radius).speed(0.1).range(0.0..=1e4).suffix(" mm")).drag_started();
                    }
                    Effect::Warp { kind, amount, k2 } => {
                        egui::ComboBox::from_id_salt(("fxw", i)).selected_text(kind.name()).width(96.0).show_ui(ui, |ui| {
                            for (w, n, _) in Warp::ALL {
                                ui.selectable_value(kind, w, n);
                            }
                        });
                        started |= ui.add(DragValue::new(amount).speed(0.01)).drag_started();
                        started |= ui.add(DragValue::new(k2).speed(0.01)).drag_started();
                    }
                    Effect::Simplify { tol } => {
                        ui.label("Simplify");
                        started |= ui.add(DragValue::new(tol).speed(0.01).range(0.0..=50.0)).drag_started();
                    }
                    Effect::Smooth { strength } => {
                        ui.label("Smooth");
                        started |= ui.add(DragValue::new(strength).speed(0.01).range(0.0..=50.0)).drag_started();
                    }
                    Effect::Unite => {
                        ui.label("Unite (parts welded)");
                    }
                    Effect::HideContours { hidden } => {
                        ui.label(format!("Contour: {} line(s) hidden - edit in Style ▸ Fill & Line", hidden.len()));
                    }
                }
                if ui.small_button("↑").clicked() {
                    up = Some(i);
                }
                if ui.small_button("↓").clicked() {
                    down = Some(i);
                }
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
            });
        }
        let mut structural = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Add:");
            if ui.button("Offset").clicked() {
                effects.push(Effect::Offset { dist: 2.0, round: true });
                structural = true;
            }
            if ui.button("Round").clicked() {
                effects.push(Effect::RoundCorners { radius: 3.0 });
                structural = true;
            }
            if ui.button("Warp").clicked() {
                effects.push(Effect::Warp { kind: Warp::Arch, amount: 0.3, k2: 1.0 });
                structural = true;
            }
            if ui.button("Simplify").clicked() {
                effects.push(Effect::Simplify { tol: 0.2 });
                structural = true;
            }
            if ui.button("Smooth").clicked() {
                effects.push(Effect::Smooth { strength: 0.5 });
                structural = true;
            }
        });
        if let Some(i) = remove {
            effects.remove(i);
            structural = true;
        }
        if let Some(i) = up {
            if i > 0 {
                effects.swap(i, i - 1);
                structural = true;
            }
        }
        if let Some(i) = down {
            if i + 1 < effects.len() {
                effects.swap(i, i + 1);
                structural = true;
            }
        }
        let release = !effects.is_empty() && ui.button("Release to paths").clicked();

        // Commit: checkpoint on structural change or at the start of a drag.
        if (structural || started) && effects == before {
            app.checkpoint();
        }
        if effects != before {
            if structural || started {
                app.checkpoint();
            }
            if let Some(s) = app.doc.page_mut().shape_mut(id) {
                s.effects = effects;
            }
        }
        if release {
            app.checkpoint();
            if let Some(s) = app.doc.page_mut().shape_mut(id) {
                let baked = s.local_paths();
                s.effects.clear();
                s.kind = ShapeKind::Path(cutcraft_core::geom::merge_paths(&baked));
            }
            app.status = "Released effects to paths".into();
        }
    });
    ui.separator();
}

// ------------------------------------------------------------------ distort (fx)

pub fn distort_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    ui.heading("Distort");
    if sel.is_empty() {
        ui.label("Select artwork to distort. These bake the shapes into new paths (undoable).");
        return None;
    }
    if let Some(&id) = sel.first() {
        effectstack_section(app, ui, id);
    }
    ui.label("Envelope");
    let warp = FxWarp::ALL[app.ps.x.fx_warp.min(FxWarp::ALL.len() - 1)];
    egui::ComboBox::from_id_salt("fxwarp").selected_text(warp.1).width(180.0).show_ui(ui, |ui| {
        for (i, (_, name, tip)) in FxWarp::ALL.iter().enumerate() {
            ui.selectable_value(&mut app.ps.x.fx_warp, i, *name).on_hover_text(*tip);
        }
    });
    ui.small(warp.2);
    ui.horizontal(|ui| {
        ui.label("Amount");
        ui.add(egui::Slider::new(&mut app.ps.x.fx_amount, -1.0..=1.0));
    });
    let needs_k2 = matches!(warp.0, FxWarp::Wave | FxWarp::Flag | FxWarp::Perspective | FxWarp::TunaCan | FxWarp::TitleCrawl | FxWarp::Rotate3d);
    if needs_k2 {
        let (lo, hi, label) = match warp.0 {
            FxWarp::Wave | FxWarp::Flag => (0.25, 6.0, "Cycles"),
            FxWarp::Perspective => (0.1, 2.0, "Bottom scale"),
            FxWarp::TunaCan => (10.0, 180.0, "Wrap °"),
            FxWarp::TitleCrawl => (0.0, 3.0, "Depth"),
            FxWarp::Rotate3d => (0.8, 6.0, "Focal"),
            _ => (0.0, 1.0, "k"),
        };
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(egui::Slider::new(&mut app.ps.x.fx_k2, lo..=hi));
        });
    }
    if ui.button(RichText::new("Apply envelope").strong()).clicked() {
        let paths = sel_world(app, &sel);
        let result = warp.0.apply(&paths, app.ps.x.fx_amount, app.ps.x.fx_k2);
        replace_sel(app, result);
    }
    ui.separator();
    ui.heading("Drop shadow");
    ui.horizontal(|ui| {
        ui.label("Offset X");
        len_field(ui, &mut app.ps.x.shadow_dx, units);
        ui.label("Y");
        len_field(ui, &mut app.ps.x.shadow_dy, units);
    });
    if ui.button("Add drop shadow").clicked() {
        let paths = sel_world(app, &sel);
        let s = fx::drop_shadow(&paths, app.ps.x.shadow_dx, app.ps.x.shadow_dy);
        add_behind(app, s, shadow_style(), "Drop shadow");
    }
    ui.separator();
    ui.heading("Cast shadow");
    ui.horizontal(|ui| {
        ui.label("Lean");
        ui.add(egui::Slider::new(&mut app.ps.x.cast_skew, -3.0..=3.0));
        ui.label("Squash");
        ui.add(egui::Slider::new(&mut app.ps.x.cast_squash, 0.05..=1.0));
    });
    if ui.button("Add cast shadow").clicked() {
        let paths = sel_world(app, &sel);
        let s = fx::cast_shadow(&paths, app.ps.x.cast_skew, app.ps.x.cast_squash);
        add_behind(app, s, shadow_style(), "Cast shadow");
    }
    ui.separator();
    ui.heading("3D extrude");
    ui.horizontal(|ui| {
        ui.label("Depth X");
        len_field(ui, &mut app.ps.x.extrude_dx, units);
        ui.label("Y");
        len_field(ui, &mut app.ps.x.extrude_dy, units);
    });
    if ui.button("Add extrude").clicked() {
        let paths = sel_world(app, &sel);
        let v = fx::extrude(&paths, app.ps.x.extrude_dx, app.ps.x.extrude_dy, 160);
        add_behind(app, geom::merge_paths(&v), Style { fill: None, stroke: Some(Rgba(90, 90, 90, 255)), ..Style::default() }, "Extrude");
    }
    ui.separator();
    ui.heading("Round corners");
    ui.horizontal(|ui| {
        ui.label("Radius");
        len_field(ui, &mut app.ps.x.round_radius, units);
    });
    if ui.button("Round corners").clicked() {
        let paths = sel_world(app, &sel);
        let r = fx::round_corners(&paths, app.ps.x.round_radius);
        replace_sel(app, r);
    }
    None
}

fn shadow_style() -> Style {
    Style { fill: Some(Rgba(150, 150, 150, 255)), stroke: None, cut: CutStyle::NoCut, ..Style::default() }
}

// ------------------------------------------------------------------ path tools

pub fn pathtools_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    ui.heading("Path tools");
    if sel.is_empty() {
        ui.label("Select shapes to use these tools.");
        return None;
    }
    let cut_style = Style { stroke: Some(Rgba::RED), ..Style::default() };

    if ui.button("Convex hull").on_hover_text("Smallest enclosing outline").clicked() {
        let r = pathtools::convex_hull(&sel_world(app, &sel));
        add_new(app, r, Style { fill: None, ..cut_style.clone() }, "Hull");
    }

    egui::CollapsingHeader::new("Lattice").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Cell");
            len_field(ui, &mut app.ps.x.lattice_cell, units);
            ui.label("Wall");
            len_field(ui, &mut app.ps.x.lattice_bar, units);
        });
        if ui.button("Make lattice").clicked() {
            let r = pathtools::lattice(&sel_world(app, &sel), app.ps.x.lattice_cell, app.ps.x.lattice_bar);
            replace_sel(app, r);
        }
    });

    egui::CollapsingHeader::new("Knockout").show(ui, |ui| {
        ui.label("Bottom shape gets a recess for the shapes above it.");
        ui.horizontal(|ui| {
            ui.label("Gap");
            len_field(ui, &mut app.ps.x.knockout_gap, units);
        });
        if ui.add_enabled(sel.len() > 1, egui::Button::new("Knockout")).clicked() {
            let ordered: Vec<ShapeId> = app.doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).map(|s| s.id).collect();
            let bottom = sel_world(app, &ordered[..1]);
            let top = sel_world(app, &ordered[1..]);
            let r = pathtools::knockout(&bottom, &top, app.ps.x.knockout_gap);
            replace_sel(app, r);
        }
    });

    egui::CollapsingHeader::new("Stencil bridges").show(ui, |ui| {
        ui.label("Break closed outlines so interior islands stay attached.");
        ui.horizontal(|ui| {
            ui.label("Bridge width");
            len_field(ui, &mut app.ps.x.bridge_w, units);
            ui.label("Count");
            ui.add(DragValue::new(&mut app.ps.x.bridge_n).range(1..=12));
        });
        if ui.button("Add bridges").clicked() {
            let r = pathtools::stencil_bridges(&sel_world(app, &sel), app.ps.x.bridge_w, app.ps.x.bridge_n);
            replace_sel(app, r);
        }
    });

    egui::CollapsingHeader::new("Line fill / engrave").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Spacing");
            len_field(ui, &mut app.ps.x.linefill_spacing, units);
            ui.label("Angle");
            ui.add(DragValue::new(&mut app.ps.x.linefill_angle).suffix("°"));
        });
        if ui.button("Fill with lines").clicked() {
            let r = pathtools::line_fill(&sel_world(app, &sel), app.ps.x.linefill_spacing, app.ps.x.linefill_angle);
            add_new(app, r, Style { fill: None, stroke: Some(Rgba(30, 30, 200, 255)), ..Style::default() }, "Line fill");
        }
    });

    egui::CollapsingHeader::new("Symmetrical mirror").show(ui, |ui| {
        ui.label("Mirror across the right edge and weld into a symmetric design.");
        if ui.button("Mirror & weld").clicked() {
            if let Some(bb) = ops::selection_bbox(&app.doc, &sel) {
                let r = pathtools::symmetrical_mirror(&sel_world(app, &sel), bb.x1, true);
                replace_sel(app, r);
            }
        }
    });

    egui::CollapsingHeader::new("Clean up").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Tolerance");
            len_field(ui, &mut app.ps.x.dedup_tol, units);
        });
        if ui.button("Remove double lines").clicked() {
            let r = pathtools::remove_double_lines(&sel_world(app, &sel), app.ps.x.dedup_tol);
            replace_sel(app, r);
        }
        ui.horizontal(|ui| {
            ui.label("Join gap");
            len_field(ui, &mut app.ps.x.join_tol, units);
        });
        if ui.button("Join open paths").clicked() {
            let r = pathtools::join_segments(&sel_world(app, &sel), app.ps.x.join_tol);
            replace_sel(app, r);
        }
    });

    egui::CollapsingHeader::new("Pierce points").show(ui, |ui| {
        ui.label("Adds a tiny dot at each subpath start (plunge holes).");
        if ui.button("Add pierce points").clicked() {
            let pts = pathtools::pierce_points(&sel_world(app, &sel));
            let circles: Vec<BezPath> = pts.iter().map(|c| geom::circle_path(*c, 0.3)).collect();
            add_new(app, geom::merge_paths(&circles), Style { fill: Some(Rgba::BLACK), stroke: None, ..Style::default() }, "Pierce");
        }
    });

    egui::CollapsingHeader::new("Object on path").show(ui, |ui| {
        ui.label("Topmost selected shape is the path; the rest are repeated along it.");
        ui.horizontal(|ui| {
            ui.label("Copies");
            ui.add(DragValue::new(&mut app.ps.x.oop_count).range(1..=500));
        });
        ui.checkbox(&mut app.ps.x.oop_rotate, "Rotate to follow path");
        ui.checkbox(&mut app.ps.x.oop_keep, "Keep the path");
        if ui.add_enabled(sel.len() > 1, egui::Button::new("Place on path")).clicked() {
            let ordered: Vec<ShapeId> = app.doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).map(|s| s.id).collect();
            let path_id = *ordered.last().unwrap();
            let objects: Vec<ShapeId> = ordered[..ordered.len() - 1].to_vec();
            let (count, rot, keep) = (app.ps.x.oop_count, app.ps.x.oop_rotate, app.ps.x.oop_keep);
            app.edit(move |doc, _| Some(effects::object_on_path(doc, &objects, path_id, count, rot, keep)));
        }
    });
    None
}

// ------------------------------------------------------------------ colour sync

/// Colour Sync (as in Cricut Design Space): every line colour in the design,
/// each one a material when cutting "By line colour". Merge colours to cut
/// them together on one material.
pub fn colour_sync_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Colour sync");
    ui.label(RichText::new("Each line colour is cut as its own material when Send is set to “By line colour”. Merge colours to cut them together and use fewer materials.").small().weak());
    ui.add_space(4.0);
    // every outline colour, how many shapes use it, and which top-level shapes
    let page = app.doc.page();
    let mut colours: Vec<(Rgba, usize, Vec<ShapeId>)> = vec![];
    for s in page.shapes.iter().filter(|s| page.is_visible(s)) {
        for leaf in s.leaves() {
            let Some(c) = leaf.style.stroke.filter(|_| !leaf.is_image()) else { continue };
            match colours.iter_mut().find(|(x, _, _)| *x == c) {
                Some((_, n, ids)) => {
                    *n += 1;
                    if !ids.contains(&s.id) {
                        ids.push(s.id);
                    }
                }
                None => colours.push((c, 1, vec![s.id])),
            }
        }
    }
    if colours.is_empty() {
        ui.weak("No outlined shapes yet.");
        return None;
    }
    let mut merge: Option<(Rgba, Rgba)> = None;
    let mut select: Option<Vec<ShapeId>> = None;
    for (c, n, ids) in &colours {
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 4.0, crate::render::c32(*c));
            ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
            if resp.on_hover_text("Select the shapes in this colour").clicked() {
                select = Some(ids.clone());
            }
            ui.label(format!("{}  ·  {n} shape{}", c.hex(), if *n == 1 { "" } else { "s" }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt(("merge", c.hex())).selected_text("Merge into…").width(110.0).show_ui(ui, |ui| {
                    for (other, _, _) in colours.iter().filter(|(o, _, _)| o != c) {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(100.0, 20.0), egui::Sense::click());
                        ui.painter().rect_filled(r.shrink(2.0), 3.0, crate::render::c32(*other));
                        if resp.clicked() {
                            merge = Some((*c, *other));
                        }
                    }
                });
            });
        });
    }
    if let Some(ids) = select {
        app.sel = ids;
    }
    if let Some((from, to)) = merge {
        app.checkpoint_named("Colour sync");
        for s in app.doc.page_mut().shapes.iter_mut() {
            ops::recolour_outline(s, Some(from), to);
        }
        app.status = format!("Merged {} into {}", from.hex(), to.hex());
    }
    None
}
