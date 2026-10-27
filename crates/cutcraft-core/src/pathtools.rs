//! Geometry-only path operations: convex hull, lattice fill, knockout,
//! remove-double-lines, symmetrical mirror and stencil bridges.

use crate::geom::{self, BoolOp, CornerStyle, Polyline};
use kurbo::{Affine, BezPath, Point, Rect};
use std::collections::HashMap;

/// Convex hull of every point in the paths (Andrew's monotone chain).
pub fn convex_hull(paths: &[BezPath]) -> BezPath {
    let mut pts: Vec<Point> = geom::flatten_all(paths, 0.1).into_iter().flat_map(|p| p.pts).collect();
    if pts.len() < 3 {
        return geom::merge_paths(paths);
    }
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
    let cross = |o: Point, a: Point, b: Point| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut hull: Vec<Point> = Vec::with_capacity(pts.len() + 1);
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev() {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    geom::polyline_path(&hull, true)
}

/// Lattice: cut a grid of square holes out of the shape. `cell` is the hole
/// pitch, `bar` the wall width left between holes.
pub fn lattice(paths: &[BezPath], cell: f64, bar: f64) -> BezPath {
    let region = geom::boolean(paths, &[], BoolOp::Union);
    let Some(bb) = geom::paths_bbox(&[region.clone()]) else { return region };
    let cell = cell.max(bar + 0.5);
    let hole = (cell - bar).max(0.2);
    // keep holes clear of the border
    let inner = geom::offset(&[region.clone()], -bar, CornerStyle::Round);
    let mut holes = vec![];
    let mut y = bb.y0 + bar;
    while y + hole <= bb.y1 {
        let mut x = bb.x0 + bar;
        while x + hole <= bb.x1 {
            let r = geom::rect_path(Rect::new(x, y, x + hole, y + hole));
            holes.push(r);
            x += cell;
        }
        y += cell;
    }
    if holes.is_empty() {
        return region;
    }
    // only holes fully inside the inset region
    let clipped: Vec<BezPath> = holes.into_iter().filter(|hp| {
        let hb = geom::paths_bbox(&[hp.clone()]).unwrap();
        geom::contains(&[inner.clone()], hb.center())
            && geom::contains(&[inner.clone()], hb.origin())
            && geom::contains(&[inner.clone()], Point::new(hb.x1, hb.y1))
    }).collect();
    geom::boolean(&[region], &clipped, BoolOp::Difference)
}

/// Knockout: subtract the `top` shapes (grown by `gap`) from the `bottom`
/// shapes, so the top pieces drop into a recess with a uniform gap.
pub fn knockout(bottom: &[BezPath], top: &[BezPath], gap: f64) -> BezPath {
    let grown = if gap > 0.0 { geom::offset(top, gap, CornerStyle::Round) } else { geom::merge_paths(top) };
    geom::boolean(bottom, &[grown], BoolOp::Difference)
}

/// Mirror the artwork across a vertical line at `axis_x`; weld the two halves
/// when `weld` is set, otherwise keep both copies.
pub fn symmetrical_mirror(paths: &[BezPath], axis_x: f64, weld: bool) -> BezPath {
    let m = Affine::translate((2.0 * axis_x, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0);
    let mirrored = geom::transformed(paths, m);
    if weld {
        geom::boolean(paths, &mirrored, BoolOp::Union)
    } else {
        geom::merge_paths(&[geom::merge_paths(paths), geom::merge_paths(&mirrored)])
    }
}

/// Remove duplicated / overlapping line segments (e.g. where two cut shapes
/// share an edge), so the blade doesn't cut the same line twice.
pub fn remove_double_lines(paths: &[BezPath], tol: f64) -> BezPath {
    let tol = tol.max(1e-4);
    let key = |p: Point| ((p.x / tol).round() as i64, (p.y / tol).round() as i64);
    let mut counts: HashMap<((i64, i64), (i64, i64)), (Point, Point, usize)> = HashMap::new();
    for pl in geom::flatten_all(paths, tol / 2.0) {
        let n = pl.pts.len();
        let segs = if pl.closed { n } else { n.saturating_sub(1) };
        for i in 0..segs {
            let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
            if a.distance(b) < tol {
                continue;
            }
            let (ka, kb) = (key(a), key(b));
            let canon = if ka <= kb { (ka, kb) } else { (kb, ka) };
            counts.entry(canon).and_modify(|e| e.2 += 1).or_insert((a, b, 1));
        }
    }
    let mut out = BezPath::new();
    for (_, (a, b, c)) in counts {
        if c == 1 {
            out.extend(geom::polyline_path(&[a, b], false).elements().iter().copied());
        }
    }
    out
}

/// Stencil bridges: break each closed subpath with `count` gaps of width
/// `bridge` mm, so interior islands stay attached to the surrounding material.
pub fn stencil_bridges(paths: &[BezPath], bridge: f64, count: usize) -> BezPath {
    let count = count.max(1);
    let bridge = bridge.max(0.1);
    let mut out = BezPath::new();
    for sub in geom::subpaths(&geom::merge_paths(paths)) {
        let pls = geom::flatten(&sub, 0.05);
        for pl in pls {
            if !pl.closed || pl.pts.len() < 3 {
                out.extend(geom::polyline_to_path(&pl).elements().iter().copied());
                continue;
            }
            let len = pl.length();
            if len < bridge * count as f64 * 2.0 {
                out.extend(geom::polyline_to_path(&pl).elements().iter().copied());
                continue;
            }
            // positions (arc length) of the gap centres
            let gaps: Vec<(f64, f64)> =
                (0..count).map(|i| len * (i as f64 + 0.5) / count as f64).map(|c| (c - bridge / 2.0, c + bridge / 2.0)).collect();
            let in_gap = |s: f64| gaps.iter().any(|(a, b)| s >= *a && s <= *b);
            // walk the contour, emitting runs that are outside any gap
            let pts = &pl.pts;
            let n = pts.len();
            let mut acc = 0.0;
            let mut cur: Vec<Point> = vec![];
            let push = |cur: &mut Vec<Point>, out: &mut BezPath| {
                if cur.len() > 1 {
                    out.extend(geom::polyline_path(cur, false).elements().iter().copied());
                }
                cur.clear();
            };
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                let seg = a.distance(b);
                // sample start of segment
                if !in_gap(acc) {
                    if cur.is_empty() {
                        cur.push(a);
                    }
                }
                // subdivide the segment so a gap can start/end mid-edge
                let steps = (seg / bridge).ceil().max(1.0) as usize;
                for s in 1..=steps {
                    let t = s as f64 / steps as f64;
                    let sl = acc + seg * t;
                    let p = a.lerp(b, t);
                    if in_gap(sl) {
                        push(&mut cur, &mut out);
                    } else {
                        if cur.is_empty() {
                            cur.push(a.lerp(b, (s - 1) as f64 / steps as f64));
                        }
                        cur.push(p);
                    }
                }
                acc += seg;
            }
            push(&mut cur, &mut out);
        }
    }
    out
}

/// Pierce points: a tiny dot (single plunge) at the start of each subpath.
pub fn pierce_points(paths: &[BezPath]) -> Vec<Point> {
    geom::subpaths(&geom::merge_paths(paths))
        .iter()
        .filter_map(|s| geom::flatten(s, 1.0).into_iter().next().and_then(|pl| pl.pts.first().copied()))
        .collect()
}

/// Line fill: parallel lines across the shape at `angle_deg`, `spacing` apart,
/// clipped to the artwork (for engraving or pen fills). Returns open lines.
pub fn line_fill(paths: &[BezPath], spacing: f64, angle_deg: f64) -> BezPath {
    let region = geom::boolean(paths, &[], BoolOp::Union);
    let Some(bb) = geom::paths_bbox(&[region.clone()]) else { return BezPath::new() };
    let spacing = spacing.max(0.1);
    let c = bb.center();
    let r = bb.width().hypot(bb.height()) / 2.0 + spacing;
    let rot = Affine::rotate_about(angle_deg.to_radians(), c);
    let mut lines = vec![];
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
    geom::clip_lines(&lines, &[region], true)
}

/// Join open subpaths whose endpoints coincide within `tol` into longer paths.
pub fn join_segments(paths: &[BezPath], tol: f64) -> BezPath {
    let tol = tol.max(1e-4);
    let mut chains: Vec<Vec<Point>> = geom::flatten_all(paths, tol / 2.0)
        .into_iter()
        .filter(|p| !p.closed && p.pts.len() > 1)
        .map(|p| p.pts)
        .collect();
    let mut closed: Vec<Polyline> =
        geom::flatten_all(paths, tol / 2.0).into_iter().filter(|p| p.closed).collect();
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..chains.len() {
            for j in 0..chains.len() {
                if i == j {
                    continue;
                }
                let (ai, bi) = (chains[i][0], *chains[i].last().unwrap());
                let (aj, bj) = (chains[j][0], *chains[j].last().unwrap());
                let mut cj = chains[j].clone();
                if bi.distance(aj) < tol {
                    chains[i].extend(cj.drain(1..));
                } else if bi.distance(bj) < tol {
                    cj.reverse();
                    chains[i].extend(cj.drain(1..));
                } else if ai.distance(bj) < tol {
                    let mut nc = cj;
                    nc.extend(chains[i].drain(1..));
                    chains[i] = nc;
                } else if ai.distance(aj) < tol {
                    let mut nc = cj;
                    nc.reverse();
                    nc.extend(chains[i].drain(1..));
                    chains[i] = nc;
                } else {
                    continue;
                }
                chains.remove(j);
                merged = true;
                break 'outer;
            }
        }
    }
    // close chains whose ends meet
    for c in &chains {
        let cl = c.len() > 2 && c[0].distance(*c.last().unwrap()) < tol;
        let mut pts = c.clone();
        if cl {
            pts.pop();
        }
        closed.push(Polyline { pts, closed: cl });
    }
    geom::polylines_to_path(&closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn hull_of_scattered_points() {
        let mut b = BezPath::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (5.0, 5.0)] {
            b.move_to(Point::new(x, y));
            b.line_to(Point::new(x + 0.01, y));
        }
        let h = convex_hull(&[b]);
        use geo::Area;
        let a = geom::to_multipolygon(&[h]).unsigned_area();
        assert!((a - 100.0).abs() < 0.5);
    }

    #[test]
    fn lattice_cuts_holes() {
        let sq = geom::rect_path(Rect::new(0.0, 0.0, 60.0, 60.0));
        let l = lattice(&[sq], 15.0, 4.0);
        use geo::Area;
        let a = geom::to_multipolygon(&[l]).unsigned_area();
        assert!(a < 3600.0 - 100.0, "holes removed area: {a}");
    }

    #[test]
    fn remove_double_lines_drops_shared_edge() {
        // two squares sharing the edge x=10
        let a = geom::rect_path(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = geom::rect_path(Rect::new(10.0, 0.0, 20.0, 10.0));
        let r = remove_double_lines(&[a, b], 0.1);
        // the shared vertical edge should be gone: total length < both perimeters
        let len = geom::path_length(&r);
        assert!(len < 80.0 - 15.0, "len {len}");
    }

    #[test]
    fn symmetrical_mirror_is_symmetric() {
        let tri = geom::polyline_path(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(0.0, 20.0)], true);
        let m = symmetrical_mirror(&[tri], 20.0, true);
        let bb = m.bounding_box();
        assert!((bb.x0 - 0.0).abs() < 0.5 && (bb.x1 - 40.0).abs() < 0.5);
    }

    #[test]
    fn stencil_bridges_opens_the_contour() {
        let circle = geom::circle_path(Point::new(0.0, 0.0), 20.0);
        let b = stencil_bridges(&[circle], 3.0, 4);
        // bridging a closed loop yields open subpaths (arcs)
        let open = geom::flatten(&b, 0.1).iter().filter(|p| !p.closed).count();
        assert!(open >= 4, "open arcs: {open}");
    }

    #[test]
    fn join_segments_reconnects() {
        let a = geom::polyline_path(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false);
        let b = geom::polyline_path(&[Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false);
        let j = join_segments(&[a, b], 0.1);
        assert_eq!(geom::subpaths(&j).len(), 1);
    }

    #[test]
    fn line_fill_inside_shape() {
        let sq = geom::rect_path(Rect::new(0.0, 0.0, 20.0, 20.0));
        let f = line_fill(&[sq], 2.0, 0.0);
        let bb = f.bounding_box();
        assert!(bb.x0 >= -0.5 && bb.x1 <= 20.5 && geom::path_length(&f) > 50.0);
    }
}
