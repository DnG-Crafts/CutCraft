//! Editing operations (the equivalent of the original's Scene commands).
//! Each function mutates the active page and returns the new selection.

use crate::doc::{Document, Shape, ShapeId, ShapeKind, Style};
use crate::geom::{self, BoolOp, CornerStyle};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, Vec2};

pub type Sel = Vec<ShapeId>;

pub fn selection_bbox(doc: &Document, sel: &[ShapeId]) -> Option<Rect> {
    let mut r = None;
    for id in sel {
        if let Some(bb) = doc.page().shape(*id).and_then(|s| s.bbox()) {
            r = geom::union_rect(r, bb);
        }
    }
    r
}

/// Apply an affine (page space) to every selected shape.
pub fn transform(doc: &mut Document, sel: &[ShapeId], a: Affine) {
    for id in sel {
        if let Some(s) = doc.page_mut().shape_mut(*id) {
            s.transform = a * s.transform;
        }
    }
}

/// For Fit Into Box: the frame (the selected shape with the largest bounding box)
/// and everything else.
pub fn frame_and_items(doc: &Document, sel: &[ShapeId]) -> Option<(ShapeId, Vec<ShapeId>)> {
    if sel.len() < 2 {
        return None;
    }
    let area = |id: &ShapeId| doc.page().shape(*id).and_then(|s| s.bbox()).map(|b| b.area()).unwrap_or(0.0);
    let frame = *sel.iter().max_by(|a, b| area(a).total_cmp(&area(b)))?;
    Some((frame, sel.iter().copied().filter(|id| *id != frame).collect()))
}

/// Scale `items` as one unit (keeping proportions) to fit inside `frame`'s
/// bounds less `margin` mm all round, centred in it. Ported from BurnCraft.
pub fn fit_into(doc: &mut Document, items: &[ShapeId], frame: ShapeId, margin: f64) -> bool {
    let (Some(f), Some(b)) = (doc.page().shape(frame).and_then(|s| s.bbox()), selection_bbox(doc, items)) else { return false };
    if b.width() <= 0.0 && b.height() <= 0.0 {
        return false;
    }
    let room = |frame: f64, item: f64| if item > 1e-9 { (frame - 2.0 * margin).max(0.01) / item } else { f64::MAX };
    let s = room(f.width(), b.width()).min(room(f.height(), b.height()));
    transform(doc, items, Affine::translate(f.center().to_vec2()) * Affine::scale(s) * Affine::translate(-b.center().to_vec2()));
    true
}

pub fn move_by(doc: &mut Document, sel: &[ShapeId], d: Vec2) {
    transform(doc, sel, Affine::translate(d));
}

pub fn move_to(doc: &mut Document, sel: &[ShapeId], x: f64, y: f64) {
    if let Some(bb) = selection_bbox(doc, sel) {
        move_by(doc, sel, Vec2::new(x - bb.x0, y - bb.y0));
    }
}

pub fn rotate(doc: &mut Document, sel: &[ShapeId], deg: f64) {
    if let Some(bb) = selection_bbox(doc, sel) {
        let c = bb.center();
        transform(doc, sel, Affine::rotate_about(deg.to_radians(), c));
    }
}

/// Rotate each shape individually about its own centre.
pub fn rotate_each(doc: &mut Document, sel: &[ShapeId], deg: f64) {
    for id in sel {
        rotate(doc, &[*id], deg);
    }
}

pub fn scale(doc: &mut Document, sel: &[ShapeId], sx: f64, sy: f64) {
    if let Some(bb) = selection_bbox(doc, sel) {
        let c = bb.center();
        let a = Affine::translate(c.to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-c.to_vec2());
        transform(doc, sel, a);
    }
}

pub fn set_size(doc: &mut Document, sel: &[ShapeId], w: Option<f64>, h: Option<f64>, keep_aspect: bool) {
    let Some(bb) = selection_bbox(doc, sel) else { return };
    let mut sx = w.map(|w| w / bb.width().max(1e-9)).unwrap_or(1.0);
    let mut sy = h.map(|h| h / bb.height().max(1e-9)).unwrap_or(1.0);
    if keep_aspect {
        if w.is_some() && h.is_none() {
            sy = sx;
        } else if h.is_some() && w.is_none() {
            sx = sy;
        }
    }
    let o = bb.origin().to_vec2();
    transform(doc, sel, Affine::translate(o) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-o));
}

pub fn shear(doc: &mut Document, sel: &[ShapeId], deg_x: f64, deg_y: f64) {
    if let Some(bb) = selection_bbox(doc, sel) {
        let c = bb.center().to_vec2();
        let s = Affine::new([1.0, deg_y.to_radians().tan(), deg_x.to_radians().tan(), 1.0, 0.0, 0.0]);
        transform(doc, sel, Affine::translate(c) * s * Affine::translate(-c));
    }
}

pub fn flip(doc: &mut Document, sel: &[ShapeId], horizontal: bool) {
    if horizontal { scale(doc, sel, -1.0, 1.0) } else { scale(doc, sel, 1.0, -1.0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignMode {
    Left,
    CenterH,
    Right,
    Top,
    Middle,
    Bottom,
    Center,
}

/// Align to the selection bbox, or to the page when only one shape is selected.
pub fn align(doc: &mut Document, sel: &[ShapeId], mode: AlignMode) {
    let target = if sel.len() == 1 { Some(doc.page().rect()) } else { selection_bbox(doc, sel) };
    let Some(t) = target else { return };
    for id in sel {
        let Some(bb) = doc.page().shape(*id).and_then(|s| s.bbox()) else { continue };
        let d = match mode {
            AlignMode::Left => Vec2::new(t.x0 - bb.x0, 0.0),
            AlignMode::Right => Vec2::new(t.x1 - bb.x1, 0.0),
            AlignMode::CenterH => Vec2::new(t.center().x - bb.center().x, 0.0),
            AlignMode::Top => Vec2::new(0.0, t.y0 - bb.y0),
            AlignMode::Bottom => Vec2::new(0.0, t.y1 - bb.y1),
            AlignMode::Middle => Vec2::new(0.0, t.center().y - bb.center().y),
            AlignMode::Center => t.center() - bb.center(),
        };
        move_by(doc, &[*id], d);
    }
}

pub fn distribute(doc: &mut Document, sel: &[ShapeId], horizontal: bool) {
    let mut items: Vec<(ShapeId, Rect)> =
        sel.iter().filter_map(|id| doc.page().shape(*id).and_then(|s| s.bbox()).map(|b| (*id, b))).collect();
    if items.len() < 3 {
        return;
    }
    let key = |r: &Rect| if horizontal { r.x0 } else { r.y0 };
    items.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
    let size = |r: &Rect| if horizontal { r.width() } else { r.height() };
    let first = items[0].1;
    let last = items[items.len() - 1].1;
    let span = if horizontal { last.x1 - first.x0 } else { last.y1 - first.y0 };
    let total: f64 = items.iter().map(|i| size(&i.1)).sum();
    let gap = (span - total) / (items.len() - 1) as f64;
    let mut pos = key(&first);
    for (id, r) in items {
        let d = pos - key(&r);
        move_by(doc, &[id], if horizontal { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) });
        pos += size(&r) + gap;
    }
}

// ------------------------------------------------------------- structure

pub fn delete(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    vec![]
}

pub fn duplicate(doc: &mut Document, sel: &[ShapeId], offset: Vec2) -> Sel {
    let src: Vec<Shape> = doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).cloned().collect();
    let mut out = vec![];
    for mut s in src {
        doc.reid(&mut s);
        s.transform = Affine::translate(offset) * s.transform;
        out.push(s.id);
        doc.page_mut().shapes.push(s);
    }
    out
}

pub fn paste(doc: &mut Document, clip: &[Shape], offset: Vec2, layer: u32) -> Sel {
    let mut out = vec![];
    for s in clip {
        let mut s = s.clone();
        doc.reid(&mut s);
        s.transform = Affine::translate(offset) * s.transform;
        s.layer = layer;
        out.push(s.id);
        doc.page_mut().shapes.push(s);
    }
    out
}

pub fn group(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    if sel.len() < 2 {
        return sel.to_vec();
    }
    let page = doc.page_mut();
    let first_idx = page.shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
    let layer = page.shapes[first_idx].layer;
    let children: Vec<Shape> = page.shapes.iter().filter(|s| sel.contains(&s.id)).cloned().collect();
    page.shapes.retain(|s| !sel.contains(&s.id));
    let id = doc.alloc_id();
    let g = Shape {
        id,
        name: String::new(),
        layer,
        transform: Affine::IDENTITY,
        style: Style::default(),
        kind: ShapeKind::Group(children),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    };
    let page = doc.page_mut();
    let idx = first_idx.min(page.shapes.len());
    page.shapes.insert(idx, g);
    vec![id]
}

pub fn ungroup(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut out = vec![];
    let mut i = 0;
    while i < doc.page().shapes.len() {
        let s = doc.page().shapes[i].clone();
        if sel.contains(&s.id) {
            if let ShapeKind::Group(ch) = s.kind {
                doc.page_mut().shapes.remove(i);
                for (k, mut c) in ch.into_iter().enumerate() {
                    c.transform = s.transform * c.transform;
                    c.layer = s.layer;
                    out.push(c.id);
                    doc.page_mut().shapes.insert(i + k, c);
                }
                continue;
            }
            out.push(s.id);
        }
        i += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Front,
    Back,
    Forward,
    Backward,
}

pub fn arrange(doc: &mut Document, sel: &[ShapeId], how: Arrange) {
    let shapes = &mut doc.page_mut().shapes;
    match how {
        Arrange::Front => {
            let (s, mut rest): (Vec<_>, Vec<_>) = shapes.drain(..).partition(|s| sel.contains(&s.id));
            rest.extend(s);
            *shapes = rest;
        }
        Arrange::Back => {
            let (mut s, rest): (Vec<_>, Vec<_>) = shapes.drain(..).partition(|s| sel.contains(&s.id));
            s.extend(rest);
            *shapes = s;
        }
        Arrange::Forward => {
            for i in (0..shapes.len().saturating_sub(1)).rev() {
                if sel.contains(&shapes[i].id) && !sel.contains(&shapes[i + 1].id) {
                    shapes.swap(i, i + 1);
                }
            }
        }
        Arrange::Backward => {
            for i in 1..shapes.len() {
                if sel.contains(&shapes[i].id) && !sel.contains(&shapes[i - 1].id) {
                    shapes.swap(i, i - 1);
                }
            }
        }
    }
}

// ------------------------------------------------------------- path ops

/// Replace selected shapes by a single path shape (page space geometry).
fn replace_with(doc: &mut Document, sel: &[ShapeId], path: BezPath) -> Sel {
    let page = doc.page();
    let Some(idx) = page.shapes.iter().position(|s| sel.contains(&s.id)) else { return vec![] };
    let style = page.shapes[idx].style.clone();
    let layer = page.shapes[idx].layer;
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    if path.elements().is_empty() {
        return vec![];
    }
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, path, style);
    s.layer = layer;
    let page = doc.page_mut();
    page.shapes.insert(idx.min(page.shapes.len()), s);
    vec![id]
}

fn world(doc: &Document, id: ShapeId) -> Vec<BezPath> {
    doc.page().shape(id).map(|s| s.world_paths()).unwrap_or_default()
}

/// Weld: union of all selected shapes.
pub fn weld(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let all: Vec<BezPath> = sel.iter().flat_map(|id| world(doc, *id)).collect();
    replace_with(doc, sel, geom::union_each(&all))
}

/// Subtract: the bottom-most selected shape minus all others.
pub fn subtract(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let ordered = z_ordered(doc, sel);
    if ordered.len() < 2 {
        return sel.to_vec();
    }
    let base = world(doc, ordered[0]);
    let cutters: Vec<BezPath> = ordered[1..].iter().flat_map(|id| world(doc, *id)).collect();
    let mut res = base;
    for c in cutters {
        res = vec![geom::boolean(&res, &[c], BoolOp::Difference)];
    }
    replace_with(doc, sel, geom::merge_paths(&res))
}

/// Subtract all: the shapes on top cut through every shape below them that
/// they overlap, and are removed. A cutter is a shape nothing selected
/// overlaps from above that does overlap something selected below it, so one
/// shape across several cuts them all, and many holes on one shape all cut it.
pub fn subtract_all(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let ordered = z_ordered(doc, sel);
    if ordered.len() < 2 {
        return sel.to_vec();
    }
    let paths: Vec<Vec<BezPath>> = ordered.iter().map(|id| world(doc, *id)).collect();
    let boxes: Vec<Option<Rect>> = paths.iter().map(|p| p.iter().map(|b| b.bounding_box()).reduce(|a, b| a.union(b))).collect();
    let n = ordered.len();
    // overlap[i][j]: shapes i and j share some area
    let mut overlap = vec![vec![false; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let touch = match (boxes[i], boxes[j]) {
                (Some(a), Some(b)) => a.overlaps(b) && {
                    let common = geom::boolean(&paths[i], &paths[j], BoolOp::Intersection);
                    !common.elements().is_empty() && common.bounding_box().area() > 1e-9
                },
                _ => false,
            };
            overlap[i][j] = touch;
            overlap[j][i] = touch;
        }
    }
    let cutter: Vec<bool> = (0..n).map(|i| !(i + 1..n).any(|j| overlap[i][j]) && (0..i).any(|j| overlap[i][j])).collect();
    if !cutter.iter().any(|&c| c) {
        return sel.to_vec();
    }
    let mut out = vec![];
    for i in 0..n {
        if cutter[i] {
            continue;
        }
        let cuts: Vec<usize> = (i + 1..n).filter(|&j| cutter[j] && overlap[i][j]).collect();
        if cuts.is_empty() {
            out.push(ordered[i]);
            continue;
        }
        let mut res = paths[i].clone();
        for j in cuts {
            res = vec![geom::boolean(&res, &paths[j], BoolOp::Difference)];
        }
        out.extend(replace_with(doc, &[ordered[i]], geom::merge_paths(&res)));
    }
    let gone: Vec<ShapeId> = (0..n).filter(|&i| cutter[i]).map(|i| ordered[i]).collect();
    doc.page_mut().shapes.retain(|s| !gone.contains(&s.id));
    out
}

pub fn intersect(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut iter = sel.iter();
    let Some(first) = iter.next() else { return vec![] };
    let mut acc = world(doc, *first);
    for id in iter {
        acc = vec![geom::boolean(&acc, &world(doc, *id), BoolOp::Intersection)];
    }
    replace_with(doc, sel, geom::merge_paths(&acc))
}

/// Unite: group the shapes and weld them live — one outline is cut, but the
/// parts stay separate underneath (Ungroup gets them back).
pub fn unite(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    if sel.len() < 2 {
        return sel.to_vec();
    }
    let g = group(doc, sel);
    if let Some(s) = g.first().and_then(|id| doc.page_mut().shape_mut(*id)) {
        s.effects.push(crate::effectstack::Effect::Unite);
        s.name = "United".into();
    }
    g
}

/// Exclude: every shape loses the areas where it overlaps the others; each
/// keeps its own style and layer (Cricut's Exclude).
pub fn exclude(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let ids = z_ordered(doc, sel);
    if ids.len() < 2 {
        return sel.to_vec();
    }
    let geoms: Vec<Vec<BezPath>> = ids.iter().map(|id| world(doc, *id)).collect();
    let mut out = vec![];
    for (i, id) in ids.iter().enumerate() {
        let others: Vec<BezPath> = geoms.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, g)| g.clone()).collect();
        let rest = geom::boolean(&geoms[i], &others, BoolOp::Difference);
        out.extend(replace_with(doc, &[*id], rest));
    }
    out
}

/// Divide (shatter): split shapes into every non-overlapping region - each
/// area covered by the same shapes becomes a shape of its own, in the style
/// of the topmost shape over it.
pub fn divide(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let ids = z_ordered(doc, sel);
    let shapes: Vec<Shape> = ids.iter().filter_map(|id| doc.page().shape(*id).cloned()).collect();
    let geoms: Vec<Vec<BezPath>> = shapes.iter().map(|s| s.world_paths()).collect();
    let faces = crate::shapebuilder::faces(&geoms);
    if faces.is_empty() {
        return sel.to_vec();
    }
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    let mut out = vec![];
    for f in faces {
        let Some(from) = f.covers.iter().max().and_then(|i| shapes.get(*i)) else { continue };
        out.push(doc.add_path(f.path, from.style.clone(), from.layer));
    }
    out
}

/// Split a path into separate regions (each outer ring with its holes).
pub fn split_regions(path: &BezPath) -> Vec<BezPath> {
    let r = crate::shapebuilder::regions(path);
    if !r.is_empty() || path.elements().is_empty() {
        return r;
    }
    // fallback: on flattened outlines
    let mp = geom::to_multipolygon(&[path.clone()]);
    mp.0.iter().map(|p| geom::from_multipolygon(&geo::MultiPolygon(vec![p.clone()]))).collect()
}

/// Combine selected shapes into one compound path (no boolean).
pub fn make_compound(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let all: Vec<BezPath> = z_ordered(doc, sel).iter().flat_map(|id| world(doc, *id)).collect();
    replace_with(doc, sel, geom::merge_paths(&all))
}

/// Release compound path: every subpath becomes its own shape.
pub fn release_compound(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut out = vec![];
    for id in sel.to_vec() {
        let Some(s) = doc.page().shape(id).cloned() else { continue };
        let w = geom::merge_paths(&s.world_paths());
        let subs = geom::subpaths(&w);
        if subs.len() < 2 {
            out.push(id);
            continue;
        }
        doc.page_mut().shapes.retain(|x| x.id != id);
        for sp in subs {
            out.push(doc.add_path(sp, s.style.clone(), s.layer));
        }
    }
    out
}

/// Convert text, images frames or groups into plain path shapes.
pub fn convert_to_path(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut out = vec![];
    for id in sel.to_vec() {
        let Some(s) = doc.page().shape(id).cloned() else { continue };
        if matches!(s.kind, ShapeKind::Path(_)) {
            out.push(id);
            continue;
        }
        let mut st = s.style.clone();
        if matches!(s.kind, ShapeKind::Text { .. }) && st.fill.is_none() {
            st.fill = None;
        }
        out.extend(replace_with(doc, &[id], geom::merge_paths(&s.world_paths())));
    }
    out
}

pub fn offset(doc: &mut Document, sel: &[ShapeId], dist: f64, corners: CornerStyle, keep_original: bool) -> Sel {
    let all: Vec<BezPath> = sel.iter().flat_map(|id| world(doc, *id)).collect();
    let p = geom::offset(&all, dist, corners);
    if p.elements().is_empty() {
        return sel.to_vec();
    }
    let (style, layer) = sel
        .first()
        .and_then(|id| doc.page().shape(*id))
        .map(|s| (Style { fill: None, ..s.style.clone() }, s.layer))
        .unwrap_or_default();
    if !keep_original {
        doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    }
    vec![doc.add_path(p, style, layer)]
}

pub fn simplify(doc: &mut Document, sel: &[ShapeId], tol: f64, smooth: bool) {
    for id in sel {
        if let Some(s) = doc.page_mut().shape_mut(*id) {
            if let ShapeKind::Path(p) = &mut s.kind {
                *p = geom::simplify_path(p, tol, smooth);
            }
        }
    }
}

/// Knife: split shapes crossed by the polyline `cut` (page coordinates).
pub fn knife(doc: &mut Document, cut: &[Point]) -> Sel {
    if cut.len() < 2 {
        return vec![];
    }
    let blade = geom::offset(&[geom::polyline_path(cut, false)], 0.02, CornerStyle::Sharp);
    let bb = blade.bounding_box();
    let mut out = vec![];
    let candidates: Vec<ShapeId> = doc
        .page()
        .shapes
        .iter()
        .filter(|s| doc.page().is_editable(s) && s.bbox().map(|b| b.overlaps(bb)).unwrap_or(false) && !s.is_image())
        .map(|s| s.id)
        .collect();
    for id in candidates {
        let w = world(doc, id);
        let r = geom::boolean(&w, &[blade.clone()], BoolOp::Difference);
        let parts = split_regions(&r);
        if parts.len() < 2 {
            continue;
        }
        let s = doc.page().shape(id).cloned().unwrap();
        doc.page_mut().shapes.retain(|x| x.id != id);
        for p in parts {
            out.push(doc.add_path(p, s.style.clone(), s.layer));
        }
    }
    out
}

/// Eraser: remove a stroke of given radius from editable shapes.
pub fn erase(doc: &mut Document, stroke: &[Point], radius: f64) {
    if stroke.is_empty() {
        return;
    }
    let brush = if stroke.len() == 1 {
        geom::circle_path(stroke[0], radius)
    } else {
        geom::offset(&[geom::polyline_path(stroke, false)], radius, CornerStyle::Round)
    };
    let bb = brush.bounding_box();
    let ids: Vec<ShapeId> = doc
        .page()
        .shapes
        .iter()
        .filter(|s| doc.page().is_editable(s) && !s.is_image() && s.bbox().map(|b| b.overlaps(bb)).unwrap_or(false))
        .map(|s| s.id)
        .collect();
    for id in ids {
        let w = world(doc, id);
        let r = geom::boolean(&w, &[brush.clone()], BoolOp::Difference);
        replace_with(doc, &[id], r);
    }
}

/// The shapes of `sel`, bottom to top.
pub fn z_ordered(doc: &Document, sel: &[ShapeId]) -> Vec<ShapeId> {
    doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).map(|s| s.id).collect()
}

// ------------------------------------------------------------- warps

/// Wrap the selection (grouped if several) in a live warp shape.
pub fn make_warp(doc: &mut Document, sel: &[ShapeId], mode: crate::warp::WarpMode) -> Sel {
    use crate::warp::{WarpData, WarpMode};
    let sel = if sel.len() > 1 { group(doc, sel) } else { sel.to_vec() };
    let Some(id) = sel.first().copied() else { return vec![] };
    let Some(idx) = doc.page().shape_index(id) else { return vec![] };
    let original = doc.page_mut().shapes.remove(idx);
    let layer = original.layer;
    let style = original.style.clone();
    let data = match mode {
        WarpMode::Mesh { rows, cols, .. } => WarpData::new_mesh(original, rows, cols),
        WarpMode::Conical(p) => WarpData::new_conical(original, p),
    };
    let nid = doc.alloc_id();
    let s = Shape {
        id: nid,
        name: String::new(),
        layer,
        transform: Affine::IDENTITY,
        style,
        kind: ShapeKind::Warp(data),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    };
    doc.page_mut().shapes.insert(idx, s);
    vec![nid]
}

/// Undo a warp: put the original artwork back.
pub fn warp_revert(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut out = vec![];
    for id in sel {
        let Some(idx) = doc.page().shape_index(*id) else { continue };
        let s = doc.page().shapes[idx].clone();
        if let ShapeKind::Warp(w) = s.kind {
            let mut o = *w.original;
            o.transform = s.transform * o.transform;
            out.push(o.id);
            doc.page_mut().shapes[idx] = o;
        } else {
            out.push(*id);
        }
    }
    out
}

// ------------------------------------------------------------- replicate

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Replicate {
    Left,
    Right,
    Above,
    Below,
    Row(usize),
    Column(usize),
    Rotate(usize),
    MirrorLeft,
    MirrorRight,
    MirrorAbove,
    MirrorBelow,
    FillPage,
}

pub fn replicate(doc: &mut Document, sel: &[ShapeId], how: Replicate, gap: f64) -> Sel {
    let Some(bb) = selection_bbox(doc, sel) else { return vec![] };
    let (w, h) = (bb.width() + gap, bb.height() + gap);
    let mut out = sel.to_vec();
    let mut dup = |doc: &mut Document, a: Affine| {
        let ids = duplicate(doc, sel, Vec2::ZERO);
        transform(doc, &ids, a);
        out.extend(ids);
    };
    match how {
        Replicate::Left => dup(doc, Affine::translate((-w, 0.0))),
        Replicate::Right => dup(doc, Affine::translate((w, 0.0))),
        Replicate::Above => dup(doc, Affine::translate((0.0, -h))),
        Replicate::Below => dup(doc, Affine::translate((0.0, h))),
        Replicate::Row(n) => (1..n).for_each(|i| dup(doc, Affine::translate((w * i as f64, 0.0)))),
        Replicate::Column(n) => (1..n).for_each(|i| dup(doc, Affine::translate((0.0, h * i as f64)))),
        Replicate::Rotate(n) => {
            // copies arranged around a centre below the shape
            let c = Point::new(bb.center().x, bb.y1 + gap);
            (1..n).for_each(|i| dup(doc, Affine::rotate_about(std::f64::consts::TAU * i as f64 / n as f64, c)))
        }
        Replicate::MirrorLeft => dup(doc, mirror_x(bb.x0 - gap / 2.0)),
        Replicate::MirrorRight => dup(doc, mirror_x(bb.x1 + gap / 2.0)),
        Replicate::MirrorAbove => dup(doc, mirror_y(bb.y0 - gap / 2.0)),
        Replicate::MirrorBelow => dup(doc, mirror_y(bb.y1 + gap / 2.0)),
        Replicate::FillPage => {
            let pr = doc.page().rect();
            let nx = ((pr.width() - bb.x0 + gap) / w).floor().max(1.0) as usize;
            let ny = ((pr.height() - bb.y0 + gap) / h).floor().max(1.0) as usize;
            for j in 0..ny {
                for i in 0..nx {
                    if i == 0 && j == 0 {
                        continue;
                    }
                    dup(doc, Affine::translate((w * i as f64, h * j as f64)));
                }
            }
        }
    }
    out
}

fn mirror_x(x: f64) -> Affine {
    Affine::translate((2.0 * x, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0)
}
fn mirror_y(y: f64) -> Affine {
    Affine::translate((0.0, 2.0 * y)) * Affine::scale_non_uniform(1.0, -1.0)
}

/// Advanced replicate: `n` copies each offset by (dx,dy) and rotated by `rot` degrees.
pub fn replicate_advanced(doc: &mut Document, sel: &[ShapeId], n: usize, dx: f64, dy: f64, rot: f64) -> Sel {
    let mut out = sel.to_vec();
    let Some(bb) = selection_bbox(doc, sel) else { return out };
    for i in 1..=n {
        let ids = duplicate(doc, sel, Vec2::ZERO);
        let c = bb.center();
        let a = Affine::translate((dx * i as f64, dy * i as f64)) * Affine::rotate_about((rot * i as f64).to_radians(), c);
        transform(doc, &ids, a);
        out.extend(ids);
    }
    out
}

/// The shapes a selection box from `start` to `end` picks. Dragged left to
/// right it picks shapes entirely inside the box; dragged right to left, any
/// shape whose lines the box touches (and pictures it overlaps). Locked and
/// hidden shapes are skipped. Ported from BurnCraft.
pub fn marquee_pick(page: &crate::doc::Page, start: Point, end: Point) -> Sel {
    let area = Rect::from_points(start, end);
    let crossing = end.x < start.x;
    page.shapes
        .iter()
        .filter(|s| page.is_editable(s))
        .filter(|s| {
            let Some(b) = s.bbox() else { return false };
            if !crossing {
                return area.contains(b.origin()) && area.contains(Point::new(b.x1, b.y1));
            }
            let overlaps = |b: Rect| b.x0 <= area.x1 && area.x0 <= b.x1 && b.y0 <= area.y1 && area.y0 <= b.y1;
            overlaps(b)
                && s.leaves().iter().any(|l| match l.kind {
                    ShapeKind::Image(_) => l.bbox().is_some_and(overlaps),
                    _ => geom::outline_touches(&l.world_paths(), area),
                })
        })
        .map(|s| s.id)
        .collect()
}

/// Sets the outline colour of `s` (and, for a group, every shape in it) to
/// `to`, where the outline is currently `from` — or wherever it has an outline
/// at all when `from` is `None`. Shapes without an outline (fill-only art,
/// pictures) are left alone. Returns whether anything changed.
pub fn recolour_outline(s: &mut Shape, from: Option<crate::doc::Rgba>, to: crate::doc::Rgba) -> bool {
    let mut changed = false;
    if let ShapeKind::Group(children) = &mut s.kind {
        for c in children {
            changed |= recolour_outline(c, from, to);
        }
        return changed;
    }
    match s.style.stroke {
        Some(c) if from.is_none_or(|f| f == c) && c != to => {
            s.style.stroke = Some(to);
            true
        }
        _ => false,
    }
}

/// Moves `ids` (keeping their order) to just above `target` in the stack
/// (`above`: drawn over it) or just below it.
pub fn move_in_stack(doc: &mut Document, ids: &[ShapeId], target: ShapeId, above: bool) {
    if ids.contains(&target) {
        return;
    }
    let page = doc.page_mut();
    let moving: Vec<Shape> = page.shapes.iter().filter(|s| ids.contains(&s.id)).cloned().collect();
    if moving.is_empty() {
        return;
    }
    page.shapes.retain(|s| !ids.contains(&s.id));
    let Some(t) = page.shapes.iter().position(|s| s.id == target) else {
        page.shapes.extend(moving);
        return;
    };
    let at = if above { t + 1 } else { t };
    for (k, s) in moving.into_iter().enumerate() {
        page.shapes.insert(at + k, s);
    }
}

/// Moves shapes to another layer. Shapes still drawn in their old layer's
/// colour take the new layer's colour.
pub fn move_to_layer(doc: &mut Document, ids: &[ShapeId], layer: u32) {
    let page = doc.page_mut();
    let colour = |p: &crate::doc::Page, id: u32| p.layer(id).map(|l| l.color);
    let to = colour(page, layer);
    for id in ids {
        let from = page.shape(*id).and_then(|sh| colour(page, sh.layer));
        if let Some(sh) = page.shape_mut(*id) {
            sh.layer = layer;
            if let (Some(from), Some(to)) = (from, to) {
                recolour_outline(sh, Some(from), to);
            }
        }
    }
}

/// Masks the selected pictures with the selected shapes: keeps only the
/// part of each picture inside the shapes (`keep_inside`, trimming the
/// picture to them), or cuts the shapes out of it. The shapes stay (a ready
/// cut line) unless `remove_shapes`. Returns the new selection, or why not.
pub fn image_mask(doc: &mut Document, sel: &[ShapeId], keep_inside: bool, remove_shapes: bool) -> Result<Sel, String> {
    let page = doc.page();
    let is_image = |id: &ShapeId| page.shape(*id).is_some_and(|s| matches!(s.kind, ShapeKind::Image(_)));
    let images: Vec<ShapeId> = z_ordered(doc, sel).into_iter().filter(is_image).collect();
    let mut cutters: Vec<ShapeId> = z_ordered(doc, sel).into_iter().filter(|id| !is_image(id)).collect();
    // a cut line set not to print (Flatten's, or a Print & Cut upload's) is
    // the picture's own outline: with another shape selected, it's not one to use
    let prints = |id: &ShapeId| page.shape(*id).is_some_and(|s| s.leaves().iter().any(|l| !l.style.no_print));
    if cutters.iter().any(prints) {
        cutters.retain(prints);
    }
    if images.is_empty() {
        let grouped = sel.iter().any(|id| page.shape(*id).is_some_and(|s| matches!(&s.kind, ShapeKind::Group(ch) if ch.iter().any(|c| c.is_image()))));
        return Err(if grouped {
            "the picture is grouped with its cut line (Print & Cut) - ungroup it first (Image panel ▸ Ungroup to edit the picture)".into()
        } else {
            "select a picture and the shape(s) to use on it".into()
        });
    }
    if cutters.is_empty() {
        return Err("select a picture and the shape(s) to use on it".into());
    }
    // the area any of them covers (each shape's own holes kept)
    let each: Vec<BezPath> = cutters.iter().map(|id| geom::merge_paths(&world(doc, *id))).collect();
    let shape: Vec<BezPath> = vec![geom::union_each(&each)];
    if geom::flatten_all(&shape, 0.1).iter().all(|p| !p.closed) {
        return Err("the shape needs to be closed (a line can't cut a picture)".into());
    }
    let mut done = 0;
    let mut last_err = String::new();
    for id in &images {
        let Some(s) = doc.page().shape(*id) else { continue };
        let ShapeKind::Image(img) = &s.kind else { continue };
        let local = geom::transformed(&shape, s.transform.inverse());
        let masked = match crate::raster::mask(&img.png, &local, keep_inside, img.width_mm, img.height_mm) {
            Ok(m) => m,
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        // the untouched picture stays, for the Restore brush
        let original = img.original.clone().unwrap_or_else(|| img.png.clone());
        // keeping the inside: trim the picture to the shape
        let trim = keep_inside.then(|| geom::paths_bbox(&local)).flatten().and_then(|b| {
            let (png, w, h, kept) = crate::raster::crop(&masked, b, img.width_mm, img.height_mm).ok()?;
            let orig = crate::raster::crop(&original, kept, img.width_mm, img.height_mm).ok().map(|(o, ..)| o);
            Some((png, w, h, kept, orig))
        });
        let Some(s) = doc.page_mut().shape_mut(*id) else { continue };
        match trim {
            Some((png, w, h, kept, orig)) => {
                s.transform = s.transform * Affine::translate(kept.origin().to_vec2());
                if let ShapeKind::Image(img) = &mut s.kind {
                    img.png = std::sync::Arc::new(png);
                    (img.px_w, img.px_h) = (w, h);
                    (img.width_mm, img.height_mm) = (kept.width(), kept.height());
                    img.original = orig.map(std::sync::Arc::new);
                }
            }
            None => {
                if let ShapeKind::Image(img) = &mut s.kind {
                    img.png = std::sync::Arc::new(masked);
                    img.original = Some(original);
                }
            }
        }
        done += 1;
    }
    if done == 0 {
        return Err(if last_err.is_empty() { "nothing to do".into() } else { last_err });
    }
    if remove_shapes {
        doc.page_mut().shapes.retain(|s| !cutters.contains(&s.id));
        return Ok(images);
    }
    Ok(sel.to_vec())
}
