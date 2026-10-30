//! Right-hand side panels (page, transform, replicate, modify, text, style,
//! layers, trace, effects, print & cut, send, preferences).

use crate::app::{Action, App};
use crate::render::c32;
use cutcraft_core::cutjob::{self, GroupMode};
use cutcraft_core::device::{self, Progress, Transport};
use cutcraft_core::doc::{Align, CutStyle, Layer, MatKind, Rgba, ShapeKind, TextData};
use cutcraft_core::effects::{self, StoneLayout};
use cutcraft_core::geom::CornerStyle;
use cutcraft_core::gpgl::{self, JobOptions};
use cutcraft_core::kurbo::{Affine, Point, Rect};
use cutcraft_core::materials::{self, Action as CutAction, Tool as CutTool};
use cutcraft_core::ops::{self, AlignMode, Replicate};
use cutcraft_core::prefs::Units;
use cutcraft_core::trace::{self, TraceMode, TraceParams};
use cutcraft_core::{machines, regmarks::RegMarks};
use egui::{Color32, DragValue, RichText, Ui};
use std::sync::atomic::Ordering;

/// The image tool's modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    /// Brush parts of a picture away.
    Erase,
    /// Brush erased parts back.
    Restore,
    /// Click an area of one colour to clear it.
    SelectErase,
    /// Drag a box to keep only that part.
    Crop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Page,
    Transform,
    Replicate,
    Modify,
    Text,
    Style,
    Layers,
    Trace,
    Effects,
    PrintCut,
    Send,
    Prefs,
    Warp,
    PixScan,
    Barcode,
    MailMerge,
    Fonts,
    Distort,
    PathTools,
    Dieline,
    Script,
    Image,
    Art,
    History,
    Params,
    Colours,
    DrawTools,
    Icons,
    WebFonts,
    Queue,
}

impl Panel {
    pub const ALL: [(Panel, &'static str); 30] = [
        (Panel::Page, "Page"),
        (Panel::Transform, "Transform"),
        (Panel::Replicate, "Replicate"),
        (Panel::Modify, "Modify"),
        (Panel::PathTools, "Path tools"),
        (Panel::Warp, "Warp"),
        (Panel::Distort, "Distort"),
        (Panel::DrawTools, "Drawing tools"),
        (Panel::Text, "Text"),
        (Panel::Fonts, "Fonts"),
        (Panel::WebFonts, "Web fonts"),
        (Panel::Icons, "Icons"),
        (Panel::Style, "Fill & Line"),
        (Panel::Colours, "Colours"),
        (Panel::Layers, "Layers"),
        (Panel::Trace, "Trace"),
        (Panel::Image, "Image"),
        (Panel::PixScan, "PixScan"),
        (Panel::Effects, "Effects"),
        (Panel::Art, "Art"),
        (Panel::History, "History"),
        (Panel::Params, "Parameters"),
        (Panel::Dieline, "Dieline"),
        (Panel::Barcode, "Barcode"),
        (Panel::MailMerge, "Mail merge"),
        (Panel::Script, "Script"),
        (Panel::PrintCut, "Print & Cut"),
        (Panel::Queue, "Queue"),
        (Panel::Send, "Send"),
        (Panel::Prefs, "Prefs"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    Usb,
    Bluetooth,
    Tcp,
    File,
}

pub struct PanelState {
    pub current_layer: u32,
    pub text: TextData,
    pub focus_text: bool,
    pub font_filter: String,
    pub corner_radius: f64,
    pub polygon_sides: usize,
    pub star_points: usize,
    pub star_inner: f64,
    pub eraser_radius: f64,
    /// Corner tool: bevel instead of round.
    pub corner_chamfer: bool,
    /// Edit-bar rotate box (degrees to turn by on Enter); separate from the
    /// Transform panel's rotate step.
    pub bar_rot: f64,
    /// Undo depth the History list last showed (to scroll to the current step).
    pub history_seen: usize,
    /// The panel last shown in each side-panel category.
    pub cat_panel: [Option<Panel>; 7],
    /// What the image tool does.
    pub image_mode: ImageMode,
    /// Select & Erase: how different a colour may be (0..100).
    pub select_tolerance: f64,
    /// Select & Erase: every similar colour, not just the touching area.
    pub select_everywhere: bool,
    /// Kerning: the letter whose following space is being edited.
    pub kern_letter: usize,
    /// Glyph picker: (family, glyphs with thumbnails) for the font shown.
    pub glyphs: Option<(String, Vec<(char, Vec<Vec<cutcraft_core::kurbo::Point>>)>)>,
    /// Pattern fill: the picture, its tile width and angle.
    pub pattern: Option<std::sync::Arc<Vec<u8>>>,
    pub pattern_tile: f64,
    pub pattern_angle: f64,
    /// Product templates: chosen product and size.
    pub product: usize,
    pub product_size: usize,
    // transform
    pub keep_aspect: bool,
    pub rot: f64,
    pub scale_pct: f64,
    pub shear: [f64; 2],
    // replicate
    pub rep_gap: f64,
    pub rep_n: usize,
    pub rep_dx: f64,
    pub rep_dy: f64,
    pub rep_rot: f64,
    // modify
    pub offset_dist: f64,
    pub offset_corner: CornerStyle,
    pub offset_internal: bool,
    pub offset_keep: bool,
    pub simplify_tol: f64,
    pub simplify_smooth: bool,
    // trace
    pub trace: TraceParams,
    pub trace_colour: Color32,
    pub trace_live: bool,
    // effects
    pub stone_d: f64,
    pub stone_gap: f64,
    pub stone_layout: usize,
    pub hatch_spacing: f64,
    pub hatch_angle: f64,
    pub hatch_cross: bool,
    pub weed_margin: f64,
    pub weed_lines: usize,
    pub contour_dist: f64,
    pub contour_col: Color32,
    pub shadow_dist: f64,
    pub layer_col: Color32,
    pub puzzle_rows: usize,
    pub puzzle_cols: usize,
    pub puzzle_knob: f64,
    pub puzzle_seed: u64,
    pub nest_pad: f64,
    pub nest_margin: f64,
    pub nest_rotate: bool,
    pub tile_w: f64,
    pub tile_h: f64,
    pub tile_overlap: f64,
    /// 0 = straight (overlapping tiles), 1 = dovetail, 2 = puzzle knob.
    pub tile_joint: usize,
    pub tile_joint_size: f64,
    pub tile_joint_spacing: f64,
    // send
    pub conn: Conn,
    pub usb_devices: Vec<device::UsbDevice>,
    pub usb_idx: usize,
    pub tcp_addr: String,
    pub device_info: String,
    pub feed_after: bool,
    pub page_preset: usize,
    // new feature panels
    pub stip_fill: f64,
    pub stip_edge: f64,
    pub stip_use_fill: bool,
    pub stip_use_edge: bool,
    pub stip_pattern: usize,
    pub stip_contrast: f64,
    pub emb_spacing: f64,
    pub emb_hatch: bool,
    pub emb_angle: f64,
    pub pop_depth: f64,
    pub pop_overlap: f64,
    pub x: crate::panels2::Extra,
}

impl Default for PanelState {
    fn default() -> Self {
        PanelState {
            current_layer: 0,
            text: TextData::default(),
            focus_text: false,
            font_filter: String::new(),
            corner_radius: 3.0,
            polygon_sides: 6,
            star_points: 5,
            star_inner: 0.45,
            eraser_radius: 2.0,
            corner_chamfer: false,
            bar_rot: 0.0,
            history_seen: usize::MAX,
            cat_panel: [None; 7],
            image_mode: ImageMode::Erase,
            select_tolerance: 20.0,
            select_everywhere: false,
            kern_letter: 0,
            glyphs: None,
            pattern: None,
            pattern_tile: 25.0,
            pattern_angle: 0.0,
            product: 0,
            product_size: 1,
            keep_aspect: true,
            rot: 15.0,
            scale_pct: 100.0,
            shear: [0.0, 0.0],
            rep_gap: 2.0,
            rep_n: 3,
            rep_dx: 20.0,
            rep_dy: 0.0,
            rep_rot: 0.0,
            offset_dist: 3.0,
            offset_corner: CornerStyle::Round,
            offset_internal: false,
            offset_keep: true,
            simplify_tol: 0.2,
            simplify_smooth: true,
            trace: TraceParams::default(),
            trace_colour: Color32::BLACK,
            trace_live: true,
            stone_d: 3.0,
            stone_gap: 1.0,
            stone_layout: 0,
            hatch_spacing: 2.0,
            hatch_angle: 45.0,
            hatch_cross: false,
            weed_margin: 5.0,
            weed_lines: 2,
            contour_dist: 3.0,
            contour_col: Color32::WHITE,
            shadow_dist: 3.0,
            layer_col: Color32::from_rgb(40, 40, 40),
            puzzle_rows: 4,
            puzzle_cols: 5,
            puzzle_knob: 0.22,
            puzzle_seed: 7,
            nest_pad: 3.0,
            nest_margin: 5.0,
            nest_rotate: true,
            tile_w: 300.0,
            tile_h: 300.0,
            tile_overlap: 10.0,
            tile_joint: 0,
            tile_joint_size: 14.0,
            tile_joint_spacing: 50.0,
            conn: Conn::Usb,
            usb_devices: vec![],
            usb_idx: 0,
            tcp_addr: String::new(),
            device_info: String::new(),
            feed_after: true,
            page_preset: 0,
            stip_fill: 2.0,
            stip_edge: 1.5,
            stip_use_fill: true,
            stip_use_edge: true,
            stip_pattern: 1,
            stip_contrast: 1.2,
            emb_spacing: 0.8,
            emb_hatch: false,
            emb_angle: 45.0,
            pop_depth: 20.0,
            pop_overlap: 3.0,
            x: Default::default(),
        }
    }
}

const PAGE_PRESETS: [(&str, f64, f64); 9] = [
    ("Custom", 0.0, 0.0),
    ("Letter (8.5 × 11 in)", 215.9, 279.4),
    ("Legal (8.5 × 14 in)", 215.9, 355.6),
    ("A4 (210 × 297 mm)", 210.0, 297.0),
    ("A3 (297 × 420 mm)", 297.0, 420.0),
    ("12 × 12 in", 304.8, 304.8),
    ("12 × 24 in", 304.8, 609.6),
    ("15 × 15 in", 381.0, 381.0),
    ("24 × 24 in", 609.6, 609.6),
];

/// Length field shown in the user's units, stored in mm.
fn len_field(ui: &mut Ui, v: &mut f64, units: Units) -> egui::Response {
    let f = units.factor();
    let mut x = *v * f;
    let r = ui.add(DragValue::new(&mut x).speed(0.1 * f).max_decimals(3).suffix(format!(" {}", units.suffix())));
    if r.changed() {
        *v = x / f;
    }
    r
}

fn rgba_to_c32(c: Rgba) -> Color32 {
    c32(c)
}
fn c32_to_rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba(r, g, b, a)
}

pub fn job_options(app: &App) -> JobOptions {
    let p = app.doc.page();
    JobOptions {
        origin_mm: (0.0, 0.0),
        media_mm: (p.width, p.height),
        track_enhancing: app.prefs.track_enhancing,
        regmarks: if p.regmarks.enabled { Some(p.regmarks.clone()) } else { None },
        feed_after: app.ps.feed_after,
        dialect: app.prefs.gpgl_dialect,
    }
}

pub fn show(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let mut act = None;
    crate::chrome::panel_header(app, ui);
    app.label_ctx = Panel::ALL.iter().find(|(p, _)| *p == app.panel).map(|(_, n)| *n);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        act = match app.panel {
            Panel::Page => page_panel(app, ui),
            Panel::Transform => transform_panel(app, ui),
            Panel::Replicate => replicate_panel(app, ui),
            Panel::Modify => modify_panel(app, ui),
            Panel::Text => text_panel(app, ui),
            Panel::Style => style_panel(app, ui),
            Panel::Layers => layers_panel(app, ui),
            Panel::Trace => trace_panel(app, ui),
            Panel::Effects => effects_panel(app, ui),
            Panel::PrintCut => printcut_panel(app, ui),
            Panel::Send => send_panel(app, ui),
            Panel::Prefs => prefs_panel(app, ui),
            Panel::Warp => crate::panels2::warp_panel(app, ui),
            Panel::PixScan => crate::panels2::pixscan_panel(app, ui),
            Panel::Barcode => crate::panels2::barcode_panel(app, ui),
            Panel::MailMerge => crate::panels2::mailmerge_panel(app, ui),
            Panel::Fonts => crate::panels2::fonts_panel(app, ui),
            Panel::Distort => crate::panels2::distort_panel(app, ui),
            Panel::PathTools => crate::panels2::pathtools_panel(app, ui),
            Panel::Dieline => crate::panels2::dieline_panel(app, ui),
            Panel::Script => crate::panels2::script_panel(app, ui),
            Panel::Image => crate::panels2::image_panel(app, ui),
            Panel::Art => crate::panels2::art_panel(app, ui),
            Panel::History => history_panel(app, ui),
            Panel::Params => crate::paramspanel::params_panel(app, ui),
            Panel::Colours => crate::panels2::colour_sync_panel(app, ui),
            Panel::DrawTools => drawtools_panel(app, ui),
            Panel::Icons => crate::webui::icons_panel(app, ui),
            Panel::WebFonts => crate::webui::fonts_panel(app, ui),
            Panel::Queue => crate::queue_ui::queue_panel(app, ui),
        };
    });
    app.label_ctx = None;
    if app.panel != Panel::Trace {
        app.cs.trace_preview = None;
    }
    act
}

// ------------------------------------------------------------------ history

/// Every undo step by name; clicking one undoes or redoes to just after it.
fn history_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("History");
    ui.label(RichText::new("Click a step to go back to it. Greyed steps can be redone until you make a new edit.").small().weak());
    ui.add_space(4.0);
    let cur = app.undo.len();
    let mut entries: Vec<(usize, String)> = vec![(0, if cur >= 200 { "Oldest kept state".into() } else { "Start".into() })];
    entries.extend(app.undo.iter().enumerate().map(|(i, (_, l))| (i + 1, l.clone())));
    entries.extend(app.redo.iter().rev().enumerate().map(|(j, (_, l))| (cur + j + 1, l.clone())));
    // scroll the current step into view whenever it changes
    let follow = app.ps.history_seen != cur;
    app.ps.history_seen = cur;
    let mut jump = None;
    for (steps, label) in &entries {
        let text = if *steps > cur { RichText::new(label).weak() } else { RichText::new(label) };
        let r = ui.selectable_label(*steps == cur, text);
        if *steps == cur && follow {
            r.scroll_to_me(Some(egui::Align::Center));
        }
        if r.clicked() && *steps != cur {
            jump = Some(*steps);
        }
    }
    if let Some(s) = jump {
        app.history_jump(s);
    }
    None
}

// ------------------------------------------------------------------ page

fn page_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Page setup");
    let mut machine = app.doc.machine.clone();
    egui::ComboBox::from_label("Machine").selected_text(machines::by_id(&machine).name).show_ui(ui, |ui| {
        for m in machines::MACHINES {
            ui.selectable_value(&mut machine, m.id.to_string(), m.name);
        }
    });
    if machine != app.doc.machine {
        app.checkpoint();
        app.doc.machine = machine.clone();
        app.prefs.default_machine = machine;
        // Adopt the machine's wire language as the default protocol.
        app.prefs.protocol = machines::by_id(&app.doc.machine).protocol;
    }
    let m = machines::by_id(&app.doc.machine);
    ui.label(format!("Max cutting width {:.0} mm · {} tool slot(s)", m.max_width, m.tool_slots));
    ui.separator();

    let mut preset = app.ps.page_preset;
    egui::ComboBox::from_label("Media size").selected_text(PAGE_PRESETS[preset].0).show_ui(ui, |ui| {
        for (i, p) in PAGE_PRESETS.iter().enumerate() {
            ui.selectable_value(&mut preset, i, p.0);
        }
    });
    if preset != app.ps.page_preset {
        app.ps.page_preset = preset;
        if preset > 0 {
            app.checkpoint();
            let (_, w, h) = PAGE_PRESETS[preset];
            let p = app.doc.page_mut();
            p.width = w;
            p.height = h;
            app.view.fit_requested = true;
        }
    }
    let mut w = app.doc.page().width;
    let mut h = app.doc.page().height;
    let mut changed = false;
    egui::Grid::new("pg").num_columns(2).show(ui, |ui| {
        ui.label("Width");
        changed |= len_field(ui, &mut w, units).changed();
        ui.end_row();
        ui.label("Height");
        changed |= len_field(ui, &mut h, units).changed();
        ui.end_row();
    });
    if changed {
        let p = app.doc.page_mut();
        p.width = w.max(10.0);
        p.height = h.max(10.0);
        app.ps.page_preset = 0;
        app.rev += 1;
    }
    ui.horizontal(|ui| {
        if ui.button("↻ Swap orientation").clicked() {
            app.checkpoint();
            let p = app.doc.page_mut();
            std::mem::swap(&mut p.width, &mut p.height);
            app.view.fit_requested = true;
        }
        if ui.button("Fit page in view").clicked() {
            app.view.fit_requested = true;
        }
    });
    let mut mat = app.doc.page().mat;
    egui::ComboBox::from_label("Cutting mat").selected_text(mat.label()).show_ui(ui, |ui| {
        for mk in MatKind::ALL {
            ui.selectable_value(&mut mat, mk, mk.label());
        }
    });
    if mat != app.doc.page().mat {
        app.checkpoint();
        app.doc.page_mut().mat = mat;
        // media can't be bigger than the mat it sits on
        if fit_media_to_mat(app.doc.page_mut()) {
            app.ps.page_preset = 0;
            app.status = "Media trimmed to fit the mat".into();
        }
        app.view.fit_requested = true;
    }
    let p = app.doc.page();
    if let Some((mw, mh)) = p.mat.size_mm() {
        if p.width > mw + 0.01 || p.height > mh + 0.01 {
            ui.colored_label(ui.visuals().warn_fg_color, format!("The media is larger than the {} - it won't fit.", p.mat.label()));
            if ui.button("Fit media to mat").clicked() {
                app.checkpoint();
                fit_media_to_mat(app.doc.page_mut());
                app.ps.page_preset = 0;
                app.view.fit_requested = true;
            }
        }
    }
    // media is opaque: no transparency in the picker
    let c = app.doc.page().media_color;
    let mut col = [c.0, c.1, c.2];
    ui.horizontal(|ui| {
        ui.label("Media colour");
        if ui.color_edit_button_srgb(&mut col).changed() {
            app.doc.page_mut().media_color = Rgba(col[0], col[1], col[2], 255);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Cut border");
        let mut b = app.doc.page().cut_border;
        if len_field(ui, &mut b, units).changed() {
            app.doc.page_mut().cut_border = b.max(0.0);
        }
    });
    ui.separator();
    ui.heading("Grid");
    let g = &mut app.doc.pages[app.doc.active].grid;
    ui.checkbox(&mut g.visible, "Show grid");
    ui.checkbox(&mut g.snap, "Snap to grid");
    ui.horizontal(|ui| {
        ui.label("Spacing");
        len_field(ui, &mut g.spacing, units);
        ui.label("Divisions");
        ui.add(DragValue::new(&mut g.subdivisions).range(1..=20));
    });
    ui.separator();
    ui.heading("Pages");
    let mut act = None;
    let n = app.doc.pages.len();
    for i in 0..n {
        ui.horizontal(|ui| {
            if ui.selectable_label(i == app.doc.active, format!("{}", i + 1)).clicked() {
                app.doc.active = i;
                app.sel.clear();
                app.rev += 1;
            }
            ui.text_edit_singleline(&mut app.doc.pages[i].name);
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Add page").clicked() {
            act = Some(Action::AddPage);
        }
        if ui.button("Delete page").clicked() {
            act = Some(Action::DeletePage);
        }
    });
    act
}

// ------------------------------------------------------------------ drawing tools

/// Settings for the drawing tools: shape tools, the corner tool and erasers.
fn drawtools_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Drawing tools");
    ui.label("Settings the tools on the left use as you draw.");
    egui::Grid::new("dt").num_columns(2).show(ui, |ui| {
        ui.label("Corner radius").on_hover_text("Used by the rounded-rectangle tool and the Round-corner tool");
        len_field(ui, &mut app.ps.corner_radius, units);
        ui.end_row();
        ui.label("Corner tool");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.ps.corner_chamfer, false, "Round");
            ui.selectable_value(&mut app.ps.corner_chamfer, true, "Chamfer");
        });
        ui.end_row();
        ui.label("Polygon sides");
        ui.add(DragValue::new(&mut app.ps.polygon_sides).range(3..=64));
        ui.end_row();
        ui.label("Star points");
        ui.add(DragValue::new(&mut app.ps.star_points).range(3..=64));
        ui.end_row();
        ui.label("Star inner ratio");
        ui.add(egui::Slider::new(&mut app.ps.star_inner, 0.1..=0.95));
        ui.end_row();
        ui.label("Eraser radius").on_hover_text("Brush size for the path eraser and the image eraser");
        len_field(ui, &mut app.ps.eraser_radius, units);
        ui.end_row();
    });
    None
}

// ------------------------------------------------------------------ transform

fn transform_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    if sel.is_empty() {
        ui.label("Select one or more shapes.");
        return None;
    }
    let bb = ops::selection_bbox(&app.doc, &sel)?;
    ui.heading("Position & size");
    let mut v = [bb.x0, bb.y0, bb.width(), bb.height()];
    let mut changed = [false; 4];
    egui::Grid::new("tf").num_columns(4).show(ui, |ui| {
        ui.label("X");
        changed[0] = len_field(ui, &mut v[0], units).changed();
        ui.label("Y");
        changed[1] = len_field(ui, &mut v[1], units).changed();
        ui.end_row();
        ui.label("W");
        changed[2] = len_field(ui, &mut v[2], units).changed();
        ui.label("H");
        changed[3] = len_field(ui, &mut v[3], units).changed();
        ui.end_row();
    });
    ui.checkbox(&mut app.ps.keep_aspect, "Lock aspect ratio");
    if changed[0] || changed[1] {
        let (x, y) = (v[0], v[1]);
        app.edit(|d, s| {
            ops::move_to(d, s, x, y);
            None
        });
    }
    if changed[2] || changed[3] {
        let ka = app.ps.keep_aspect;
        let (w, h) = (if changed[2] { Some(v[2].max(0.01)) } else { None }, if changed[3] { Some(v[3].max(0.01)) } else { None });
        app.edit(|d, s| {
            ops::set_size(d, s, w, h, ka);
            None
        });
    }
    ui.separator();
    ui.heading("Rotate");
    let mut act = None;
    ui.horizontal(|ui| {
        for d in [-90.0, -45.0, 45.0, 90.0, 180.0] {
            if ui.button(format!("{d}°")).clicked() {
                act = Some(Action::Rotate(d));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(DragValue::new(&mut app.ps.rot).suffix("°").speed(1.0));
        if ui.button("Rotate").clicked() {
            act = Some(Action::Rotate(app.ps.rot));
        }
        if ui.button("Each").on_hover_text("Rotate every shape about its own centre").clicked() {
            let r = app.ps.rot;
            app.edit(|d, s| {
                ops::rotate_each(d, s, r);
                None
            });
        }
    });
    ui.separator();
    ui.heading("Scale");
    ui.horizontal(|ui| {
        for p in [50.0, 75.0, 150.0, 200.0] {
            if ui.button(format!("{p}%")).clicked() {
                app.edit(|d, s| {
                    ops::scale(d, s, p / 100.0, p / 100.0);
                    None
                });
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(DragValue::new(&mut app.ps.scale_pct).suffix(" %").range(1.0..=10000.0));
        if ui.button("Scale").clicked() {
            let k = app.ps.scale_pct / 100.0;
            app.edit(|d, s| {
                ops::scale(d, s, k, k);
                None
            });
        }
    });
    ui.separator();
    ui.heading("Shear");
    ui.horizontal(|ui| {
        ui.label("H");
        ui.add(DragValue::new(&mut app.ps.shear[0]).suffix("°").range(-80.0..=80.0));
        ui.label("V");
        ui.add(DragValue::new(&mut app.ps.shear[1]).suffix("°").range(-80.0..=80.0));
        if ui.button("Apply").clicked() {
            let [x, y] = app.ps.shear;
            app.edit(|d, s| {
                ops::shear(d, s, x, y);
                None
            });
        }
    });
    ui.separator();
    ui.heading("Flip, align & arrange");
    ui.horizontal_wrapped(|ui| {
        if ui.button("⇆ Flip H").clicked() {
            act = Some(Action::FlipH);
        }
        if ui.button("⇅ Flip V").clicked() {
            act = Some(Action::FlipV);
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (l, m) in [
            ("Left", AlignMode::Left),
            ("Centre", AlignMode::CenterH),
            ("Right", AlignMode::Right),
            ("Top", AlignMode::Top),
            ("Middle", AlignMode::Middle),
            ("Bottom", AlignMode::Bottom),
            ("Centre both", AlignMode::Center),
        ] {
            if ui.button(l).clicked() {
                act = Some(Action::Align(m));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Distribute H").clicked() {
            act = Some(Action::DistributeH);
        }
        if ui.button("Distribute V").clicked() {
            act = Some(Action::DistributeV);
        }
    });
    if sel.len() == 1 {
        ui.small("With one shape selected, alignment is relative to the page.");
    }
    ui.horizontal_wrapped(|ui| {
        for (l, a) in [
            ("To front", ops::Arrange::Front),
            ("Forward", ops::Arrange::Forward),
            ("Backward", ops::Arrange::Backward),
            ("To back", ops::Arrange::Back),
        ] {
            if ui.button(l).clicked() {
                act = Some(Action::Arrange(a));
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Group").clicked() {
            act = Some(Action::Group);
        }
        if ui.button("Ungroup").clicked() {
            act = Some(Action::Ungroup);
        }
    });
    act
}

// ------------------------------------------------------------------ replicate

fn replicate_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    if app.editable_sel().is_empty() {
        ui.label("Select shapes to replicate.");
        return None;
    }
    ui.heading("Replicate");
    ui.horizontal(|ui| {
        ui.label("Spacing");
        len_field(ui, &mut app.ps.rep_gap, units);
    });
    let gap = app.ps.rep_gap;
    let buttons: [(&str, Replicate); 15] = [
        ("Duplicate left", Replicate::Left),
        ("Duplicate right", Replicate::Right),
        ("Duplicate above", Replicate::Above),
        ("Duplicate below", Replicate::Below),
        ("Row of 3", Replicate::Row(3)),
        ("Row of 4", Replicate::Row(4)),
        ("Column of 3", Replicate::Column(3)),
        ("Column of 4", Replicate::Column(4)),
        ("Rotate 2", Replicate::Rotate(2)),
        ("Rotate 4", Replicate::Rotate(4)),
        ("Rotate 6", Replicate::Rotate(6)),
        ("Mirror left", Replicate::MirrorLeft),
        ("Mirror right", Replicate::MirrorRight),
        ("Mirror above/below", Replicate::MirrorBelow),
        ("Fill page", Replicate::FillPage),
    ];
    egui::Grid::new("rep").num_columns(2).show(ui, |ui| {
        for (i, (l, r)) in buttons.iter().enumerate() {
            if ui.add_sized([140.0, 22.0], egui::Button::new(*l)).clicked() {
                let r = *r;
                app.edit(|d, s| Some(ops::replicate(d, s, r, gap)));
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.separator();
    ui.heading("Advanced");
    egui::Grid::new("repadv").num_columns(2).show(ui, |ui| {
        ui.label("Copies");
        ui.add(DragValue::new(&mut app.ps.rep_n).range(1..=500));
        ui.end_row();
        ui.label("Offset X");
        len_field(ui, &mut app.ps.rep_dx, units);
        ui.end_row();
        ui.label("Offset Y");
        len_field(ui, &mut app.ps.rep_dy, units);
        ui.end_row();
        ui.label("Rotate each");
        ui.add(DragValue::new(&mut app.ps.rep_rot).suffix("°"));
        ui.end_row();
    });
    if ui.button("Apply").clicked() {
        let (n, dx, dy, r) = (app.ps.rep_n, app.ps.rep_dx, app.ps.rep_dy, app.ps.rep_rot);
        app.edit(|d, s| Some(ops::replicate_advanced(d, s, n, dx, dy, r)));
    }

    ui.separator();
    ui.heading("Blend");
    ui.small("Steps between two shapes: the outline morphs from one to the other, with the colours and line width in between - graduated layers for paper and shadow-box art.");
    ui.horizontal(|ui| {
        ui.label("Steps");
        ui.add(DragValue::new(&mut app.ps.x.blend_steps).range(1..=200));
        let two = app.editable_sel().len() == 2;
        if ui.add_enabled(two, egui::Button::new("Blend")).on_disabled_hover_text("Select exactly two shapes").clicked() {
            let n = app.ps.x.blend_steps;
            app.edit(|d, s| Some(cutcraft_core::blend::blend(d, s, n)));
        }
    });

    ui.separator();
    ui.heading("Fill sheet / batch");
    ui.small("Nests as many copies as fit on the page.");
    ui.checkbox(&mut app.ps.x.batch_rotate, "Allow rotation");
    let (gap, rot) = (app.ps.rep_gap, app.ps.x.batch_rotate);
    ui.horizontal(|ui| {
        if ui.button("Calculate per sheet").clicked() {
            app.ps.x.batch_per = cutcraft_core::effects::copies_per_sheet(&app.doc, &app.editable_sel(), gap, rot, 5.0);
        }
        if app.ps.x.batch_per > 0 {
            ui.label(format!("{} fit per sheet", app.ps.x.batch_per));
        }
    });
    if app.ps.x.batch_per > 0 {
        ui.horizontal(|ui| {
            ui.label("Target quantity");
            ui.add(DragValue::new(&mut app.ps.rep_n).range(1..=1_000_000));
        });
        let sheets = app.ps.rep_n.div_ceil(app.ps.x.batch_per);
        ui.label(format!("→ {} sheet(s) for {} pieces", sheets, app.ps.rep_n));
    }
    if ui.button("Fill sheet with copies").clicked() {
        app.edit(move |d, s| {
            let ids = cutcraft_core::effects::fill_sheet(d, s, gap, rot, 5.0);
            if ids.is_empty() { None } else { Some(ids) }
        });
    }
    None
}

// ------------------------------------------------------------------ modify

fn modify_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let mut act = None;
    ui.heading("Modify");
    egui::Grid::new("mod").num_columns(2).show(ui, |ui| {
        for (i, (l, a, tip)) in [
            ("Weld", Action::Weld, "Join overlapping shapes into one"),
            ("Subtract", Action::Subtract, "Bottom shape minus the shapes above"),
            ("Subtract all", Action::SubtractAll, "The shapes on top cut through every shape below them and are removed"),
            ("Intersect", Action::Intersect, "Keep only the overlap"),
            ("Exclude", Action::Exclude, "Remove only where the shapes overlap; each keeps its colour"),
            ("Unite", Action::Unite, "Cut as one welded outline but keep the parts (ungroup to separate them)"),
            ("Flatten", Action::Flatten, "Make one printed picture with a single outer cut line (Print Then Cut)"),
            ("Unflatten", Action::Unflatten, "Bring a flattened design's shapes back"),
            ("Divide (slice)", Action::Divide, "Split into every overlapping region, like a cookie cutter"),
            ("Make compound", Action::MakeCompound, "Combine into one path (holes stay holes)"),
            ("Release compound", Action::ReleaseCompound, "Split a path into its sub-paths"),
            ("Convert to path", Action::ConvertToPath, "Turn text or groups into editable paths"),
        ]
        .into_iter()
        .enumerate()
        {
            if ui.add_sized([140.0, 22.0], egui::Button::new(l)).on_hover_text(tip).clicked() {
                act = Some(a);
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.separator();
    ui.heading("Offset");
    egui::Grid::new("off").num_columns(2).show(ui, |ui| {
        ui.label("Distance");
        len_field(ui, &mut app.ps.offset_dist, units);
        ui.end_row();
        ui.label("Corners");
        ui.horizontal(|ui| {
            ui.radio_value(&mut app.ps.offset_corner, CornerStyle::Round, "Round");
            ui.radio_value(&mut app.ps.offset_corner, CornerStyle::Sharp, "Sharp");
        });
        ui.end_row();
    });
    ui.checkbox(&mut app.ps.offset_internal, "Internal offset");
    ui.checkbox(&mut app.ps.offset_keep, "Keep original");
    if ui.button("Apply offset").clicked() {
        let d = if app.ps.offset_internal { -app.ps.offset_dist } else { app.ps.offset_dist };
        let (c, k) = (app.ps.offset_corner, app.ps.offset_keep);
        let s = app.editable_sel();
        if s.is_empty() {
            app.status = "Select shapes to offset".into();
        } else {
            app.edit(|doc, _| Some(ops::offset(doc, &s, d, c, k)));
        }
    }
    ui.separator();
    ui.heading("Simplify / smooth");
    ui.horizontal(|ui| {
        ui.label("Tolerance");
        len_field(ui, &mut app.ps.simplify_tol, units);
        ui.checkbox(&mut app.ps.simplify_smooth, "Smooth");
    });
    if ui.button("Simplify").clicked() {
        let (t, s) = (app.ps.simplify_tol, app.ps.simplify_smooth);
        app.edit(|d, sel| {
            ops::simplify(d, sel, t, s);
            None
        });
    }
    ui.separator();
    ui.heading("Point editing");
    ui.label("Use the ◇ tool: drag points and handles, double-click a line to add a point, Delete removes the selected point.");
    ui.horizontal(|ui| {
        if ui.button("Line ⇄ curve").on_hover_text("Toggle the segment ending at the selected point").clicked() {
            crate::canvas::toggle_segment(app);
        }
        if ui.button("Delete point").clicked() {
            crate::canvas::delete_node(app);
        }
    });
    act
}

// ------------------------------------------------------------------ text

fn text_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let text_id = app.sel.iter().copied().find(|id| matches!(app.doc.page().shape(*id).map(|s| &s.kind), Some(ShapeKind::Text { .. })));
    // edit either the selected text's data or the defaults for new text
    let mut data = match text_id.and_then(|id| app.doc.page().shape(id)) {
        Some(s) => match &s.kind {
            ShapeKind::Text { data, .. } => data.clone(),
            _ => unreachable!(),
        },
        None => app.ps.text.clone(),
    };
    let before = data.clone();
    ui.heading(if text_id.is_some() { "Text (selected)" } else { "Text defaults - click the canvas with the T tool" });
    let te = ui.add(egui::TextEdit::multiline(&mut data.text).desired_rows(3).desired_width(f32::INFINITY));
    if app.ps.focus_text {
        te.request_focus();
        app.ps.focus_text = false;
    }
    ui.horizontal(|ui| {
        ui.label("Font");
        ui.add(egui::TextEdit::singleline(&mut app.ps.font_filter).hint_text("search fonts").desired_width(120.0));
    });
    let filter = app.ps.font_filter.to_lowercase();
    egui::ScrollArea::vertical().id_salt("fonts").max_height(150.0).show(ui, |ui| {
        for f in app.fonts.families() {
            if !filter.is_empty() && !f.to_lowercase().contains(&filter) {
                continue;
            }
            if ui.selectable_label(&data.family == f, f).clicked() {
                data.family = f.clone();
            }
        }
    });
    egui::Grid::new("txt").num_columns(2).show(ui, |ui| {
        ui.label("Size");
        len_field(ui, &mut data.size, units);
        ui.end_row();
        ui.label("Style");
        ui.horizontal(|ui| {
            ui.toggle_value(&mut data.bold, RichText::new("B").strong());
            ui.toggle_value(&mut data.italic, RichText::new("I").italics());
            ui.toggle_value(&mut data.vertical, "Vertical");
            if data.family != cutcraft_core::singleline::FAMILY {
                ui.toggle_value(&mut data.weld, "Weld")
                    .on_hover_text("Fuse overlapping letters into one outline - use with script fonts so the blade doesn't cut each letter where they touch");
            }
        });
        ui.end_row();
        if data.family == cutcraft_core::singleline::FAMILY {
            ui.label("");
            ui.small("Single-line font: letters are pen strokes - give it a Sketch/pen action in Send.");
            ui.end_row();
        }
        ui.label("Align");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut data.align, Align::Left, "Left");
            ui.selectable_value(&mut data.align, Align::Center, "Centre");
            ui.selectable_value(&mut data.align, Align::Right, "Right");
        });
        ui.end_row();
        ui.label("Char spacing");
        ui.add(egui::Slider::new(&mut data.char_spacing, -50.0..=200.0).suffix(" %"));
        ui.end_row();
        ui.label("Line spacing");
        ui.add(egui::Slider::new(&mut data.line_spacing, 0.5..=3.0));
        ui.end_row();
        if text_id.is_some() && data.text.chars().count() > 1 {
            ui.label("Kerning");
            ui.vertical(|ui| {
                // pick a letter; the space after it is adjusted
                let chars: Vec<char> = data.text.chars().collect();
                app.ps.kern_letter = app.ps.kern_letter.min(chars.len() - 2);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (i, c) in chars.iter().enumerate().take(chars.len() - 1) {
                        if c.is_whitespace() && *c == '\n' {
                            continue;
                        }
                        let on = i == app.ps.kern_letter;
                        let label = format!("{}{}", c, chars[i + 1]);
                        if ui.selectable_label(on, label).on_hover_text("Adjust the space between these two letters").clicked() {
                            app.ps.kern_letter = i;
                        }
                    }
                });
                let i = app.ps.kern_letter;
                let mut k = data.kern_after(i);
                ui.horizontal(|ui| {
                    if ui.add(egui::Slider::new(&mut k, -10.0..=10.0).suffix(" mm").step_by(0.05)).changed() {
                        data.set_kern(i, k);
                    }
                    if !data.kern.is_empty() && ui.small_button("Reset all").clicked() {
                        data.kern.clear();
                    }
                });
            });
            ui.end_row();
        }
        if data.on_path.is_none() && !data.vertical {
            ui.label("Curve");
            ui.add(egui::Slider::new(&mut data.curve, -360.0..=360.0).suffix("°").step_by(1.0))
                .on_hover_text("Bend the text round a circle: positive arches it up (badges), negative curves it down (smiles), 0 is straight");
            ui.end_row();
        }
    });
    // glyph picker: every character the font has (swashes, symbols, accents)
    egui::CollapsingHeader::new("Glyphs").id_salt("glyphs").show(ui, |ui| {
        let key = format!("{}|{}|{}", data.family, data.bold, data.italic);
        if app.ps.glyphs.as_ref().is_none_or(|(k, _)| *k != key) {
            let chars = app.fonts.glyph_chars(&data.family, data.bold, data.italic);
            let list = chars
                .into_iter()
                .take(4000)
                .map(|c| {
                    let one = TextData { text: c.to_string(), size: 10.0, curve: 0.0, kern: vec![], on_path: None, weld: false, ..data.clone() };
                    (c, cutcraft_core::library::thumbnail(&app.fonts.layout(&one)))
                })
                .collect();
            app.ps.glyphs = Some((key, list));
        }
        let Some((_, list)) = &app.ps.glyphs else { return };
        ui.small(format!("{} glyphs - click one to add it to the text", list.len()));
        const CELL: f32 = 34.0;
        let cols = ((ui.available_width() / (CELL + 2.0)) as usize).max(1);
        let rows = list.len().div_ceil(cols);
        let mut add = None;
        egui::ScrollArea::vertical().id_salt("glyph-grid").max_height(220.0).show_rows(ui, CELL + 2.0, rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                    for (c, thumb) in list.iter().skip(r * cols).take(cols) {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click());
                        let bg = if resp.hovered() { ui.visuals().widgets.hovered.weak_bg_fill } else { ui.visuals().extreme_bg_color };
                        ui.painter().rect_filled(rect, 3.0, bg);
                        let inner = rect.shrink(5.0);
                        for l in thumb {
                            let pts: Vec<egui::Pos2> = l.iter().map(|p| inner.min + egui::vec2(p.x as f32, p.y as f32) * inner.width()).collect();
                            ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.0, ui.visuals().text_color())));
                        }
                        if resp.on_hover_text(format!("U+{:04X}", *c as u32)).clicked() {
                            add = Some(*c);
                        }
                    }
                });
            }
        });
        if let Some(c) = add {
            data.text.push(c);
        }
    });
    if data != before {
        match text_id {
            Some(id) => {
                if !app.ps.text_editing_session(id) {
                    app.checkpoint();
                }
                if let Some(s) = app.doc.page_mut().shape_mut(id) {
                    if let ShapeKind::Text { data: d, .. } = &mut s.kind {
                        *d = data.clone();
                    }
                }
                app.relayout_text(id);
                app.dirty = true;
            }
            None => app.ps.text = data.clone(),
        }
    }
    let mut act = None;
    if text_id.is_some() {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ungroup to letters").on_hover_text("Each letter becomes its own text you can move, rotate or resize").clicked() {
                act = Some(Action::UngroupLetters);
            }
            if ui.button("Ungroup to lines").on_hover_text("Each line becomes its own text").clicked() {
                act = Some(Action::UngroupLines);
            }
        });
    }
    if let Some(id) = text_id {
        ui.separator();
        ui.heading("Text on path");
        let path_id = app.sel.iter().copied().find(|x| matches!(app.doc.page().shape(*x).map(|s| &s.kind), Some(ShapeKind::Path(_))));
        if data.on_path.is_some() {
            let mut off = data.path_offset;
            ui.horizontal(|ui| {
                ui.label("Start offset");
                if len_field(ui, &mut off, units).changed() {
                    if let Some(ShapeKind::Text { data, .. }) = app.doc.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                        data.path_offset = off;
                    }
                    app.relayout_text(id);
                }
            });
            if ui.button("Detach from path").clicked() {
                app.checkpoint();
                if let Some(ShapeKind::Text { data, .. }) = app.doc.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                    data.on_path = None;
                }
                app.relayout_text(id);
            }
        } else if let Some(pid) = path_id {
            if ui.button("Attach text to selected path").clicked() {
                app.checkpoint();
                let path = cutcraft_core::geom::merge_paths(&app.doc.page().shape(pid).unwrap().world_paths());
                if let Some(s) = app.doc.page_mut().shape_mut(id) {
                    // text-on-path outline is produced directly in page space
                    let inv = s.transform.inverse();
                    let mut local = path.clone();
                    local.apply_affine(inv);
                    if let ShapeKind::Text { data, .. } = &mut s.kind {
                        data.on_path = Some(local);
                    }
                }
                app.relayout_text(id);
            }
        } else {
            ui.small("Select this text and a path to place the text along it.");
        }
        ui.separator();
        if ui.button("Convert text to path").clicked() {
            act = Some(Action::ConvertToPath);
        }
        ui.separator();
        crate::panels2::spelling_section(app, ui, id);
    }
    act
}

impl PanelState {
    /// Group rapid typing into a single undo step per text shape.
    fn text_editing_session(&mut self, id: u64) -> bool {
        thread_local! { static LAST: std::cell::Cell<(u64, f64)> = const { std::cell::Cell::new((0, 0.0)) }; }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        LAST.with(|l| {
            let (lid, t) = l.get();
            l.set((id, now));
            lid == id && now - t < 1.5
        })
    }
}

// ------------------------------------------------------------------ style

fn style_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    let Some(first) = sel.first().and_then(|id| app.doc.page().shape(*id)) else {
        ui.label("Select shapes to change fill, line and cut style.");
        return None;
    };
    let mut st = first.style.clone();
    let before = st.clone();
    ui.heading("Fill");
    let mut has_fill = st.fill.is_some();
    let mut fc = rgba_to_c32(st.fill.unwrap_or(Rgba(120, 160, 220, 255)));
    use crate::eyedrop::Target;
    if let Some(c) = app.eyedrop.take(Target::Fill) {
        (fc, has_fill) = (c, true);
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut has_fill, "Filled");
        ui.color_edit_button_srgba(&mut fc);
        pick_button(app, ui, Target::Fill);
    });
    swatches(ui, &mut fc, &mut has_fill, "Transparent fill: the shape stays filled, with no colour");
    st.fill = if has_fill { Some(c32_to_rgba(fc)) } else { None };
    ui.separator();
    ui.heading("Line");
    let mut has_line = st.stroke.is_some();
    let mut lc = rgba_to_c32(st.stroke.unwrap_or(Rgba::RED));
    if let Some(c) = app.eyedrop.take(Target::Line) {
        (lc, has_line) = (c, true);
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut has_line, "Show line");
        ui.color_edit_button_srgba(&mut lc);
        pick_button(app, ui, Target::Line);
    });
    swatches(ui, &mut lc, &mut has_line, "Transparent line: it's still there and still cuts, with no colour");
    st.stroke = if has_line { Some(c32_to_rgba(lc)) } else { None };
    ui.horizontal(|ui| {
        ui.label("Thickness");
        len_field(ui, &mut st.stroke_width, units);
    });
    ui.checkbox(&mut st.no_print, "Don't print (Print & Cut)");
    ui.separator();
    ui.heading("Cut style");
    let mut kind = match st.cut {
        CutStyle::NoCut => 0,
        CutStyle::Cut => 1,
        CutStyle::CutEdge => 2,
        CutStyle::Perforate { .. } => 3,
    };
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut kind, 0, "No cut");
        ui.radio_value(&mut kind, 1, "Cut");
        ui.radio_value(&mut kind, 2, "Cut edge");
        ui.radio_value(&mut kind, 3, "Perforate");
    });
    let (mut on, mut off) = match st.cut {
        CutStyle::Perforate { on, off } => (on, off),
        _ => (3.0, 1.0),
    };
    if kind == 3 {
        ui.horizontal(|ui| {
            ui.label("Cut");
            len_field(ui, &mut on, units);
            ui.label("Gap");
            len_field(ui, &mut off, units);
        });
    }
    st.cut = match kind {
        0 => CutStyle::NoCut,
        1 => CutStyle::Cut,
        2 => CutStyle::CutEdge,
        _ => CutStyle::Perforate { on: on.max(0.1), off: off.max(0.1) },
    };
    if st != before {
        // one undo step per burst of changes (e.g. dragging a colour picker)
        if !app.ps.text_editing_session(u64::MAX - 1) {
            app.checkpoint();
        }
        for id in &sel {
            if let Some(s) = app.doc.page_mut().shape_mut(*id) {
                apply_style(s, &st);
            }
        }
        app.rev += 1;
        app.dirty = true;
    }
    ui.separator();
    ui.small("Tip: Send ▸ Action by colour cuts each line colour with its own settings.");
    ui.separator();
    contour_section(app, ui);
    ui.separator();
    pattern_section(app, ui);
    None
}

fn apply_style(s: &mut cutcraft_core::doc::Shape, st: &cutcraft_core::doc::Style) {
    s.style = st.clone();
    if let ShapeKind::Group(ch) = &mut s.kind {
        for c in ch {
            apply_style(c, st);
        }
    }
}

/// Starts picking a colour from anywhere on screen.
fn pick_button(app: &mut App, ui: &mut Ui, target: crate::eyedrop::Target) {
    if !crate::eyedrop::available() {
        return;
    }
    let on = app.eyedrop.active() == Some(target);
    if ui.selectable_label(on, "⊙ Pick").on_hover_text("Pick a colour from anywhere on screen - click the colour you want, Esc cancels").clicked() && !on {
        app.eyedrop.start(target);
        app.status = "Click anywhere on screen to pick its colour - Esc cancels".into();
    }
}

/// A row of colours to click, led by a transparent one.
fn swatches(ui: &mut Ui, c: &mut Color32, on: &mut bool, clear_tip: &str) {
    const PALETTE: [(u8, u8, u8); 14] = [
        (0, 0, 0), (255, 255, 255), (128, 128, 128), (220, 30, 30), (255, 140, 0), (250, 210, 0), (40, 170, 60),
        (0, 160, 160), (30, 90, 220), (120, 60, 200), (230, 80, 160), (140, 90, 40), (255, 200, 200), (180, 220, 255),
    ];
    ui.horizontal_wrapped(|ui| {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
        // a checkerboard with a red slash
        let p = ui.painter();
        p.rect_filled(rect, 2.0, Color32::WHITE);
        for (i, j) in [(0.0, 0.0), (1.0, 1.0)] {
            let q = egui::Rect::from_min_size(rect.min + egui::vec2(i * 8.0, j * 8.0), egui::vec2(8.0, 8.0));
            p.rect_filled(q, 0.0, Color32::from_gray(200));
        }
        p.line_segment([rect.left_bottom() + egui::vec2(2.0, -2.0), rect.right_top() + egui::vec2(-2.0, 2.0)], egui::Stroke::new(1.5, Color32::from_rgb(220, 30, 30)));
        let chosen = *on && c.a() == 0;
        let ring = if chosen { ui.visuals().selection.stroke.color } else { Color32::GRAY };
        p.rect_stroke(rect, 2.0, egui::Stroke::new(if chosen { 2.0 } else { 1.0 }, ring), egui::StrokeKind::Inside);
        if resp.on_hover_text(clear_tip).clicked() {
            *c = Color32::TRANSPARENT;
            *on = true;
        }
        for (r, g, b) in PALETTE {
            let col = Color32::from_rgb(r, g, b);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 2.0, col);
            ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside);
            if resp.clicked() {
                *c = col;
                *on = true;
            }
        }
    });
}

/// Shrinks the media to fit on the page's mat; returns whether it changed.
fn fit_media_to_mat(p: &mut cutcraft_core::doc::Page) -> bool {
    let Some((mw, mh)) = p.mat.size_mm() else { return false };
    let (w, h) = (p.width.min(mw), p.height.min(mh));
    let changed = (w, h) != (p.width, p.height);
    (p.width, p.height) = (w, h);
    changed
}

/// Contour: tick which of a shape's outlines are cut — hide inner lines
/// without breaking the shape up (a live effect, so it can be undone any time).
fn contour_section(app: &mut App, ui: &mut Ui) {
    use cutcraft_core::effectstack::{self, Effect};
    use cutcraft_core::kurbo::Shape as _;
    ui.heading("Contour");
    let sel = app.editable_sel();
    let Some(id) = sel.first().copied().filter(|_| sel.len() == 1) else {
        ui.weak("Select one shape to choose which of its lines are cut.");
        return;
    };
    let Some(s) = app.doc.page().shape(id) else { return };
    if matches!(s.kind, ShapeKind::Group(_) | ShapeKind::Image(_)) {
        ui.weak("Contour works on a single path or text (convert groups first).");
        return;
    }
    // every outline, before any hiding
    let mut plain = s.clone();
    let hidden: Vec<usize> = plain.effects.iter().find_map(|e| if let Effect::HideContours { hidden } = e { Some(hidden.clone()) } else { None }).unwrap_or_default();
    plain.effects.retain(|e| !matches!(e, Effect::HideContours { .. }));
    let lines = effectstack::subpaths(&plain.world_paths());
    if lines.len() < 2 {
        ui.weak("This shape has a single outline.");
        return;
    }
    let mut new_hidden = hidden.clone();
    ui.horizontal(|ui| {
        if ui.button("Hide all inner").on_hover_text("Cut only the outside edge").clicked() {
            let biggest = lines.iter().enumerate().max_by(|a, b| a.1.bounding_box().area().total_cmp(&b.1.bounding_box().area())).map(|(i, _)| i).unwrap_or(0);
            new_hidden = (0..lines.len()).filter(|i| *i != biggest).collect();
        }
        if ui.button("Show all").clicked() {
            new_hidden.clear();
        }
    });
    egui::ScrollArea::vertical().id_salt("contours").max_height(180.0).show(ui, |ui| {
        for (i, l) in lines.iter().enumerate() {
            let mut cut = !new_hidden.contains(&i);
            let b = l.bounding_box();
            let r = ui.checkbox(&mut cut, format!("Line {} · {:.0} × {:.0} mm", i + 1, b.width(), b.height()));
            if r.hovered() {
                app.cs.highlight = Some(l.clone());
            }
            if r.changed() {
                if cut {
                    new_hidden.retain(|x| *x != i);
                } else {
                    new_hidden.push(i);
                }
            }
        }
    });
    if new_hidden != hidden {
        app.checkpoint_named("Contour");
        if let Some(s) = app.doc.page_mut().shape_mut(id) {
            s.effects.retain(|e| !matches!(e, Effect::HideContours { .. }));
            if !new_hidden.is_empty() {
                new_hidden.sort();
                s.effects.insert(0, Effect::HideContours { hidden: new_hidden });
            }
        }
    }
}

/// Pattern fill: fill a shape with a repeating picture to print (Print Then
/// Cut); the shape's outline is still cut.
fn pattern_section(app: &mut App, ui: &mut Ui) {
    ui.heading("Pattern fill");
    ui.horizontal(|ui| {
        if ui.button(if app.ps.pattern.is_some() { "Change pattern…" } else { "Choose pattern…" }).clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "gif", "webp"]).pick_file() {
                match std::fs::read(&path) {
                    Ok(b) if image::load_from_memory(&b).is_ok() => app.ps.pattern = Some(std::sync::Arc::new(b)),
                    _ => app.status = "That file isn't a picture CutCraft can read".into(),
                }
            }
        }
        if app.ps.pattern.is_some() {
            ui.label("✔ pattern loaded");
        }
    });
    egui::Grid::new("pattern").num_columns(2).show(ui, |ui| {
        ui.label("Tile width");
        ui.add(DragValue::new(&mut app.ps.pattern_tile).range(2.0..=500.0).suffix(" mm"));
        ui.end_row();
        ui.label("Angle");
        ui.add(DragValue::new(&mut app.ps.pattern_angle).range(-180.0..=180.0).suffix("°"));
        ui.end_row();
    });
    let one = app.editable_sel().len() == 1;
    if ui.add_enabled(one && app.ps.pattern.is_some(), egui::Button::new("Fill selected shape")).on_disabled_hover_text("Choose a pattern and select one closed shape").clicked() {
        let id = app.editable_sel()[0];
        let pat = app.ps.pattern.clone().unwrap();
        let (tile, angle) = (app.ps.pattern_tile, app.ps.pattern_angle);
        app.checkpoint_named("Pattern fill");
        match cutcraft_core::printflat::pattern_fill(&mut app.doc, id, &pat, tile, angle) {
            Ok(sel) => {
                app.sel = sel;
                app.status = "Pattern filled - it prints; the outline is still cut (use Print & Cut marks)".into();
            }
            Err(e) => {
                app.undo();
                app.status = format!("Pattern fill: {e}");
            }
        }
    }
}

// ------------------------------------------------------------------ layers

fn layers_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Layers");
    let mut remove = None;
    let cur = app.current_layer();
    let n = app.doc.page().layers.len();
    let now = ui.input(|i| i.time);
    // (layer, old colour, new colour) when a layer's colour is changed
    let mut recolour = None;
    for i in 0..n {
        let id = app.doc.page().layers[i].id;
        ui.horizontal(|ui| {
            if ui.radio(cur == id, "").on_hover_text("Current layer for new shapes").clicked() {
                app.ps.current_layer = id;
            }
            let l = &mut app.doc.pages[app.doc.active].layers[i];
            if crate::icons::button(ui, crate::icons::Icon::Preview, 22.0, l.visible, true, if l.visible { "Visible - click to hide" } else { "Hidden - click to show" }).clicked() {
                l.visible = !l.visible;
            }
            let lock = if l.locked { crate::icons::Icon::Lock } else { crate::icons::Icon::Unlocked };
            if crate::icons::button(ui, lock, 22.0, l.locked, true, if l.locked { "Locked - click to unlock" } else { "Unlocked - click to lock" }).clicked() {
                l.locked = !l.locked;
            }
            let mut c = rgba_to_c32(l.color);
            if ui.color_edit_button_srgba(&mut c).on_hover_text("Layer colour: shapes on this layer that use it follow it").changed() {
                recolour = Some((id, l.color, c32_to_rgba(c)));
            }
            ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(110.0));
            if n > 1 && ui.small_button("✖").clicked() {
                remove = Some(id);
            }
        });
    }
    if let Some((id, old, new)) = recolour {
        // dragging in the colour picker is one undo step
        app.checkpoint_merged("Layer colour", now);
        let p = app.doc.page_mut();
        if let Some(l) = p.layers.iter_mut().find(|l| l.id == id) {
            l.color = new;
        }
        for s in p.shapes.iter_mut().filter(|s| s.layer == id) {
            ops::recolour_outline(s, Some(old), new);
        }
    }
    if let Some(id) = remove {
        app.checkpoint();
        let p = app.doc.page_mut();
        p.layers.retain(|l| l.id != id);
        let fallback = p.layers[0].id;
        for s in p.shapes.iter_mut() {
            if s.layer == id {
                s.layer = fallback;
            }
        }
    }
    // wraps, so the buttons never widen the side panel
    ui.horizontal_wrapped(|ui| {
        if ui.button("Add layer").clicked() {
            app.checkpoint();
            let p = app.doc.page_mut();
            let id = p.layers.iter().map(|l| l.id).max().unwrap_or(0) + 1;
            let cols = [Rgba(30, 90, 220, 255), Rgba(40, 170, 60, 255), Rgba(255, 140, 0, 255), Rgba(120, 60, 200, 255)];
            p.layers.push(Layer { id, name: format!("Layer {}", id + 1), visible: true, locked: false, color: cols[id as usize % 4] });
            app.ps.current_layer = id;
        }
        if ui.button("Move selection here").on_hover_text("Shapes still in their old layer's colour take this layer's colour").clicked() {
            let l = app.current_layer();
            app.edit(|d, s| {
                let page = d.page_mut();
                let colour = |p: &cutcraft_core::doc::Page, id: u32| p.layer(id).map(|l| l.color);
                let to = colour(page, l);
                for id in s {
                    let from = page.shape(*id).and_then(|sh| colour(page, sh.layer));
                    if let Some(sh) = page.shape_mut(*id) {
                        sh.layer = l;
                        if let (Some(from), Some(to)) = (from, to) {
                            ops::recolour_outline(sh, Some(from), to);
                        }
                    }
                }
                None
            });
        }
        if ui.button("Apply layer colours").on_hover_text("Give every outlined shape its layer's colour (shapes with only a fill are left alone)").clicked() {
            app.checkpoint();
            let p = app.doc.page_mut();
            let colours: Vec<(u32, Rgba)> = p.layers.iter().map(|l| (l.id, l.color)).collect();
            let mut n = 0;
            for s in p.shapes.iter_mut() {
                if let Some((_, c)) = colours.iter().find(|(id, _)| *id == s.layer) {
                    n += ops::recolour_outline(s, None, *c) as usize;
                }
            }
            app.status = format!("{n} shape(s) recoloured to their layer's colour");
        }
    });
    ui.separator();
    ui.heading("Objects");
    let mut clicked = None;
    let mut toggle_hide = None;
    let mut toggle_lock = None;
    for s in app.doc.page().shapes.iter().rev() {
        ui.horizontal(|ui| {
            if crate::icons::button(ui, crate::icons::Icon::Preview, 20.0, !s.hidden, true, if s.hidden { "Hidden - click to show" } else { "Visible - click to hide" }).clicked() {
                toggle_hide = Some(s.id);
            }
            if crate::icons::button(ui, if s.locked { crate::icons::Icon::Lock } else { crate::icons::Icon::Unlocked }, 20.0, s.locked, true, if s.locked { "Locked - click to unlock" } else { "Unlocked - click to lock" }).clicked() {
                toggle_lock = Some(s.id);
            }
            let layer = app.doc.page().layer(s.layer).map(|l| l.name.as_str()).unwrap_or("?");
            if ui.selectable_label(app.sel.contains(&s.id), format!("{}  ·  {}", s.display_name(), layer)).clicked() {
                clicked = Some(s.id);
            }
        });
    }
    if let Some(id) = clicked {
        if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
            if let Some(p) = app.sel.iter().position(|x| *x == id) {
                app.sel.remove(p);
            } else {
                app.sel.push(id);
            }
        } else {
            app.sel = vec![id];
        }
    }
    if let Some(id) = toggle_hide {
        app.checkpoint();
        if let Some(s) = app.doc.page_mut().shape_mut(id) {
            s.hidden = !s.hidden;
        }
    }
    if let Some(id) = toggle_lock {
        app.checkpoint();
        if let Some(s) = app.doc.page_mut().shape_mut(id) {
            s.locked = !s.locked;
        }
        app.sel.retain(|x| *x != id);
    }
    if app.sel.len() == 1 {
        let id = app.sel[0];
        if let Some(s) = app.doc.page_mut().shape_mut(id) {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut s.name);
            });
        }
    }
    None
}

// ------------------------------------------------------------------ trace

/// Otsu auto-threshold for the selected image, computed on a downscaled copy.
fn auto_threshold(app: &App, img_id: cutcraft_core::doc::ShapeId) -> Option<u8> {
    let s = app.doc.page().shape(img_id)?;
    let ShapeKind::Image(img) = &s.kind else { return None };
    let dec = image::load_from_memory(&img.png).ok()?;
    let k = (1000.0 / dec.width().max(dec.height()) as f64).min(1.0);
    let dec = if k < 1.0 {
        dec.resize((dec.width() as f64 * k) as u32, (dec.height() as f64 * k) as u32, image::imageops::FilterType::Triangle)
    } else {
        dec
    };
    Some(trace::otsu_threshold(&dec.to_rgba8()))
}

fn trace_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Trace");
    let img_id = app.sel.iter().copied().find(|id| app.doc.page().shape(*id).map(|s| s.is_image()).unwrap_or(false));
    let Some(img_id) = img_id else {
        ui.label("Import an image (File ▸ Import or drag & drop) and select it.");
        app.cs.trace_preview = None;
        return None;
    };
    let before = app.ps.trace.clone();
    let mut sel = match app.ps.trace.mode {
        TraceMode::Threshold => 0,
        TraceMode::Colour(_) => 1,
        TraceMode::Subject => 2,
    };
    ui.horizontal(|ui| {
        ui.radio_value(&mut sel, 0, "Brightness");
        ui.radio_value(&mut sel, 1, "Colour");
        ui.radio_value(&mut sel, 2, "Subject");
    });
    if sel == 2 {
        ui.small("Isolates an object from a plain background - for a photo of a real item.");
    }
    let p = &mut app.ps.trace;
    if sel == 1 {
        ui.horizontal(|ui| {
            ui.label("Colour");
            ui.color_edit_button_srgba(&mut app.ps.trace_colour);
        });
        let c = app.ps.trace_colour;
        p.mode = TraceMode::Colour([c.r(), c.g(), c.b()]);
    } else if sel == 2 {
        p.mode = TraceMode::Subject;
    } else {
        p.mode = TraceMode::Threshold;
    }
    let thresh_label = match sel {
        1 => "Tolerance",
        2 => "Background tol.",
        _ => "Threshold",
    };
    egui::Grid::new("tr").num_columns(2).show(ui, |ui| {
        ui.label(thresh_label);
        ui.add(egui::Slider::new(&mut p.threshold, 0..=255));
        ui.end_row();
        ui.label("Low-pass (blur)");
        ui.add(egui::Slider::new(&mut p.blur, 0..=10));
        ui.end_row();
        ui.label("Despeckle (px)");
        ui.add(egui::Slider::new(&mut p.despeckle, 0..=500).logarithmic(true));
        ui.end_row();
        ui.label("Smoothness");
        ui.add(egui::Slider::new(&mut p.tolerance, 0.3..=5.0));
        ui.end_row();
    });
    ui.checkbox(&mut p.smooth, "Fit curves");
    ui.checkbox(&mut p.centerline, "Centre-line (single stroke - for pens/foil/engrave)");
    ui.checkbox(&mut p.invert, "Invert");
    // Auto threshold (Otsu) for brightness mode.
    if sel == 0 && ui.button("Auto threshold").clicked() {
        if let Some(t) = auto_threshold(app, img_id) {
            app.ps.trace.threshold = t;
        }
    }
    ui.checkbox(&mut app.ps.trace_live, "Live preview");
    let params = app.ps.trace.clone();
    let compute = |app: &App, outer: bool| -> Option<cutcraft_core::kurbo::BezPath> {
        let s = app.doc.page().shape(img_id)?;
        let ShapeKind::Image(img) = &s.kind else { return None };
        let dec = image::load_from_memory(&img.png).ok()?;
        // trace at most ~1500 px on the long side for speed
        let k = (1500.0 / dec.width().max(dec.height()) as f64).min(1.0);
        let dec = if k < 1.0 {
            dec.resize((dec.width() as f64 * k) as u32, (dec.height() as f64 * k) as u32, image::imageops::FilterType::Triangle)
        } else {
            dec
        };
        let mut tp = params.clone();
        tp.outer_only = outer;
        let mut path = trace::trace(&dec.to_rgba8(), &tp);
        let a = s.transform * Affine::scale_non_uniform(img.width_mm / dec.width() as f64, img.height_mm / dec.height() as f64);
        path.apply_affine(a);
        Some(path)
    };
    let stale = app.cs.trace_preview.as_ref().map(|(id, _)| *id != img_id).unwrap_or(true) || before != app.ps.trace;
    if app.ps.trace_live && stale {
        app.cs.trace_preview = compute(app, false).map(|p| (img_id, p));
    }
    if !app.ps.trace_live {
        app.cs.trace_preview = None;
    }
    let do_trace = |app: &mut App, outer: bool, detach: bool| {
        if let Some(p) = compute(app, outer) {
            if p.elements().is_empty() {
                app.status = "Trace found nothing - adjust the threshold".into();
                return;
            }
            app.checkpoint();
            let mut st = app.default_style();
            st.fill = None;
            let layer = app.current_layer();
            let id = app.doc.add_path(p, st, layer);
            if detach {
                app.doc.page_mut().shapes.retain(|s| s.id != img_id);
            }
            app.sel = vec![id];
            app.cs.trace_preview = None;
            app.status = "Traced".into();
        }
    };
    ui.horizontal_wrapped(|ui| {
        if ui.button("Trace").clicked() {
            do_trace(app, false, false);
        }
        if ui.button("Trace outer edge").clicked() {
            do_trace(app, true, false);
        }
        if ui.button("Trace & detach").clicked() {
            do_trace(app, false, true);
        }
    });
    None
}

// ------------------------------------------------------------------ effects

fn effects_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    let sel = app.editable_sel();
    let has_sel = !sel.is_empty();
    egui::CollapsingHeader::new("Rhinestones").default_open(true).show(ui, |ui| {
        egui::Grid::new("rs").num_columns(2).show(ui, |ui| {
            ui.label("Stone diameter");
            ui.horizontal(|ui| {
                len_field(ui, &mut app.ps.stone_d, units);
                for (n, d) in [("SS6", 2.0), ("SS10", 2.8), ("SS16", 3.9), ("SS20", 4.7)] {
                    if ui.small_button(n).clicked() {
                        app.ps.stone_d = d;
                    }
                }
            });
            ui.end_row();
            ui.label("Spacing");
            len_field(ui, &mut app.ps.stone_gap, units);
            ui.end_row();
            ui.label("Layout");
            egui::ComboBox::from_id_salt("sl").selected_text(["Outline", "Double outline", "Fill"][app.ps.stone_layout]).show_ui(ui, |ui| {
                for (i, n) in ["Outline", "Double outline", "Fill"].iter().enumerate() {
                    ui.selectable_value(&mut app.ps.stone_layout, i, *n);
                }
            });
            ui.end_row();
        });
        if ui.add_enabled(has_sel, egui::Button::new("Create rhinestone template")).clicked() {
            let (d, g) = (app.ps.stone_d, app.ps.stone_gap);
            let l = [StoneLayout::Outline, StoneLayout::DoubleOutline, StoneLayout::Fill][app.ps.stone_layout];
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::rhinestones(doc, &s, d, g, l)));
        }
    });
    egui::CollapsingHeader::new("Sketch fill (pens)").show(ui, |ui| {
        egui::Grid::new("hatch").num_columns(2).show(ui, |ui| {
            ui.label("Line spacing");
            len_field(ui, &mut app.ps.hatch_spacing, units);
            ui.end_row();
            ui.label("Angle");
            ui.add(DragValue::new(&mut app.ps.hatch_angle).suffix("°"));
            ui.end_row();
        });
        ui.checkbox(&mut app.ps.hatch_cross, "Cross-hatch");
        if ui.add_enabled(has_sel, egui::Button::new("Add sketch fill")).clicked() {
            let (sp, a, c) = (app.ps.hatch_spacing, app.ps.hatch_angle, app.ps.hatch_cross);
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::hatch(doc, &s, sp, a, c)));
        }
    });
    egui::CollapsingHeader::new("Stipple dots").show(ui, |ui| {
        egui::Grid::new("stip").num_columns(2).show(ui, |ui| {
            ui.checkbox(&mut app.ps.stip_use_fill, "Fill spacing");
            len_field(ui, &mut app.ps.stip_fill, units);
            ui.end_row();
            ui.checkbox(&mut app.ps.stip_use_edge, "Edge spacing");
            len_field(ui, &mut app.ps.stip_edge, units);
            ui.end_row();
            ui.label("Pattern");
            ui.horizontal(|ui| {
                ui.radio_value(&mut app.ps.stip_pattern, 0, "Grid");
                ui.radio_value(&mut app.ps.stip_pattern, 1, "Hex");
                ui.radio_value(&mut app.ps.stip_pattern, 2, "Random");
            });
            ui.end_row();
        });
        if ui.add_enabled(has_sel, egui::Button::new("Stipple selection")).clicked() {
            let f = app.ps.stip_use_fill.then_some(app.ps.stip_fill);
            let e = app.ps.stip_use_edge.then_some(app.ps.stip_edge);
            let pat = [effects::StipplePattern::Grid, effects::StipplePattern::Hex, effects::StipplePattern::Random][app.ps.stip_pattern];
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::stipple(doc, &s, f, pat, e, 7)));
        }
        let img = sel.iter().copied().find(|id| app.doc.page().shape(*id).map(|s| s.is_image()).unwrap_or(false));
        ui.horizontal(|ui| {
            ui.label("Image contrast");
            ui.add(egui::Slider::new(&mut app.ps.stip_contrast, 0.5..=3.0));
        });
        if ui.add_enabled(img.is_some(), egui::Button::new("Stipple selected image")).clicked() {
            let (sp, c) = (app.ps.stip_fill, app.ps.stip_contrast);
            let id = img.unwrap();
            app.edit(|doc, _| Some(effects::stipple_image(doc, id, sp, c)));
        }
        ui.small("Send with action “Stipple (dots)”: every dot becomes one tool plunge.");
    });
    egui::CollapsingHeader::new("Emboss fill").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Line spacing");
            len_field(ui, &mut app.ps.emb_spacing, units);
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut app.ps.emb_hatch, false, "Concentric");
            ui.radio_value(&mut app.ps.emb_hatch, true, "Hatch");
            if app.ps.emb_hatch {
                ui.add(DragValue::new(&mut app.ps.emb_angle).suffix("°"));
            }
        });
        if ui.add_enabled(has_sel, egui::Button::new("Create emboss paths")).clicked() {
            let (sp, h, a) = (app.ps.emb_spacing, app.ps.emb_hatch, app.ps.emb_angle);
            let s = sel.clone();
            let fill = if h { effects::EmbossFill::Hatch } else { effects::EmbossFill::Concentric };
            app.edit(|doc, _| Some(effects::emboss(doc, &s, sp, fill, a)));
        }
        ui.small("Purple lines: send them with the emboss tool (Send ▸ By line colour ▸ Emboss).");
    });
    egui::CollapsingHeader::new("Pop-up card").show(ui, |ui| {
        ui.label("Card folds across the middle of the page. Select the artwork that should pop up.");
        egui::Grid::new("pop").num_columns(2).show(ui, |ui| {
            ui.label("Pop-out depth");
            len_field(ui, &mut app.ps.pop_depth, units);
            ui.end_row();
            ui.label("Attachment overlap");
            len_field(ui, &mut app.ps.pop_overlap, units);
            ui.end_row();
        });
        if ui.add_enabled(has_sel, egui::Button::new("Make pop-up")).clicked() {
            let (d, o) = (app.ps.pop_depth, app.ps.pop_overlap);
            let fold = app.doc.page().height / 2.0;
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::popup(doc, &s, fold, d, o)));
            app.status = "Pop-up made: red = cut, blue dashed = fold".into();
        }
    });
    egui::CollapsingHeader::new("Contour / sticker outline").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Distance");
            len_field(ui, &mut app.ps.contour_dist, units);
            ui.color_edit_button_srgba(&mut app.ps.contour_col);
        });
        if ui.add_enabled(has_sel, egui::Button::new("Add contour")).clicked() {
            let (d, c) = (app.ps.contour_dist, c32_to_rgba(app.ps.contour_col));
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::contour(doc, &s, d, c)));
        }
    });
    egui::CollapsingHeader::new("Shadow / blackout layer").show(ui, |ui| {
        ui.small("Solid outer silhouette behind the artwork - a base layer for HTV/paper stacking.");
        ui.horizontal(|ui| {
            ui.label("Layer colour");
            ui.color_edit_button_srgba(&mut app.ps.layer_col);
        });
        if ui.add_enabled(has_sel, egui::Button::new("Add blackout layer")).clicked() {
            let c = c32_to_rgba(app.ps.layer_col);
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::blackout(doc, &s, c)));
        }
        ui.horizontal(|ui| {
            ui.label("Shadow grow");
            len_field(ui, &mut app.ps.shadow_dist, units);
        });
        if ui.add_enabled(has_sel, egui::Button::new("Add shadow layer")).clicked() {
            let (d, c) = (app.ps.shadow_dist, c32_to_rgba(app.ps.layer_col));
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::shadow_layer(doc, &s, d, c)));
        }
    });
    egui::CollapsingHeader::new("Weeding").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Border margin");
            len_field(ui, &mut app.ps.weed_margin, units);
        });
        ui.horizontal(|ui| {
            ui.label("Weed lines each way");
            ui.add(DragValue::new(&mut app.ps.weed_lines).range(0..=20));
        });
        if ui.add_enabled(has_sel, egui::Button::new("Add weed box & lines")).clicked() {
            let (m, n) = (app.ps.weed_margin, app.ps.weed_lines);
            let s = sel.clone();
            app.edit(|doc, _| Some(effects::weed(doc, &s, m, n)));
        }
    });
    egui::CollapsingHeader::new("Puzzle").show(ui, |ui| {
        egui::Grid::new("pz").num_columns(2).show(ui, |ui| {
            ui.label("Rows × columns");
            ui.horizontal(|ui| {
                ui.add(DragValue::new(&mut app.ps.puzzle_rows).range(1..=40));
                ui.add(DragValue::new(&mut app.ps.puzzle_cols).range(1..=40));
            });
            ui.end_row();
            ui.label("Knob size");
            ui.add(egui::Slider::new(&mut app.ps.puzzle_knob, 0.0..=0.4));
            ui.end_row();
            ui.label("Random seed");
            ui.add(DragValue::new(&mut app.ps.puzzle_seed));
            ui.end_row();
        });
        if ui.button(if has_sel { "Puzzle over selection area" } else { "Puzzle over page" }).clicked() {
            let area = ops::selection_bbox(&app.doc, &sel).unwrap_or_else(|| {
                let p = app.doc.page();
                Rect::new(10.0, 10.0, p.width - 10.0, p.height - 10.0)
            });
            let (r, c, k, sd) = (app.ps.puzzle_rows, app.ps.puzzle_cols, app.ps.puzzle_knob, app.ps.puzzle_seed);
            app.edit(|doc, _| Some(effects::puzzle(doc, area, r, c, k, sd)));
        }
    });
    egui::CollapsingHeader::new("Nesting").show(ui, |ui| {
        egui::Grid::new("nest").num_columns(2).show(ui, |ui| {
            ui.label("Padding");
            len_field(ui, &mut app.ps.nest_pad, units);
            ui.end_row();
            ui.label("Page margin");
            len_field(ui, &mut app.ps.nest_margin, units);
            ui.end_row();
        });
        ui.checkbox(&mut app.ps.nest_rotate, "Allow 90° rotation");
        if ui.add_enabled(sel.len() > 1, egui::Button::new("Nest selection")).clicked() {
            let (p, m, r) = (app.ps.nest_pad, app.ps.nest_margin, app.ps.nest_rotate);
            let s = sel.clone();
            let mut ok = true;
            app.edit(|doc, _| {
                ok = effects::nest(doc, &s, p, r, m);
                None
            });
            app.status = if ok { "Nested".into() } else { "Nested - some shapes don't fit on the page".into() };
        }
        ui.small("Packs the real outlines so shapes tuck into each other; rotation tries 90° steps.");
    });
    egui::CollapsingHeader::new("Tile / slice large designs").show(ui, |ui| {
        egui::Grid::new("tile").num_columns(2).show(ui, |ui| {
            ui.label("Tile width");
            len_field(ui, &mut app.ps.tile_w, units);
            ui.end_row();
            ui.label("Tile height");
            len_field(ui, &mut app.ps.tile_h, units);
            ui.end_row();
            ui.label("Seams");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.ps.tile_joint, 0, "Straight");
                ui.selectable_value(&mut app.ps.tile_joint, 1, "Dovetail");
                ui.selectable_value(&mut app.ps.tile_joint, 2, "Puzzle");
            });
            ui.end_row();
            if app.ps.tile_joint == 0 {
                ui.label("Overlap");
                len_field(ui, &mut app.ps.tile_overlap, units);
                ui.end_row();
            } else {
                ui.label("Joint size");
                len_field(ui, &mut app.ps.tile_joint_size, units);
                ui.end_row();
                ui.label("Joint spacing");
                len_field(ui, &mut app.ps.tile_joint_spacing, units);
                ui.end_row();
            }
        });
        if app.ps.tile_joint != 0 {
            ui.small("Seams are cut as interlocking joints so the pieces lock together - for big signs and wall art.");
        }
        if ui.button("Split page into tile pages").clicked() {
            let (w, h) = (app.ps.tile_w, app.ps.tile_h);
            app.checkpoint();
            let n = match app.ps.tile_joint {
                0 => effects::tile_to_pages(&mut app.doc, w, h, app.ps.tile_overlap),
                k => {
                    let joint = if k == 1 { cutcraft_core::tiles::Joint::Dovetail } else { cutcraft_core::tiles::Joint::Puzzle };
                    effects::tile_to_pages_jointed(&mut app.doc, w, h, joint, app.ps.tile_joint_size, app.ps.tile_joint_spacing)
                }
            };
            app.status = format!("Created {n} tile page(s) - see the page tabs");
        }
    });
    None
}

// ------------------------------------------------------------------ print & cut

fn printcut_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Registration marks");
    let mut r: RegMarks = app.doc.page().regmarks.clone();
    let before = r.clone();
    ui.checkbox(&mut r.enabled, "Enable registration marks");
    egui::Grid::new("rm").num_columns(2).show(ui, |ui| {
        ui.label("Mark length");
        len_field(ui, &mut r.length, units);
        ui.end_row();
        ui.label("Thickness");
        len_field(ui, &mut r.thickness, units);
        ui.end_row();
        ui.label("Inset from edge");
        len_field(ui, &mut r.inset, units);
        ui.end_row();
    });
    if r != before {
        app.checkpoint();
        app.doc.page_mut().regmarks = r;
    }
    ui.separator();
    ui.label("Workflow:");
    ui.label("1. Keep artwork inside the dashed orange safe area.");
    ui.label("2. Export the print file and print it at 100% scale.");
    ui.label("3. Load the printed sheet and Send - the cutter scans the marks first.");
    let mut act = None;
    if ui.button("Export print PDF…").clicked() {
        act = Some(Action::ExportPdf);
    }
    ui.small("Shapes marked “Don't print” and all cut-only lines are left out of the print.");
    ui.separator();
    ui.heading("Calibration");
    match app.prefs.print_cut_calibration.get(&app.doc.machine) {
        Some(c) => ui.label(format!("Calibrated for this machine: {}.", c.describe(app.doc.page()))),
        None => ui.label("If cuts land a little off the print every time, calibrate to correct it."),
    };
    if ui.button("Calibrate Print & Cut…").on_hover_text("Print and cut a test sheet, photograph it, and CutCraft corrects the offset").clicked() {
        crate::calibrate::open(app);
    }
    act
}

// ------------------------------------------------------------------ send

fn send_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Send to cutter");
    let mut act = None;
    // ---- grouping mode
    let mut mode = app.plan.mode;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut mode, GroupMode::Simple, "Simple");
        ui.selectable_value(&mut mode, GroupMode::ByColor, "By line colour");
        ui.selectable_value(&mut mode, GroupMode::ByFill, "By fill colour");
        ui.selectable_value(&mut mode, GroupMode::ByLayer, "By layer");
    });
    app.plan.mode = mode;
    cutjob::sync_plan(&mut app.plan, app.doc.page());
    let mut mats = materials::builtin();
    mats.extend(app.prefs.user_materials.clone());
    let m = machines::by_id(&app.doc.machine);
    // a Brother ScanNCut sets force, speed and blade itself: only what each group does matters
    let brother = is_brother(app);
    let mut changed = false;
    for (gi, g) in app.plan.groups.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut g.enabled, RichText::new(&g.label).strong()).changed();
                if let Some(c) = Rgba::from_hex(&g.key) {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, c32(c));
                }
            });
            if !g.enabled {
                return;
            }
            egui::Grid::new(("grp", gi)).num_columns(2).show(ui, |ui| {
                ui.label("Material");
                egui::ComboBox::from_id_salt(("mat", gi)).selected_text(&g.material).width(180.0).show_ui(ui, |ui| {
                    for mat in &mats {
                        if ui.selectable_label(g.material == mat.name, format!("{}  ({})", mat.name, mat.category)).clicked() {
                            g.material = mat.name.clone();
                            g.settings = mat.settings.clone();
                            // iron-on is cut from the back: mirror it
                            g.mirror = is_iron_on(mat);
                            changed = true;
                        }
                    }
                });
                ui.end_row();
                ui.label("Action");
                egui::ComboBox::from_id_salt(("act", gi)).selected_text(g.action.label()).show_ui(ui, |ui| {
                    for a in CutAction::ALL {
                        changed |= ui.selectable_value(&mut g.action, a, a.label()).changed();
                    }
                });
                ui.end_row();
                ui.label("Tool");
                egui::ComboBox::from_id_salt(("tool", gi)).selected_text(g.settings.tool.label()).show_ui(ui, |ui| {
                    for t in CutTool::ALL {
                        changed |= ui.selectable_value(&mut g.settings.tool, t, t.label()).changed();
                    }
                });
                ui.end_row();
                ui.label("Mirror");
                changed |= ui
                    .checkbox(&mut g.mirror, "Cut mirror-image")
                    .on_hover_text("For iron-on / heat-transfer vinyl and infusible ink, which are cut from the back (ticked automatically for those materials)")
                    .changed();
                ui.end_row();
                if brother {
                    return;
                }
                if m.tool_slots > 1 {
                    ui.label("Carriage");
                    ui.horizontal(|ui| {
                        changed |= ui.radio_value(&mut g.slot, 1, "1").changed();
                        changed |= ui.radio_value(&mut g.slot, 2, "2").changed();
                    });
                    ui.end_row();
                }
                ui.label("Force");
                changed |= ui.add(egui::Slider::new(&mut g.settings.force, 1..=m.max_force)).changed();
                ui.end_row();
                ui.label("Speed");
                changed |= ui.add(egui::Slider::new(&mut g.settings.speed, 1..=10)).changed();
                ui.end_row();
                if g.settings.tool.is_blade() {
                    ui.label("Blade depth");
                    changed |= ui.add(egui::Slider::new(&mut g.settings.depth, 0..=10)).changed();
                    ui.end_row();
                }
                ui.label("Passes");
                changed |= ui.add(egui::Slider::new(&mut g.settings.passes, 1..=10)).changed();
                ui.end_row();
                ui.label("Overcut (mm)");
                changed |= ui.add(DragValue::new(&mut g.settings.overcut).range(0.0..=5.0).speed(0.05)).changed();
                ui.end_row();
            });
            if brother {
                return;
            }
            changed |= ui
                .checkbox(&mut g.settings.blade_compensation, "Software blade-offset compensation")
                .on_hover_text("Most machines compensate in firmware; enable only for generic plotters")
                .changed();
        });
    }
    if !brother {
        ui.horizontal(|ui| {
            ui.label("Copies");
            changed |= ui.add(DragValue::new(&mut app.plan.copies).range(1..=100)).changed();
            changed |= ui.checkbox(&mut app.plan.clip_to_page, "Only cut inside media").changed();
        });
        ui.checkbox(&mut app.ps.feed_after, "Feed media out when done");
        ui.checkbox(&mut app.prefs.track_enhancing, "Track enhancing");
    }
    if ui.button("Save current settings as a material…").clicked() {
        if let Some(g) = app.plan.groups.first() {
            let mut mat = materials::Material { name: format!("{} (custom)", g.material), category: "User".into(), settings: g.settings.clone(), user: true };
            let mut i = 2;
            while mats.iter().any(|x| x.name == mat.name) {
                mat.name = format!("{} (custom {i})", g.material);
                i += 1;
            }
            app.status = format!("Saved material “{}”", mat.name);
            app.prefs.user_materials.push(mat);
            let _ = app.prefs.save();
        }
    }

    // ---- preview & statistics
    ui.separator();
    if changed || app.cs.preview_job.is_none() {
        app.cs.preview_job = Some(cutjob::generate(app.doc.page(), &app.plan));
    }
    if let Some(job) = &app.cs.preview_job {
        let st = job.stats();
        let n: usize = job.passes.iter().map(|p| p.paths.len()).sum();
        ui.label(format!(
            "{} paths · cut {:.2} m · travel {:.2} m · ≈ {}:{:02} min",
            n,
            st.cut_mm / 1000.0,
            st.travel_mm / 1000.0,
            (st.seconds / 60.0) as u64,
            (st.seconds % 60.0) as u64
        ));
        egui::CollapsingHeader::new("Cost estimate").show(ui, |ui| {
            let c = &mut app.ps.x.cost;
            egui::Grid::new("cost").num_columns(2).show(ui, |ui| {
                ui.label("Material $/m²");
                ui.add(egui::DragValue::new(&mut c.material_price_per_m2).speed(0.5).range(0.0..=1e5).prefix("$"));
                ui.end_row();
                ui.label("Labour $/hour");
                ui.add(egui::DragValue::new(&mut c.labor_per_hour).speed(0.5).range(0.0..=1e5).prefix("$"));
                ui.end_row();
                ui.label("Pieces");
                ui.add(egui::DragValue::new(&mut c.pieces).range(1..=100000));
                ui.end_row();
                ui.label("Waste %");
                let mut pct = (c.waste_factor - 1.0) * 100.0;
                if ui.add(egui::DragValue::new(&mut pct).speed(1.0).range(0.0..=200.0).suffix("%")).changed() {
                    c.waste_factor = 1.0 + pct / 100.0;
                }
                ui.end_row();
            });
            let e = job.estimate(&app.ps.x.cost);
            ui.label(format!("Material used: {:.0} cm²  ({:.3} m²)", e.used_area_mm2 / 100.0, e.used_area_mm2 / 1e6));
            ui.label(format!("Material ${:.2}  ·  Labour ${:.2}", e.material_cost, e.labor_cost));
            ui.strong(format!("Total ${:.2}   ·   ${:.2} / piece", e.total, e.per_piece));
        });
    }
    ui.horizontal(|ui| {
        if ui.button(if app.cs.preview_playing { "⏸ Pause preview" } else { "▶ Preview cut order" }).clicked() {
            if app.cs.preview_t >= 1.0 {
                app.cs.preview_t = 0.0;
            }
            app.cs.preview_playing = !app.cs.preview_playing;
            app.cs.preview_job = Some(cutjob::generate(app.doc.page(), &app.plan));
        }
        ui.add(egui::Slider::new(&mut app.cs.preview_t, 0.0..=1.0).show_value(false));
    });

    // ---- protocol (none to choose for a Brother machine)
    if !brother {
        ui.separator();
        ui.heading("Protocol");
        ui.horizontal(|ui| {
            ui.label("Language:");
            egui::ComboBox::from_id_salt("proto")
                .selected_text(gpgl::Protocol::ALL.iter().find(|(p, _)| *p == protocol(app)).map(|(_, n)| *n).unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (p, name) in gpgl::Protocol::ALL {
                        // Brother's file format comes with choosing a Brother machine
                    if p != gpgl::Protocol::Fcm {
                        ui.selectable_value(&mut app.prefs.protocol, p, name);
                    }
                    }
                });
        });
        if protocol(app) == gpgl::Protocol::Gpgl {
            ui.horizontal(|ui| {
                ui.label("GPGL dialect:");
                egui::ComboBox::from_id_salt("dialect")
                    .selected_text(gpgl::GpglDialect::ALL.iter().find(|(d, _)| *d == app.prefs.gpgl_dialect).map(|(_, n)| *n).unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for (d, name) in gpgl::GpglDialect::ALL {
                            ui.selectable_value(&mut app.prefs.gpgl_dialect, d, name);
                        }
                    });
            });
            if app.prefs.gpgl_dialect == gpgl::GpglDialect::Graphtec {
                ui.small("Experimental Graphtec command forms - run a test cut first.");
            }
        } else {
            ui.small("HP-GL output for non-Silhouette cutters (Roland, GCC, USCutter, …).");
        }

    }

    // ---- connection (a Brother machine has none: its job is saved as a file)
    ui.separator();
    if brother {
        ui.heading("Brother ScanNCut");
        ui.label("Connect the ScanNCut by USB and Send, or save an .fcm file for CanvasWorkspace or a USB stick.");
        ui.small(format!(
            "Uses the {} mat's cut area ({:.0} × {:.0} mm). Cut groups cut; Sketch/Score groups draw with the pen; Emboss groups emboss. The machine does its own blade compensation.",
            if app.doc.page().mat == cutcraft_core::doc::MatKind::Mat12x24 { "12 × 24 in" } else { "12 × 12 in" },
            cutcraft_core::fcm::cut_area_mm(app.doc.page().mat).0,
            cutcraft_core::fcm::cut_area_mm(app.doc.page().mat).1
        ));
    }
    if !brother {
        ui.heading("Connection");
        ui.horizontal(|ui| {
            ui.radio_value(&mut app.ps.conn, Conn::Usb, "USB");
            ui.radio_value(&mut app.ps.conn, Conn::Bluetooth, "Bluetooth");
            ui.radio_value(&mut app.ps.conn, Conn::Tcp, "Network");
            ui.radio_value(&mut app.ps.conn, Conn::File, "File");
        });
        match app.ps.conn {
            Conn::Usb => {
                ui.horizontal(|ui| {
                    if ui.button("⟳ Scan").clicked() || app.ps.usb_devices.is_empty() && app.ps.device_info.is_empty() {
                        let mut all = device::list_winusb();
                        all.extend(device::list_usb());
                        all.extend(device::list_nusb()); // cross-platform (Linux/macOS/Windows)
                        // cutters (vid_0b4d) first, WinUSB ahead of printer-class
                        all.sort_by_key(|d| (!d.path.to_lowercase().contains("0b4d"), !d.winusb));
                        app.ps.usb_devices = all;
                        app.ps.usb_idx = 0;
                        app.ps.device_info = format!("{} USB device(s) found", app.ps.usb_devices.len());
                    }
                });
                ui.add(egui::Label::new(&app.ps.device_info).wrap());
                if !app.ps.usb_devices.is_empty() {
                    let idx = app.ps.usb_idx.min(app.ps.usb_devices.len() - 1);
                    egui::ComboBox::from_id_salt("usb").selected_text(&app.ps.usb_devices[idx].description).width(260.0).show_ui(ui, |ui| {
                        for (i, d) in app.ps.usb_devices.iter().enumerate() {
                            ui.selectable_value(&mut app.ps.usb_idx, i, format!("{} - {}", d.description, short(&d.path)));
                        }
                    });
                }
            }
            Conn::Bluetooth => crate::panels2::bluetooth_section(app, ui),
            Conn::Tcp => {
                if app.ps.tcp_addr.is_empty() {
                    app.ps.tcp_addr = app.prefs.last_tcp_address.clone();
                }
                ui.horizontal(|ui| {
                    ui.label("Address");
                    ui.text_edit_singleline(&mut app.ps.tcp_addr);
                });
            }
            Conn::File => {
                ui.small("Writes the GPGL stream to a file instead of a device.");
            }
        }
        if app.ps.conn != Conn::File && ui.button("Query status & firmware").clicked() {
            match open_transport(app) {
                Ok(Some(mut t)) => {
                    let st = device::query_status(t.as_mut()).map(|s| format!("{s:?}")).unwrap_or_else(|e| e.to_string());
                    let fw = device::query_firmware(t.as_mut()).unwrap_or_else(|e| e.to_string());
                    app.ps.device_info = format!("Status: {st} · Firmware: {fw}");
                }
                Ok(None) => {}
                Err(e) => app.ps.device_info = format!("Error: {e}"),
            }
        }
    }

    // ---- send buttons
    ui.separator();
    let prog = app.progress.lock().map(|p| p.clone()).unwrap_or_default();
    if app.sending && prog.done {
        app.sending = false;
        app.status = prog.message.clone();
    }
    ui.horizontal(|ui| {
        let label = if brother { "✂ Send to ScanNCut" } else { "✂ Send" };
        let send = ui.add_enabled(!app.sending, egui::Button::new(RichText::new(label).strong().size(16.0)));
        if send.on_hover_text(if brother { "Send the job to the ScanNCut over USB - put the machine on its home screen first" } else { "Send the job to the cutter" }).clicked() {
            start_send(app, false);
        }
        if ui.add_enabled(!app.sending, egui::Button::new("Test cut")).on_hover_text(if brother { "Sends a small square-with-triangle test cut to the ScanNCut" } else { "Cuts a small square-with-triangle at the origin" }).clicked() {
            start_send(app, true);
        }
        if brother && ui.add_enabled(!app.sending, egui::Button::new("💾 Save .fcm…")).on_hover_text("Save the job as a Brother ScanNCut file for CanvasWorkspace or a USB stick").clicked() {
            let (page, plan) = (app.doc.page().clone(), app.plan.clone());
            app.save_fcm(&page, &plan, "design.fcm");
        }
        if ui.add_enabled(app.sending, egui::Button::new("Cancel")).clicked() {
            app.cancel.store(true, Ordering::Relaxed);
        }
    });
    if app.sending || prog.done {
        let frac = if prog.total > 0 { prog.sent as f32 / prog.total as f32 } else { 0.0 };
        ui.add(egui::ProgressBar::new(frac).show_percentage());
        ui.label(&prog.message);
        if let Some(e) = &prog.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
    }
    if brother {
        ui.small("Connect the ScanNCut by USB and put it on its home screen, then Send. Do a test cut on scrap before cutting good material.");
    } else {
        ui.horizontal(|ui| {
            if ui.button("Export GPGL…").clicked() {
                act = Some(Action::ExportGpgl);
            }
            if ui.button("Export PLT…").clicked() {
                act = Some(Action::ExportPlt);
            }
        });
        ui.small("⚠ The cutter command set follows public GPGL documentation. Do a test cut on scrap before cutting good material.");
    }
    act
}

fn short(p: &str) -> String {
    if p.len() > 40 { format!("…{}", &p[p.len() - 38..]) } else { p.to_string() }
}

fn open_transport(app: &mut App) -> anyhow::Result<Option<Box<dyn Transport>>> {
    Ok(Some(match app.ps.conn {
        Conn::Usb => {
            if app.ps.usb_devices.is_empty() {
                app.ps.usb_devices = device::list_winusb();
                app.ps.usb_devices.extend(device::list_usb());
                app.ps.usb_devices.extend(device::list_nusb());
            }
            let d = app.ps.usb_devices.get(app.ps.usb_idx).cloned().ok_or_else(|| anyhow::anyhow!("No USB cutter found - check the cable and power, then Scan"))?;
            if let Some(ids) = d.path.strip_prefix("nusb:") {
                let (v, p) = ids.split_once(':').ok_or_else(|| anyhow::anyhow!("bad nusb id"))?;
                let vid = u16::from_str_radix(v, 16)?;
                let pid = u16::from_str_radix(p, 16)?;
                Box::new(device::NusbTransport::open(vid, pid)?)
            } else if d.winusb {
                Box::new(device::WinUsbTransport::open(&d)?)
            } else {
                Box::new(device::UsbTransport::open(&d)?)
            }
        }
        Conn::Tcp => {
            app.prefs.last_tcp_address = app.ps.tcp_addr.clone();
            Box::new(device::TcpTransport::connect(&app.ps.tcp_addr)?)
        }
        Conn::Bluetooth => crate::panels2::bluetooth_opener(app)?()?,
        Conn::File => {
            let Some(p) = rfd::FileDialog::new().add_filter("GPGL", &["gpgl"]).set_file_name("job.gpgl").save_file() else {
                return Ok(None);
            };
            Box::new(device::FileTransport::create(p)?)
        }
    }))
}

enum Payload {
    Raw(Vec<u8>),
    Staged(gpgl::Staged),
}

/// Iron-on / heat-transfer and infusible-ink materials are cut mirrored.
pub fn is_iron_on(m: &materials::Material) -> bool {
    let s = format!("{} {}", m.name, m.category).to_lowercase();
    ["heat transfer", "iron-on", "iron on", "htv", "infusible"].iter().any(|k| s.contains(k))
}

/// A Brother ScanNCut is selected: jobs go as .fcm files, over USB or saved.
pub fn is_brother(app: &App) -> bool {
    machines::by_id(&app.doc.machine).protocol == gpgl::Protocol::Fcm
}

/// The wire language to send in: the chosen one, except that Brother's file
/// format only goes with a Brother machine (an old saved choice falls back
/// to the machine's own language).
fn protocol(app: &App) -> gpgl::Protocol {
    match app.prefs.protocol {
        gpgl::Protocol::Fcm => machines::by_id(&app.doc.machine).protocol,
        p => p,
    }
}

fn start_send(app: &mut App, test: bool) {
    if is_brother(app) {
        let (page, plan) = if test {
            let (d, plan) = test_cut_doc(app);
            (d.page().clone(), plan)
        } else {
            (app.doc.page().clone(), app.plan.clone())
        };
        app.send_fcm_usb(&page, &plan);
        return;
    }
    let job = if test {
        let (d, plan) = test_cut_doc(app);
        cutjob::generate(d.page(), &plan)
    } else {
        cutjob::generate(app.doc.page(), &app.plan)
    };
    send_job(app, job, test);
}

/// Sends a ready cut job to the connected cutter (USB, network or Bluetooth).
pub fn send_job(app: &mut App, job: cutjob::CutJob, test: bool) {
    if job.is_empty() {
        app.status = "Nothing to cut - give shapes a cut style and enable a group".into();
        return;
    }
    // a pen part then a blade part on one carriage: pause between them
    let (first, last) = crate::wizards::queue_tool_changes(app, job);
    send_part(app, first, test, last);
    if !last {
        app.status = format!("{} - you'll be asked to change tools between parts", app.status);
    }
}

/// Sends one part of a job; only the last part feeds the media out.
pub fn send_part(app: &mut App, mut job: cutjob::CutJob, test: bool, last: bool) {
    let mut opt = job_options(app);
    if test {
        opt.regmarks = None;
        opt.feed_after = false;
    }
    // Print & Cut: move the lines by this machine's calibration (but not
    // while cutting the calibration sheet itself)
    let calibrating = app.calib.is_some() || cutcraft_core::calibration::is_sheet(app.doc.page());
    if opt.regmarks.is_some() && !calibrating {
        if let Some(c) = app.prefs.print_cut_calibration.get(&app.doc.machine) {
            cutcraft_core::calibration::correct_job(&mut job, c);
        }
    }
    if !last {
        opt.feed_after = false;
    }
    // Use the interactive registration-mark workflow when the Graphtec dialect
    // has reg-marks enabled; otherwise stream the whole job in one shot.
    let staged = protocol(app) == gpgl::Protocol::Gpgl
        && app.prefs.gpgl_dialect == gpgl::GpglDialect::Graphtec
        && opt.regmarks.is_some();
    let payload = if staged {
        Payload::Staged(gpgl::encode_staged(&job, &opt))
    } else {
        Payload::Raw(match protocol(app) {
            gpgl::Protocol::Gpgl => gpgl::encode(&job, &opt),
            gpgl::Protocol::Hpgl => gpgl::encode_hpgl(&job, app.doc.page().height).into_bytes(),
            gpgl::Protocol::Dmpl => gpgl::encode_dmpl(&job, app.doc.page().height).into_bytes(),
            gpgl::Protocol::Fcm => unreachable!("saved as a file above"),
        })
    };
    if app.ps.conn == Conn::Bluetooth {
        // Bluetooth connects slowly: open the link on the worker thread
        match crate::panels2::bluetooth_opener(app) {
            Ok(open) => {
                app.cancel.store(false, Ordering::Relaxed);
                *app.progress.lock().unwrap() = Progress::default();
                app.sending = true;
                match payload {
                    Payload::Raw(b) => device::spawn_send_lazy(open, b, app.progress.clone(), app.cancel.clone()),
                    Payload::Staged(s) => device::spawn_send_staged_lazy(open, s, app.progress.clone(), app.cancel.clone()),
                };
                app.status = "Connecting over Bluetooth…".into();
            }
            Err(e) => app.status = format!("Bluetooth: {e}"),
        }
        return;
    }
    match open_transport(app) {
        Ok(Some(t)) => {
            app.cancel.store(false, Ordering::Relaxed);
            *app.progress.lock().unwrap() = Progress::default();
            app.sending = true;
            match payload {
                Payload::Raw(b) => device::spawn_send(t, b, app.progress.clone(), app.cancel.clone()),
                Payload::Staged(s) => device::spawn_send_staged(t, s, app.progress.clone(), app.cancel.clone()),
            };
            app.status = if test { "Test cut started".into() } else { "Sending job…".into() };
        }
        Ok(None) => {}
        Err(e) => app.status = format!("Connection failed: {e}"),
    }
}

/// The small square-and-triangle test cut, with the first enabled group's settings.
fn test_cut_doc(app: &App) -> (cutcraft_core::doc::Document, cutjob::JobPlan) {
    let mut d = cutcraft_core::doc::Document::default();
    d.page_mut().mat = app.doc.page().mat;
    let sq = cutcraft_core::geom::rect_path(Rect::new(5.0, 5.0, 15.0, 15.0));
    let tri = cutcraft_core::geom::polyline_path(&[Point::new(7.0, 13.0), Point::new(10.0, 7.0), Point::new(13.0, 13.0)], true);
    d.add_path(sq, Default::default(), 0);
    d.add_path(tri, Default::default(), 0);
    let mut plan = app.plan.clone();
    plan.mode = GroupMode::Simple;
    plan.copies = 1;
    cutjob::sync_plan(&mut plan, d.page());
    if let Some(g) = app.plan.groups.iter().find(|g| g.enabled) {
        plan.groups[0].settings = g.settings.clone();
        plan.groups[0].slot = g.slot;
    }
    (d, plan)
}

// ------------------------------------------------------------------ preferences

fn prefs_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Preferences");
    let before = app.prefs.clone();
    ui.horizontal(|ui| {
        ui.label("Units");
        ui.radio_value(&mut app.prefs.units, Units::Mm, "Millimetres");
        ui.radio_value(&mut app.prefs.units, Units::Inch, "Inches");
    });
    if ui.checkbox(&mut app.prefs.dark_mode, "Dark theme").changed() {
        crate::chrome::apply_theme(ui.ctx(), app.prefs.dark_mode);
    }
    ui.checkbox(&mut app.prefs.show_rulers, "Show rulers");
    egui::Grid::new("prefs").num_columns(2).show(ui, |ui| {
        ui.label("Nudge distance");
        ui.add(DragValue::new(&mut app.prefs.nudge_mm).suffix(" mm").range(0.01..=50.0).speed(0.1));
        ui.end_row();
        ui.label("Paste / duplicate offset");
        ui.add(DragValue::new(&mut app.prefs.paste_offset_mm).suffix(" mm").range(0.0..=50.0).speed(0.1));
        ui.end_row();
        ui.label("Back up unsaved work every");
        ui.add(DragValue::new(&mut app.prefs.autosave_minutes).suffix(" min").range(0..=120))
            .on_hover_text("For crash recovery: if CutCraft closes unexpectedly, the backup is offered next time it starts. 0 = off.");
        ui.end_row();
        ui.label("Default font");
        ui.text_edit_singleline(&mut app.prefs.default_font);
        ui.end_row();
        ui.label("Default text size");
        ui.add(DragValue::new(&mut app.prefs.default_text_size).suffix(" mm").range(1.0..=500.0));
        ui.end_row();
    });
    if !app.prefs.user_materials.is_empty() {
        ui.separator();
        ui.heading("My materials");
        let mut remove = None;
        for (i, m) in app.prefs.user_materials.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut m.name);
                if ui.small_button("✖").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            app.prefs.user_materials.remove(i);
        }
    }
    if app.prefs != before {
        let _ = app.prefs.save();
    }
    ui.separator();
    ui.small(format!("Settings folder: {}", cutcraft_core::prefs::config_dir().display()));
    None
}
