//! Per-corner rounding and chamfering: find the corners of a path (sharp ones,
//! and ones already rounded by this module so they can be changed again), and
//! round or bevel any subset of them. Powers the Round-corner tool.
//!
//! Ported from BurnCraft's `bc-geom/lib.rs` (fillet / corners / set_corner).
//! The enum is `CornerCut` here because `geom::CornerStyle` already names
//! offset joins in CutCraft.

use kurbo::{BezPath, ParamCurve, ParamCurveArclen, PathEl, Point, Vec2};

/// Rounds every corner of `path` where two straight edges meet, with circular arcs of `radius` mm.
/// Curved segments are left as they are; radii are reduced where edges are too short.
pub fn fillet(path: &BezPath, radius: f64) -> BezPath {
    fillet_where(path, radius, |_| true)
}

/// Corners of `path` that can be rounded: points where two straight edges meet at an angle.
pub fn corner_points(path: &BezPath) -> Vec<Point> {
    subpaths(path).iter().flat_map(|s| (0..s.segs.len()).filter_map(|i| s.corner(i).map(|(_, v, _)| v))).collect()
}

/// How a corner is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CornerCut {
    /// A circular arc of the given radius.
    #[default]
    Round,
    /// A straight bevel starting the given distance back from the corner along each edge.
    Chamfer,
}

/// Rounds the corners for which `pick(corner point)` is true (see [`fillet`]).
pub fn fillet_where(path: &BezPath, radius: f64, pick: impl Fn(Point) -> bool) -> BezPath {
    corner_where(path, radius, CornerCut::Round, pick)
}

/// Rounds or chamfers the corners for which `pick(corner point)` is true.
pub fn corner_where(path: &BezPath, radius: f64, style: CornerCut, pick: impl Fn(Point) -> bool) -> BezPath {
    if radius <= 0.0 {
        return path.clone();
    }
    let mut out = BezPath::new();
    for sub in subpaths(path) {
        let n = sub.segs.len();
        // Corner after segment i: tangent distance t, keyed by i.
        let rounded: Vec<Option<(Vec2, Point, Vec2, f64)>> = (0..n)
            .map(|i| {
                let (d_in, v, d_out) = sub.corner(i)?;
                if !pick(v) {
                    return None;
                }
                let angle = (-d_in).dot(d_out).clamp(-1.0, 1.0).acos();
                // Limited to half of each neighbouring edge, so two rounded ends of one edge never overlap.
                let len = |seg: &kurbo::PathSeg| seg.arclen(1e-6);
                let max_t = (len(&sub.segs[i]) / 2.0).min(len(&sub.segs[(i + 1) % n]) / 2.0);
                let t = match style {
                    CornerCut::Round => radius / (angle / 2.0).tan(),
                    CornerCut::Chamfer => radius,
                }
                .min(max_t);
                Some((d_in, v, d_out, t))
            })
            .collect();
        // Trimmed start of segment j (after a rounded corner) and end of segment i (before one).
        let start_of = |j: usize| -> Point {
            let prev = if j == 0 { if sub.closed { Some(n - 1) } else { None } } else { Some(j - 1) };
            match prev.and_then(|p| rounded[p]) {
                Some((_, v, d_out, t)) => v + d_out * t,
                None => sub.segs[j].start(),
            }
        };
        let end_of = |i: usize| -> Point {
            match rounded[i] {
                Some((d_in, v, _, t)) => v - d_in * t,
                None => sub.segs[i].end(),
            }
        };
        out.move_to(start_of(0));
        for (i, (seg, corner)) in sub.segs.iter().zip(&rounded).enumerate() {
            match seg {
                kurbo::PathSeg::Line(_) => out.line_to(end_of(i)),
                kurbo::PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                kurbo::PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
            }
            if let Some((_, v, _, _)) = *corner {
                match style {
                    // A quadratic through the corner approximates the arc closely and keeps tangents.
                    CornerCut::Round => out.quad_to(v, start_of((i + 1) % n)),
                    CornerCut::Chamfer => out.line_to(start_of((i + 1) % n)),
                }
            }
        }
        if sub.closed {
            out.close_path();
        }
    }
    out
}

/// A corner the corner tool can work on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    /// Where the two straight edges meet (for a rounded corner, where they would meet).
    pub vertex: Point,
    /// Where to show and click it: the vertex, or the middle of the rounding.
    pub handle: Point,
    /// The current radius of a rounded corner; `None` when sharp.
    pub radius: Option<f64>,
}

/// Sharp corners and corners rounded by [`fillet`] (a quadratic between two straight edges,
/// tangent to both), so rounded corners can be changed again.
pub fn corners(path: &BezPath) -> Vec<Corner> {
    let mut out = Vec::new();
    for sub in subpaths(path) {
        let n = sub.segs.len();
        for i in 0..n {
            if let Some((_, v, _)) = sub.corner(i) {
                out.push(Corner { vertex: v, handle: v, radius: None });
            }
            if let Some((k, r)) = sub.rounded_at(i) {
                let kurbo::PathSeg::Quad(q) = sub.segs[k] else { continue };
                out.push(Corner { vertex: q.p1, handle: q.eval(0.5), radius: Some(r) });
            }
        }
    }
    out
}

/// Sets the corner at `vertex` (sharp or rounded) to `radius`; 0 makes it sharp again.
pub fn set_corner_radius(path: &BezPath, vertex: Point, radius: f64) -> BezPath {
    set_corner(path, vertex, radius, CornerCut::Round)
}

/// Sets the corner at `vertex` (sharp or rounded) to a rounding or chamfer of `size`; 0 makes it sharp.
/// (A chamfer turns one corner into two sharp ones, so it can't be found again as a single corner.)
pub fn set_corner(path: &BezPath, vertex: Point, size: f64, style: CornerCut) -> BezPath {
    let is_it = |p: Point| p.distance(vertex) < 1e-6;
    let mut subs = subpaths(path);
    // Undo any existing rounding at this corner: extend both edges back to the vertex.
    for sub in &mut subs {
        let n = sub.segs.len();
        let Some(k) = (0..n).find_map(|i| sub.rounded_at(i).map(|(k, _)| k).filter(|&k| matches!(sub.segs[k], kurbo::PathSeg::Quad(q) if is_it(q.p1)))) else { continue };
        let (prev, next) = ((k + n - 1) % n, (k + 1) % n);
        let (kurbo::PathSeg::Line(a), kurbo::PathSeg::Line(b)) = (sub.segs[prev], sub.segs[next]) else { continue };
        sub.segs[prev] = kurbo::PathSeg::Line(kurbo::Line::new(a.p0, vertex));
        sub.segs[next] = kurbo::PathSeg::Line(kurbo::Line::new(vertex, b.p1));
        sub.segs.remove(k);
    }
    let sharp = build(&subs);
    corner_where(&sharp, size, style, is_it)
}

fn build(subs: &[Subpath]) -> BezPath {
    let mut out = BezPath::new();
    for sub in subs {
        let Some(first) = sub.segs.first() else { continue };
        out.move_to(first.start());
        for seg in &sub.segs {
            match seg {
                kurbo::PathSeg::Line(l) => out.line_to(l.p1),
                kurbo::PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                kurbo::PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
            }
        }
        if sub.closed {
            out.close_path();
        }
    }
    out
}
/// One subpath as segments (a closing edge becomes a line).
struct Subpath {
    segs: Vec<kurbo::PathSeg>,
    closed: bool,
}

impl Subpath {
    /// A rounded corner whose quadratic is segment `i + 1` (between straight segments `i` and `i + 2`,
    /// tangent to both): (index of the quadratic, its radius).
    fn rounded_at(&self, i: usize) -> Option<(usize, f64)> {
        let n = self.segs.len();
        if n < 3 || (!self.closed && i + 2 >= n) {
            return None;
        }
        let k = (i + 1) % n;
        let (kurbo::PathSeg::Line(a), kurbo::PathSeg::Quad(q), kurbo::PathSeg::Line(b)) = (self.segs[i], self.segs[k], self.segs[(i + 2) % n]) else { return None };
        let dir = |v: Vec2| (v.hypot() > 1e-9).then(|| v.normalize());
        let (Some(la), Some(qa), Some(qb), Some(lb)) = (dir(a.p1 - a.p0), dir(q.p1 - q.p0), dir(q.p2 - q.p1), dir(b.p1 - b.p0)) else { return None };
        let tangent = |x: Vec2, y: Vec2| x.dot(y) > 1.0 - 1e-6;
        if !(tangent(la, qa) && tangent(qb, lb)) {
            return None;
        }
        let angle = (-qa).dot(qb).clamp(-1.0, 1.0).acos();
        if !(1e-3..std::f64::consts::PI - 1e-3).contains(&angle) {
            return None;
        }
        Some((k, (q.p1 - q.p0).hypot() * (angle / 2.0).tan()))
    }
    /// The corner after segment `i`, if both neighbours are straight and meet at an angle:
    /// (incoming direction, corner point, outgoing direction).
    fn corner(&self, i: usize) -> Option<(Vec2, Point, Vec2)> {
        let n = self.segs.len();
        if !self.closed && i + 1 >= n {
            return None;
        }
        let (kurbo::PathSeg::Line(a), kurbo::PathSeg::Line(b)) = (self.segs[i], self.segs[(i + 1) % n]) else { return None };
        let (la, lb) = (a.p1 - a.p0, b.p1 - b.p0);
        if la.hypot() < 1e-9 || lb.hypot() < 1e-9 {
            return None;
        }
        let (d_in, d_out) = (la.normalize(), lb.normalize());
        let angle = (-d_in).dot(d_out).clamp(-1.0, 1.0).acos();
        (1e-3..std::f64::consts::PI - 1e-3).contains(&angle).then_some((d_in, a.p1, d_out))
    }
}

fn subpaths(path: &BezPath) -> Vec<Subpath> {
    let mut subs: Vec<Subpath> = Vec::new();
    let (mut start, mut cur) = (Point::ORIGIN, Point::ORIGIN);
    let mut segs: Vec<kurbo::PathSeg> = Vec::new();
    let mut flush = |segs: &mut Vec<kurbo::PathSeg>, closed: bool| {
        if !segs.is_empty() {
            subs.push(Subpath { segs: std::mem::take(segs), closed });
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut segs, false);
                (start, cur) = (p, p);
            }
            PathEl::LineTo(p) => {
                segs.push(kurbo::PathSeg::Line(kurbo::Line::new(cur, p)));
                cur = p;
            }
            PathEl::QuadTo(a, p) => {
                segs.push(kurbo::PathSeg::Quad(kurbo::QuadBez::new(cur, a, p)));
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                segs.push(kurbo::PathSeg::Cubic(kurbo::CubicBez::new(cur, a, b, p)));
                cur = p;
            }
            PathEl::ClosePath => {
                if cur.distance(start) > 1e-9 {
                    segs.push(kurbo::PathSeg::Line(kurbo::Line::new(cur, start)));
                }
                flush(&mut segs, true);
                cur = start;
            }
        }
    }
    flush(&mut segs, false);
    subs
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    fn square() -> BezPath {
        crate::geom::polyline_path(&[Point::new(0.0, 0.0), Point::new(40.0, 0.0), Point::new(40.0, 40.0), Point::new(0.0, 40.0)], true)
    }

    #[test]
    fn finds_four_sharp_corners() {
        let c = corners(&square());
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|c| c.radius.is_none()));
    }

    #[test]
    fn rounds_one_corner_and_can_change_it_again() {
        let p = set_corner(&square(), Point::new(40.0, 0.0), 8.0, CornerCut::Round);
        let c = corners(&p);
        assert_eq!(c.len(), 4, "3 sharp + 1 rounded");
        let r = c.iter().find(|c| c.radius.is_some()).unwrap();
        assert!((r.radius.unwrap() - 8.0).abs() < 1e-6 && r.vertex.distance(Point::new(40.0, 0.0)) < 1e-6);
        // rounding removes a little area at that corner only
        assert!(p.area().abs() < 1600.0 && p.area().abs() > 1600.0 - 8.0 * 8.0);
        // change the radius, then make it sharp again
        let bigger = set_corner(&p, Point::new(40.0, 0.0), 15.0, CornerCut::Round);
        assert!(corners(&bigger).iter().any(|c| c.radius.map(|r| (r - 15.0).abs() < 1e-6).unwrap_or(false)));
        let sharp = set_corner(&bigger, Point::new(40.0, 0.0), 0.0, CornerCut::Round);
        assert!((sharp.area().abs() - 1600.0).abs() < 1e-6);
    }

    #[test]
    fn chamfer_bevels_a_corner() {
        let p = set_corner(&square(), Point::new(0.0, 0.0), 10.0, CornerCut::Chamfer);
        // a 10 mm chamfer removes a right triangle of area 50
        assert!((p.area().abs() - (1600.0 - 50.0)).abs() < 1e-6, "{}", p.area());
        assert_eq!(corners(&p).len(), 5, "the chamfer makes two new sharp corners");
    }

    #[test]
    fn fillet_all_limits_radius_to_edges() {
        let p = fillet(&square(), 100.0); // far too big: clamps to half an edge
        assert!(p.bounding_box().width() <= 40.0 + 1e-6);
        assert!(corners(&p).iter().all(|c| c.radius.is_some()));
    }
}
