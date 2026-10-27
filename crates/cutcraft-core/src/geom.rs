//! Geometry helpers on top of `kurbo` (curves) and `geo` (polygon booleans and offsets).
//! All coordinates are millimetres, y grows downwards (page coordinates).

use geo::algorithm::bool_ops::{FillRule, OpType};
use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::{BooleanOps, Buffer, Coord, LineString, MultiLineString, MultiPolygon, Polygon};
use kurbo::{Affine, BezPath, ParamCurve, ParamCurveArclen, PathEl, PathSeg, Point, Rect, Shape};

/// Default flattening tolerance (mm) for boolean ops and cutting.
pub const FLATTEN_TOL: f64 = 0.02;

/// A flattened subpath.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub pts: Vec<Point>,
    pub closed: bool,
}

impl Polyline {
    pub fn length(&self) -> f64 {
        let mut l: f64 = self.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
        if self.closed && self.pts.len() > 1 {
            l += self.pts[self.pts.len() - 1].distance(self.pts[0]);
        }
        l
    }
    pub fn bbox(&self) -> Rect {
        bbox_points(&self.pts)
    }
    pub fn signed_area(&self) -> f64 {
        let n = self.pts.len();
        if n < 3 {
            return 0.0;
        }
        let mut a = 0.0;
        for i in 0..n {
            let p = self.pts[i];
            let q = self.pts[(i + 1) % n];
            a += p.x * q.y - q.x * p.y;
        }
        a * 0.5
    }
}

pub fn bbox_points(pts: &[Point]) -> Rect {
    let mut r = Rect::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in pts {
        r.x0 = r.x0.min(p.x);
        r.y0 = r.y0.min(p.y);
        r.x1 = r.x1.max(p.x);
        r.y1 = r.y1.max(p.y);
    }
    r
}

pub fn rect_is_valid(r: &Rect) -> bool {
    r.x0.is_finite() && r.y0.is_finite() && r.x1 >= r.x0 && r.y1 >= r.y0
}

pub fn union_rect(a: Option<Rect>, b: Rect) -> Option<Rect> {
    if !rect_is_valid(&b) {
        return a;
    }
    Some(match a {
        Some(a) => a.union(b),
        None => b,
    })
}

/// Bounding box of several paths (None if empty).
pub fn paths_bbox(paths: &[BezPath]) -> Option<Rect> {
    let mut r = None;
    for p in paths {
        if p.elements().is_empty() {
            continue;
        }
        r = union_rect(r, p.bounding_box());
    }
    r
}

/// Flatten a path into polylines.
pub fn flatten(path: &BezPath, tol: f64) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() > 1 {
                out.push(Polyline { pts: std::mem::take(&mut cur), closed: false });
            }
            cur.clear();
            cur.push(p);
        }
        PathEl::LineTo(p) => {
            if cur.last().map(|l| l.distance(p) > 1e-9).unwrap_or(true) {
                cur.push(p)
            }
        }
        PathEl::ClosePath => {
            if cur.len() > 1 {
                let first = cur[0];
                if cur.last().unwrap().distance(first) < 1e-9 {
                    cur.pop();
                }
                out.push(Polyline { pts: std::mem::take(&mut cur), closed: true });
            }
        }
        _ => {}
    });
    if cur.len() > 1 {
        // treat an open path whose ends meet as closed
        let closed = cur.len() > 2 && cur[0].distance(*cur.last().unwrap()) < 1e-6;
        if closed {
            cur.pop();
        }
        out.push(Polyline { pts: cur, closed });
    }
    out
}

pub fn flatten_all(paths: &[BezPath], tol: f64) -> Vec<Polyline> {
    paths.iter().flat_map(|p| flatten(p, tol)).collect()
}

/// Flatten the paths, subdivide every segment so no edge is longer than `step` mm,
/// map each point through `f`, and rebuild. Used by non-linear distortion effects so
/// straight edges bend smoothly.
pub fn remap_densified(paths: &[BezPath], step: f64, f: &dyn Fn(Point) -> Point) -> Vec<BezPath> {
    let step = step.max(0.05);
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let pls: Vec<Polyline> = flatten(path, (step / 4.0).min(FLATTEN_TOL))
            .into_iter()
            .map(|pl| {
                let n = pl.pts.len();
                let segs = if pl.closed { n } else { n.saturating_sub(1) };
                let mut dense = Vec::with_capacity(n * 2);
                for i in 0..segs {
                    let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
                    let steps = (a.distance(b) / step).ceil().max(1.0) as usize;
                    for s in 0..steps {
                        dense.push(f(a.lerp(b, s as f64 / steps as f64)));
                    }
                }
                if !pl.closed {
                    if let Some(l) = pl.pts.last() {
                        dense.push(f(*l));
                    }
                }
                Polyline { pts: dense, closed: pl.closed }
            })
            .collect();
        out.push(polylines_to_path(&pls));
    }
    out
}

pub fn polyline_to_path(pl: &Polyline) -> BezPath {
    let mut b = BezPath::new();
    for (i, p) in pl.pts.iter().enumerate() {
        if i == 0 {
            b.move_to(*p)
        } else {
            b.line_to(*p)
        }
    }
    if pl.closed {
        b.close_path();
    }
    b
}

pub fn polylines_to_path(pls: &[Polyline]) -> BezPath {
    let mut b = BezPath::new();
    for pl in pls {
        b.extend(polyline_to_path(pl).elements().iter().copied());
    }
    b
}

// ---------------------------------------------------------------- geo bridge

fn ring(pl: &Polyline) -> LineString<f64> {
    let mut c: Vec<Coord<f64>> = pl.pts.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
    if let Some(f) = c.first().copied() {
        c.push(f);
    }
    LineString::new(c)
}

/// Convert closed subpaths of the given paths to a normalized multipolygon (even-odd fill).
pub fn to_multipolygon(paths: &[BezPath]) -> MultiPolygon<f64> {
    let rings: Vec<Polygon<f64>> = flatten_all(paths, FLATTEN_TOL)
        .iter()
        .filter(|p| p.closed && p.pts.len() >= 3)
        .map(|p| Polygon::new(ring(p), vec![]))
        .collect();
    let raw = MultiPolygon(rings);
    raw.boolean_op_with_fill_rule(&MultiPolygon::<f64>(vec![]), OpType::Union, FillRule::EvenOdd)
}

/// Open subpaths of the given paths as a multilinestring.
pub fn open_lines(paths: &[BezPath]) -> MultiLineString<f64> {
    MultiLineString(
        flatten_all(paths, FLATTEN_TOL)
            .iter()
            .filter(|p| !p.closed)
            .map(|p| LineString::new(p.pts.iter().map(|q| Coord { x: q.x, y: q.y }).collect()))
            .collect(),
    )
}

fn ls_points(ls: &LineString<f64>) -> Vec<Point> {
    let mut v: Vec<Point> = ls.0.iter().map(|c| Point::new(c.x, c.y)).collect();
    if v.len() > 1 && v[0].distance(*v.last().unwrap()) < 1e-9 {
        v.pop();
    }
    v
}

pub fn from_multipolygon(mp: &MultiPolygon<f64>) -> BezPath {
    let mut pls = Vec::new();
    for poly in &mp.0 {
        let e = ls_points(poly.exterior());
        if e.len() >= 3 {
            pls.push(Polyline { pts: e, closed: true });
        }
        for h in poly.interiors() {
            let h = ls_points(h);
            if h.len() >= 3 {
                pls.push(Polyline { pts: h, closed: true });
            }
        }
    }
    polylines_to_path(&pls)
}

pub fn from_multilinestring(ml: &MultiLineString<f64>) -> BezPath {
    let pls: Vec<Polyline> = ml
        .0
        .iter()
        .map(|l| Polyline { pts: l.0.iter().map(|c| Point::new(c.x, c.y)).collect(), closed: false })
        .filter(|p| p.pts.len() > 1)
        .collect();
    polylines_to_path(&pls)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Difference,
    Intersection,
    Xor,
}

/// Union of several paths, where each path's own subpaths keep even-odd holes
/// (the middle of an "o") but different paths merge where they overlap. This is
/// what "weld" means; passing everything to [`boolean`] at once would instead
/// XOR the overlaps into holes.
pub fn union_each(paths: &[BezPath]) -> BezPath {
    let mut acc = MultiPolygon::<f64>(vec![]);
    for p in paths {
        if p.elements().is_empty() {
            continue;
        }
        acc = acc.union(&to_multipolygon(std::slice::from_ref(p)));
    }
    from_multipolygon(&acc)
}

pub fn boolean(a: &[BezPath], b: &[BezPath], op: BoolOp) -> BezPath {
    let ma = to_multipolygon(a);
    let mb = to_multipolygon(b);
    let r = match op {
        BoolOp::Union => ma.union(&mb),
        BoolOp::Difference => ma.difference(&mb),
        BoolOp::Intersection => ma.intersection(&mb),
        BoolOp::Xor => ma.xor(&mb),
    };
    from_multipolygon(&r)
}

/// Clip open lines against closed paths. `inside` keeps the part inside the region.
pub fn clip_lines(lines: &[BezPath], region: &[BezPath], inside: bool) -> BezPath {
    let mp = to_multipolygon(region);
    let ml = open_lines(lines);
    from_multilinestring(&mp.clip(&ml, !inside))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CornerStyle {
    Round,
    Sharp,
}

/// Offset (positive = outward, negative = inward) closed regions; open paths are stroked.
pub fn offset(paths: &[BezPath], dist: f64, corners: CornerStyle) -> BezPath {
    let join = || match corners {
        CornerStyle::Round => LineJoin::Round(0.1),
        CornerStyle::Sharp => LineJoin::Miter(4.0),
    };
    let mp = to_multipolygon(paths);
    let mut out = if mp.0.is_empty() {
        MultiPolygon(vec![])
    } else {
        mp.buffer_with_style(BufferStyle::new(dist).line_join(join()))
    };
    let ml = open_lines(paths);
    if !ml.0.is_empty() && dist > 0.0 {
        let s = ml.buffer_with_style(BufferStyle::new(dist).line_join(join()));
        out = out.union(&s);
    }
    from_multipolygon(&out)
}

/// The area a pen `width` wide covers drawing every subpath of `paths`
/// (closed ones all the way round): how an SVG stroke looks, as an outline
/// that can be cut.
pub fn stroke_outline(paths: &[BezPath], width: f64, round_join: bool, round_cap: bool) -> BezPath {
    let lines = MultiLineString(
        flatten_all(paths, FLATTEN_TOL)
            .iter()
            .filter(|p| p.pts.len() >= 2)
            .map(|p| {
                let mut c: Vec<Coord<f64>> = p.pts.iter().map(|q| Coord { x: q.x, y: q.y }).collect();
                if p.closed {
                    c.push(c[0]);
                }
                LineString::new(c)
            })
            .collect(),
    );
    if lines.0.is_empty() || width <= 0.0 {
        return BezPath::new();
    }
    let style = BufferStyle::new(width / 2.0)
        .line_join(if round_join { LineJoin::Round(0.1) } else { LineJoin::Miter(4.0) })
        .line_cap(if round_cap { LineCap::Round(0.1) } else { LineCap::Butt });
    from_multipolygon(&lines.buffer_with_style(style))
}

/// Ramer–Douglas–Peucker simplification.
pub fn rdp(pts: &[Point], eps: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let mut best = (0.0, 0usize);
        for i in a + 1..b {
            let d = seg_dist(pts[i], pa, pb);
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Whether any line of `paths` passes through `rect` (crossing selection).
/// Ported from BurnCraft: a segment touches the box if it survives clipping
/// to it (Liang–Barsky).
pub fn outline_touches(paths: &[BezPath], rect: Rect) -> bool {
    let touches = |a: Point, b: Point| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (p, q) in [(-dx, a.x - rect.x0), (dx, rect.x1 - a.x), (-dy, a.y - rect.y0), (dy, rect.y1 - a.y)] {
            if p.abs() < 1e-12 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    };
    flatten_all(paths, 0.1).iter().any(|pl| {
        let n = pl.pts.len();
        match n {
            0 => false,
            1 => rect.contains(pl.pts[0]),
            _ => pl.pts.windows(2).any(|w| touches(w[0], w[1])) || (pl.closed && touches(pl.pts[n - 1], pl.pts[0])),
        }
    })
}

pub fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 < 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Fit smooth cubic Béziers (Catmull–Rom) through a polyline, keeping sharp corners
/// whose turning angle exceeds `corner_deg`.
pub fn smooth_polyline(pl: &Polyline, corner_deg: f64) -> BezPath {
    let pts = &pl.pts;
    let n = pts.len();
    let mut b = BezPath::new();
    if n < 3 {
        return polyline_to_path(pl);
    }
    let get = |i: isize| -> Point {
        if pl.closed {
            pts[i.rem_euclid(n as isize) as usize]
        } else {
            pts[i.clamp(0, n as isize - 1) as usize]
        }
    };
    let corner = |i: isize| -> bool {
        if !pl.closed && (i == 0 || i == n as isize - 1) {
            return true;
        }
        let a = get(i) - get(i - 1);
        let c = get(i + 1) - get(i);
        let ang = (a.cross(c)).atan2(a.dot(c)).abs().to_degrees();
        ang > corner_deg
    };
    b.move_to(pts[0]);
    let segs = if pl.closed { n } else { n - 1 };
    for i in 0..segs as isize {
        let p0 = get(i);
        let p1 = get(i + 1);
        let t0 = if corner(i) { p1 - p0 } else { (get(i + 1) - get(i - 1)) * 0.5 };
        let t1 = if corner(i + 1) { p1 - p0 } else { (get(i + 2) - get(i)) * 0.5 };
        let c1 = p0 + t0 / 3.0;
        let c2 = p1 - t1 / 3.0;
        b.curve_to(c1, c2, p1);
    }
    if pl.closed {
        b.close_path();
    }
    b
}

/// Simplify and smooth all subpaths of a path.
pub fn simplify_path(path: &BezPath, tolerance: f64, smooth: bool) -> BezPath {
    let mut out = BezPath::new();
    for pl in flatten(path, tolerance / 4.0) {
        let mut pts = pl.pts.clone();
        if pl.closed {
            pts.push(pts[0]);
        }
        let mut s = rdp(&pts, tolerance);
        if pl.closed && s.len() > 1 {
            s.pop();
        }
        let spl = Polyline { pts: s, closed: pl.closed };
        let p = if smooth { smooth_polyline(&spl, 50.0) } else { polyline_to_path(&spl) };
        out.extend(p.elements().iter().copied());
    }
    out
}

/// Split a path into its subpaths.
pub fn subpaths(path: &BezPath) -> Vec<BezPath> {
    let mut out = Vec::new();
    let mut cur = BezPath::new();
    for el in path.elements() {
        if let PathEl::MoveTo(_) = el {
            if cur.elements().len() > 1 {
                out.push(std::mem::take(&mut cur));
            }
            cur = BezPath::new();
        }
        cur.push(*el);
    }
    if cur.elements().len() > 1 {
        out.push(cur);
    }
    out
}

/// Even-odd point in path test (flattened).
pub fn contains(paths: &[BezPath], p: Point) -> bool {
    let mut inside = false;
    for pl in flatten_all(paths, 0.1) {
        if !pl.closed && pl.pts.len() < 3 {
            continue;
        }
        let n = pl.pts.len();
        for i in 0..n {
            let a = pl.pts[i];
            let b = pl.pts[(i + 1) % n];
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// Minimum distance from a point to the outline of the paths.
pub fn distance_to_outline(paths: &[BezPath], p: Point) -> f64 {
    let mut best = f64::INFINITY;
    for pl in flatten_all(paths, 0.1) {
        let n = pl.pts.len();
        let segs = if pl.closed { n } else { n.saturating_sub(1) };
        for i in 0..segs {
            best = best.min(seg_dist(p, pl.pts[i], pl.pts[(i + 1) % n]));
        }
        if n == 1 {
            best = best.min(p.distance(pl.pts[0]));
        }
    }
    best
}

// ---------------------------------------------------------------- primitives

pub fn rect_path(r: Rect) -> BezPath {
    r.to_path(0.1)
}

pub fn rounded_rect_path(r: Rect, radius: f64) -> BezPath {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    kurbo::RoundedRect::from_rect(r, rad).to_path(0.01)
}

pub fn ellipse_path(r: Rect) -> BezPath {
    kurbo::Ellipse::from_rect(r).to_path(0.01)
}

pub fn circle_path(c: Point, radius: f64) -> BezPath {
    kurbo::Circle::new(c, radius).to_path(0.01)
}

/// Regular polygon inscribed in rect (first vertex at top).
pub fn polygon_path(r: Rect, sides: usize) -> BezPath {
    let sides = sides.max(3);
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let mut b = BezPath::new();
    for i in 0..sides {
        let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / sides as f64;
        let p = Point::new(c.x + rx * a.cos(), c.y + ry * a.sin());
        if i == 0 { b.move_to(p) } else { b.line_to(p) }
    }
    b.close_path();
    b
}

pub fn star_path(r: Rect, points: usize, inner_ratio: f64) -> BezPath {
    let points = points.max(3);
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let mut b = BezPath::new();
    for i in 0..points * 2 {
        let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / points as f64;
        let k = if i % 2 == 0 { 1.0 } else { inner_ratio };
        let p = Point::new(c.x + rx * k * a.cos(), c.y + ry * k * a.sin());
        if i == 0 { b.move_to(p) } else { b.line_to(p) }
    }
    b.close_path();
    b
}

/// Path through points as straight segments.
pub fn polyline_path(pts: &[Point], closed: bool) -> BezPath {
    polyline_to_path(&Polyline { pts: pts.to_vec(), closed })
}

pub fn transformed(paths: &[BezPath], a: Affine) -> Vec<BezPath> {
    paths
        .iter()
        .map(|p| {
            let mut q = p.clone();
            q.apply_affine(a);
            q
        })
        .collect()
}

pub fn merge_paths(paths: &[BezPath]) -> BezPath {
    let mut b = BezPath::new();
    for p in paths {
        b.extend(p.elements().iter().copied());
    }
    b
}

/// Total arc length of a path.
pub fn path_length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(1e-3)).sum()
}

/// Point and unit tangent at arc-length `s` along a path (clamped).
pub fn point_at_length(path: &BezPath, s: f64) -> Option<(Point, kurbo::Vec2)> {
    let mut acc = 0.0;
    let segs: Vec<PathSeg> = path.segments().collect();
    for seg in &segs {
        let l = seg.arclen(1e-3);
        if acc + l >= s && l > 0.0 {
            let t = seg.inv_arclen(s - acc, 1e-3);
            let p = seg.eval(t);
            let p2 = seg.eval((t + 1e-3).min(1.0));
            let p1 = seg.eval((t - 1e-3).max(0.0));
            let d = p2 - p1;
            let len = d.hypot().max(1e-12);
            return Some((p, d / len));
        }
        acc += l;
    }
    let last = segs.last()?;
    let p = last.eval(1.0);
    let d = p - last.eval(0.99);
    Some((p, d / d.hypot().max(1e-12)))
}

/// Dash a set of polylines (perforation): `on` mm cut, `off` mm gap.
pub fn dash(pls: &[Polyline], on: f64, off: f64) -> Vec<Polyline> {
    let mut out = Vec::new();
    if on <= 0.0 {
        return pls.to_vec();
    }
    for pl in pls {
        let mut pts = pl.pts.clone();
        if pl.closed && !pts.is_empty() {
            pts.push(pts[0]);
        }
        let mut drawing = true;
        let mut remaining = on;
        let mut cur: Vec<Point> = vec![pts[0]];
        for w in pts.windows(2) {
            let (mut a, b) = (w[0], w[1]);
            let mut seg = a.distance(b);
            while seg > 1e-12 {
                if seg <= remaining {
                    remaining -= seg;
                    if drawing {
                        cur.push(b);
                    }
                    seg = 0.0;
                } else {
                    let p = a.lerp(b, remaining / seg);
                    seg -= remaining;
                    a = p;
                    if drawing {
                        cur.push(p);
                        out.push(Polyline { pts: std::mem::take(&mut cur), closed: false });
                    } else {
                        cur = vec![p];
                    }
                    drawing = !drawing;
                    remaining = if drawing { on } else { off };
                }
            }
        }
        if drawing && cur.len() > 1 {
            out.push(Polyline { pts: cur, closed: false });
        }
    }
    out
}

// ---------------------------------------------------------------- serde helper

/// Serialize a BezPath as an SVG path string.
pub mod svg_path_serde {
    use kurbo::BezPath;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(p: &BezPath, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&p.to_svg())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BezPath, D::Error> {
        let s = String::deserialize(d)?;
        BezPath::from_svg(&s).map_err(serde::de::Error::custom)
    }
}

pub mod affine_serde {
    use kurbo::Affine;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(a: &Affine, s: S) -> Result<S::Ok, S::Error> {
        a.as_coeffs().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Affine, D::Error> {
        Ok(Affine::new(<[f64; 6]>::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_of_overlapping_squares() {
        let a = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect_path(Rect::new(5.0, 5.0, 15.0, 15.0));
        let u = boolean(&[a], &[b], BoolOp::Union);
        let mp = to_multipolygon(&[u]);
        use geo::Area;
        assert!((mp.unsigned_area() - 175.0).abs() < 0.5);
    }

    #[test]
    fn offset_grows_square() {
        let a = rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let o = offset(&[a], 1.0, CornerStyle::Sharp);
        let bb = o.bounding_box();
        assert!((bb.width() - 12.0).abs() < 0.05);
    }

    #[test]
    fn dash_splits() {
        let pl = Polyline { pts: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)], closed: false };
        let d = dash(&[pl], 2.0, 1.0);
        assert_eq!(d.len(), 4);
    }
}
