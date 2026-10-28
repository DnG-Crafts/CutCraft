//! Design canvas: rendering, view navigation and all interactive tools.

use crate::app::{App, Tool};
use crate::panels::{ImageMode, Panel};
use crate::render;
use cutcraft_core::cutjob::CutJob;
use cutcraft_core::doc::{Shape, ShapeId, ShapeKind, TextData};
use cutcraft_core::geom::{self, Polyline};
use cutcraft_core::kurbo::{self, Affine, BezPath, ParamCurveNearest, PathEl, PathSeg, Point, Rect, Vec2};
use cutcraft_core::ops;
use egui::{Color32, CursorIcon, Key, PointerButton, Pos2, Sense, Stroke, StrokeKind};

pub struct View {
    /// Screen pixels per millimetre.
    pub zoom: f32,
    /// Offset of the page origin from the canvas top-left, px.
    pub pan: egui::Vec2,
    pub fit_requested: bool,
    pub fit_rect: Option<Rect>,
    pub canvas: egui::Rect,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 2.0, pan: egui::vec2(40.0, 40.0), fit_requested: true, fit_rect: None, canvas: egui::Rect::NOTHING }
    }
}

impl View {
    pub fn to_screen(&self, p: Point) -> Pos2 {
        self.canvas.min + self.pan + egui::vec2(p.x as f32, p.y as f32) * self.zoom
    }
    pub fn to_mm(&self, s: Pos2) -> Point {
        let v = (s - self.canvas.min - self.pan) / self.zoom;
        Point::new(v.x as f64, v.y as f64)
    }
    pub fn zoom_by(&mut self, f: f32, about: Option<Pos2>) {
        let about = about.unwrap_or(self.canvas.center());
        let mm = self.to_mm(about);
        self.zoom = (self.zoom * f).clamp(0.05, 200.0);
        let now = self.to_screen(mm);
        self.pan += about - now;
    }
    fn fit(&mut self, r: Rect) {
        if !self.canvas.is_positive() || r.width() <= 0.0 || r.height() <= 0.0 {
            return;
        }
        let m = 30.0;
        let zx = (self.canvas.width() - 2.0 * m) / r.width() as f32;
        let zy = (self.canvas.height() - 2.0 * m) / r.height() as f32;
        self.zoom = zx.min(zy).clamp(0.05, 200.0);
        let c = egui::vec2(r.center().x as f32, r.center().y as f32) * self.zoom;
        self.pan = self.canvas.size() / 2.0 - c;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeHandle {
    End(usize),
    C1(usize),
    C2(usize),
}

#[derive(Default)]
pub enum Drag {
    #[default]
    None,
    Pan,
    Move { start: Point, orig: Vec<(ShapeId, Affine)> },
    Scale { handle: usize, bbox: Rect, orig: Vec<(ShapeId, Affine)> },
    Rotate { center: Point, a0: f64, orig: Vec<(ShapeId, Affine)> },
    Rubber { start: Point },
    Create { start: Point },
    Freehand { pts: Vec<Point> },
    Knife { start: Point },
    /// Shape builder: the faces dragged across so far.
    Build { picked: Vec<usize> },
    Eraser { pts: Vec<Point> },
    ImageErase { pts: Vec<Point> },
    /// Image tool in Crop mode: the box being dragged.
    ImageCrop { start: Point },
    Node { h: NodeHandle, id: ShapeId },
    WarpPt { id: ShapeId, idx: usize },
    ZoomRect { start: Point },
    /// Dragging a ruler guide: a new one (`index` None) or an existing one.
    Guide { index: Option<usize>, vertical: bool },
    Measure { start: Point },
    Dimension { start: Point },
}

#[derive(Default)]
pub struct CanvasState {
    pub drag: Drag,
    pub poly: Vec<Point>,
    pub node_sel: Option<NodeHandle>,
    pub cursor_mm: Option<Point>,
    pub show_cut_lines_only: bool,
    pub preview_job: Option<CutJob>,
    pub preview_t: f32,
    pub preview_playing: bool,
    pub trace_preview: Option<(ShapeId, BezPath)>,
    /// Smart-guide targets (x lines, y lines) for the move in progress.
    pub guide_targets: Option<(Vec<f64>, Vec<f64>)>,
    /// Guide lines to draw this frame: (vertical?, position mm).
    pub alignments: Vec<(bool, f64)>,
    /// Object-snap points (other shapes' corners, centres and nodes) and the
    /// (revision, selection) they were built for.
    pub snap_pts: Vec<Point>,
    pub snap_key: Option<(u64, Vec<ShapeId>)>,
    /// Live drawing symmetry (View ▸ Symmetry).
    pub symmetry: Symmetry,
    /// A cut line to highlight (the Contour list's hovered line), page space.
    pub highlight: Option<BezPath>,
    /// The last measurement (Measure tool), kept on screen until the next.
    pub measured: Option<(Point, Point)>,
    /// Shape builder: the faces of the selected shapes, and what they were worked out for.
    pub builder: Option<((u64, Vec<ShapeId>), Vec<cutcraft_core::shapebuilder::Face>)>,
}

/// Live symmetry while drawing: new shapes are repeated as mirror images or
/// around a centre, and the set is grouped. Ported from BurnCraft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Symmetry {
    pub mode: SymmetryMode,
    /// Copies around the centre (radial mode).
    pub count: u32,
    /// Radial mode: also mirror every other slice (like a kaleidoscope).
    pub mirror: bool,
    pub center: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymmetryMode {
    Off,
    /// Mirrored left–right about a vertical line.
    LeftRight,
    /// Mirrored top–bottom about a horizontal line.
    TopBottom,
    /// Both: four copies.
    FourWay,
    /// Turned copies around the centre.
    Radial,
}

impl SymmetryMode {
    pub const ALL: [SymmetryMode; 5] = [SymmetryMode::Off, SymmetryMode::LeftRight, SymmetryMode::TopBottom, SymmetryMode::FourWay, SymmetryMode::Radial];

    pub fn label(self) -> &'static str {
        match self {
            SymmetryMode::Off => "Off",
            SymmetryMode::LeftRight => "Mirror left–right",
            SymmetryMode::TopBottom => "Mirror top–bottom",
            SymmetryMode::FourWay => "Four-way mirror",
            SymmetryMode::Radial => "Radial (mandala)",
        }
    }
}

impl Default for Symmetry {
    fn default() -> Self {
        Self { mode: SymmetryMode::Off, count: 8, mirror: true, center: Point::ORIGIN }
    }
}

impl Symmetry {
    /// The transforms that make the extra copies of a newly drawn shape (not the original).
    pub fn copies(&self) -> Vec<Affine> {
        let c = self.center.to_vec2();
        let about = |t: Affine| Affine::translate(c) * t * Affine::translate(-c);
        let flip_x = Affine::scale_non_uniform(-1.0, 1.0);
        let flip_y = Affine::scale_non_uniform(1.0, -1.0);
        match self.mode {
            SymmetryMode::Off => Vec::new(),
            SymmetryMode::LeftRight => vec![about(flip_x)],
            SymmetryMode::TopBottom => vec![about(flip_y)],
            SymmetryMode::FourWay => vec![about(flip_x), about(flip_y), about(Affine::scale(-1.0))],
            SymmetryMode::Radial => {
                let n = self.count.clamp(2, 64);
                let step = std::f64::consts::TAU / n as f64;
                let mut out: Vec<Affine> = (1..n).map(|k| about(Affine::rotate(step * k as f64))).collect();
                if self.mirror {
                    // mirror about the line through the centre at the first slice's middle
                    let mirror = Affine::rotate(step / 2.0) * flip_y * Affine::rotate(-step / 2.0);
                    out.extend((0..n).map(|k| about(Affine::rotate(step * k as f64) * mirror)));
                }
                out
            }
        }
    }

    /// Mirror lines / slice boundaries through the centre, as angles (radians).
    fn axes(&self) -> Vec<f64> {
        use std::f64::consts::{FRAC_PI_2, PI};
        match self.mode {
            SymmetryMode::Off => vec![],
            SymmetryMode::LeftRight => vec![FRAC_PI_2],
            SymmetryMode::TopBottom => vec![0.0],
            SymmetryMode::FourWay => vec![0.0, FRAC_PI_2],
            SymmetryMode::Radial => {
                let n = self.count.clamp(2, 64);
                // full lines, so half the slices' boundaries suffice when n is even
                (0..n).map(|k| PI * 2.0 * k as f64 / n as f64).filter(|a| *a < PI - 1e-9 || n % 2 == 1).collect()
            }
        }
    }
}

/// Add a newly drawn path, plus its symmetry copies (grouped with it).
fn add_drawn(app: &mut App, path: BezPath) {
    let st = app.default_style();
    let copies = app.cs.symmetry.copies();
    let mut ids = vec![app.add_shape(Shape::new_path(0, path.clone(), st.clone()))];
    for t in copies {
        ids.push(app.add_shape(Shape::new_path(0, t * path.clone(), st.clone())));
    }
    app.sel = if ids.len() > 1 { ops::group(&mut app.doc, &ids) } else { ids };
}

/// "12.30 mm (ΔX 10.00, ΔY 7.07) at 35.1°" — y is shown upwards-positive.
fn measure_text(app: &App, a: Point, b: Point) -> String {
    let u = app.prefs.units;
    let f = u.factor();
    let d = b - a;
    format!("{:.2} {} (ΔX {:.2}, ΔY {:.2}) at {:.1}°", d.hypot() * f, u.suffix(), d.x * f, -d.y * f, (-d.y).atan2(d.x).to_degrees())
}

fn draw_measure(app: &App, painter: &egui::Painter, a: Point, b: Point) {
    let (sa, sb) = (app.view.to_screen(a), app.view.to_screen(b));
    let ink = Stroke::new(1.5, Color32::from_rgb(220, 0, 200));
    painter.line_segment([sa, sb], ink);
    painter.circle_filled(sa, 3.0, ink.color);
    painter.circle_filled(sb, 3.0, ink.color);
    let galley = painter.layout_no_wrap(measure_text(app, a, b), egui::FontId::proportional(13.0), Color32::BLACK);
    let at = sa + (sb - sa) / 2.0 + egui::vec2(8.0, -8.0);
    let bg = egui::Rect::from_min_size(at - egui::vec2(3.0, galley.size().y + 1.0), galley.size() + egui::vec2(6.0, 2.0));
    painter.rect_filled(bg, 3.0, Color32::from_white_alpha(230));
    painter.galley(bg.min + egui::vec2(3.0, 1.0), galley, Color32::BLACK);
}

/// The dimension's line (with arrowheads and end ticks), its midpoint and the
/// label's angle (kept upright).
fn dimension_parts(a: Point, b: Point) -> Option<(BezPath, Point, f64)> {
    let d = b - a;
    let len = d.hypot();
    if len < 0.5 {
        return None;
    }
    let dir = d / len;
    let n = Vec2::new(-dir.y, dir.x);
    let mut path = BezPath::new();
    path.move_to(a);
    path.line_to(b);
    let arrow = 2.0f64.min(len / 4.0);
    for (tip, back) in [(a, dir), (b, -dir)] {
        path.move_to(tip + back * arrow + n * arrow * 0.4);
        path.line_to(tip);
        path.line_to(tip + back * arrow - n * arrow * 0.4);
        path.move_to(tip + n * 2.0);
        path.line_to(tip - n * 2.0);
    }
    // upright: angles kept in [-90°, 90°), so vertical labels read bottom-to-top
    let mut angle = dir.atan2();
    let eps = 1e-9;
    if angle > std::f64::consts::FRAC_PI_2 - eps || angle < -std::f64::consts::FRAC_PI_2 - eps {
        angle += std::f64::consts::PI;
    }
    Some((path, a.midpoint(b), angle))
}

/// Adds a labelled dimension from `a` to `b`: a line and its length as text,
/// grouped, styled as a drawing aid (no cut, not printed).
fn add_dimension(app: &mut App, a: Point, b: Point) {
    let Some((path, at, angle)) = dimension_parts(a, b) else { return };
    let u = app.prefs.units;
    let label = format!("{} {}", fmt_num((a.distance(b) * u.factor() * 100.0).round() / 100.0), u.suffix());
    let ink = cutcraft_core::doc::Rgba(0, 110, 200, 255);
    let aid = cutcraft_core::doc::Style { fill: None, stroke: Some(ink), stroke_width: 0.25, cut: cutcraft_core::doc::CutStyle::NoCut, no_print: true };
    app.checkpoint_named("Dimension");
    let line = app.add_shape(Shape::new_path(0, path, aid.clone()));
    let data = TextData { text: label.clone(), size: 3.5, align: cutcraft_core::doc::Align::Center, on_path: None, ..app.ps.text.clone() };
    let outline = app.fonts.layout(&data);
    let mut text = Shape::new_path(0, BezPath::new(), cutcraft_core::doc::Style { fill: Some(ink), ..aid });
    text.kind = ShapeKind::Text { data, outline };
    // centred on the line, its bottom 1.5 mm above it
    let lb = match &text.kind {
        ShapeKind::Text { outline, .. } => kurbo::Shape::bounding_box(outline),
        _ => Rect::ZERO,
    };
    text.transform = Affine::translate(at.to_vec2()) * Affine::rotate(angle) * Affine::translate((-lb.center().x, -lb.y1 - 1.5));
    let text = app.add_shape(text);
    app.sel = ops::group(&mut app.doc, &[line, text]);
    app.status = format!("Dimension {label} - a drawing aid: not cut or printed");
}

/// Trim tool click: delete the piece of outline under the pointer.
/// Works out the faces of the selected shapes for the shape builder (when
/// the selection or the design has changed).
fn builder_faces(app: &mut App) {
    let key = (app.rev, app.sel.clone());
    if app.cs.builder.as_ref().is_some_and(|(k, _)| *k == key) {
        return;
    }
    let page = app.doc.page();
    let ids = cutcraft_core::ops::z_ordered(&app.doc, &app.sel);
    let shapes: Vec<Vec<BezPath>> = ids.iter().filter_map(|id| page.shape(*id)).map(|s| s.world_paths()).collect();
    let faces = if shapes.is_empty() { vec![] } else { cutcraft_core::shapebuilder::faces(&shapes) };
    app.cs.builder = Some((key, faces));
}

fn face_under(app: &App, p: Point) -> Option<usize> {
    app.cs.builder.as_ref().and_then(|(_, faces)| cutcraft_core::shapebuilder::face_at(faces, p))
}

/// Shape builder: merges the `picked` faces into one shape, or deletes them.
fn build_shapes(app: &mut App, picked: &[usize], delete: bool) {
    let Some((_, faces)) = app.cs.builder.clone() else { return };
    if faces.is_empty() {
        app.status = "Shape builder: select the overlapping shapes to build with first".into();
        return;
    }
    if picked.is_empty() {
        app.status = "Shape builder: drag across the areas to merge - or Alt-drag to delete them".into();
        return;
    }
    app.checkpoint_named(if delete { "Shape builder: delete" } else { "Shape builder: merge" });
    let sel = app.sel.clone();
    let before: Vec<ShapeId> = app.doc.page().shapes.iter().map(|s| s.id).collect();
    let made = cutcraft_core::shapebuilder::build(&mut app.doc, &sel, &faces, picked, delete);
    // everything that came out of it stays selected, to keep building
    app.sel = app.doc.page().shapes.iter().map(|s| s.id).filter(|id| !before.contains(id)).collect();
    let n = picked.len();
    app.status = match (delete, made.is_empty()) {
        (true, _) => format!("Deleted {n} area{}", if n == 1 { "" } else { "s" }),
        (false, false) => format!("Merged {n} area{} into one shape", if n == 1 { "" } else { "s" }),
        (false, true) => "Shape builder: nothing to merge".into(),
    };
}

fn trim_click(app: &mut App, cur: Point, tol: f64) {
    let mut doc = app.doc.clone();
    match cutcraft_core::trim::trim_at(&mut doc, cur, tol) {
        Some((id, gone)) => {
            app.checkpoint_named("Trim");
            app.doc = doc;
            if gone {
                app.sel.retain(|s| *s != id);
            }
            app.status = if gone { "Trimmed away the last piece".into() } else { "Trimmed - Ctrl+Z to undo".into() };
        }
        None => app.status = "Nothing to trim there: click a piece of a path between lines that cross it".into(),
    }
}

/// Tools whose new shapes symmetry repeats.
fn draws(t: Tool) -> bool {
    matches!(t, Tool::Line | Tool::Polyline | Tool::Curve | Tool::Freehand | Tool::Rect | Tool::RoundRect | Tool::Ellipse | Tool::Polygon | Tool::Star)
}

/// Ghost previews of the symmetry copies of `path`, and the symmetry axes.
fn draw_symmetry(app: &App, painter: &egui::Painter, path: Option<&BezPath>, stroke: Stroke) {
    let sym = &app.cs.symmetry;
    if sym.mode == SymmetryMode::Off || !draws(app.tool) {
        return;
    }
    let view = &app.view;
    let ghost = Stroke::new(1.0, stroke.color.gamma_multiply(0.45));
    if let Some(p) = path {
        for t in sym.copies() {
            for pl in geom::flatten(&(t * p.clone()), 0.1) {
                let pts: Vec<Pos2> = pl.pts.iter().map(|q| view.to_screen(*q)).collect();
                painter.add(if pl.closed { crate::render::closed_outline(pts, ghost) } else { egui::Shape::line(pts, ghost) });
            }
        }
    }
    let axis = Stroke::new(1.0, Color32::from_rgb(0, 160, 200).gamma_multiply(0.7));
    let reach = 4000.0;
    for a in sym.axes() {
        let d = Vec2::new(a.cos(), a.sin()) * reach;
        let (p0, p1) = (view.to_screen(sym.center - d), view.to_screen(sym.center + d));
        painter.add(egui::Shape::dashed_line(&[p0, p1], axis, 8.0, 5.0));
    }
    painter.circle_stroke(view.to_screen(sym.center), 5.0, axis);
}

const HANDLE_PX: f32 = 5.0;
/// Ruler thickness, px.
const RULER_PX: f32 = 18.0;
const GUIDE_BLUE: Color32 = Color32::from_rgb(0, 170, 255);
const SEL_BLUE: Color32 = Color32::from_rgb(40, 140, 255);

// ------------------------------------------------------------------ toolbar & status

pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        // page tabs
        let mut switch = None;
        for (i, p) in app.doc.pages.iter().enumerate() {
            if ui.selectable_label(i == app.doc.active, &p.name).clicked() {
                switch = Some(i);
            }
        }
        if let Some(i) = switch {
            app.doc.active = i;
            app.sel.clear();
            app.rev += 1;
        }
        if ui.small_button("+").on_hover_text("Add page").clicked() {
            app.do_action(crate::app::Action::AddPage, ui.ctx());
        }
        ui.separator();
        let u = app.prefs.units;
        if let Some(c) = app.cs.cursor_mm {
            ui.monospace(format!("{:>7.2}, {:>7.2} {}", c.x * u.factor(), c.y * u.factor(), u.suffix()));
        }
        ui.monospace(format!("{:>4.0}%", app.view.zoom / (96.0 / 25.4) * 100.0));
        if let Some(bb) = ops::selection_bbox(&app.doc, &app.sel) {
            ui.separator();
            ui.label(format!(
                "{} selected · {:.2} × {:.2} {}",
                app.sel.len(),
                bb.width() * u.factor(),
                bb.height() * u.factor(),
                u.suffix()
            ));
        }
        ui.separator();
        ui.label(&app.status);
    });
}

// ------------------------------------------------------------------ helpers

/// Snap to a nearby object point (corner, centre or node of another shape),
/// otherwise to the grid. Object snapping ported from BurnCraft.
fn snap(app: &App, p: Point) -> Point {
    if app.prefs.snap_objects {
        let tol = SNAP_PX / app.view.zoom as f64;
        if let Some(q) = app.cs.snap_pts.iter().copied().filter(|q| q.distance(p) <= tol).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))) {
            return q;
        }
    }
    let mut q = grid_snap(app, p);
    let tol = SNAP_PX / app.view.zoom as f64;
    let near = |v: f64, vertical: bool| {
        app.doc.page().guides.iter().filter(|g| g.vertical == vertical && (g.pos - v).abs() <= tol).min_by(|a, b| (a.pos - v).abs().total_cmp(&(b.pos - v).abs())).map(|g| g.pos)
    };
    if let Some(x) = near(p.x, true) {
        q.x = x;
    }
    if let Some(y) = near(p.y, false) {
        q.y = y;
    }
    q
}

/// The ruler guide within a few pixels of `screen`, if any.
fn guide_at(app: &App, screen: Pos2) -> Option<usize> {
    app.doc.page().guides.iter().position(|g| {
        let at = app.view.to_screen(Point::new(g.pos, g.pos));
        (if g.vertical { at.x - screen.x } else { at.y - screen.y }).abs() <= 4.0
    })
}

/// Which ruler (top → horizontal guide, left → vertical) a screen point is on.
fn in_ruler(app: &App, p: Pos2) -> Option<bool> {
    let c = app.view.canvas;
    if !app.prefs.show_rulers || !c.contains(p) {
        return None;
    }
    if p.y < c.min.y + RULER_PX {
        Some(false)
    } else if p.x < c.min.x + RULER_PX {
        Some(true)
    } else {
        None
    }
}

/// Snapping distance on screen, in pixels.
const SNAP_PX: f64 = 6.0;

fn grid_snap(app: &App, p: Point) -> Point {
    let g = &app.doc.page().grid;
    if !g.snap {
        return p;
    }
    let s = g.spacing / g.subdivisions.max(1) as f64;
    Point::new((p.x / s).round() * s, (p.y / s).round() * s)
}

/// Rebuild the object-snap points when the document or selection changed.
fn refresh_snap_points(app: &mut App) {
    if !app.prefs.snap_objects {
        app.cs.snap_pts.clear();
        return;
    }
    let key = (app.rev, app.sel.clone());
    if app.cs.snap_key.as_ref() == Some(&key) {
        return;
    }
    const LIMIT: usize = 20_000;
    let mut pts = Vec::new();
    let page = app.doc.page();
    for s in page.shapes.iter().filter(|s| !app.sel.contains(&s.id) && page.is_visible(s)) {
        if let Some(b) = s.bbox() {
            pts.extend([b.center(), Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]);
        }
        if matches!(s.kind, ShapeKind::Path(_)) {
            for p in s.world_paths() {
                for el in p.elements() {
                    match *el {
                        PathEl::MoveTo(q) | PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) => pts.push(q),
                        PathEl::ClosePath => {}
                    }
                }
            }
        }
        if pts.len() > LIMIT {
            break;
        }
    }
    app.cs.snap_pts = pts;
    app.cs.snap_key = Some(key);
}

/// Smart-guide targets for moving the selection: the page's edges and centre
/// and every other visible shape's edges and centres.
fn guide_targets(app: &App) -> (Vec<f64>, Vec<f64>) {
    let page = app.doc.page();
    let r = page.rect();
    let (mut xs, mut ys) = (vec![r.x0, r.center().x, r.x1], vec![r.y0, r.center().y, r.y1]);
    for b in page.shapes.iter().filter(|s| !app.sel.contains(&s.id) && page.is_visible(s)).filter_map(|s| s.bbox()).take(2000) {
        xs.extend([b.x0, b.center().x, b.x1]);
        ys.extend([b.y0, b.center().y, b.y1]);
    }
    for g in &page.guides {
        if g.vertical { xs.push(g.pos) } else { ys.push(g.pos) }
    }
    (xs, ys)
}

/// The smallest correction (within `tol`) that lines one of `values` up with a
/// target, and the target line itself.
fn best_alignment(values: [f64; 3], targets: &[f64], tol: f64) -> Option<(f64, f64)> {
    values
        .iter()
        .flat_map(|v| targets.iter().map(move |t| (t - v, *t)))
        .filter(|(d, _)| d.abs() <= tol)
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

fn handle_points(bb: Rect) -> [Point; 8] {
    let (cx, cy) = (bb.center().x, bb.center().y);
    [
        Point::new(bb.x0, bb.y0),
        Point::new(cx, bb.y0),
        Point::new(bb.x1, bb.y0),
        Point::new(bb.x1, cy),
        Point::new(bb.x1, bb.y1),
        Point::new(cx, bb.y1),
        Point::new(bb.x0, bb.y1),
        Point::new(bb.x0, cy),
    ]
}

fn rot_handle(view: &View, bb: Rect) -> Pos2 {
    view.to_screen(Point::new(bb.center().x, bb.y0)) - egui::vec2(0.0, 24.0)
}

fn orig_transforms(app: &App) -> Vec<(ShapeId, Affine)> {
    app.editable_sel().iter().filter_map(|id| app.doc.page().shape(*id).map(|s| (*id, s.transform))).collect()
}

fn apply_transforms(app: &mut App, orig: &[(ShapeId, Affine)], a: Affine) {
    for (id, t) in orig {
        if let Some(s) = app.doc.page_mut().shape_mut(*id) {
            s.transform = a * *t;
        }
    }
    app.touch();
}

/// Nodes of a path: (element index, kind, point) in local coordinates.
fn path_nodes(p: &BezPath) -> Vec<(NodeHandle, Point)> {
    let mut v = vec![];
    for (i, el) in p.elements().iter().enumerate() {
        match el {
            PathEl::MoveTo(q) | PathEl::LineTo(q) => v.push((NodeHandle::End(i), *q)),
            PathEl::QuadTo(c, q) => {
                v.push((NodeHandle::C1(i), *c));
                v.push((NodeHandle::End(i), *q));
            }
            PathEl::CurveTo(a, b, q) => {
                v.push((NodeHandle::C1(i), *a));
                v.push((NodeHandle::C2(i), *b));
                v.push((NodeHandle::End(i), *q));
            }
            PathEl::ClosePath => {}
        }
    }
    v
}

fn set_node(p: &mut BezPath, h: NodeHandle, to: Point) {
    let mut els: Vec<PathEl> = p.elements().to_vec();
    match h {
        NodeHandle::End(i) => {
            let old = els[i].end_point().unwrap_or(to);
            let d = to - old;
            match &mut els[i] {
                PathEl::MoveTo(q) | PathEl::LineTo(q) => *q = to,
                PathEl::QuadTo(_, q) => *q = to,
                PathEl::CurveTo(_, b, q) => {
                    *b += d;
                    *q = to
                }
                PathEl::ClosePath => {}
            }
            // carry the outgoing control point
            if let Some(PathEl::CurveTo(a, _, _)) = els.get_mut(i + 1) {
                *a += d;
            }
            // keep a closed subpath's start and end together
            if let PathEl::MoveTo(_) = els[i] {
                let mut j = i + 1;
                while j < els.len() && !matches!(els[j], PathEl::MoveTo(_)) {
                    j += 1;
                }
                if j >= 2 && matches!(els[j - 1], PathEl::ClosePath) {
                    if let Some(e) = els[j - 2].end_point() {
                        if e.distance(old) < 1e-6 {
                            set_end(&mut els[j - 2], to);
                        }
                    }
                }
            }
        }
        NodeHandle::C1(i) => match &mut els[i] {
            PathEl::QuadTo(c, _) => *c = to,
            PathEl::CurveTo(a, _, _) => *a = to,
            _ => {}
        },
        NodeHandle::C2(i) => {
            if let PathEl::CurveTo(_, b, _) = &mut els[i] {
                *b = to
            }
        }
    }
    *p = BezPath::from_vec(els);
}

fn set_end(el: &mut PathEl, to: Point) {
    match el {
        PathEl::MoveTo(q) | PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) => *q = to,
        PathEl::ClosePath => {}
    }
}

fn node_target(app: &App) -> Option<(ShapeId, BezPath, Affine)> {
    if app.sel.len() != 1 {
        return None;
    }
    let s = app.doc.page().shape(app.sel[0])?;
    match &s.kind {
        ShapeKind::Path(p) => Some((s.id, p.clone(), s.transform)),
        _ => None,
    }
}

/// Delete the selected node in point-edit mode. Returns true if handled.
pub fn delete_node(app: &mut App) -> bool {
    let (Some(NodeHandle::End(i)), Some((id, p, _))) = (app.cs.node_sel, node_target(app)) else { return false };
    let mut els = p.elements().to_vec();
    if i >= els.len() {
        return false;
    }
    let was_move = matches!(els[i], PathEl::MoveTo(_));
    els.remove(i);
    if was_move {
        if let Some(e) = els.get(i).and_then(|e| e.end_point()) {
            els[i] = PathEl::MoveTo(e);
        }
    }
    app.checkpoint_named("Delete point");
    if let Some(s) = app.doc.page_mut().shape_mut(id) {
        s.kind = ShapeKind::Path(BezPath::from_vec(els));
    }
    app.cs.node_sel = None;
    true
}

/// Toggle the segment ending at the selected node between line and curve.
pub fn toggle_segment(app: &mut App) {
    let (Some(NodeHandle::End(i)), Some((id, p, _))) = (app.cs.node_sel, node_target(app)) else { return };
    let mut els = p.elements().to_vec();
    let start = els[..i].iter().rev().find_map(|e| e.end_point());
    let Some(s0) = start else { return };
    els[i] = match els[i] {
        PathEl::LineTo(q) => PathEl::CurveTo(s0.lerp(q, 1.0 / 3.0), s0.lerp(q, 2.0 / 3.0), q),
        PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) => PathEl::LineTo(q),
        e => e,
    };
    app.checkpoint_named("Toggle segment");
    if let Some(s) = app.doc.page_mut().shape_mut(id) {
        s.kind = ShapeKind::Path(BezPath::from_vec(els));
    }
}

/// Split the path segment nearest to `local` (shape-local coords).
fn add_node(app: &mut App, id: ShapeId, p: &BezPath, local: Point) {
    let els = p.elements().to_vec();
    let mut best: Option<(f64, usize, f64, PathSeg)> = None;
    let mut cur = Point::ZERO;
    for (i, el) in els.iter().enumerate() {
        let seg = match *el {
            PathEl::MoveTo(q) => {
                cur = q;
                continue;
            }
            PathEl::LineTo(q) => PathSeg::Line(kurbo::Line::new(cur, q)),
            PathEl::QuadTo(c, q) => PathSeg::Quad(kurbo::QuadBez::new(cur, c, q)),
            PathEl::CurveTo(a, b, q) => PathSeg::Cubic(kurbo::CubicBez::new(cur, a, b, q)),
            PathEl::ClosePath => continue,
        };
        let n = seg.nearest(local, 1e-4);
        if best.map(|b| n.distance_sq < b.0).unwrap_or(true) {
            best = Some((n.distance_sq, i, n.t, seg));
        }
        cur = el.end_point().unwrap_or(cur);
    }
    let Some((_, i, t, seg)) = best else { return };
    let to_el = |s: PathSeg| match s {
        PathSeg::Line(l) => PathEl::LineTo(l.p1),
        PathSeg::Quad(q) => PathEl::QuadTo(q.p1, q.p2),
        PathSeg::Cubic(c) => PathEl::CurveTo(c.p1, c.p2, c.p3),
    };
    let a = to_el(kurbo::ParamCurve::subsegment(&seg, 0.0..t));
    let b = to_el(kurbo::ParamCurve::subsegment(&seg, t..1.0));
    let mut out = els.clone();
    out[i] = a;
    out.insert(i + 1, b);
    app.checkpoint_named("Add point");
    if let Some(s) = app.doc.page_mut().shape_mut(id) {
        s.kind = ShapeKind::Path(BezPath::from_vec(out));
    }
    app.cs.node_sel = Some(NodeHandle::End(i));
}

fn create_shape_path(tool: Tool, a: Point, b: Point, shift: bool, app: &App) -> Option<BezPath> {
    let mut r = Rect::from_points(a, b);
    if shift && tool != Tool::Line {
        let s = r.width().max(r.height());
        let sx = if b.x < a.x { -s } else { s };
        let sy = if b.y < a.y { -s } else { s };
        r = Rect::from_points(a, Point::new(a.x + sx, a.y + sy));
    }
    if r.width() < 0.2 && r.height() < 0.2 {
        return None;
    }
    let ps = &app.ps;
    Some(match tool {
        Tool::Line => {
            let mut e = b;
            if shift {
                let d = b - a;
                let ang = (d.y.atan2(d.x) / (std::f64::consts::PI / 4.0)).round() * std::f64::consts::PI / 4.0;
                e = a + Vec2::new(ang.cos(), ang.sin()) * d.hypot();
            }
            geom::polyline_path(&[a, e], false)
        }
        Tool::Rect => geom::rect_path(r),
        Tool::RoundRect => geom::rounded_rect_path(r, ps.corner_radius),
        Tool::Ellipse => geom::ellipse_path(r),
        Tool::Polygon => geom::polygon_path(r, ps.polygon_sides),
        Tool::Star => geom::star_path(r, ps.star_points, ps.star_inner),
        _ => return None,
    })
}

fn finish_poly(app: &mut App, closed: bool) {
    let pts = std::mem::take(&mut app.cs.poly);
    if pts.len() < 2 {
        return;
    }
    let path = if app.tool == Tool::Curve {
        geom::smooth_polyline(&Polyline { pts, closed }, 179.0)
    } else {
        geom::polyline_path(&pts, closed)
    };
    app.checkpoint_named("Draw");
    add_drawn(app, path);
}

// ------------------------------------------------------------------ main canvas

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    app.view.canvas = resp.rect;
    let ctx = ui.ctx().clone();

    if app.view.fit_requested {
        let p = app.doc.page();
        let mut r = p.rect();
        if let Some((w, h)) = p.mat.size_mm() {
            r = r.union(Rect::new(-12.0, -12.0, w + 12.0, h + 12.0));
        }
        app.view.fit(r);
        app.view.fit_requested = false;
    }
    if let Some(r) = app.view.fit_rect.take() {
        app.view.fit(r.inflate(5.0, 5.0));
    }

    let hover = resp.hover_pos();
    app.cs.cursor_mm = hover.map(|h| app.view.to_mm(h));

    // ---- navigation: wheel zoom, middle drag pan
    if resp.hovered() {
        let (scroll, zoom_delta, shift) = ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.shift));
        if zoom_delta != 1.0 {
            app.view.zoom_by(zoom_delta, hover);
        } else if scroll != egui::Vec2::ZERO {
            if shift {
                app.view.pan += egui::vec2(scroll.y, 0.0);
            } else {
                app.view.zoom_by((scroll.y / 400.0).exp(), hover);
            }
        }
    }
    if resp.dragged_by(PointerButton::Middle) || (resp.dragged_by(PointerButton::Primary) && app.tool == Tool::Pan) {
        app.view.pan += resp.drag_delta();
    }

    let bg = ui.visuals().extreme_bg_color;
    draw_scene(app, &painter, &ctx, bg);
    interact(app, &resp, &ctx);
    resp.context_menu(|ui| selection_context_menu(app, ui, &ctx));
    draw_overlays(app, &painter, &ctx);
    crate::paramspanel::draw_constraints(app, &painter);
    draw_guides(app, &painter, hover);
    if app.prefs.show_rulers {
        draw_rulers(app, &painter);
    }

    // cursors
    if resp.hovered() {
        let on_guide = hover.and_then(|h| in_ruler(app, h).or_else(|| if app.tool == Tool::Select { guide_at(app, h).map(|i| app.doc.page().guides[i].vertical) } else { None }));
        let c = match app.tool {
            _ if matches!(app.cs.drag, Drag::Guide { vertical: true, .. }) || (matches!(app.cs.drag, Drag::None) && on_guide == Some(true)) => CursorIcon::ResizeHorizontal,
            _ if matches!(app.cs.drag, Drag::Guide { vertical: false, .. }) || (matches!(app.cs.drag, Drag::None) && on_guide == Some(false)) => CursorIcon::ResizeVertical,
            Tool::Pan => CursorIcon::Grab,
            Tool::Zoom => CursorIcon::ZoomIn,
            Tool::Text => CursorIcon::Text,
            Tool::Select | Tool::Node => CursorIcon::Default,
            _ => CursorIcon::Crosshair,
        };
        ctx.set_cursor_icon(c);
    }
}

fn texture_for(textures: &mut std::collections::HashMap<usize, egui::TextureHandle>, ctx: &egui::Context, png: &std::sync::Arc<Vec<u8>>) -> Option<egui::TextureId> {
    let key = std::sync::Arc::as_ptr(png) as usize;
    if !textures.contains_key(&key) {
        let img = image::load_from_memory(png).ok()?;
        let max = 4096;
        let img = if img.width() > max || img.height() > max { img.resize(max, max, image::imageops::FilterType::Triangle) } else { img };
        let rgba = img.to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        let h = ctx.load_texture(format!("img{key}"), ci, egui::TextureOptions::LINEAR);
        textures.insert(key, h);
    }
    textures.get(&key).map(|h| h.id())
}

fn draw_scene(app: &mut App, painter: &egui::Painter, ctx: &egui::Context, bg: Color32) {
    let view = &app.view;
    painter.rect_filled(view.canvas, 0.0, bg);
    let page = &app.doc.pages[app.doc.active];
    let z = view.zoom;

    // cutting mat
    if let Some((mw, mh)) = page.mat.size_mm() {
        let r = egui::Rect::from_two_pos(view.to_screen(Point::new(-12.0, -12.0)), view.to_screen(Point::new(mw + 12.0, mh + 12.0)));
        painter.rect_filled(r, 6.0, Color32::from_rgb(170, 180, 190));
        let inner = egui::Rect::from_two_pos(view.to_screen(Point::ZERO), view.to_screen(Point::new(mw, mh)));
        painter.rect_filled(inner, 0.0, Color32::from_rgb(200, 208, 214));
        let inch = 25.4;
        let mut x = 0.0;
        while x <= mw + 0.1 {
            painter.line_segment([view.to_screen(Point::new(x, 0.0)), view.to_screen(Point::new(x, mh))], Stroke::new(1.0, Color32::from_rgb(180, 190, 198)));
            x += inch;
        }
        let mut y = 0.0;
        while y <= mh + 0.1 {
            painter.line_segment([view.to_screen(Point::new(0.0, y)), view.to_screen(Point::new(mw, y))], Stroke::new(1.0, Color32::from_rgb(180, 190, 198)));
            y += inch;
        }
    }

    // media / page
    let pr = egui::Rect::from_two_pos(view.to_screen(Point::ZERO), view.to_screen(Point::new(page.width, page.height)));
    painter.rect_filled(pr.translate(egui::vec2(3.0, 3.0)), 0.0, Color32::from_black_alpha(60));
    // media is opaque (older files may carry a transparent colour)
    let mc = page.media_color;
    painter.rect_filled(pr, 0.0, Color32::from_rgb(mc.0, mc.1, mc.2));
    painter.rect_stroke(pr, 0.0, Stroke::new(1.0, Color32::GRAY), StrokeKind::Outside);
    if page.cut_border > 0.0 {
        let b = page.cut_border;
        let r = egui::Rect::from_two_pos(view.to_screen(Point::new(b, b)), view.to_screen(Point::new(page.width - b, page.height - b)));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(220, 60, 60)), StrokeKind::Inside);
    }

    // grid
    if page.grid.visible && page.grid.spacing > 0.0 {
        let major = page.grid.spacing;
        let minor = major / page.grid.subdivisions.max(1) as f64;
        let draw_minor = minor * z as f64 > 6.0;
        let step = if draw_minor { minor } else { major };
        if step * z as f64 > 3.0 {
            let mut i = 0;
            let mut x = 0.0;
            while x <= page.width + 1e-6 {
                let is_major = (x / major - (x / major).round()).abs() < 1e-6;
                let c = if is_major { Color32::from_rgba_unmultiplied(80, 120, 200, 70) } else { Color32::from_rgba_unmultiplied(80, 120, 200, 28) };
                painter.line_segment([view.to_screen(Point::new(x, 0.0)), view.to_screen(Point::new(x, page.height))], Stroke::new(1.0, c));
                i += 1;
                x = i as f64 * step;
            }
            let mut i = 0;
            let mut y = 0.0;
            while y <= page.height + 1e-6 {
                let is_major = (y / major - (y / major).round()).abs() < 1e-6;
                let c = if is_major { Color32::from_rgba_unmultiplied(80, 120, 200, 70) } else { Color32::from_rgba_unmultiplied(80, 120, 200, 28) };
                painter.line_segment([view.to_screen(Point::new(0.0, y)), view.to_screen(Point::new(page.width, y))], Stroke::new(1.0, c));
                i += 1;
                y = i as f64 * step;
            }
        }
    }

    // shapes: the cached picture (see render.rs)
    let frame = render::Frame { page, zoom: z, origin: view.to_screen(Point::ORIGIN), canvas: view.canvas, cut_only: app.cs.show_cut_lines_only };
    let (parts, shift, frame_no) = app.cache.picture(ctx, &frame);
    // pictures need their textures, which needs `app`: note them, draw after
    let mut images = vec![];
    for part in parts {
        match part {
            // kept on the graphics card (see gpu.rs)
            render::Part::Mesh { id, mesh } if app.gpu.available() => {
                painter.add(app.gpu.draw(*id, mesh, shift, view.canvas, frame_no));
            }
            render::Part::Mesh { mesh, .. } if shift == egui::Vec2::ZERO => {
                painter.add(egui::Shape::Mesh(mesh.clone()));
            }
            render::Part::Mesh { mesh, .. } => {
                let mut m = (**mesh).clone();
                m.translate(shift);
                painter.add(egui::Shape::mesh(m));
            }
            render::Part::Image { png, w, h, at } => {
                images.push((painter.add(egui::Shape::Noop), png.clone(), *w, *h, *at));
            }
        }
    }
    for (slot, png, w, h, a) in images {
        if let Some(tex) = texture_for(&mut app.textures, ctx, &png) {
            let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)];
            let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
            let mut mesh = egui::Mesh::with_texture(tex);
            for (c, uv) in corners.iter().zip(uvs) {
                mesh.vertices.push(egui::epaint::Vertex { pos: app.view.to_screen(a * *c), uv, color: Color32::WHITE });
            }
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            painter.set(slot, egui::Shape::mesh(mesh));
        }
    }

    // registration marks
    for m in page.regmarks.print_geometry(page.width, page.height) {
        for pl in geom::flatten(&m, 0.1) {
            let pts: Vec<Pos2> = pl.pts.iter().map(|p| app.view.to_screen(*p)).collect();
            painter.add(egui::Shape::convex_polygon(pts, Color32::BLACK, Stroke::NONE));
        }
    }
    if page.regmarks.enabled {
        let sa = page.regmarks.safe_area(page.width, page.height);
        let r = egui::Rect::from_two_pos(app.view.to_screen(sa.origin()), app.view.to_screen(Point::new(sa.x1, sa.y1)));
        painter.add(egui::Shape::dashed_line(
            &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
            Stroke::new(1.0, Color32::from_rgb(200, 120, 0)),
            6.0,
            4.0,
        ));
    }
}

fn draw_overlays(app: &mut App, painter: &egui::Painter, ctx: &egui::Context) {
    let view = &app.view;
    let page = app.doc.page();

    // the Contour list's hovered line
    if let Some(p) = app.cs.highlight.take() {
        for pl in geom::flatten(&p, 0.1) {
            let pts: Vec<Pos2> = pl.pts.iter().map(|p| view.to_screen(*p)).collect();
            painter.add(if pl.closed { crate::render::closed_outline(pts, Stroke::new(3.0, crate::chrome::ACCENT)) } else { egui::Shape::line(pts, Stroke::new(3.0, crate::chrome::ACCENT)) });
        }
    }
    // trace preview
    if let Some((_, p)) = &app.cs.trace_preview {
        for pl in geom::flatten(p, 0.1) {
            let pts: Vec<Pos2> = pl.pts.iter().map(|p| view.to_screen(*p)).collect();
            painter.add(crate::render::closed_outline(pts, Stroke::new(1.5, Color32::from_rgb(255, 200, 0))));
        }
    }

    // cut preview / simulation
    if app.panel == Panel::Send {
        if let Some(job) = &app.cs.preview_job {
            let total: usize = job.passes.iter().map(|p| p.paths.len()).sum::<usize>().max(1);
            let shown = (app.cs.preview_t * total as f32).ceil() as usize;
            let mut k = 0;
            let mut last = Point::ZERO;
            let mut head = None;
            for pass in &job.passes {
                for pl in &pass.paths {
                    if k >= shown {
                        break;
                    }
                    let t = k as f32 / total as f32;
                    let col = Color32::from_rgb((40.0 + 200.0 * t) as u8, (200.0 - 150.0 * t) as u8, 60);
                    let mut pts: Vec<Pos2> = pl.pts.iter().map(|p| view.to_screen(*p)).collect();
                    if pl.closed && !pts.is_empty() {
                        pts.push(pts[0]);
                    }
                    if let Some(f) = pts.first() {
                        painter.add(egui::Shape::dashed_line(&[view.to_screen(last), *f], Stroke::new(1.0, Color32::from_gray(150)), 4.0, 4.0));
                        painter.circle_filled(*f, 3.0, col);
                    }
                    painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, col)));
                    last = if pl.closed { pl.pts[0] } else { *pl.pts.last().unwrap() };
                    head = pts.last().copied();
                    k += 1;
                }
            }
            if let Some(h) = head {
                painter.circle_stroke(h, 7.0, Stroke::new(2.0, Color32::from_rgb(255, 80, 0)));
            }
            if app.cs.preview_playing {
                app.cs.preview_t = (app.cs.preview_t + 0.004).min(1.0);
                if app.cs.preview_t >= 1.0 {
                    app.cs.preview_playing = false;
                }
                ctx.request_repaint();
            }
        }
    }

    // selection
    if let Some(bb) = ops::selection_bbox(&app.doc, &app.sel) {
        for id in &app.sel {
            if let Some(b) = page.shape(*id).and_then(|s| s.bbox()) {
                let r = egui::Rect::from_two_pos(view.to_screen(b.origin()), view.to_screen(Point::new(b.x1, b.y1)));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, SEL_BLUE.gamma_multiply(0.5)), StrokeKind::Middle);
            }
        }
        if app.tool == Tool::Select {
            let r = egui::Rect::from_two_pos(view.to_screen(bb.origin()), view.to_screen(Point::new(bb.x1, bb.y1)));
            painter.add(egui::Shape::dashed_line(
                &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                Stroke::new(1.0, SEL_BLUE),
                5.0,
                3.0,
            ));
            for h in handle_points(bb) {
                let p = view.to_screen(h);
                painter.rect_filled(egui::Rect::from_center_size(p, egui::vec2(HANDLE_PX * 2.0, HANDLE_PX * 2.0)), 1.0, Color32::WHITE);
                painter.rect_stroke(
                    egui::Rect::from_center_size(p, egui::vec2(HANDLE_PX * 2.0, HANDLE_PX * 2.0)),
                    1.0,
                    Stroke::new(1.0, SEL_BLUE),
                    StrokeKind::Middle,
                );
            }
            let rh = rot_handle(view, bb);
            painter.line_segment([rh, view.to_screen(Point::new(bb.center().x, bb.y0))], Stroke::new(1.0, SEL_BLUE));
            painter.circle_filled(rh, HANDLE_PX + 1.0, Color32::from_rgb(60, 200, 90));
            // size label
            let u = app.prefs.units;
            painter.text(
                r.center_bottom() + egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                format!("{:.2} × {:.2} {}", bb.width() * u.factor(), bb.height() * u.factor(), u.suffix()),
                egui::FontId::proportional(11.0),
                SEL_BLUE,
            );
        }
    }

    // warp grid
    if app.tool == Tool::Node {
        if let Some((_, pts, rows, cols)) = crate::panels2::warp_handles(app) {
            let grid = Stroke::new(1.0, Color32::from_rgb(160, 90, 220));
            for j in 0..=rows {
                let row: Vec<Pos2> = (0..=cols).map(|i| view.to_screen(pts[j * (cols + 1) + i])).collect();
                painter.add(egui::Shape::line(row, grid));
            }
            for i in 0..=cols {
                let col: Vec<Pos2> = (0..=rows).map(|j| view.to_screen(pts[j * (cols + 1) + i])).collect();
                painter.add(egui::Shape::line(col, grid));
            }
            for p in &pts {
                painter.circle_filled(view.to_screen(*p), 4.5, Color32::WHITE);
                painter.circle_stroke(view.to_screen(*p), 4.5, grid);
            }
        }
    }
    // point editing
    if app.tool == Tool::Node {
        if let Some((_, p, a)) = node_target(app) {
            let nodes = path_nodes(&p);
            // control arms
            let els = p.elements();
            let mut cur = Point::ZERO;
            for el in els {
                match el {
                    PathEl::CurveTo(c1, c2, q) => {
                        painter.line_segment([view.to_screen(a * cur), view.to_screen(a * *c1)], Stroke::new(1.0, Color32::GRAY));
                        painter.line_segment([view.to_screen(a * *q), view.to_screen(a * *c2)], Stroke::new(1.0, Color32::GRAY));
                    }
                    PathEl::QuadTo(c, q) => {
                        painter.line_segment([view.to_screen(a * cur), view.to_screen(a * *c)], Stroke::new(1.0, Color32::GRAY));
                        painter.line_segment([view.to_screen(a * *q), view.to_screen(a * *c)], Stroke::new(1.0, Color32::GRAY));
                    }
                    _ => {}
                }
                if let Some(e) = el.end_point() {
                    cur = e;
                }
            }
            for (h, q) in nodes {
                let sp = view.to_screen(a * q);
                let selected = app.cs.node_sel == Some(h);
                match h {
                    NodeHandle::End(_) => {
                        let r = egui::Rect::from_center_size(sp, egui::vec2(8.0, 8.0));
                        painter.rect_filled(r, 0.0, if selected { SEL_BLUE } else { Color32::WHITE });
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, SEL_BLUE), StrokeKind::Middle);
                    }
                    _ => {
                        painter.circle_filled(sp, 3.5, if selected { SEL_BLUE } else { Color32::LIGHT_GRAY });
                    }
                }
            }
        }
    }

    // in-progress drawing previews
    let stroke = Stroke::new(1.0, Color32::from_rgb(230, 40, 40));
    let cursor = app.cs.cursor_mm;
    // the shape being drawn, for symmetry ghosts
    let mut ghost: Option<BezPath> = None;
    match &app.cs.drag {
        Drag::Create { start } => {
            if let Some(c) = cursor {
                let shift = ctx.input(|i| i.modifiers.shift);
                if let Some(p) = create_shape_path(app.tool, *start, snap(app, c), shift, app) {
                    for pl in geom::flatten(&p, 0.1) {
                        let pts: Vec<Pos2> = pl.pts.iter().map(|q| view.to_screen(*q)).collect();
                        painter.add(if pl.closed { crate::render::closed_outline(pts, stroke) } else { egui::Shape::line(pts, stroke) });
                    }
                    ghost = Some(p);
                }
            }
        }
        Drag::Rubber { start } | Drag::ZoomRect { start } => {
            if let Some(c) = cursor {
                let r = egui::Rect::from_two_pos(view.to_screen(*start), view.to_screen(c));
                // dragged right to left = crossing selection: dashed and green
                let crossing = matches!(app.cs.drag, Drag::Rubber { .. }) && c.x < start.x;
                if crossing {
                    let green = Color32::from_rgb(30, 170, 80);
                    painter.rect_filled(r, 0.0, green.gamma_multiply(0.1));
                    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, green), 6.0, 4.0));
                } else {
                    painter.rect_filled(r, 0.0, SEL_BLUE.gamma_multiply(0.1));
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, SEL_BLUE), StrokeKind::Middle);
                }
            }
        }
        Drag::Freehand { pts } | Drag::Eraser { pts } | Drag::ImageErase { pts } => {
            let sp: Vec<Pos2> = pts.iter().map(|q| view.to_screen(*q)).collect();
            let brush = matches!(app.cs.drag, Drag::Eraser { .. } | Drag::ImageErase { .. });
            let w = if brush { (app.ps.eraser_radius * 2.0) as f32 * view.zoom } else { 1.0 };
            let c = if brush { Color32::from_rgba_unmultiplied(255, 120, 120, 90) } else { stroke.color };
            painter.add(egui::Shape::line(sp, Stroke::new(w, c)));
            if !brush && pts.len() > 1 {
                ghost = Some(geom::polyline_path(pts, false));
            }
        }
        Drag::Measure { start } => {
            if let Some(c) = cursor {
                draw_measure(app, painter, *start, snap(app, c));
            }
        }
        Drag::Dimension { start } => {
            if let Some((path, _, _)) = cursor.and_then(|c| dimension_parts(*start, snap(app, c))) {
                let ink = Stroke::new(1.5, Color32::from_rgb(0, 110, 200));
                for pl in geom::flatten(&path, 0.05) {
                    painter.add(egui::Shape::line(pl.pts.iter().map(|q| view.to_screen(*q)).collect(), ink));
                }
                let c = snap(app, cursor.unwrap_or(*start));
                let u = app.prefs.units;
                let label = format!("{:.2} {}", start.distance(c) * u.factor(), u.suffix());
                let galley = painter.layout_no_wrap(label, egui::FontId::proportional(12.0), Color32::WHITE);
                let pos = view.to_screen(start.midpoint(c)) + egui::vec2(8.0, -20.0);
                painter.rect_filled(egui::Rect::from_min_size(pos, galley.size()).expand(3.0), 3.0, ink.color);
                painter.galley(pos, galley, Color32::WHITE);
            }
        }
        Drag::ImageCrop { start } => {
            if let Some(c) = cursor {
                let r = egui::Rect::from_two_pos(view.to_screen(*start), view.to_screen(c));
                painter.rect_filled(r, 0.0, Color32::from_white_alpha(25));
                let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.5, crate::chrome::ACCENT), 6.0, 4.0));
            }
        }
        Drag::Knife { start } => {
            if let Some(c) = cursor {
                painter.add(egui::Shape::dashed_line(&[view.to_screen(*start), view.to_screen(c)], Stroke::new(1.5, Color32::from_rgb(255, 60, 0)), 6.0, 3.0));
            }
        }
        _ => {}
    }
    if !app.cs.poly.is_empty() {
        let mut pts: Vec<Point> = app.cs.poly.clone();
        if let Some(c) = cursor {
            pts.push(snap(app, c));
        }
        let path = if app.tool == Tool::Curve {
            geom::smooth_polyline(&Polyline { pts: pts.clone(), closed: false }, 179.0)
        } else {
            geom::polyline_path(&pts, false)
        };
        for pl in geom::flatten(&path, 0.1) {
            painter.add(egui::Shape::line(pl.pts.iter().map(|q| view.to_screen(*q)).collect(), stroke));
        }
        for p in &app.cs.poly {
            painter.circle_filled(view.to_screen(*p), 3.0, stroke.color);
        }
        ghost = Some(path);
    }
    draw_symmetry(app, painter, ghost.as_ref(), stroke);
    if app.tool == Tool::Measure && matches!(app.cs.drag, Drag::None) {
        if let Some((a, b)) = app.cs.measured {
            draw_measure(app, painter, a, b);
        }
    }
    // Shape builder: the faces, the one under the pointer, and those dragged across
    if app.tool == Tool::ShapeBuilder {
        if let Some((_, faces)) = &app.cs.builder {
            let delete = painter.ctx().input(|i| i.modifiers.alt);
            let hot = if delete { Color32::from_rgb(230, 60, 60) } else { crate::chrome::ACCENT };
            let mut lit: Vec<usize> = match &app.cs.drag {
                Drag::Build { picked } => picked.clone(),
                _ => vec![],
            };
            if let Some(f) = cursor.and_then(|c| cutcraft_core::shapebuilder::face_at(faces, c)) {
                if !lit.contains(&f) {
                    lit.push(f);
                }
            }
            for (i, f) in faces.iter().enumerate() {
                let on = lit.contains(&i);
                if on {
                    let (v, idx) = crate::render::tessellate(std::slice::from_ref(&f.path), 0.05);
                    let mut mesh = egui::Mesh::default();
                    for p in v {
                        mesh.colored_vertex(view.to_screen(Point::new(p[0] as f64, p[1] as f64)), hot.gamma_multiply(0.25));
                    }
                    mesh.indices = idx;
                    painter.add(egui::Shape::mesh(mesh));
                }
                for pl in geom::flatten(&f.path, 0.1) {
                    let mut pts: Vec<Pos2> = pl.pts.iter().map(|q| view.to_screen(*q)).collect();
                    if on {
                        painter.add(crate::render::closed_outline(pts, Stroke::new(2.5, hot)));
                    } else {
                        if let Some(first) = pts.first().copied() {
                            pts.push(first);
                        }
                        painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, hot.gamma_multiply(0.5)), 4.0, 3.0));
                    }
                }
            }
        }
    }
    // Trim: show the outline a click would trim
    if app.tool == Tool::Trim && matches!(app.cs.drag, Drag::None) {
        let near = cursor.and_then(|c| cutcraft_core::trim::outline_near(&app.doc, c, 6.0 / view.zoom as f64));
        if let Some(s) = near.and_then(|id| app.doc.page().shape(id)) {
            for pl in geom::flatten_all(&s.world_paths(), 0.1) {
                let pts: Vec<Pos2> = pl.pts.iter().map(|q| view.to_screen(*q)).collect();
                let st = Stroke::new(2.5, Color32::from_rgb(230, 60, 60));
                painter.add(if pl.closed { crate::render::closed_outline(pts, st) } else { egui::Shape::line(pts, st) });
            }
        }
    }
    if app.tool == Tool::Eraser || app.tool == Tool::ImageEraser && matches!(app.ps.image_mode, ImageMode::Erase | ImageMode::Restore) {
        if let Some(c) = cursor {
            painter.circle_stroke(view.to_screen(c), (app.ps.eraser_radius as f32 * view.zoom).max(2.0), Stroke::new(1.0, Color32::RED));
        }
    }
    // Smart-guide lines for the move in progress.
    let canvas = app.view.canvas;
    for (vertical, pos) in &app.cs.alignments {
        let guide = Color32::from_rgb(230, 60, 200);
        let (a, b) = if *vertical {
            let x = view.to_screen(Point::new(*pos, 0.0)).x;
            (egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom()))
        } else {
            let y = view.to_screen(Point::new(0.0, *pos)).y;
            (egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y))
        };
        painter.add(egui::Shape::dashed_line(&[a, b], Stroke::new(1.0, guide), 5.0, 3.0));
    }
    if app.tool == Tool::Corner {
        // Corner markers on the selected paths: hollow = sharp, filled = rounded.
        let hover = cursor.map(|c| view.to_screen(c));
        for (_, _, handle, rounded) in corner_targets(app) {
            let p = view.to_screen(handle);
            let hot = hover.map(|h| h.distance(p) < 9.0).unwrap_or(false);
            let r = if hot { 6.0 } else { 4.5 };
            painter.circle_filled(p, r, if rounded { SEL_BLUE } else { Color32::WHITE });
            painter.circle_stroke(p, r, Stroke::new(1.5, SEL_BLUE));
        }
    }
}

/// Ruler guides (dashed) and the one being dragged (solid, with its position).
fn draw_guides(app: &App, painter: &egui::Painter, hover: Option<Pos2>) {
    let c = app.view.canvas;
    let full = |vertical: bool, pos: f64| -> [Pos2; 2] {
        let at = app.view.to_screen(Point::new(pos, pos));
        if vertical { [egui::pos2(at.x, c.top()), egui::pos2(at.x, c.bottom())] } else { [egui::pos2(c.left(), at.y), egui::pos2(c.right(), at.y)] }
    };
    let moving = match app.cs.drag {
        Drag::Guide { index, .. } => index,
        _ => None,
    };
    for (i, g) in app.doc.page().guides.iter().enumerate() {
        if Some(i) != moving {
            painter.add(egui::Shape::dashed_line(&full(g.vertical, g.pos), Stroke::new(1.0, GUIDE_BLUE), 6.0, 4.0));
        }
    }
    if let (Drag::Guide { vertical, .. }, Some(h)) = (&app.cs.drag, hover) {
        let q = snap(app, app.view.to_mm(h));
        let pos = if *vertical { q.x } else { q.y };
        let off = in_ruler(app, h).is_some();
        let line = full(*vertical, pos);
        painter.line_segment(line, Stroke::new(1.5, if off { GUIDE_BLUE.gamma_multiply(0.4) } else { GUIDE_BLUE }));
        let u = app.prefs.units;
        let label = if off { "release to remove".to_string() } else { format!("{} {}", fmt_num((pos * u.factor() * 100.0).round() / 100.0), u.suffix()) };
        // keep the label inside the canvas near its right edge
        let (at, align) = if h.x > c.right() - 160.0 { (h + egui::vec2(-12.0, 12.0), egui::Align2::RIGHT_TOP) } else { (h + egui::vec2(12.0, 12.0), egui::Align2::LEFT_TOP) };
        painter.text(at, align, label, egui::FontId::proportional(12.0), GUIDE_BLUE);
    }
}

fn draw_rulers(app: &App, painter: &egui::Painter) {
    let view = &app.view;
    let c = view.canvas;
    // follow the window theme
    let (bg, fg) = if app.prefs.dark_mode { (Color32::from_gray(38), Color32::from_gray(190)) } else { (Color32::from_gray(232), Color32::from_gray(80)) };
    let rh = RULER_PX;
    painter.rect_filled(egui::Rect::from_min_max(c.min, egui::pos2(c.max.x, c.min.y + rh)), 0.0, bg);
    painter.rect_filled(egui::Rect::from_min_max(c.min, egui::pos2(c.min.x + rh, c.max.y)), 0.0, bg);
    let u = app.prefs.units;
    let unit_mm = if u == cutcraft_core::prefs::Units::Inch { 25.4 } else { 10.0 };
    // choose a label step that is ~>= 50 px
    let mut step = unit_mm;
    while step * (view.zoom as f64) < 50.0 {
        step *= 2.0;
    }
    while step * (view.zoom as f64) > 160.0 && step > unit_mm / 8.0 {
        step /= 2.0;
    }
    let tl = view.to_mm(c.min);
    let br = view.to_mm(c.max);
    let mut x = (tl.x / step).floor() * step;
    while x <= br.x {
        let sx = view.to_screen(Point::new(x, 0.0)).x;
        painter.line_segment([egui::pos2(sx, c.min.y + 8.0), egui::pos2(sx, c.min.y + rh)], Stroke::new(1.0, fg));
        painter.text(egui::pos2(sx + 2.0, c.min.y + 1.0), egui::Align2::LEFT_TOP, format!("{}", fmt_num(x * u.factor())), egui::FontId::monospace(9.0), fg);
        x += step;
    }
    let mut y = (tl.y / step).floor() * step;
    while y <= br.y {
        let sy = view.to_screen(Point::new(0.0, y)).y;
        painter.line_segment([egui::pos2(c.min.x + 8.0, sy), egui::pos2(c.min.x + rh, sy)], Stroke::new(1.0, fg));
        painter.text(egui::pos2(c.min.x + 1.0, sy + 2.0), egui::Align2::LEFT_TOP, format!("{}", fmt_num(y * u.factor())), egui::FontId::monospace(9.0), fg);
        y += step;
    }
    if let Some(m) = app.cs.cursor_mm {
        let s = view.to_screen(m);
        painter.line_segment([egui::pos2(s.x, c.min.y), egui::pos2(s.x, c.min.y + rh)], Stroke::new(1.0, Color32::from_rgb(255, 120, 0)));
        painter.line_segment([egui::pos2(c.min.x, s.y), egui::pos2(c.min.x + rh, s.y)], Stroke::new(1.0, Color32::from_rgb(255, 120, 0)));
    }
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 { format!("{}", v.round() as i64) } else { format!("{:.1}", v) }
}

// ------------------------------------------------------------------ corner tool & image eraser

/// Corners the Round-corner tool can act on, for the selected path shapes:
/// (shape, vertex in the shape's local coords, marker position in mm, already rounded?).
fn corner_targets(app: &App) -> Vec<(ShapeId, Point, Point, bool)> {
    let mut out = Vec::new();
    for id in &app.sel {
        let Some(s) = app.doc.page().shape(*id) else { continue };
        if s.locked {
            continue;
        }
        if let ShapeKind::Path(p) = &s.kind {
            for c in cutcraft_core::corners::corners(p) {
                out.push((*id, c.vertex, s.transform * c.handle, c.radius.is_some()));
            }
        }
    }
    out
}

/// Round-corner tool click: round/chamfer the corner under the pointer, or
/// select the shape under it so its corners show.
fn corner_click(app: &mut App, cur_screen: Pos2, cur: Point, tol: f64) {
    use cutcraft_core::corners::{CornerCut, set_corner};
    let hit = corner_targets(app)
        .into_iter()
        .map(|(id, v, h, _)| (id, v, app.view.to_screen(h).distance(cur_screen)))
        .filter(|t| t.2 < 9.0)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((id, vertex, _)) = hit else {
        app.sel = app.doc.page().hit_test(cur, tol).into_iter().collect();
        let is_path = app.sel.first().and_then(|id| app.doc.page().shape(*id)).map(|s| matches!(s.kind, ShapeKind::Path(_))).unwrap_or(false);
        app.status = if app.sel.is_empty() {
            "Round corner: click a shape, then click one of its corner markers".into()
        } else if is_path {
            "Click a corner marker to round it (filled markers are already rounded; radius 0 makes one sharp)".into()
        } else {
            "Convert text, groups and images to a path first (Object ▸ Convert to path)".into()
        };
        return;
    };
    let style = if app.ps.corner_chamfer { CornerCut::Chamfer } else { CornerCut::Round };
    let size = app.ps.corner_radius.max(0.0);
    app.checkpoint_named("Round corner");
    if let Some(s) = app.doc.page_mut().shape_mut(id) {
        // The radius is in mm on the page; convert to the shape's local units.
        let k = s.transform.determinant().abs().sqrt().max(1e-9);
        if let ShapeKind::Path(p) = &mut s.kind {
            *p = set_corner(p, vertex, size / k, style);
        }
    }
    app.status = match (size > 0.0, app.ps.corner_chamfer) {
        (false, _) => "Corner made sharp".into(),
        (true, false) => format!("Corner rounded ({size:.1} mm)"),
        (true, true) => format!("Corner chamfered ({size:.1} mm)"),
    };
}

/// Image eraser: rub the brush path out of the selected pictures (or, with none
/// selected, any picture under the brush). Erased parts become transparent.
/// The picture the image tool works on at `p`: a selected one, else the top one under it.
fn image_at(app: &App, p: Point, reach: f64) -> Vec<ShapeId> {
    let page = app.doc.page();
    let is_target = |s: &Shape| s.is_image() && !s.locked && !s.hidden && page.is_editable(s);
    let selected: Vec<ShapeId> = app.sel.iter().copied().filter(|id| page.shape(*id).is_some_and(is_target)).collect();
    if !selected.is_empty() {
        return selected;
    }
    page.shapes.iter().rev().filter(|s| is_target(s) && s.bbox().is_some_and(|b| b.inflate(reach, reach).contains(p))).map(|s| s.id).take(1).collect()
}

/// Image eraser / restore brush along `pts`.
fn image_erase(app: &mut App, pts: &[Point], restore: bool) {
    let r = app.ps.eraser_radius.max(0.1);
    let mut targets: Vec<ShapeId> = vec![];
    for p in pts {
        for id in image_at(app, *p, r) {
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
    }
    let mut edited = 0;
    let mut nothing_saved = false;
    for id in targets {
        let Some(s) = app.doc.page().shape(id) else { continue };
        let ShapeKind::Image(img) = &s.kind else { continue };
        let inv = s.transform.inverse();
        let k = s.transform.determinant().abs().sqrt().max(1e-9);
        let spots: Vec<(f64, f64)> = pts.iter().map(|p| inv * *p).map(|q| (q.x, q.y)).collect();
        let result = if restore {
            match &img.original {
                Some(orig) => cutcraft_core::raster::restore(&img.png, orig, &spots, r / k, img.width_mm, img.height_mm),
                None => {
                    nothing_saved = true;
                    continue;
                }
            }
        } else {
            cutcraft_core::raster::erase(&img.png, &spots, r / k, img.width_mm, img.height_mm)
        };
        if let Ok(png) = result {
            if edited == 0 {
                app.checkpoint_named(if restore { "Restore image" } else { "Erase image" });
            }
            if let Some(ShapeKind::Image(img)) = app.doc.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                // keep the untouched picture for the Restore brush
                if img.original.is_none() {
                    img.original = Some(img.png.clone());
                }
                img.png = std::sync::Arc::new(png);
            }
            edited += 1;
        }
    }
    app.status = match (edited, restore, nothing_saved) {
        (0, true, true) => "Restore: this picture hasn't been erased".into(),
        (0, true, false) => "Restore: nothing erased under the brush".into(),
        (0, false, _) => "Image eraser: no picture under the brush".into(),
        (_, true, _) => "Restored - Ctrl+Z to undo".into(),
        (_, false, _) => "Erased - Ctrl+Z to undo".into(),
    };
}

/// Select & Erase: clear the area of one colour under the click.
fn image_select_erase(app: &mut App, cur: Point) {
    let Some(id) = image_at(app, cur, 0.0).into_iter().find(|id| app.doc.page().shape(*id).and_then(|s| s.bbox()).is_some_and(|b| b.contains(cur))) else {
        app.status = "Select & Erase: click on a picture".into();
        return;
    };
    let Some(s) = app.doc.page().shape(id) else { return };
    let ShapeKind::Image(img) = &s.kind else { return };
    let q = s.transform.inverse() * cur;
    match cutcraft_core::raster::select_erase(&img.png, (q.x, q.y), app.ps.select_tolerance, app.ps.select_everywhere, img.width_mm, img.height_mm) {
        Ok((png, n)) => {
            app.checkpoint_named("Select & erase");
            if let Some(ShapeKind::Image(img)) = app.doc.page_mut().shape_mut(id).map(|s| &mut s.kind) {
                if img.original.is_none() {
                    img.original = Some(img.png.clone());
                }
                img.png = std::sync::Arc::new(png);
            }
            app.status = format!("Cleared {n} pixels - Ctrl+Z to undo, or use Restore to paint parts back");
        }
        Err(e) => app.status = format!("Select & Erase: {e}"),
    }
}

/// Crop: keep only the part of the picture inside `r` (page mm).
fn image_crop(app: &mut App, r: Rect) {
    let Some(id) = image_at(app, r.center(), 0.0).into_iter().next() else {
        app.status = "Crop: drag a box over a picture".into();
        return;
    };
    let Some(s) = app.doc.page().shape(id) else { return };
    let ShapeKind::Image(img) = &s.kind else { return };
    let inv = s.transform.inverse();
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].map(|p| inv * p);
    let local = corners.iter().skip(1).fold(Rect::from_points(corners[0], corners[0]), |b, p| b.union_pt(*p));
    match cutcraft_core::raster::crop(&img.png, local, img.width_mm, img.height_mm) {
        Ok((png, w, h, kept)) => {
            let original = img.original.as_ref().and_then(|o| cutcraft_core::raster::crop(o, kept, img.width_mm, img.height_mm).ok()).map(|(o, ..)| std::sync::Arc::new(o));
            app.checkpoint_named("Crop image");
            if let Some(s) = app.doc.page_mut().shape_mut(id) {
                s.transform = s.transform * Affine::translate(kept.origin().to_vec2());
                if let ShapeKind::Image(img) = &mut s.kind {
                    img.png = std::sync::Arc::new(png);
                    (img.px_w, img.px_h) = (w, h);
                    (img.width_mm, img.height_mm) = (kept.width(), kept.height());
                    img.original = original;
                }
            }
            app.status = "Cropped - Ctrl+Z to undo".into();
        }
        Err(e) => app.status = format!("Crop: {e}"),
    }
}

// ------------------------------------------------------------------ context menu

/// Right-click menu on the canvas, with items appropriate to the selection.
fn selection_context_menu(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    use crate::app::Action::*;
    ui.set_min_width(170.0);

    let sel = app.sel.clone();
    let n = sel.len();
    let (mut has_group, mut has_text, mut has_image, mut has_nonpath) = (false, false, false, false);
    for id in &sel {
        if let Some(s) = app.doc.page().shape(*id) {
            match &s.kind {
                ShapeKind::Group(_) => {
                    has_group = true;
                    has_nonpath = true;
                }
                ShapeKind::Text { .. } => {
                    has_text = true;
                    has_nonpath = true;
                }
                ShapeKind::Image(_) => {
                    has_image = true;
                    has_nonpath = true;
                }
                ShapeKind::Warp(_) => has_nonpath = true,
                ShapeKind::Path(_) => {}
            }
        }
    }

    // Nothing selected → canvas-level items.
    if n == 0 {
        if ui.button("Paste").clicked() {
            app.do_action(Paste, ctx);
            ui.close();
        }
        if ui.button("Select all").clicked() {
            app.do_action(SelectAll, ctx);
            ui.close();
        }
        ui.separator();
        if ui.button("Add page").clicked() {
            app.do_action(AddPage, ctx);
            ui.close();
        }
        if ui.button("Page setup…").clicked() {
            app.do_action(ShowPanel(Panel::Page), ctx);
            ui.close();
        }
        return;
    }

    if ui.button("Cut").clicked() {
        app.do_action(Cut, ctx);
        ui.close();
    }
    if ui.button("Copy").clicked() {
        app.do_action(Copy, ctx);
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        app.do_action(Duplicate, ctx);
        ui.close();
    }
    if ui.button("Delete").clicked() {
        app.do_action(Delete, ctx);
        ui.close();
    }
    ui.separator();

    if n >= 2 {
        if ui.button("Group").clicked() {
            app.do_action(Group, ctx);
            ui.close();
        }
        if ui.button("Fit into box").on_hover_text("Scale the rest into the largest selected shape").clicked() {
            app.do_action(FitIntoBox, ctx);
            ui.close();
        }
        ui.menu_button("Combine", |ui| {
            if ui.button("Weld (union)").clicked() {
                app.do_action(Weld, ctx);
                ui.close();
            }
            if ui.button("Subtract").clicked() {
                app.do_action(Subtract, ctx);
                ui.close();
            }
            if ui.button("Intersect").clicked() {
                app.do_action(Intersect, ctx);
                ui.close();
            }
            if ui.button("Exclude").clicked() {
                app.do_action(Exclude, ctx);
                ui.close();
            }
            if ui.button("Unite (keep parts)").clicked() {
                app.do_action(Unite, ctx);
                ui.close();
            }
            if ui.button("Divide").clicked() {
                app.do_action(Divide, ctx);
                ui.close();
            }
        });
    }
    if has_group && ui.button("Ungroup").clicked() {
        app.do_action(Ungroup, ctx);
        ui.close();
    }
    ui.menu_button("Arrange", |ui| {
        if ui.button("Bring to front").clicked() {
            app.do_action(Arrange(ops::Arrange::Front), ctx);
            ui.close();
        }
        if ui.button("Bring forward").clicked() {
            app.do_action(Arrange(ops::Arrange::Forward), ctx);
            ui.close();
        }
        if ui.button("Send backward").clicked() {
            app.do_action(Arrange(ops::Arrange::Backward), ctx);
            ui.close();
        }
        if ui.button("Send to back").clicked() {
            app.do_action(Arrange(ops::Arrange::Back), ctx);
            ui.close();
        }
    });
    ui.menu_button("Transform", |ui| {
        if ui.button("Flip horizontal").clicked() {
            app.do_action(FlipH, ctx);
            ui.close();
        }
        if ui.button("Flip vertical").clicked() {
            app.do_action(FlipV, ctx);
            ui.close();
        }
        if ui.button("Rotate 90°").clicked() {
            app.do_action(Rotate(90.0), ctx);
            ui.close();
        }
    });
    ui.separator();

    // Type-specific shortcuts.
    if has_image && ui.button("Trace to cut lines…").clicked() {
        app.do_action(ShowPanel(Panel::Trace), ctx);
        ui.close();
    }
    if has_image && ui.button("Image tools (adjust, photo → lines, shadow box)…").clicked() {
        app.do_action(ShowPanel(Panel::Image), ctx);
        ui.close();
    }
    if has_text && ui.button("Edit text…").clicked() {
        app.do_action(ShowPanel(Panel::Text), ctx);
        ui.close();
    }
    if ui.button("Flatten (print then cut)").clicked() {
        app.do_action(Flatten, ctx);
        ui.close();
    }
    if sel.iter().any(|id| app.doc.page().shape(*id).is_some_and(|s| s.name == cutcraft_core::printflat::FLATTENED)) && ui.button("Unflatten").clicked() {
        app.do_action(Unflatten, ctx);
        ui.close();
    }
    if has_text && ui.button("Ungroup to letters").clicked() {
        app.do_action(UngroupLetters, ctx);
        ui.close();
    }
    if has_nonpath && ui.button("Convert to path").clicked() {
        app.do_action(ConvertToPath, ctx);
        ui.close();
    }
    if ui.button("Distort / live effects…").clicked() {
        app.do_action(ShowPanel(Panel::Distort), ctx);
        ui.close();
    }
    if ui.button("Fill sheet with copies…").clicked() {
        app.do_action(ShowPanel(Panel::Replicate), ctx);
        ui.close();
    }
    ui.separator();
    if ui.button("Deselect").clicked() {
        app.do_action(Deselect, ctx);
        ui.close();
    }
}

// ------------------------------------------------------------------ interaction

fn interact(app: &mut App, resp: &egui::Response, ctx: &egui::Context) {
    let Some(cur_screen) = resp.interact_pointer_pos().or(resp.hover_pos()) else { return };
    let cur = app.view.to_mm(cur_screen);
    let (shift, esc, enter) = ctx.input(|i| (i.modifiers.shift, i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
    let tol = 5.0 / app.view.zoom as f64;
    refresh_snap_points(app);
    app.cs.alignments.clear();
    if app.tool == Tool::ShapeBuilder {
        builder_faces(app);
    }

    if esc {
        if !app.cs.poly.is_empty() {
            app.cs.poly.clear();
        } else if !ctx.egui_wants_keyboard_input() {
            app.sel.clear();
            app.cs.node_sel = None;
        }
        app.cs.drag = Drag::None;
        app.cs.measured = None;
    }
    if enter && !app.cs.poly.is_empty() && !ctx.egui_wants_keyboard_input() {
        finish_poly(app, false);
    }

    // -------- drag start
    if resp.drag_started_by(PointerButton::Primary) {
        let start_screen = ctx.input(|i| i.pointer.press_origin()).unwrap_or(cur_screen);
        let start = app.view.to_mm(start_screen);
        app.cs.drag = match app.tool {
            _ if in_ruler(app, start_screen).is_some() => Drag::Guide { index: None, vertical: in_ruler(app, start_screen) == Some(true) },
            Tool::Select => start_select_drag(app, start, start_screen, shift),
            Tool::Node => {
                let mut d = Drag::None;
                if let Some((id, pts, _, _)) = crate::panels2::warp_handles(app) {
                    if let Some((idx, _)) = pts
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (i, app.view.to_screen(*p).distance(start_screen)))
                        .filter(|(_, d)| *d < 9.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                    {
                        app.checkpoint_named("Edit points");
                        d = Drag::WarpPt { id, idx };
                    }
                }
                let target = if matches!(d, Drag::None) { node_target(app) } else { None };
                if let Some((id, p, a)) = target {
                    let near = path_nodes(&p)
                        .into_iter()
                        .map(|(h, q)| (h, app.view.to_screen(a * q).distance(start_screen)))
                        .filter(|(_, d)| *d < 8.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    if let Some((h, _)) = near {
                        app.checkpoint_named("Edit points");
                        app.cs.node_sel = Some(h);
                        d = Drag::Node { h, id };
                    }
                }
                if matches!(d, Drag::None) { Drag::Rubber { start } } else { d }
            }
            Tool::Pan => Drag::Pan,
            Tool::Zoom => Drag::ZoomRect { start },
            Tool::Line | Tool::Rect | Tool::RoundRect | Tool::Ellipse | Tool::Polygon | Tool::Star => {
                Drag::Create { start: snap(app, start) }
            }
            Tool::Freehand => Drag::Freehand { pts: vec![start] },
            Tool::Knife => Drag::Knife { start },
            Tool::ShapeBuilder => Drag::Build { picked: face_under(app, start).into_iter().collect() },
            Tool::Eraser => Drag::Eraser { pts: vec![start] },
            Tool::ImageEraser => match app.ps.image_mode {
                ImageMode::Crop => Drag::ImageCrop { start },
                ImageMode::SelectErase => Drag::None,
                ImageMode::Erase | ImageMode::Restore => Drag::ImageErase { pts: vec![start] },
            },
            Tool::Measure => {
                app.cs.measured = None;
                Drag::Measure { start: snap(app, start) }
            }
            Tool::Dimension => Drag::Dimension { start: snap(app, start) },
            _ => Drag::None,
        };
    }

    // -------- dragging
    if resp.dragged_by(PointerButton::Primary) {
        match &mut app.cs.drag {
            Drag::Move { start, orig } => {
                let mut d = cur - *start;
                if shift {
                    if d.x.abs() > d.y.abs() { d.y = 0.0 } else { d.x = 0.0 }
                }
                let orig = orig.clone();
                let mut a = Affine::translate(d);
                if app.doc.page().grid.snap {
                    // snap the selection's top-left corner
                    let bb0 = orig_bbox(app, &orig);
                    if let Some(bb0) = bb0 {
                        let p = grid_snap(app, Point::new(bb0.x0 + d.x, bb0.y0 + d.y));
                        a = Affine::translate((p.x - bb0.x0, p.y - bb0.y0));
                    }
                }
                // Smart guides: line the selection's edges or centre up with others.
                if app.prefs.smart_guides {
                    if let (Some((xs, ys)), Some(bb0)) = (app.cs.guide_targets.clone(), orig_bbox(app, &orig)) {
                        let gtol = SNAP_PX / app.view.zoom as f64;
                        let off = a.translation();
                        let moved = bb0 + off;
                        let (mut dx, mut dy) = (off.x, off.y);
                        if let Some((c, line)) = best_alignment([moved.x0, moved.center().x, moved.x1], &xs, gtol) {
                            dx += c;
                            app.cs.alignments.push((true, line));
                        }
                        if let Some((c, line)) = best_alignment([moved.y0, moved.center().y, moved.y1], &ys, gtol) {
                            dy += c;
                            app.cs.alignments.push((false, line));
                        }
                        a = Affine::translate((dx, dy));
                    }
                }
                apply_transforms(app, &orig, a);
            }
            Drag::Scale { handle, bbox, orig } => {
                let (h, bb, orig) = (*handle, *bbox, orig.clone());
                let hp = handle_points(bb);
                let anchor = hp[(h + 4) % 8];
                let from = hp[h];
                let fx = if (from.x - anchor.x).abs() > 1e-9 { (cur.x - anchor.x) / (from.x - anchor.x) } else { 1.0 };
                let fy = if (from.y - anchor.y).abs() > 1e-9 { (cur.y - anchor.y) / (from.y - anchor.y) } else { 1.0 };
                let (mut sx, mut sy) = match h {
                    1 | 5 => (1.0, fy),
                    3 | 7 => (fx, 1.0),
                    _ => (fx, fy),
                };
                if h % 2 == 0 && !shift {
                    let s = if (sx - 1.0).abs() > (sy - 1.0).abs() { sx } else { sy };
                    sx = s;
                    sy = s;
                }
                let sx = if sx.abs() < 1e-3 { 1e-3 } else { sx };
                let sy = if sy.abs() < 1e-3 { 1e-3 } else { sy };
                let a = Affine::translate(anchor.to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-anchor.to_vec2());
                apply_transforms(app, &orig, a);
            }
            Drag::Rotate { center, a0, orig } => {
                let (c, a0, orig) = (*center, *a0, orig.clone());
                let mut ang = (cur - c).atan2() - a0;
                if shift {
                    let step = 15f64.to_radians();
                    ang = (ang / step).round() * step;
                }
                apply_transforms(app, &orig, Affine::rotate_about(ang, c));
                app.status = format!("Rotate {:.1}°", ang.to_degrees());
            }
            Drag::Build { picked } => {
                let f = app.cs.builder.as_ref().and_then(|(_, faces)| cutcraft_core::shapebuilder::face_at(faces, cur));
                if let Some(f) = f.filter(|f| !picked.contains(f)) {
                    picked.push(f);
                }
            }
            Drag::Freehand { pts } | Drag::Eraser { pts } | Drag::ImageErase { pts } => {
                if pts.last().map(|l| l.distance(cur) > 0.3 / app.view.zoom as f64 * 2.0).unwrap_or(true) {
                    pts.push(cur);
                }
            }
            Drag::WarpPt { id, idx } => {
                let (id, idx) = (*id, *idx);
                crate::panels2::set_warp_point(app, id, idx, cur);
            }
            Drag::Node { h, id } => {
                let (h, id) = (*h, *id);
                if let Some(s) = app.doc.page_mut().shape_mut(id) {
                    let inv = s.transform.inverse();
                    if let ShapeKind::Path(p) = &mut s.kind {
                        set_node(p, h, inv * cur);
                    }
                }
                app.touch();
            }
            _ => {}
        }
    }

    // -------- drag end
    if resp.drag_stopped_by(PointerButton::Primary) {
        let drag = std::mem::take(&mut app.cs.drag);
        match drag {
            Drag::Measure { start } => {
                let end = snap(app, cur);
                app.status = measure_text(app, start, end);
                app.cs.measured = Some((start, end));
            }
            Drag::Dimension { start } => add_dimension(app, start, snap(app, cur)),
            Drag::Guide { index, vertical } => {
                let off = !app.view.canvas.contains(cur_screen) || in_ruler(app, cur_screen).is_some();
                let q = snap(app, cur);
                let pos = if vertical { q.x } else { q.y };
                let n = app.doc.page().guides.len();
                match index {
                    Some(i) if i < n && off => {
                        app.checkpoint_named("Remove guide");
                        app.doc.page_mut().guides.remove(i);
                    }
                    Some(i) if i < n => {
                        app.checkpoint_named("Move guide");
                        app.doc.page_mut().guides[i].pos = pos;
                    }
                    None if !off => {
                        app.checkpoint_named("Add guide");
                        app.doc.page_mut().guides.push(cutcraft_core::doc::Guide { vertical, pos });
                    }
                    _ => {}
                }
            }
            Drag::Rubber { start } => {
                // left to right: shapes wholly inside; right to left: anything the box touches
                let hits = ops::marquee_pick(app.doc.page(), start, cur);
                if shift {
                    for h in hits {
                        if !app.sel.contains(&h) {
                            app.sel.push(h);
                        }
                    }
                } else {
                    app.sel = hits;
                }
            }
            Drag::ZoomRect { start } => {
                let r = Rect::from_points(start, cur);
                if r.width() * (app.view.zoom as f64) > 8.0 {
                    app.view.fit_rect = Some(r);
                }
            }
            Drag::Create { start } => {
                if let Some(p) = create_shape_path(app.tool, start, snap(app, cur), shift, app) {
                    app.checkpoint_named("Draw shape");
                    add_drawn(app, p);
                }
            }
            Drag::Freehand { pts } => {
                if pts.len() > 2 {
                    let closed = pts[0].distance(*pts.last().unwrap()) < 8.0 / app.view.zoom as f64;
                    let raw = geom::polyline_path(&pts, closed);
                    let p = geom::simplify_path(&raw, 0.25, true);
                    app.checkpoint_named("Freehand");
                    add_drawn(app, p);
                }
            }
            Drag::Build { picked } => {
                let alt = ctx.input(|i| i.modifiers.alt);
                build_shapes(app, &picked, alt);
            }
            Drag::Knife { start } => {
                app.checkpoint_named("Knife");
                let parts = ops::knife(&mut app.doc, &[start, cur]);
                app.status = if parts.is_empty() { "Knife: nothing was cut".into() } else { format!("Knife: {} pieces", parts.len()) };
                app.sel = parts;
            }
            Drag::Eraser { pts } => {
                app.checkpoint_named("Erase");
                let r = app.ps.eraser_radius;
                ops::erase(&mut app.doc, &pts, r);
                app.sel.retain(|id| app.doc.page().shape(*id).is_some());
            }
            Drag::ImageErase { pts } => image_erase(app, &pts, app.ps.image_mode == ImageMode::Restore),
            Drag::ImageCrop { start } => image_crop(app, Rect::from_points(start, cur)),
            Drag::Move { .. } | Drag::Scale { .. } | Drag::Rotate { .. } | Drag::Node { .. } | Drag::WarpPt { .. } => {
                app.rev += 1;
            }
            _ => {}
        }
    }

    // -------- clicks
    // Right-click selects the object under the cursor (for its context menu),
    // unless it's already part of the selection.
    if resp.secondary_clicked() {
        if let Some(id) = app.doc.page().hit_test(cur, tol) {
            if !app.sel.contains(&id) {
                app.sel = vec![id];
            }
        }
    }

    if resp.clicked_by(PointerButton::Primary) {
        match app.tool {
            Tool::Select => {
                let hit = app.doc.page().hit_test(cur, tol);
                match hit {
                    Some(id) if shift => {
                        if let Some(i) = app.sel.iter().position(|x| *x == id) {
                            app.sel.remove(i);
                        } else {
                            app.sel.push(id);
                        }
                    }
                    Some(id) => app.sel = vec![id],
                    None if !shift => app.sel.clear(),
                    None => {}
                }
            }
            Tool::Node => {
                let mut on_node = false;
                if let Some((_, p, a)) = node_target(app) {
                    if let Some((h, _)) = path_nodes(&p)
                        .into_iter()
                        .map(|(h, q)| (h, app.view.to_screen(a * q).distance(cur_screen)))
                        .filter(|(_, d)| *d < 8.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                    {
                        app.cs.node_sel = Some(h);
                        on_node = true;
                    }
                }
                if !on_node {
                    app.cs.node_sel = None;
                    app.sel = app.doc.page().hit_test(cur, tol).into_iter().collect();
                    if let Some(id) = app.sel.first().copied() {
                        if matches!(app.doc.page().shape(id).map(|s| &s.kind), Some(ShapeKind::Text { .. } | ShapeKind::Group(_))) {
                            app.status = "Convert text/groups to a path (Object ▸ Convert to path) to edit points".into();
                        }
                    }
                }
            }
            Tool::Zoom => app.view.zoom_by(if shift { 0.67 } else { 1.5 }, Some(cur_screen)),
            Tool::Corner => corner_click(app, cur_screen, cur, tol),
            Tool::Trim => trim_click(app, cur, 6.0 / app.view.zoom as f64),
            Tool::ShapeBuilder => {
                let picked: Vec<usize> = face_under(app, cur).into_iter().collect();
                build_shapes(app, &picked, ctx.input(|i| i.modifiers.alt));
            }
            Tool::ImageEraser => match app.ps.image_mode {
                ImageMode::SelectErase => image_select_erase(app, cur),
                ImageMode::Erase | ImageMode::Restore => image_erase(app, &[cur], app.ps.image_mode == ImageMode::Restore),
                ImageMode::Crop => {}
            },
            Tool::Polyline | Tool::Curve => {
                let p = snap(app, cur);
                if app.cs.poly.len() >= 3 && app.view.to_screen(app.cs.poly[0]).distance(cur_screen) < 8.0 {
                    finish_poly(app, true);
                } else {
                    app.cs.poly.push(p);
                }
            }
            Tool::Text => {
                let mut data = TextData { text: "Text".into(), ..app.ps.text.clone() };
                data.on_path = None;
                let id = app.create_text(snap(app, cur), data);
                app.sel = vec![id];
                app.panel = Panel::Text;
                app.ps.focus_text = true;
                app.tool = Tool::Select;
            }
            _ => {}
        }
    }

    if resp.double_clicked_by(PointerButton::Primary) {
        match app.tool {
            Tool::Polyline | Tool::Curve => {
                // the double-click's first click already added a point
                app.cs.poly.pop();
                finish_poly(app, false);
            }
            Tool::Select => {
                if let Some(id) = app.doc.page().hit_test(cur, tol) {
                    match app.doc.page().shape(id).map(|s| &s.kind) {
                        Some(ShapeKind::Text { .. }) => {
                            app.sel = vec![id];
                            app.panel = Panel::Text;
                            app.ps.focus_text = true;
                        }
                        Some(ShapeKind::Path(_)) => {
                            app.sel = vec![id];
                            app.tool = Tool::Node;
                        }
                        _ => {}
                    }
                }
            }
            Tool::Node => {
                if let Some((id, p, a)) = node_target(app) {
                    let w = geom::transformed(&[p.clone()], a);
                    if geom::distance_to_outline(&w, cur) < tol * 1.5 {
                        add_node(app, id, &p, a.inverse() * cur);
                    }
                }
            }
            _ => {}
        }
    }
}

fn orig_bbox(app: &App, orig: &[(ShapeId, Affine)]) -> Option<Rect> {
    let mut r = None;
    for (id, t) in orig {
        if let Some(s) = app.doc.page().shape(*id) {
            if let Some(b) = geom::paths_bbox(&geom::transformed(&s.local_paths(), *t)) {
                r = geom::union_rect(r, b);
            }
        }
    }
    r
}

fn start_select_drag(app: &mut App, start: Point, start_screen: Pos2, shift: bool) -> Drag {
    // handles of the current selection first
    if let Some(bb) = ops::selection_bbox(&app.doc, &app.editable_sel()) {
        if rot_handle(&app.view, bb).distance(start_screen) < HANDLE_PX + 4.0 {
            app.checkpoint_named("Rotate");
            let c = bb.center();
            return Drag::Rotate { center: c, a0: (start - c).atan2(), orig: orig_transforms(app) };
        }
        for (i, h) in handle_points(bb).iter().enumerate() {
            if app.view.to_screen(*h).distance(start_screen) < HANDLE_PX + 3.0 {
                app.checkpoint_named("Resize");
                return Drag::Scale { handle: i, bbox: bb, orig: orig_transforms(app) };
            }
        }
    }
    let tol = 5.0 / app.view.zoom as f64;
    let in_sel_box = ops::selection_bbox(&app.doc, &app.sel).map(|b| b.contains(start)).unwrap_or(false);
    match app.doc.page().hit_test(start, tol) {
        Some(id) => {
            if !app.sel.contains(&id) {
                if shift {
                    app.sel.push(id);
                } else {
                    app.sel = vec![id];
                }
            }
            if app.editable_sel().is_empty() {
                return Drag::None;
            }
            app.checkpoint_named("Move");
            app.cs.guide_targets = Some(guide_targets(app));
            Drag::Move { start, orig: orig_transforms(app) }
        }
        None if in_sel_box && !shift => {
            app.checkpoint_named("Move");
            app.cs.guide_targets = Some(guide_targets(app));
            Drag::Move { start, orig: orig_transforms(app) }
        }
        None => match guide_at(app, start_screen) {
            Some(i) => Drag::Guide { index: Some(i), vertical: app.doc.page().guides[i].vertical },
            None => Drag::Rubber { start },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetry_copies_mirror_and_turn_about_the_centre() {
        let c = Point::new(100.0, 50.0);
        let p = Point::new(130.0, 60.0);
        let mut s = Symmetry { mode: SymmetryMode::LeftRight, center: c, ..Default::default() };
        let q = s.copies()[0] * p;
        assert!((q - Point::new(70.0, 60.0)).hypot() < 1e-9);
        s.mode = SymmetryMode::FourWay;
        assert_eq!(s.copies().len(), 3);
        assert!(s.copies().iter().any(|t| (*t * p - Point::new(70.0, 40.0)).hypot() < 1e-9));
        s.mode = SymmetryMode::Radial;
        s.count = 6;
        s.mirror = false;
        assert_eq!(s.copies().len(), 5);
        // every copy keeps its distance from the centre
        assert!(s.copies().iter().all(|t| ((*t * p - c).hypot() - (p - c).hypot()).abs() < 1e-9));
        s.mirror = true;
        assert_eq!(s.copies().len(), 11);
        // the mirrored copy of a point on the first slice's middle line is that point
        let mid = c + Vec2::new((30f64).to_radians().cos(), (30f64).to_radians().sin()) * 20.0;
        assert!((s.copies()[5] * mid - mid).hypot() < 1e-9);
        s.mode = SymmetryMode::Off;
        assert!(s.copies().is_empty());
    }

    #[test]
    fn alignment_snaps_within_tolerance_to_the_nearest_target() {
        // selection edges at x = 10 (left), 15 (centre), 20 (right)
        let targets = [100.0, 21.0, 9.5];
        // left edge 10 → 9.5 is 0.5 away; right edge 20 → 21 is 1.0 away: the smaller wins
        assert_eq!(best_alignment([10.0, 15.0, 20.0], &targets, 2.0), Some((-0.5, 9.5)));
        // nothing within tolerance
        assert_eq!(best_alignment([10.0, 15.0, 20.0], &[50.0], 2.0), None);
        // centres line up too
        assert_eq!(best_alignment([10.0, 15.0, 20.0], &[15.4], 1.0).map(|(_, l)| l), Some(15.4));
    }
}
