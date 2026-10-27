//! Generators: rhinestones, sketch hatch fill, weeding, puzzles, tiling, nesting, shadow/contour.

use crate::doc::{Document, Page, Rgba, Shape, ShapeId, ShapeKind, Style};
use crate::geom::{self, BoolOp, CornerStyle};
use crate::ops::{self, Sel};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, Vec2};

fn world(doc: &Document, sel: &[ShapeId]) -> Vec<BezPath> {
    sel.iter().filter_map(|id| doc.page().shape(*id)).flat_map(|s| s.world_paths()).collect()
}

fn layer_of(doc: &Document, sel: &[ShapeId]) -> u32 {
    sel.first().and_then(|id| doc.page().shape(*id)).map(|s| s.layer).unwrap_or(0)
}

fn add_group(doc: &mut Document, paths: Vec<BezPath>, style: Style, layer: u32, name: &str) -> Sel {
    if paths.is_empty() {
        return vec![];
    }
    let mut children = vec![];
    for p in paths {
        let id = doc.alloc_id();
        let mut s = Shape::new_path(id, p, style.clone());
        s.layer = layer;
        children.push(s);
    }
    let id = doc.alloc_id();
    let g = Shape {
        id,
        name: name.into(),
        layer,
        transform: Affine::IDENTITY,
        style,
        kind: ShapeKind::Group(children),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    };
    doc.page_mut().shapes.push(g);
    vec![id]
}

// ------------------------------------------------------------- rhinestones

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StoneLayout {
    /// Stones along the outline.
    Outline,
    /// Two lines: outline and an inset line.
    DoubleOutline,
    /// Hex grid fill.
    Fill,
}

/// Generate rhinestone holes (`diameter` mm) for the selection.
pub fn rhinestones(doc: &mut Document, sel: &[ShapeId], diameter: f64, spacing: f64, layout: StoneLayout) -> Sel {
    let shapes = world(doc, sel);
    if shapes.is_empty() || diameter <= 0.0 {
        return vec![];
    }
    let pitch = diameter + spacing.max(0.0);
    let r = diameter / 2.0;
    let mut centres: Vec<Point> = vec![];
    let along = |path: &BezPath, centres: &mut Vec<Point>| {
        for sp in geom::subpaths(path) {
            let len = geom::path_length(&sp);
            let n = (len / pitch).floor().max(1.0) as usize;
            let step = len / n as f64;
            for i in 0..n {
                if let Some((p, _)) = geom::point_at_length(&sp, i as f64 * step) {
                    centres.push(p);
                }
            }
        }
    };
    match layout {
        StoneLayout::Outline => {
            let outline = geom::boolean(&shapes, &[], BoolOp::Union);
            along(&outline, &mut centres);
        }
        StoneLayout::DoubleOutline => {
            let outline = geom::boolean(&shapes, &[], BoolOp::Union);
            along(&outline, &mut centres);
            let inner = geom::offset(&[outline], -pitch, CornerStyle::Round);
            along(&inner, &mut centres);
        }
        StoneLayout::Fill => {
            let region = geom::offset(&shapes, -r, CornerStyle::Round);
            if let Some(bb) = geom::paths_bbox(&[region.clone()]) {
                let dy = pitch * 3f64.sqrt() / 2.0;
                let mut y = bb.y0;
                let mut row = 0;
                while y <= bb.y1 {
                    let mut x = bb.x0 + if row % 2 == 1 { pitch / 2.0 } else { 0.0 };
                    while x <= bb.x1 {
                        let p = Point::new(x, y);
                        if geom::contains(&[region.clone()], p) {
                            centres.push(p);
                        }
                        x += pitch;
                    }
                    y += dy;
                    row += 1;
                }
            }
        }
    }
    let circles: Vec<BezPath> = centres.iter().map(|c| geom::circle_path(*c, r)).collect();
    let layer = layer_of(doc, sel);
    let style = Style { fill: Some(Rgba(200, 200, 210, 255)), stroke: Some(Rgba::BLACK), ..Style::default() };
    add_group(doc, circles, style, layer, "Rhinestones")
}

// ------------------------------------------------------------- sketch hatch

/// Parallel hatch lines inside the selection (for sketch pens).
pub fn hatch(doc: &mut Document, sel: &[ShapeId], spacing: f64, angle_deg: f64, cross: bool) -> Sel {
    let shapes = world(doc, sel);
    let Some(bb) = geom::paths_bbox(&shapes) else { return vec![] };
    let spacing = spacing.max(0.2);
    let c = bb.center();
    let r = (bb.width().hypot(bb.height())) / 2.0 + spacing;
    let mut lines: Vec<BezPath> = vec![];
    let mut angles = vec![angle_deg];
    if cross {
        angles.push(angle_deg + 90.0);
    }
    for a in angles {
        let rot = Affine::rotate_about(a.to_radians(), c);
        let mut y = -r;
        let mut flip = false;
        while y <= r {
            let (mut p0, mut p1) = (Point::new(c.x - r, c.y + y), Point::new(c.x + r, c.y + y));
            if flip {
                std::mem::swap(&mut p0, &mut p1);
            }
            lines.push(geom::polyline_path(&[rot * p0, rot * p1], false));
            y += spacing;
            flip = !flip;
        }
    }
    let clipped = geom::clip_lines(&lines, &shapes, true);
    let layer = layer_of(doc, sel);
    let style = Style { stroke: Some(Rgba(30, 30, 200, 255)), ..Style::default() };
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, clipped, style);
    s.layer = layer;
    s.name = "Sketch fill".into();
    doc.page_mut().shapes.push(s);
    vec![id]
}

// ------------------------------------------------------------- weeding

/// Weed box around the selection plus `lines` evenly spaced horizontal and vertical weed lines.
pub fn weed(doc: &mut Document, sel: &[ShapeId], margin: f64, lines: usize) -> Sel {
    let shapes = world(doc, sel);
    let Some(bb) = geom::paths_bbox(&shapes) else { return vec![] };
    let b = bb.inflate(margin, margin);
    let mut out = vec![geom::rect_path(b)];
    if lines > 0 {
        let mut ls = vec![];
        for i in 1..=lines {
            let t = i as f64 / (lines + 1) as f64;
            let x = b.x0 + b.width() * t;
            let y = b.y0 + b.height() * t;
            ls.push(geom::polyline_path(&[Point::new(x, b.y0), Point::new(x, b.y1)], false));
            ls.push(geom::polyline_path(&[Point::new(b.x0, y), Point::new(b.x1, y)], false));
        }
        // keep a small gap around the shapes so the lines don't touch them
        let keepout = geom::offset(&shapes, 1.0, CornerStyle::Round);
        out.push(geom::clip_lines(&ls, &[keepout], false));
    }
    let layer = layer_of(doc, sel);
    add_group(doc, out, Style { stroke: Some(Rgba(120, 120, 120, 255)), ..Style::default() }, layer, "Weed lines")
}

// ------------------------------------------------------------- puzzle

pub fn puzzle(doc: &mut Document, area: Rect, rows: usize, cols: usize, knob: f64, seed: u64) -> Sel {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let mut rng = seed.max(1);
    let mut rand = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % 1000) as f64 / 1000.0
    };
    let (cw, ch) = (area.width() / cols as f64, area.height() / rows as f64);
    let mut paths = vec![geom::rect_path(area)];
    // an edge from a to b with a tab bulging to one side
    let edge = |a: Point, b: Point, rand: &mut dyn FnMut() -> f64| -> BezPath {
        let d = b - a;
        let len = d.hypot();
        let u = d / len;
        let n = Vec2::new(-u.y, u.x) * if rand() > 0.5 { 1.0 } else { -1.0 };
        let k = knob * len;
        let j = (rand() - 0.5) * 0.1 * len;
        let at = |t: f64, h: f64| a + u * (t * len + j) + n * h;
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(at(0.35, 0.0));
        p.curve_to(at(0.42, 0.0), at(0.32, k), at(0.5, k));
        p.curve_to(at(0.68, k), at(0.58, 0.0), at(0.65, 0.0));
        p.line_to(b);
        p
    };
    for r in 1..rows {
        for c in 0..cols {
            let y = area.y0 + ch * r as f64;
            paths.push(edge(Point::new(area.x0 + cw * c as f64, y), Point::new(area.x0 + cw * (c + 1) as f64, y), &mut rand));
        }
    }
    for c in 1..cols {
        for r in 0..rows {
            let x = area.x0 + cw * c as f64;
            paths.push(edge(Point::new(x, area.y0 + ch * r as f64), Point::new(x, area.y0 + ch * (r + 1) as f64), &mut rand));
        }
    }
    add_group(doc, paths, Style::default(), 0, "Puzzle")
}

// ------------------------------------------------------------- tiling

/// Split the active page into new pages of `tile_w`×`tile_h` with `overlap` mm.
/// Returns the number of tiles created.
pub fn tile_to_pages(doc: &mut Document, tile_w: f64, tile_h: f64, overlap: f64) -> usize {
    let src = doc.page().clone();
    let Some(bb) = src.shapes.iter().filter_map(|s| s.bbox()).fold(None, |a, b| geom::union_rect(a, b)) else {
        return 0;
    };
    let step_x = (tile_w - overlap).max(1.0);
    let step_y = (tile_h - overlap).max(1.0);
    let nx = ((bb.width() - overlap) / step_x).ceil().max(1.0) as usize;
    let ny = ((bb.height() - overlap) / step_y).ceil().max(1.0) as usize;
    let mut count = 0;
    for j in 0..ny {
        for i in 0..nx {
            let r = Rect::new(bb.x0 + i as f64 * step_x, bb.y0 + j as f64 * step_y, 0.0, 0.0);
            let r = Rect::new(r.x0, r.y0, r.x0 + tile_w, r.y0 + tile_h);
            let clip = geom::rect_path(r);
            let mut page = Page::new(&format!("{} tile {},{}", src.name, i + 1, j + 1), tile_w, tile_h);
            page.mat = src.mat;
            page.layers = src.layers.clone();
            let back = Affine::translate((-r.x0, -r.y0));
            for s in &src.shapes {
                for leaf in s.leaves() {
                    if leaf.is_image() || leaf.hidden {
                        continue;
                    }
                    let w = leaf.world_paths();
                    let closed = geom::boolean(&w, &[clip.clone()], BoolOp::Intersection);
                    let open = geom::clip_lines(&w, &[clip.clone()], true);
                    let mut merged = geom::merge_paths(&[closed, open]);
                    if merged.elements().is_empty() {
                        continue;
                    }
                    merged.apply_affine(back);
                    let id = doc.alloc_id();
                    let mut ns = Shape::new_path(id, merged, leaf.style.clone());
                    ns.layer = leaf.layer;
                    page.shapes.push(ns);
                }
            }
            if !page.shapes.is_empty() {
                doc.pages.push(page);
                count += 1;
            }
        }
    }
    count
}

/// Like [`tile_to_pages`], but the seams between tiles are cut as interlocking
/// joints (dovetail or puzzle knob) so the pieces lock together when assembled.
/// The design is split into equal tiles (no overlap), each on its own
/// `tile_w` × `tile_h` page with room around it for the joints that stick out.
/// Returns the number of pages created. Ported from BurnCraft's tile splitter.
pub fn tile_to_pages_jointed(doc: &mut Document, tile_w: f64, tile_h: f64, joint: crate::tiles::Joint, size: f64, spacing: f64) -> usize {
    use crate::tiles::{TileSpec, tile_cells};
    let src = doc.page().clone();
    let Some(bb) = src.shapes.iter().filter_map(|s| s.bbox()).fold(None, |a, b| geom::union_rect(a, b)) else {
        return 0;
    };
    // A puzzle knob's head reaches 1.0 × size past the seam (a dovetail 0.7 ×);
    // keep that much, plus a little, free on each side of every tile.
    let margin = size.max(0.0) + 2.0;
    let (use_w, use_h) = ((tile_w - 2.0 * margin).max(5.0), (tile_h - 2.0 * margin).max(5.0));
    let cols = (bb.width() / use_w).ceil().max(1.0) as usize;
    let rows = (bb.height() / use_h).ceil().max(1.0) as usize;
    let spec = TileSpec { cols, rows, joint, size, spacing };
    let mut count = 0;
    for (i, j, cell) in tile_cells(bb, &spec) {
        let x0 = bb.x0 + bb.width() * i as f64 / cols as f64;
        let y0 = bb.y0 + bb.height() * j as f64 / rows as f64;
        let back = Affine::translate((margin - x0, margin - y0));
        let mut page = Page::new(&format!("{} tile {},{}", src.name, i + 1, j + 1), tile_w, tile_h);
        page.mat = src.mat;
        page.layers = src.layers.clone();
        for s in &src.shapes {
            for leaf in s.leaves() {
                if leaf.is_image() || leaf.hidden {
                    continue;
                }
                let w = leaf.world_paths();
                let closed = geom::boolean(&w, &[cell.clone()], BoolOp::Intersection);
                let open = geom::clip_lines(&w, &[cell.clone()], true);
                let mut merged = geom::merge_paths(&[closed, open]);
                if merged.elements().is_empty() {
                    continue;
                }
                merged.apply_affine(back);
                let id = doc.alloc_id();
                let mut ns = Shape::new_path(id, merged, leaf.style.clone());
                ns.layer = leaf.layer;
                page.shapes.push(ns);
            }
        }
        if !page.shapes.is_empty() {
            doc.pages.push(page);
            count += 1;
        }
    }
    count
}

// ------------------------------------------------------------- nesting

/// A rasterised footprint of a shape: a tight occupancy grid at `res` cells/mm
/// whose cell (0,0) corresponds to the shape's world bounding-box minimum.
struct Footprint {
    grid: Vec<bool>,
    w: usize,
    h: usize,
    angle: f64,
}

/// Flatten a path into closed polylines (world coordinates).
fn to_polys(path: &kurbo::BezPath) -> Vec<Vec<Point>> {
    use kurbo::PathEl;
    let mut polys: Vec<Vec<Point>> = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), 0.25, |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() > 1 {
                polys.push(std::mem::take(&mut cur));
            }
            cur = vec![p];
        }
        PathEl::LineTo(p) => cur.push(p),
        PathEl::ClosePath => {
            if cur.len() > 1 {
                polys.push(std::mem::take(&mut cur));
            }
        }
        _ => {}
    });
    if cur.len() > 1 {
        polys.push(cur);
    }
    polys
}

fn point_in_polys(polys: &[Vec<Point>], x: f64, y: f64) -> bool {
    let mut inside = false;
    for poly in polys {
        let n = poly.len();
        let mut j = n - 1;
        for i in 0..n {
            let (pi, pj) = (poly[i], poly[j]);
            if (pi.y > y) != (pj.y > y) && x < (pj.x - pi.x) * (y - pi.y) / (pj.y - pi.y) + pi.x {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

/// Rasterise a world-space path (rotated by `angle` degrees) into a tight mask.
fn footprint(path: &kurbo::BezPath, angle: f64, res: f64) -> Footprint {
    let rotated = if angle != 0.0 {
        let mut p = path.clone();
        p.apply_affine(kurbo::Affine::rotate(angle.to_radians()));
        p
    } else {
        path.clone()
    };
    let bb = rotated.bounding_box();
    let w = ((bb.width() * res).ceil() as usize + 1).max(1);
    let h = ((bb.height() * res).ceil() as usize + 1).max(1);
    let polys = to_polys(&rotated);
    let mut grid = vec![false; w * h];
    for cy in 0..h {
        for cx in 0..w {
            let wx = bb.x0 + (cx as f64 + 0.5) / res;
            let wy = bb.y0 + (cy as f64 + 0.5) / res;
            if point_in_polys(&polys, wx, wy) {
                grid[cy * w + cx] = true;
            }
        }
    }
    Footprint { grid, w, h, angle }
}

/// The world outline of a single shape (falls back to its bounding box).
fn shape_outline(doc: &Document, id: ShapeId) -> Option<kurbo::BezPath> {
    let v = world(doc, &[id]);
    if v.iter().any(|p| !p.elements().is_empty()) {
        return Some(geom::merge_paths(&v));
    }
    doc.page().shape(id).and_then(|s| s.bbox()).map(geom::rect_path)
}

/// True-shape nesting: rasterise each shape's real outline and pack them with a
/// top-left scan, trying rotations so concave shapes interlock. `padding` is the
/// minimum gap (mm) kept between shapes. Returns false if some didn't fit.
pub fn nest(doc: &mut Document, sel: &[ShapeId], padding: f64, allow_rotate: bool, margin: f64) -> bool {
    let pr = doc.page().rect();
    let area = Rect::new(pr.x0 + margin, pr.y0 + margin, pr.x1 - margin, pr.y1 - margin);
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return false;
    }
    // Resolution: aim for a grid no larger than ~320 cells on the long side.
    let res = (320.0 / area.width().max(area.height())).clamp(0.3, 3.0);
    let gw = (area.width() * res).ceil() as usize + 1;
    let gh = (area.height() * res).ceil() as usize + 1;
    let pad_cells = (padding * res).round().max(0.0) as usize;
    let angles: &[f64] = if allow_rotate { &[0.0, 90.0, 180.0, 270.0] } else { &[0.0] };

    // Collect outlines, largest first (area of the world bbox as a cheap proxy).
    let mut items: Vec<(ShapeId, kurbo::BezPath, f64)> = sel
        .iter()
        .filter_map(|id| shape_outline(doc, *id).map(|p| (*id, p.clone(), p.bounding_box().area())))
        .collect();
    items.sort_by(|a, b| b.2.total_cmp(&a.2));

    let mut occ = vec![false; gw * gh];
    let mut ok = true;

    for (id, path, _) in items {
        // Best placement across rotations: minimise (row, col) for top-left packing.
        let mut best: Option<(usize, usize, Footprint)> = None;
        for &angle in angles {
            let fp = footprint(&path, angle, res);
            if fp.w > gw || fp.h > gh {
                continue;
            }
            'scan: for py in 0..=gh - fp.h {
                for px in 0..=gw - fp.w {
                    // Fit test: no set footprint cell may overlap an occupied cell.
                    let fits = (0..fp.h).all(|my| {
                        (0..fp.w).all(|mx| !(fp.grid[my * fp.w + mx] && occ[(py + my) * gw + px + mx]))
                    });
                    if fits {
                        let better = best.as_ref().map(|(by, bx, _)| (py, px) < (*by, *bx)).unwrap_or(true);
                        if better {
                            best = Some((py, px, fp));
                        }
                        break 'scan;
                    }
                }
            }
        }

        let Some((py, px, fp)) = best else {
            ok = false;
            continue;
        };
        // Mark the footprint plus a `pad_cells` border as occupied (spacing).
        for my in 0..fp.h {
            for mx in 0..fp.w {
                if !fp.grid[my * fp.w + mx] {
                    continue;
                }
                let y0 = (py + my).saturating_sub(pad_cells);
                let y1 = (py + my + pad_cells).min(gh - 1);
                let x0 = (px + mx).saturating_sub(pad_cells);
                let x1 = (px + mx + pad_cells).min(gw - 1);
                for oy in y0..=y1 {
                    for ox in x0..=x1 {
                        occ[oy * gw + ox] = true;
                    }
                }
            }
        }
        // Apply: rotate the shape, then align its world bbox min to the cell.
        if fp.angle != 0.0 {
            ops::rotate(doc, &[id], fp.angle);
        }
        if let Some(b) = doc.page().shape(id).and_then(|s| s.bbox()) {
            let tx = area.x0 + px as f64 / res;
            let ty = area.y0 + py as f64 / res;
            ops::move_by(doc, &[id], Vec2::new(tx - b.x0, ty - b.y0));
        }
    }
    ok
}

// ------------------------------------------------------------- batch / fill sheet

/// Plan how to tile copies of the selection across one page, nested. Returns the
/// world transform for each copy (relative to the selection's current position).
fn plan_fill(doc: &Document, sel: &[ShapeId], spacing: f64, allow_rotate: bool, margin: f64) -> Vec<kurbo::Affine> {
    let Some(template) = shape_outline_sel(doc, sel) else { return vec![] };
    let pr = doc.page().rect();
    let area = Rect::new(pr.x0 + margin, pr.y0 + margin, pr.x1 - margin, pr.y1 - margin);
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return vec![];
    }
    let res = (320.0 / area.width().max(area.height())).clamp(0.3, 3.0);
    let gw = (area.width() * res).ceil() as usize + 1;
    let gh = (area.height() * res).ceil() as usize + 1;
    let pad_cells = (spacing * res).round().max(0.0) as usize;
    let angles: &[f64] = if allow_rotate { &[0.0, 90.0, 180.0, 270.0] } else { &[0.0] };
    let center = template.bounding_box().center();

    let mut occ = vec![false; gw * gh];
    let mut out = Vec::new();
    loop {
        let mut best: Option<(usize, usize, Footprint)> = None;
        for &angle in angles {
            let fp = footprint(&template, angle, res);
            if fp.w > gw || fp.h > gh {
                continue;
            }
            'scan: for py in 0..=gh - fp.h {
                for px in 0..=gw - fp.w {
                    let fits = (0..fp.h).all(|my| (0..fp.w).all(|mx| !(fp.grid[my * fp.w + mx] && occ[(py + my) * gw + px + mx])));
                    if fits {
                        if best.as_ref().map(|(by, bx, _)| (py, px) < (*by, *bx)).unwrap_or(true) {
                            best = Some((py, px, fp));
                        }
                        break 'scan;
                    }
                }
            }
        }
        let Some((py, px, fp)) = best else { break };
        for my in 0..fp.h {
            for mx in 0..fp.w {
                if !fp.grid[my * fp.w + mx] {
                    continue;
                }
                let (y0, y1) = ((py + my).saturating_sub(pad_cells), (py + my + pad_cells).min(gh - 1));
                let (x0, x1) = ((px + mx).saturating_sub(pad_cells), (px + mx + pad_cells).min(gw - 1));
                for oy in y0..=y1 {
                    for ox in x0..=x1 {
                        occ[oy * gw + ox] = true;
                    }
                }
            }
        }
        // Transform: rotate the design about its centre, then align its new bbox
        // min to the target cell.
        let rot = kurbo::Affine::rotate_about(fp.angle.to_radians(), center);
        let mut t = template.clone();
        t.apply_affine(rot);
        let rbb = t.bounding_box();
        let (tx, ty) = (area.x0 + px as f64 / res, area.y0 + py as f64 / res);
        out.push(kurbo::Affine::translate((tx - rbb.x0, ty - rbb.y0)) * rot);
        if out.len() >= 10_000 {
            break;
        }
    }
    out
}

/// Combined world outline of a multi-shape selection (bbox fallback).
fn shape_outline_sel(doc: &Document, sel: &[ShapeId]) -> Option<kurbo::BezPath> {
    let v = world(doc, sel);
    if v.iter().any(|p| !p.elements().is_empty()) {
        Some(geom::merge_paths(&v))
    } else {
        ops::selection_bbox(doc, sel).map(geom::rect_path)
    }
}

/// How many copies of the selection fit on one page (without changing anything).
pub fn copies_per_sheet(doc: &Document, sel: &[ShapeId], spacing: f64, allow_rotate: bool, margin: f64) -> usize {
    plan_fill(doc, sel, spacing, allow_rotate, margin).len()
}

/// Replace the selection with as many nested copies as fit on the page. Returns
/// the ids of the created copies.
pub fn fill_sheet(doc: &mut Document, sel: &[ShapeId], spacing: f64, allow_rotate: bool, margin: f64) -> Sel {
    let placements = plan_fill(doc, sel, spacing, allow_rotate, margin);
    if placements.is_empty() {
        return vec![];
    }
    let src: Vec<Shape> = doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).cloned().collect();
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    let mut out = vec![];
    for tr in &placements {
        for mut s in src.iter().cloned() {
            doc.reid(&mut s);
            s.transform = *tr * s.transform;
            out.push(s.id);
            doc.page_mut().shapes.push(s);
        }
    }
    out
}

// ------------------------------------------------------------- shadow / contour

/// Offset outline behind the selection (sticker "contour cut").
pub fn contour(doc: &mut Document, sel: &[ShapeId], dist: f64, color: Rgba) -> Sel {
    let shapes = world(doc, sel);
    let p = geom::offset(&shapes, dist, CornerStyle::Round);
    if p.elements().is_empty() {
        return vec![];
    }
    let layer = layer_of(doc, sel);
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, p, Style { fill: Some(color), stroke: Some(Rgba::RED), ..Style::default() });
    s.layer = layer;
    s.name = "Contour".into();
    // insert below the lowest selected shape
    let idx = doc.page().shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
    doc.page_mut().shapes.insert(idx, s);
    vec![id]
}

// ------------------------------------------------------------- shadow / blackout layers

/// Solid outer silhouette of world paths: union, then drop interior holes by
/// keeping only the contours that wind the same way as the largest one.
pub fn outer_silhouette(paths: &[BezPath]) -> BezPath {
    // union shape by shape: a single even-odd pass would turn overlaps into holes
    let u = geom::union_each(paths);
    let subs = geom::flatten(&u, 0.1);
    let Some(sign) = subs
        .iter()
        .max_by(|a, b| a.signed_area().abs().total_cmp(&b.signed_area().abs()))
        .map(|p| p.signed_area().signum())
    else {
        return u;
    };
    let outers: Vec<BezPath> = subs
        .iter()
        .filter(|pl| pl.signed_area().signum() == sign && pl.signed_area().abs() > 1e-6)
        .map(|pl| geom::polyline_path(&pl.pts, true))
        .collect();
    if outers.is_empty() { u } else { geom::boolean(&outers, &[], BoolOp::Union) }
}

fn insert_behind(doc: &mut Document, sel: &[ShapeId], path: BezPath, color: Rgba, name: &str) -> Sel {
    let layer = layer_of(doc, sel);
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, path, Style { fill: Some(color), stroke: Some(Rgba::RED), ..Style::default() });
    s.layer = layer;
    s.name = name.into();
    let idx = doc.page().shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
    doc.page_mut().shapes.insert(idx, s);
    vec![id]
}

/// Blackout layer: a solid outer silhouette of the selection (all interior
/// detail removed), inserted behind it — a base layer for HTV/paper stacking.
pub fn blackout(doc: &mut Document, sel: &[ShapeId], color: Rgba) -> Sel {
    let sil = outer_silhouette(&world(doc, sel));
    if sil.elements().is_empty() {
        return vec![];
    }
    insert_behind(doc, sel, sil, color, "Blackout")
}

/// Shadow layer: the outer silhouette grown by `dist` mm, inserted behind — an
/// offset backing/shadow under the artwork.
pub fn shadow_layer(doc: &mut Document, sel: &[ShapeId], dist: f64, color: Rgba) -> Sel {
    let sil = outer_silhouette(&world(doc, sel));
    let grown = if dist.abs() > 1e-6 { geom::offset(&[sil], dist, CornerStyle::Round) } else { sil };
    if grown.elements().is_empty() {
        return vec![];
    }
    insert_behind(doc, sel, grown, color, "Shadow")
}

// ------------------------------------------------------------- stipple

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StipplePattern {
    Grid,
    Hex,
    Random,
}

/// Tiny circle used to represent one stipple dot (cut job turns it into a single plunge).
pub const DOT_RADIUS: f64 = 0.25;

fn dots_group(doc: &mut Document, pts: Vec<Point>, layer: u32, name: &str) -> Sel {
    let circles: Vec<BezPath> = pts.iter().map(|c| geom::circle_path(*c, DOT_RADIUS)).collect();
    let style = Style { fill: Some(Rgba::BLACK), stroke: None, ..Style::default() };
    // one compound path keeps big dot counts fast to render and move
    if circles.is_empty() {
        return vec![];
    }
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, geom::merge_paths(&circles), style);
    s.layer = layer;
    s.name = name.into();
    doc.page_mut().shapes.push(s);
    vec![id]
}

/// Fill the selection with stipple dots and/or dots along the edge.
pub fn stipple(doc: &mut Document, sel: &[ShapeId], fill_spacing: Option<f64>, pattern: StipplePattern, edge_spacing: Option<f64>, seed: u64) -> Sel {
    let shapes = world(doc, sel);
    let region = geom::boolean(&shapes, &[], BoolOp::Union);
    let mut pts = vec![];
    if let Some(sp) = edge_spacing.filter(|s| *s > 0.1) {
        for sub in geom::subpaths(&region) {
            let len = geom::path_length(&sub);
            let n = (len / sp).round().max(1.0) as usize;
            for i in 0..n {
                if let Some((p, _)) = geom::point_at_length(&sub, len * i as f64 / n as f64) {
                    pts.push(p);
                }
            }
        }
    }
    if let Some(sp) = fill_spacing.filter(|s| *s > 0.1) {
        let inner = if edge_spacing.is_some() { geom::offset(&[region.clone()], -sp * 0.8, CornerStyle::Round) } else { region.clone() };
        if let Some(bb) = geom::paths_bbox(&[inner.clone()]) {
            let mut rng = seed.max(1);
            let mut rand = move || {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng % 10_000) as f64 / 10_000.0
            };
            let dy = if pattern == StipplePattern::Hex { sp * 3f64.sqrt() / 2.0 } else { sp };
            let mut y = bb.y0 + dy / 2.0;
            let mut row = 0;
            while y <= bb.y1 {
                let mut x = bb.x0 + sp / 2.0 + if pattern == StipplePattern::Hex && row % 2 == 1 { sp / 2.0 } else { 0.0 };
                while x <= bb.x1 {
                    let mut p = Point::new(x, y);
                    if pattern == StipplePattern::Random {
                        p += Vec2::new((rand() - 0.5) * sp * 0.8, (rand() - 0.5) * sp * 0.8);
                    }
                    if geom::contains(&[inner.clone()], p) {
                        pts.push(p);
                    }
                    x += sp;
                }
                y += dy;
                row += 1;
            }
        }
    }
    let layer = layer_of(doc, sel);
    dots_group(doc, pts, layer, "Stipple")
}

/// Stipple an image: darker areas get more dots (Floyd–Steinberg dithering on a dot grid).
pub fn stipple_image(doc: &mut Document, image_id: ShapeId, spacing: f64, contrast: f64) -> Sel {
    let Some(s) = doc.page().shape(image_id).cloned() else { return vec![] };
    let ShapeKind::Image(img) = &s.kind else { return vec![] };
    let Ok(dec) = image::load_from_memory(&img.png) else { return vec![] };
    let spacing = spacing.max(0.3);
    let (gw, gh) = ((img.width_mm / spacing).ceil() as u32, (img.height_mm / spacing).ceil() as u32);
    let small = dec.resize_exact(gw.max(1), gh.max(1), image::imageops::FilterType::Triangle).to_rgba8();
    let (w, h) = (small.width() as usize, small.height() as usize);
    let mut v: Vec<f64> = small
        .pixels()
        .map(|p| {
            let a = p[3] as f64 / 255.0;
            let l = (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64) * a + 255.0 * (1.0 - a);
            let d = 1.0 - l / 255.0; // darkness 0..1
            ((d - 0.5) * contrast + 0.5).clamp(0.0, 1.0)
        })
        .collect();
    let mut pts = vec![];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let on = v[i] >= 0.5;
            let err = v[i] - if on { 1.0 } else { 0.0 };
            if on {
                let local = Point::new((x as f64 + 0.5) * img.width_mm / w as f64, (y as f64 + 0.5) * img.height_mm / h as f64);
                pts.push(s.transform * local);
            }
            if x + 1 < w {
                v[i + 1] += err * 7.0 / 16.0;
            }
            if y + 1 < h {
                if x > 0 {
                    v[i + w - 1] += err * 3.0 / 16.0;
                }
                v[i + w] += err * 5.0 / 16.0;
                if x + 1 < w {
                    v[i + w + 1] += err / 16.0;
                }
            }
        }
    }
    dots_group(doc, pts, s.layer, "Image stipple")
}

// ------------------------------------------------------------- emboss

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbossFill {
    /// Repeated inward outlines.
    Concentric,
    /// Back-and-forth lines.
    Hatch,
}

/// Tool paths that press the whole area of the selection (for an emboss tool).
pub fn emboss(doc: &mut Document, sel: &[ShapeId], spacing: f64, fill: EmbossFill, angle: f64) -> Sel {
    let shapes = world(doc, sel);
    let region = geom::boolean(&shapes, &[], BoolOp::Union);
    let spacing = spacing.max(0.2);
    let mut paths = vec![region.clone()];
    match fill {
        EmbossFill::Concentric => {
            let mut cur = region.clone();
            for _ in 0..500 {
                cur = geom::offset(&[cur], -spacing, CornerStyle::Round);
                if cur.elements().is_empty() {
                    break;
                }
                paths.push(cur.clone());
            }
        }
        EmbossFill::Hatch => {
            let tmp_id = doc.alloc_id();
            // reuse the hatch generator on a temporary shape
            let mut t = Shape::new_path(tmp_id, region.clone(), Style::default());
            t.layer = layer_of(doc, sel);
            doc.page_mut().shapes.push(t);
            let h = hatch(doc, &[tmp_id], spacing, angle, false);
            if let Some(hs) = h.first().and_then(|id| doc.page().shape(*id)) {
                paths.extend(hs.world_paths());
            }
            doc.page_mut().shapes.retain(|s| s.id != tmp_id && !h.contains(&s.id));
        }
    }
    let layer = layer_of(doc, sel);
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, geom::merge_paths(&paths), Style { stroke: Some(Rgba(150, 60, 200, 255)), ..Style::default() });
    s.layer = layer;
    s.name = "Emboss fill".into();
    doc.page_mut().shapes.push(s);
    vec![id]
}

// ------------------------------------------------------------- pop-up card

/// Turn the selected artwork into a pop-up element on a card that folds along `fold_y`.
///
/// The art is lifted onto a box-fold strut of depth `depth`: two cuts through the fold at
/// the art's left and right edges, fold lines at `fold_y ± depth`, and the art's outline cut
/// everywhere above the strut so it stands up when the card is opened to 90°.
pub fn popup(doc: &mut Document, sel: &[ShapeId], fold_y: f64, depth: f64, overlap: f64) -> Sel {
    let Some(bb) = ops::selection_bbox(doc, sel) else { return vec![] };
    let depth = depth.max(1.0);
    let base = fold_y - depth;
    // seat the art so its bottom overlaps the strut face by `overlap`
    ops::move_by(doc, sel, Vec2::new(0.0, base + overlap - bb.y1));
    let art = world(doc, sel);
    let outline = geom::boolean(&art, &[], BoolOp::Union);
    let (x1, x2) = (bb.x0, bb.x1);
    let page_w = doc.page().width;
    // cut the outline only above the strut line
    let above = geom::rect_path(Rect::new(-1e4, -1e4, 1e4, base));
    let mut open = vec![];
    for pl in geom::flatten(&outline, 0.05) {
        let mut pts = pl.pts.clone();
        if pl.closed {
            pts.push(pts[0]);
        }
        open.push(geom::polyline_path(&pts, false));
    }
    let art_cut = geom::clip_lines(&open, &[above], true);
    let strut_cuts = geom::merge_paths(&[
        geom::polyline_path(&[Point::new(x1, base), Point::new(x1, fold_y + depth)], false),
        geom::polyline_path(&[Point::new(x2, base), Point::new(x2, fold_y + depth)], false),
    ]);
    let folds = geom::merge_paths(&[
        geom::polyline_path(&[Point::new(0.0, fold_y), Point::new(x1, fold_y)], false),
        geom::polyline_path(&[Point::new(x2, fold_y), Point::new(page_w, fold_y)], false),
        geom::polyline_path(&[Point::new(x1, base), Point::new(x2, base)], false),
        geom::polyline_path(&[Point::new(x1, fold_y + depth), Point::new(x2, fold_y + depth)], false),
    ]);
    // the original art stays for printing only
    for id in sel {
        if let Some(s) = doc.page_mut().shape_mut(*id) {
            set_cut(s, crate::doc::CutStyle::NoCut);
        }
    }
    let layer = layer_of(doc, sel);
    let mut out = vec![];
    let cut_id = doc.alloc_id();
    let mut c = Shape::new_path(cut_id, geom::merge_paths(&[art_cut, strut_cuts]), Style::default());
    c.layer = layer;
    c.name = "Pop-up cuts".into();
    doc.page_mut().shapes.push(c);
    out.push(cut_id);
    let fold_id = doc.alloc_id();
    let mut f = Shape::new_path(
        fold_id,
        folds,
        Style { stroke: Some(Rgba(30, 90, 220, 255)), cut: crate::doc::CutStyle::Perforate { on: 1.0, off: 1.0 }, ..Style::default() },
    );
    f.layer = layer;
    f.name = "Pop-up folds".into();
    doc.page_mut().shapes.push(f);
    out.push(fold_id);
    out
}

fn set_cut(s: &mut Shape, c: crate::doc::CutStyle) {
    s.style.cut = c;
    match &mut s.kind {
        ShapeKind::Group(ch) => ch.iter_mut().for_each(|x| set_cut(x, c)),
        ShapeKind::Warp(w) => set_cut(&mut w.original, c),
        _ => {}
    }
}

// ------------------------------------------------------------- object on path

/// Place `count` copies of the `objects` evenly along the first subpath of
/// `path_id`, each rotated to follow the path tangent when `rotate` is set.
pub fn object_on_path(
    doc: &mut Document,
    objects: &[ShapeId],
    path_id: ShapeId,
    count: usize,
    rotate: bool,
    keep_path: bool,
) -> Sel {
    let count = count.max(1);
    let Some(path_shape) = doc.page().shape(path_id) else { return vec![] };
    let path = geom::merge_paths(&path_shape.world_paths());
    let sub = geom::subpaths(&path).into_iter().next().unwrap_or(path);
    let len = geom::path_length(&sub);
    if len < 1e-3 {
        return vec![];
    }
    let objs: Vec<ShapeId> = objects.iter().copied().filter(|id| *id != path_id).collect();
    let template = geom::merge_paths(&world(doc, &objs));
    let Some(bb) = geom::paths_bbox(&[template.clone()]) else { return vec![] };
    let c = bb.center();
    let (style, layer) = objs
        .first()
        .and_then(|id| doc.page().shape(*id))
        .map(|s| (s.style.clone(), s.layer))
        .unwrap_or((Style::default(), 0));
    let mut children = vec![];
    // closed paths wrap around; open paths stop at the end
    let closed = geom::flatten(&sub, 1.0).first().map(|p| p.closed).unwrap_or(false);
    let denom = if closed { count } else { count.saturating_sub(1).max(1) };
    for i in 0..count {
        let s = len * i as f64 / denom as f64;
        let Some((p, tan)) = geom::point_at_length(&sub, s.min(len)) else { continue };
        let a = if rotate {
            Affine::translate(p.to_vec2()) * Affine::rotate(tan.y.atan2(tan.x)) * Affine::translate(-c.to_vec2())
        } else {
            Affine::translate(p - c)
        };
        let mut t = template.clone();
        t.apply_affine(a);
        let id = doc.alloc_id();
        let mut sh = Shape::new_path(id, t, style.clone());
        sh.layer = layer;
        children.push(sh);
    }
    if !keep_path {
        doc.page_mut().shapes.retain(|s| s.id != path_id);
    }
    doc.page_mut().shapes.retain(|s| !objs.contains(&s.id));
    if children.is_empty() {
        return vec![];
    }
    let gid = doc.alloc_id();
    let g = Shape {
        id: gid,
        name: "On path".into(),
        layer,
        transform: Affine::IDENTITY,
        style,
        kind: ShapeKind::Group(children),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    };
    doc.page_mut().shapes.push(g);
    vec![gid]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Style;

    #[test]
    fn rhinestone_outline_counts() {
        let mut d = Document::default();
        let id = d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 40.0, 40.0)), Style::default(), 0);
        let g = rhinestones(&mut d, &[id], 3.0, 1.0, StoneLayout::Outline);
        let s = d.page().shape(g[0]).unwrap();
        if let ShapeKind::Group(ch) = &s.kind {
            assert_eq!(ch.len(), 40); // 160 mm perimeter / 4 mm pitch
        } else {
            panic!()
        }
    }

    #[test]
    fn stipple_and_emboss_and_popup() {
        let mut d = Document::default();
        let id = d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 30.0, 30.0)), Style::default(), 0);
        let s = stipple(&mut d, &[id], Some(2.0), StipplePattern::Grid, None, 1);
        let dots = geom::subpaths(&geom::merge_paths(&d.page().shape(s[0]).unwrap().world_paths())).len();
        assert_eq!(dots, 100);
        let e = emboss(&mut d, &[id], 1.0, EmbossFill::Concentric, 0.0);
        assert!(geom::subpaths(&d.page().shape(e[0]).unwrap().world_paths()[0]).len() >= 9);
        let p = popup(&mut d, &[id], 150.0, 20.0, 3.0);
        assert_eq!(p.len(), 2);
        let bb = d.page().shape(id).unwrap().bbox().unwrap();
        assert!((bb.y1 - 133.0).abs() < 1e-6);
        assert_eq!(d.page().shape(id).unwrap().style.cut, crate::doc::CutStyle::NoCut);
    }

    #[test]
    fn jointed_tiles_fit_their_pages() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 500.0, 250.0)), Style::default(), 0);
        let n = tile_to_pages_jointed(&mut d, 300.0, 300.0, crate::tiles::Joint::Puzzle, 14.0, 50.0);
        assert_eq!(n, 2, "500 mm wide needs two 300 mm pages");
        for p in &d.pages[1..] {
            let bb = p.shapes.iter().filter_map(|s| s.bbox()).fold(None, |a, b| geom::union_rect(a, b)).unwrap();
            assert!(bb.x0 >= -1e-6 && bb.y0 >= -1e-6 && bb.x1 <= 300.0 + 1e-6 && bb.y1 <= 300.0 + 1e-6, "tile + knobs fit the page: {bb:?}");
        }
        // the seam has knobs, so each tile outline is more than a rectangle
        assert!(geom::flatten(&d.pages[1].shapes[0].world_paths()[0], 0.05)[0].pts.len() > 10);
    }

    #[test]
    fn tiles_large_design() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 500.0, 100.0)), Style::default(), 0);
        let n = tile_to_pages(&mut d, 300.0, 300.0, 10.0);
        assert_eq!(n, 2);
        assert_eq!(d.pages.len(), 3);
    }

    #[test]
    fn object_on_path_places_copies() {
        let mut d = Document::default();
        let obj = d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 4.0, 4.0)), Style::default(), 0);
        let path = d.add_path(geom::polyline_path(&[Point::new(0.0, 50.0), Point::new(100.0, 50.0)], false), Style::default(), 0);
        let g = object_on_path(&mut d, &[obj], path, 5, true, false);
        assert_eq!(g.len(), 1);
        if let ShapeKind::Group(ch) = &d.page().shape(g[0]).unwrap().kind {
            assert_eq!(ch.len(), 5);
        } else {
            panic!("expected group");
        }
        // path and object consumed, only the group remains
        assert_eq!(d.page().shapes.len(), 1);
    }

    #[test]
    fn blackout_removes_interior_holes() {
        let mut d = Document::default();
        let donut = geom::boolean(
            &[geom::rect_path(Rect::new(0.0, 0.0, 40.0, 40.0))],
            &[geom::rect_path(Rect::new(10.0, 10.0, 30.0, 30.0))],
            crate::geom::BoolOp::Difference,
        );
        assert_eq!(geom::subpaths(&donut).len(), 2); // outer + hole
        let id = d.add_path(donut, Style::default(), 0);
        let b = blackout(&mut d, &[id], Rgba::BLACK);
        let s = d.page().shape(b[0]).unwrap();
        assert_eq!(geom::subpaths(&s.world_paths()[0]).len(), 1); // hole filled
        // the blackout sits behind the original
        assert!(d.page().shapes.iter().position(|x| x.id == b[0]).unwrap() < d.page().shapes.iter().position(|x| x.id == id).unwrap());
    }

    #[test]
    fn shadow_grows_the_silhouette() {
        let mut d = Document::default();
        let id = d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 20.0, 20.0)), Style::default(), 0);
        let before = d.page().shape(id).unwrap().bbox().unwrap().width();
        let sh = shadow_layer(&mut d, &[id], 4.0, Rgba::BLACK);
        let after = d.page().shape(sh[0]).unwrap().bbox().unwrap().width();
        assert!(after > before + 6.0, "shadow should grow by ~2x dist ({after} vs {before})");
    }

    #[test]
    fn fill_sheet_tiles_many_copies() {
        let mut d = Document::default();
        let id = d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 40.0, 40.0)), Style::default(), 0);
        let per = copies_per_sheet(&d, &[id], 3.0, false, 5.0);
        assert!(per > 4, "should fit several 40mm squares per sheet, got {per}");
        let placed = fill_sheet(&mut d, &[id], 3.0, false, 5.0);
        assert_eq!(placed.len(), per);
        assert_eq!(d.page().shapes.len(), placed.len()); // original replaced by copies
    }

    #[test]
    fn nest_packs_into_rows() {
        let mut d = Document::default();
        let ids: Vec<_> = (0..6)
            .map(|i| d.add_path(geom::rect_path(Rect::new(i as f64 * 5.0, 200.0, i as f64 * 5.0 + 100.0, 250.0)), Style::default(), 0))
            .collect();
        assert!(nest(&mut d, &ids, 2.0, false, 0.0));
        let bb = ops::selection_bbox(&d, &ids).unwrap();
        assert!(bb.y0 < 1.0 && bb.height() < 160.0);
    }
}
