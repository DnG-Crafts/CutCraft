//! Brother ScanNCut `.fcm` files (CanvasWorkspace / ScanNCut cut files), read
//! and written with the vendored `fcmlib` crate.
//!
//! A Brother machine is driven by saving an FCM file and opening it in
//! CanvasWorkspace (or loading it on the machine), rather than by streaming
//! commands. Facts used here were measured from 1 127 real FCM files:
//! coordinates are 1/100 mm with Y pointing down; each piece's outlines are in
//! its own coordinates around its centre, placed by the piece's transform; mat
//! 0 is the 12 × 12 in mat (cut area 296.67 × 298.80 mm), mat 1 the 12 × 24 in
//! mat (296.67 × 603.60 mm); the thumbnail is a 1-bit BMP of the mat.

use crate::cutjob::JobPlan;
use crate::doc::{CutStyle, MatKind, Page, Rgba, Shape, ShapeKind, Style};
use crate::geom;
use crate::materials::{Action, Tool};
use anyhow::{Result, anyhow};
use fcmlib::{
    CutData, FcmFile, FileHeader, FileType, FileVariant, Generator, Outline, Path as FcmPath, PathShape, PathTool, Piece, PieceRestrictions, PieceTable,
    Point as FcmPoint, SegmentBezier, SegmentLine,
};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _};

/// FCM units per millimetre.
const UNITS: f64 = 100.0;

/// A Brother mat: FCM mat id and cut area (FCM units), and thumbnail layout.
struct BrotherMat {
    id: u32,
    cut: (u32, u32),
    thumb: (u32, u32),
    blocks: (u8, u8),
}

const MAT_12X12: BrotherMat = BrotherMat { id: 0, cut: (29667, 29880), thumb: (88, 88), blocks: (3, 3) };
const MAT_12X24: BrotherMat = BrotherMat { id: 1, cut: (29667, 60360), thumb: (78, 128), blocks: (3, 4) };

fn brother_mat(m: MatKind) -> &'static BrotherMat {
    if m == MatKind::Mat12x24 { &MAT_12X24 } else { &MAT_12X12 }
}

/// The cut area of the Brother mat matching `m`, in mm.
pub fn cut_area_mm(m: MatKind) -> (f64, f64) {
    let b = brother_mat(m);
    (b.cut.0 as f64 / UNITS, b.cut.1 as f64 / UNITS)
}

fn pt(p: FcmPoint) -> Point {
    Point::new(p.x as f64, p.y as f64)
}

fn fpt(p: Point) -> FcmPoint {
    FcmPoint { x: p.x.round() as i32, y: p.y.round() as i32 }
}

// ------------------------------------------------------------------ import

/// Reads an FCM file: one shape per piece (a group when it has several
/// outlines), in mm. Cut outlines are red cut lines; drawn ones black and
/// not cut. Returns notes about anything left out.
pub fn import(data: &[u8]) -> Result<(Vec<Shape>, Vec<String>)> {
    let file = FcmFile::from_bytes(data).map_err(|e| anyhow!("Couldn't read the FCM file: {e}"))?;
    let mut shapes = vec![];
    let mut rhinestones = 0;
    for (_, piece) in &file.piece_table.pieces {
        let t = piece.transform.map_or(Affine::IDENTITY, |(a, b, c, d, e, f)| Affine::new([a as f64, b as f64, c as f64, d as f64, e as f64, f as f64]));
        let to_mm = Affine::scale(1.0 / UNITS) * t;
        let mut parts = vec![];
        for path in &piece.paths {
            rhinestones += path.rhinestones.len();
            let Some(shape) = &path.shape else { continue };
            let mut b = BezPath::new();
            b.move_to(pt(shape.start));
            for o in &shape.outlines {
                match o {
                    Outline::Line(segs) => segs.iter().for_each(|s| b.line_to(pt(s.end))),
                    Outline::Bezier(segs) => segs.iter().for_each(|s| b.curve_to(pt(s.control1), pt(s.control2), pt(s.end))),
                }
            }
            if !path.tool.contains(PathTool::PATH_OPEN) {
                b.close_path();
            }
            let cut = path.tool.intersects(PathTool::TOOL_CUT | PathTool::TOOL_PERFORATING);
            let fill = path.tool.contains(PathTool::FILL).then_some(Rgba(60, 60, 60, 255));
            let style = if cut {
                Style { fill, ..Style::default() }
            } else {
                Style { fill, stroke: Some(Rgba::BLACK), stroke_width: 0.35, cut: CutStyle::NoCut, no_print: false }
            };
            parts.push(Shape::new_path(0, to_mm * b, style));
        }
        match parts.len() {
            0 => {}
            1 => shapes.push(parts.pop().unwrap()),
            _ => {
                let mut g = Shape::new_path(0, BezPath::new(), Style::default());
                g.kind = ShapeKind::Group(parts);
                shapes.push(g);
            }
        }
    }
    let mut notes = vec![];
    if rhinestones > 0 {
        notes.push(format!("{rhinestones} rhinestone position(s) skipped"));
    }
    if shapes.iter().any(|s| s.leaves().iter().any(|l| l.style.cut == CutStyle::NoCut)) {
        notes.push("drawn (pen) lines come in as not-cut lines".into());
    }
    Ok((shapes, notes))
}

// ------------------------------------------------------------------ export

/// What a shape's group in the cut plan does, as an FCM tool.
fn tool_for(action: Action, tool: Tool) -> Option<PathTool> {
    match (action, tool) {
        (Action::None | Action::Stipple, _) => None,
        (_, Tool::SketchPen) | (Action::Sketch | Action::Score, _) => Some(PathTool::TOOL_DRAW | PathTool::TOOL_DRAW_ONLY),
        (Action::Emboss, _) | (_, Tool::EmbossTool) => Some(PathTool::TOOL_EMBOSS),
        _ => Some(PathTool::TOOL_CUT),
    }
}

/// Converts one subpath (page mm) to FCM outlines in piece-local units.
fn fcm_path(sub: &BezPath, local: Affine, tool: PathTool) -> Option<FcmPath> {
    let mut start = None;
    let mut first = Point::ORIGIN;
    let mut cur = Point::ORIGIN;
    let mut closed = false;
    let mut outlines: Vec<Outline> = vec![];
    let push_line = |outlines: &mut Vec<Outline>, end: Point| match outlines.last_mut() {
        Some(Outline::Line(v)) => v.push(SegmentLine { end: fpt(end) }),
        _ => outlines.push(Outline::Line(vec![SegmentLine { end: fpt(end) }])),
    };
    let push_curve = |outlines: &mut Vec<Outline>, s: SegmentBezier| match outlines.last_mut() {
        Some(Outline::Bezier(v)) => v.push(s),
        _ => outlines.push(Outline::Bezier(vec![s])),
    };
    for el in sub.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                let p = local * p;
                start = Some(fpt(p));
                first = p;
                cur = p;
            }
            PathEl::LineTo(p) => {
                let p = local * p;
                push_line(&mut outlines, p);
                cur = p;
            }
            PathEl::QuadTo(c, p) => {
                let (c, p) = (local * c, local * p);
                // raise to a cubic
                let c1 = cur + (c - cur) * (2.0 / 3.0);
                let c2 = p + (c - p) * (2.0 / 3.0);
                push_curve(&mut outlines, SegmentBezier { control1: fpt(c1), control2: fpt(c2), end: fpt(p) });
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                let (a, b, p) = (local * a, local * b, local * p);
                push_curve(&mut outlines, SegmentBezier { control1: fpt(a), control2: fpt(b), end: fpt(p) });
                cur = p;
            }
            PathEl::ClosePath => {
                if cur.distance(first) > 0.5 {
                    push_line(&mut outlines, first);
                }
                closed = true;
            }
        }
    }
    let start = start?;
    if outlines.is_empty() {
        return None;
    }
    let tool = if closed { tool } else { tool | PathTool::PATH_OPEN };
    Some(FcmPath { tool, shape: Some(PathShape { start, outlines }), rhinestone_diameter: None, rhinestones: vec![] })
}

/// Splits a path into its subpaths.
fn subpaths(p: &BezPath) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = vec![];
    for el in p.elements() {
        if matches!(el, PathEl::MoveTo(_)) || out.is_empty() {
            out.push(BezPath::new());
        }
        out.last_mut().unwrap().push(*el);
    }
    out
}

/// A summary of what an export wrote.
#[derive(Debug, Default, PartialEq)]
pub struct ExportSummary {
    pub pieces: usize,
    pub cut_paths: usize,
    pub draw_paths: usize,
    /// Parts of the design outside the mat's cut area.
    pub outside_cut_area: bool,
    pub skipped_stipple: bool,
}

/// Writes the page as an FCM cut file for the Brother mat matching the page's
/// mat (12 × 24 in, otherwise 12 × 12 in). Each top-level shape becomes one
/// piece; what each outline does (cut / draw / emboss) follows the cut plan's
/// groups, as for any other cutter. Curves stay curves, and the machine does
/// its own blade compensation, so overcut and compensation settings are not
/// applied.
pub fn export(page: &Page, plan: &JobPlan) -> Result<(Vec<u8>, ExportSummary)> {
    let mat = brother_mat(page.mat);
    let mut summary = ExportSummary::default();
    let mut pieces: Vec<(u16, Piece)> = vec![];
    let mut thumb_paths: Vec<BezPath> = vec![];
    for s in page.shapes.iter().filter(|s| page.is_visible(s)) {
        // (path mm, tool) for every outline of this shape that does something
        let mut parts: Vec<(BezPath, PathTool)> = vec![];
        for leaf in s.leaves() {
            if leaf.hidden || leaf.is_image() || leaf.style.cut == CutStyle::NoCut {
                continue;
            }
            let group = plan.groups.iter().find(|g| crate::cutjob::leaf_in_group(plan, g, s.layer, &leaf));
            let Some(g) = group.filter(|g| g.enabled) else { continue };
            if g.action == Action::Stipple {
                summary.skipped_stipple = true;
            }
            let tool = if leaf.style.cut == CutStyle::Draw { tool_for(Action::Sketch, Tool::SketchPen) } else { tool_for(g.action, g.settings.tool) };
            let Some(tool) = tool else { continue };
            let mut paths = leaf.cut_paths();
            if g.mirror {
                // iron-on: cut flipped across the media
                let flip = Affine::new([-1.0, 0.0, 0.0, 1.0, page.width, 0.0]);
                paths = paths.into_iter().map(|p| flip * p).collect();
            }
            if matches!(leaf.style.cut, CutStyle::Perforate { .. }) || g.action == Action::Perforate {
                let (on, off) = match leaf.style.cut {
                    CutStyle::Perforate { on, off } => (on, off),
                    _ => (3.0, 1.0),
                };
                let dashed = geom::dash(&geom::flatten_all(&paths, 0.05), on, off);
                paths = dashed.iter().map(|pl| geom::polyline_path(&pl.pts, false)).collect();
            }
            for p in paths {
                for sub in subpaths(&p) {
                    parts.push((sub, tool));
                }
            }
        }
        if parts.is_empty() {
            continue;
        }
        let Some(bb) = parts.iter().map(|(p, _)| p.bounding_box()).reduce(|a, b| a.union(b)) else { continue };
        let centre = bb.center();
        // piece-local units around the piece centre; the transform places it
        let local = Affine::scale(UNITS) * Affine::translate(-centre.to_vec2());
        let paths: Vec<FcmPath> = parts.iter().filter_map(|(p, tool)| fcm_path(p, local, *tool)).collect();
        if paths.is_empty() {
            continue;
        }
        for (p, tool) in &parts {
            if tool.contains(PathTool::TOOL_CUT) {
                summary.cut_paths += 1;
            } else {
                summary.draw_paths += 1;
            }
            thumb_paths.push(p.clone());
        }
        let area = Rect::new(0.0, 0.0, mat.cut.0 as f64 / UNITS, mat.cut.1 as f64 / UNITS);
        if bb.x0 < -0.01 || bb.y0 < -0.01 || bb.x1 > area.x1 + 0.01 || bb.y1 > area.y1 + 0.01 {
            summary.outside_cut_area = true;
        }
        pieces.push((
            pieces.len() as u16,
            Piece {
                width: (bb.width() * UNITS).round() as u32,
                height: (bb.height() * UNITS).round() as u32,
                transform: Some((1.0, 0.0, 0.0, 1.0, (centre.x * UNITS) as f32, (centre.y * UNITS) as f32)),
                expansion_limit_value: 0,
                reduction_limit_value: 0,
                restriction_flags: PieceRestrictions::PROHIBITION_OF_SEAM_ALLOWANCE_SETTING,
                label: String::new(),
                paths,
            },
        ));
    }
    if pieces.is_empty() {
        return Err(anyhow!("Nothing to cut - give shapes a cut style and enable a group"));
    }
    summary.pieces = pieces.len();
    let file = FcmFile {
        file_header: FileHeader {
            variant: FileVariant::FCM,
            version: "0100".into(),
            content_id: 400000002,
            short_name: String::new(),
            long_name: " ".into(),
            author_name: " ".into(),
            copyright: " ".into(),
            thumbnail_block_size_width: mat.blocks.0,
            thumbnail_block_size_height: mat.blocks.1,
            thumbnail: thumbnail(&thumb_paths, mat),
            generator: Generator::App(207), // current CanvasWorkspace
            print_to_cut: None,
        },
        cut_data: CutData { file_type: FileType::Cut, mat_id: mat.id, cut_width: mat.cut.0, cut_height: mat.cut.1, seam_allowance_width: 2000, alignment: None },
        piece_table: PieceTable { pieces },
    };
    let bytes = file.to_bytes().map_err(|e| anyhow!("Couldn't write the FCM file: {e}"))?;
    Ok((bytes, summary))
}

/// A 1-bit BMP of the mat showing the design (0 = ink, 1 = white, bottom row
/// first), the way CanvasWorkspace stores it.
///
/// Shapes are drawn filled, with their holes left white, and open lines as
/// lines. CanvasWorkspace draws outlines instead, but a ScanNCut takes half a
/// minute or more to read an outline thumbnail where outlines run into each
/// other (holes close to an edge, say), and the transfer fails; filled ones
/// it reads at once.
fn thumbnail(paths: &[BezPath], mat: &BrotherMat) -> Vec<u8> {
    let (w, h) = (mat.thumb.0 as usize, mat.thumb.1 as usize);
    let mut ink = vec![false; w * h];
    let (mw, mh) = (mat.cut.0 as f64 / UNITS, mat.cut.1 as f64 / UNITS);
    let k = (w as f64 / mw).min(h as f64 / mh);
    let lines = geom::flatten_all(paths, 0.5);
    // every closed outline together, even-odd, so holes stay white whether
    // they're in the same path as their outline or not
    let rings: Vec<&Vec<Point>> = lines.iter().filter(|pl| pl.closed && pl.pts.len() > 2).map(|pl| &pl.pts).collect();
    for y in 0..h {
        let cy = (y as f64 + 0.5) / k;
        // where the outlines cross this row
        let mut xs: Vec<f64> = vec![];
        for ring in &rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if (a.y > cy) != (b.y > cy) {
                    xs.push(a.x + (cy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for span in xs.chunks_exact(2) {
            let from = ((span[0] * k - 0.5).ceil().max(0.0)) as usize;
            let to = ((span[1] * k - 0.5).floor()).min(w as f64 - 1.0);
            if to >= 0.0 {
                for x in from..=to as usize {
                    ink[y * w + x] = true;
                }
            }
        }
    }
    for pl in lines.iter().filter(|pl| !pl.closed) {
        for seg in pl.pts.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let steps = ((a.distance(b) * k).ceil() as usize).max(1);
            for s in 0..=steps {
                let p = a.lerp(b, s as f64 / steps as f64);
                let (x, y) = ((p.x * k) as isize, (p.y * k) as isize);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    ink[y as usize * w + x as usize] = true;
                }
            }
        }
    }
    let stride = w.div_ceil(32) * 4;
    let size = 62 + stride * h;
    let mut out = Vec::with_capacity(size);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"BM");
    u32le(&mut out, size as u32);
    u32le(&mut out, 0);
    u32le(&mut out, 62);
    u32le(&mut out, 40);
    u32le(&mut out, w as u32);
    u32le(&mut out, h as u32);
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&1u16.to_le_bytes()); // bits per pixel
    u32le(&mut out, 0); // no compression
    u32le(&mut out, 0);
    u32le(&mut out, 5669); // 144 dpi, as CanvasWorkspace writes
    u32le(&mut out, 5669);
    u32le(&mut out, 2);
    u32le(&mut out, 2);
    out.extend_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]); // palette: black, white (CanvasWorkspace sets both 4th bytes)
    for row in (0..h).rev() {
        let mut bytes = vec![0u8; stride];
        for x in 0..w {
            if !ink[row * w + x] {
                bytes[x / 8] |= 0x80 >> (x % 8);
            }
        }
        // pad bits past the image width are white too, like the originals
        for x in w..stride * 8 {
            bytes[x / 8] |= 0x80 >> (x % 8);
        }
        out.extend_from_slice(&bytes);
    }
    out
}
