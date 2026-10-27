//! Trim: delete the piece of an outline between its nearest crossings with
//! other outlines (or itself). Ported from BurnCraft's `bc-geom/cut.rs`.
//!
//! Outlines are worked on as fine polylines, so curved pieces that are trimmed
//! come out as short straight segments (0.02 mm), far below what a blade resolves.

use crate::doc::{Document, ShapeId, ShapeKind};
use crate::geom::{self, polyline_path};
use kurbo::{Affine, BezPath, PathEl, Point};

const TOLERANCE: f64 = 0.02;

/// Splits a path into its subpaths (each starting with a move-to).
fn split_subpaths(path: &BezPath) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = Vec::new();
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) || out.is_empty() {
            out.push(BezPath::new());
        }
        out.last_mut().unwrap().push(*el);
    }
    out
}

/// Where segment a0→a1 crosses b0→b1, as a fraction along a (0..=1).
fn crossing(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<f64> {
    let (r, s) = (a1 - a0, b1 - b0);
    let denom = r.cross(s);
    if denom.abs() < 1e-12 {
        return None;
    }
    let q = b0 - a0;
    let t = q.cross(s) / denom;
    let u = q.cross(r) / denom;
    ((-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u)).then_some(t.clamp(0.0, 1.0))
}

/// A polyline with its cumulative lengths.
struct Poly {
    pts: Vec<Point>,
    len: Vec<f64>,
    closed: bool,
}

impl Poly {
    fn new(mut pts: Vec<Point>, closed: bool) -> Self {
        if closed && pts.len() > 1 && pts.first() != pts.last() {
            pts.push(pts[0]);
        }
        let mut len = vec![0.0];
        for w in pts.windows(2) {
            len.push(len.last().unwrap() + w[0].distance(w[1]));
        }
        Self { pts, len, closed }
    }

    fn total(&self) -> f64 {
        *self.len.last().unwrap_or(&0.0)
    }

    fn at(&self, s: f64) -> Point {
        let i = self.len.partition_point(|&l| l <= s).clamp(1, self.pts.len() - 1);
        let seg = (self.len[i] - self.len[i - 1]).max(1e-12);
        self.pts[i - 1].lerp(self.pts[i], ((s - self.len[i - 1]) / seg).clamp(0.0, 1.0))
    }

    /// The part between distances `a` and `b` (a < b), as points.
    fn piece(&self, a: f64, b: f64) -> Vec<Point> {
        let mut out = vec![self.at(a)];
        for (p, &l) in self.pts.iter().zip(&self.len) {
            if l > a + 1e-9 && l < b - 1e-9 {
                out.push(*p);
            }
        }
        out.push(self.at(b));
        out
    }

    /// Nearest distance along the polyline to `p`, and how far `p` is from it.
    fn project(&self, p: Point) -> (f64, f64) {
        let mut best = (0.0, f64::MAX);
        for (i, w) in self.pts.windows(2).enumerate() {
            let d = w[1] - w[0];
            let t = if d.hypot2() > 0.0 { ((p - w[0]).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
            let dist = p.distance(w[0] + d * t);
            if dist < best.1 {
                best = (self.len[i] + t * d.hypot(), dist);
            }
        }
        best
    }

    /// Distances along this polyline where it crosses `other` (skipping a
    /// segment against itself and its neighbours when `same`).
    fn crossings_with(&self, other: &Poly, same: bool) -> Vec<f64> {
        let mut out = Vec::new();
        for (i, a) in self.pts.windows(2).enumerate() {
            for (j, b) in other.pts.windows(2).enumerate() {
                if same {
                    let n = self.pts.len() - 1;
                    let adjacent = i == j || i + 1 == j || j + 1 == i || (self.closed && ((i == 0 && j == n - 1) || (j == 0 && i == n - 1)));
                    if adjacent {
                        continue;
                    }
                }
                if let Some(t) = crossing(a[0], a[1], b[0], b[1]) {
                    out.push(self.len[i] + t * (self.len[i + 1] - self.len[i]));
                }
            }
        }
        out
    }
}

fn polys(path: &BezPath) -> Vec<Poly> {
    geom::flatten(path, TOLERANCE).into_iter().filter(|l| l.pts.len() >= 2).map(|l| Poly::new(l.pts, l.closed)).collect()
}

/// Removes the piece of `target`'s outline under `at` (within `tol`), between
/// its nearest crossings with `cutters` (and with itself) on either side. Open
/// outlines are trimmed back to their ends if there's no crossing that way.
/// Returns the new outline (possibly empty), or `None` if nothing was under
/// `at` or a closed outline has no crossings to trim between.
pub fn trim(target: &BezPath, cutters: &[BezPath], at: Point, tol: f64) -> Option<BezPath> {
    let subs = split_subpaths(target);
    let (hit, poly, s_click) = subs
        .iter()
        .enumerate()
        .filter_map(|(i, sp)| {
            let p = polys(sp).into_iter().next()?;
            let (s, d) = p.project(at);
            (d <= tol).then_some((i, p, s, d))
        })
        .min_by(|a, b| a.3.total_cmp(&b.3))
        .map(|(i, p, s, _)| (i, p, s))?;
    let mut cuts: Vec<f64> = Vec::new();
    for (i, sp) in subs.iter().enumerate() {
        for other in polys(sp) {
            cuts.extend(poly.crossings_with(&other, i == hit));
        }
    }
    for c in cutters {
        for other in polys(c) {
            cuts.extend(poly.crossings_with(&other, false));
        }
    }
    let total = poly.total();
    let eps = 1e-6;
    cuts.retain(|s| *s > eps && *s < total - eps || poly.closed);
    cuts.sort_by(|a, b| a.total_cmp(b));
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let below = cuts.iter().copied().rev().find(|&s| s < s_click - eps);
    let above = cuts.iter().copied().find(|&s| s > s_click + eps);
    let mut kept: Vec<Vec<Point>> = Vec::new();
    if poly.closed {
        // keep the rest of the loop: from the crossing after the click round to the one before it
        let (lo, hi) = match (below, above) {
            (Some(lo), Some(hi)) => (lo, hi),
            // crossings on one side only: wrap round the loop for the other
            (None, Some(hi)) if cuts.len() >= 2 => (*cuts.last().unwrap() - total, hi),
            (Some(lo), None) if cuts.len() >= 2 => (lo, cuts[0] + total),
            _ => return None,
        };
        let mut pts = if hi <= total { poly.piece(hi, total) } else { Vec::new() };
        let wrap_end = if lo < 0.0 { lo + total } else { lo };
        let start = if hi > total { hi - total } else { 0.0 };
        let mut tail = poly.piece(start, wrap_end);
        if !pts.is_empty() {
            tail.remove(0);
        }
        pts.extend(tail);
        kept.push(pts);
    } else {
        let lo = below.unwrap_or(0.0);
        let hi = above.unwrap_or(total);
        if lo > eps {
            kept.push(poly.piece(0.0, lo));
        }
        if hi < total - eps {
            kept.push(poly.piece(hi, total));
        }
    }
    let mut out = BezPath::new();
    for (i, sp) in subs.iter().enumerate() {
        if i == hit {
            for pts in kept.iter().filter(|p| p.len() >= 2) {
                out.extend(polyline_path(pts, false));
            }
        } else {
            out.extend(sp.iter());
        }
    }
    Some(out)
}

/// The visible, unlocked path shape whose outline passes within `tol` of `p`.
pub fn outline_near(doc: &Document, p: Point, tol: f64) -> Option<ShapeId> {
    let page = doc.page();
    page.shapes
        .iter()
        .filter(|s| page.is_editable(s) && matches!(s.kind, ShapeKind::Path(_)))
        .map(|s| (s.id, geom::distance_to_outline(&s.world_paths(), p)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Trim tool: deletes the piece of the outline under `p` between its crossings
/// with other visible shapes (or itself). A shape trimmed to nothing is
/// removed. Returns the shape changed and whether it was removed.
pub fn trim_at(doc: &mut Document, p: Point, tol: f64) -> Option<(ShapeId, bool)> {
    let id = outline_near(doc, p, tol)?;
    let page = doc.page();
    let target = page.shape(id)?.world_paths().into_iter().fold(BezPath::new(), |mut a, b| {
        a.extend(b);
        a
    });
    let cutters: Vec<BezPath> = page.shapes.iter().filter(|s| s.id != id && page.is_visible(s) && !s.is_image()).flat_map(|s| s.world_paths()).collect();
    let trimmed = trim(&target, &cutters, p, tol)?;
    let gone = trimmed.elements().is_empty();
    if gone {
        doc.page_mut().shapes.retain(|s| s.id != id);
    } else if let Some(s) = doc.page_mut().shape_mut(id) {
        s.kind = ShapeKind::Path(trimmed);
        s.transform = Affine::IDENTITY;
    }
    Some((id, gone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Style;
    use kurbo::{Rect, Shape as _};

    fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
        polyline_path(&[Point::new(a.0, a.1), Point::new(b.0, b.1)], false)
    }

    #[test]
    fn trim_a_line_between_two_crossings() {
        let target = line((0.0, 0.0), (100.0, 0.0));
        let cutters = [line((30.0, -10.0), (30.0, 10.0)), line((60.0, -10.0), (60.0, 10.0))];
        let out = trim(&target, &cutters, Point::new(45.0, 0.5), 1.0).unwrap();
        let pieces = geom::flatten(&out, 0.01);
        assert_eq!(pieces.len(), 2);
        let b: Vec<_> = pieces.iter().map(|l| (l.pts[0].x, l.pts.last().unwrap().x)).collect();
        assert!((b[0].1 - 30.0).abs() < 1e-6 && (b[1].0 - 60.0).abs() < 1e-6, "{b:?}");
        // trimming the end piece takes it back to the crossing
        let out = trim(&target, &cutters, Point::new(90.0, 0.0), 1.0).unwrap();
        assert!((out.bounding_box().x1 - 60.0).abs() < 1e-6);
    }

    #[test]
    fn trim_a_box_side_between_crossing_lines() {
        let sq = geom::rect_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        let cutters = [line((30.0, -10.0), (30.0, 10.0)), line((70.0, -10.0), (70.0, 10.0))];
        let out = trim(&sq, &cutters, Point::new(50.0, 0.0), 1.0).unwrap();
        let pieces = geom::flatten(&out, 0.01);
        assert_eq!(pieces.len(), 1, "one open outline");
        assert!(!pieces[0].closed);
        let xs = [pieces[0].pts[0].x, pieces[0].pts.last().unwrap().x];
        assert!(xs.iter().any(|x| (x - 70.0).abs() < 1e-6) && xs.iter().any(|x| (x - 30.0).abs() < 1e-6), "{xs:?}");
        // a loop with nothing crossing it can't be trimmed
        assert!(trim(&sq, &[], Point::new(50.0, 0.0), 1.0).is_none());
    }

    #[test]
    fn trim_on_the_document_removes_a_fully_trimmed_line() {
        let mut d = Document::default();
        let a = d.add_path(line((0.0, 50.0), (100.0, 50.0)), Style::default(), 0);
        d.add_path(line((40.0, 0.0), (40.0, 100.0)), Style::default(), 0);
        let (id, gone) = trim_at(&mut d, Point::new(80.0, 50.0), 1.0).unwrap();
        assert_eq!((id, gone), (a, false));
        let b = d.page().shape(a).unwrap().world_paths()[0].bounding_box();
        assert!((b.x1 - 40.0).abs() < 1e-6);
        let (_, gone) = trim_at(&mut d, Point::new(20.0, 50.0), 1.0).unwrap();
        assert!(gone, "nothing left of the line");
        assert!(trim_at(&mut d, Point::new(200.0, 200.0), 1.0).is_none());
    }
}
