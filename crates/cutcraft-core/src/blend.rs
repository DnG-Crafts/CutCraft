//! Blend: steps between two shapes - the outline morphing from one to the
//! other, with the colours and line width in between. Popular for layered
//! paper and shadow-box art (graduated layers).
//!
//! Outlines with different numbers of points are matched by resampling both
//! evenly along their length, turning them the same way round and starting
//! them at the best-matching point; each step is then fitted with smooth
//! curves again.

use crate::doc::{Document, Rgba, Shape, ShapeId, ShapeKind, Style};
use crate::geom::{self, Polyline};
use kurbo::{BezPath, Point, Vec2};

/// Points each outline is resampled to.
const SAMPLES: usize = 160;

fn lerp_pt(a: Point, b: Point, t: f64) -> Point {
    a + (b - a) * t
}

fn lerp_rgba(a: Rgba, b: Rgba, t: f64) -> Rgba {
    let c = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round().clamp(0.0, 255.0) as u8;
    Rgba(c(a.0, b.0), c(a.1, b.1), c(a.2, b.2), c(a.3, b.3))
}

fn lerp_colour(a: Option<Rgba>, b: Option<Rgba>, t: f64) -> Option<Rgba> {
    match (a, b) {
        (Some(a), Some(b)) => Some(lerp_rgba(a, b, t)),
        // one side has none: the other's colour, fading in or out
        (Some(a), None) => Some(Rgba(a.0, a.1, a.2, (a.3 as f64 * (1.0 - t)).round() as u8)).filter(|_| t < 1.0),
        (None, Some(b)) => Some(Rgba(b.0, b.1, b.2, (b.3 as f64 * t).round() as u8)).filter(|_| t > 0.0),
        (None, None) => None,
    }
}

/// `n` points evenly spaced along a polyline (closed: around the loop).
fn resample(pl: &Polyline, n: usize) -> Vec<Point> {
    let mut pts = pl.pts.clone();
    if pl.closed {
        if let Some(f) = pts.first().copied() {
            pts.push(f);
        }
    }
    let lens: Vec<f64> = pts.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f64 = lens.iter().sum();
    if pts.len() < 2 || total <= 0.0 {
        return vec![pts.first().copied().unwrap_or_default(); n];
    }
    let count = if pl.closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(n);
    let (mut seg, mut at) = (0usize, 0.0f64);
    for i in 0..n {
        let want = total * i as f64 / count as f64;
        while seg + 1 < lens.len() && at + lens[seg] < want {
            at += lens[seg];
            seg += 1;
        }
        let t = if lens[seg] > 0.0 { ((want - at) / lens[seg]).clamp(0.0, 1.0) } else { 0.0 };
        out.push(lerp_pt(pts[seg], pts[seg + 1], t));
    }
    out
}

fn centroid(pts: &[Point]) -> Point {
    let s = pts.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2());
    (s / pts.len().max(1) as f64).to_point()
}

/// Turns `b` (a closed loop of the same number of points as `a`) the same
/// way round as `a` and starts it at the point best matching `a`'s start.
fn align_loop(a: &[Point], mut b: Vec<Point>) -> Vec<Point> {
    let area = |p: &[Point]| p.iter().zip(p.iter().cycle().skip(1)).map(|(u, v)| u.x * v.y - v.x * u.y).sum::<f64>();
    if area(a).signum() != area(&b).signum() {
        b.reverse();
    }
    // compare shapes, not positions
    let (ca, cb) = (centroid(a).to_vec2(), centroid(&b).to_vec2());
    let n = a.len();
    let best = (0..n)
        .min_by(|&k1, &k2| {
            let cost = |k: usize| (0..n).map(|i| ((a[i] - ca) - (b[(i + k) % n] - cb)).hypot2()).sum::<f64>();
            cost(k1).total_cmp(&cost(k2))
        })
        .unwrap_or(0);
    b.rotate_left(best);
    b
}

/// The outlines of a shape, largest first.
fn outlines(s: &Shape) -> Vec<Polyline> {
    let mut pls: Vec<Polyline> = geom::flatten_all(&s.world_paths(), 0.02).into_iter().filter(|p| p.pts.len() > 1).collect();
    pls.sort_by(|a, b| b.signed_area().abs().total_cmp(&a.signed_area().abs()).then(b.length().total_cmp(&a.length())));
    pls
}

/// Pairs the outlines of two shapes, resampled and lined up: all of them
/// when both have the same number, otherwise just the largest of each.
fn pairs(a: &Shape, b: &Shape) -> Vec<(Vec<Point>, Vec<Point>, bool)> {
    let (oa, ob) = (outlines(a), outlines(b));
    let take = if oa.len() == ob.len() { oa.len() } else { 1.min(oa.len()).min(ob.len()) };
    let mut out = vec![];
    for (pa, pb) in oa.iter().zip(ob.iter()).take(take) {
        let closed = pa.closed && pb.closed;
        let open = |p: &Polyline| Polyline { pts: p.pts.clone(), closed: false };
        let (pa, pb) = if closed { (pa.clone(), pb.clone()) } else { (open(pa), open(pb)) };
        let ra = resample(&pa, SAMPLES);
        let mut rb = resample(&pb, SAMPLES);
        if closed {
            rb = align_loop(&ra, rb);
        } else if ra[0].distance(rb[0]) + ra[SAMPLES - 1].distance(rb[SAMPLES - 1]) > ra[0].distance(rb[SAMPLES - 1]) + ra[SAMPLES - 1].distance(rb[0]) {
            rb.reverse();
        }
        out.push((ra, rb, closed));
    }
    out
}

/// The outline `t` of the way (0 to 1) from `a` to `b`, as smooth curves.
fn step_path(pairs: &[(Vec<Point>, Vec<Point>, bool)], t: f64) -> BezPath {
    let mut out = BezPath::new();
    for (a, b, closed) in pairs {
        let pts: Vec<Point> = a.iter().zip(b).map(|(p, q)| lerp_pt(*p, *q, t)).collect();
        let fitted = if *closed { crate::curvefit::fit_closed(&pts, 0.03, 40.0, 3) } else { crate::curvefit::fit_open(&pts, 0.03) };
        out.extend(fitted.elements().iter().copied());
    }
    out
}

fn step_style(a: &Style, b: &Style, t: f64) -> Style {
    Style {
        fill: lerp_colour(a.fill, b.fill, t),
        stroke: lerp_colour(a.stroke, b.stroke, t),
        stroke_width: a.stroke_width + (b.stroke_width - a.stroke_width) * t,
        ..a.clone()
    }
}

/// Blends the two selected shapes in `steps` steps: the steps are added as
/// one group, between the two shapes. Returns the group (or the selection
/// unchanged if it isn't two shapes).
pub fn blend(doc: &mut Document, sel: &[ShapeId], steps: usize) -> Vec<ShapeId> {
    let ids = crate::ops::z_ordered(doc, sel);
    let [ia, ib] = ids[..] else { return sel.to_vec() };
    let page = doc.page();
    let (Some(a), Some(b)) = (page.shape(ia).cloned(), page.shape(ib).cloned()) else { return sel.to_vec() };
    if matches!(a.kind, ShapeKind::Image(_)) || matches!(b.kind, ShapeKind::Image(_)) {
        return sel.to_vec();
    }
    let pairs = pairs(&a, &b);
    if pairs.is_empty() || steps == 0 {
        return sel.to_vec();
    }
    let mut children = vec![];
    for i in 1..=steps {
        let t = i as f64 / (steps + 1) as f64;
        let id = doc.alloc_id();
        let mut s = Shape::new_path(id, step_path(&pairs, t), step_style(&a.style, &b.style, t));
        s.layer = a.layer;
        children.push(s);
    }
    let gid = doc.alloc_id();
    let mut group = Shape::new_path(gid, BezPath::new(), Style::default());
    group.kind = ShapeKind::Group(children);
    group.layer = a.layer;
    group.name = format!("Blend ({steps} steps)");
    // between the two shapes
    let page = doc.page_mut();
    let at = page.shapes.iter().position(|s| s.id == ib).unwrap_or(page.shapes.len());
    page.shapes.insert(at, group);
    vec![gid]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Rect, Shape as _};

    #[test]
    fn a_circle_to_a_square_in_steps() {
        let mut doc = Document::default();
        let red = Style { fill: Some(Rgba(255, 0, 0, 255)), ..Default::default() };
        let blue = Style { fill: Some(Rgba(0, 0, 255, 255)), ..Default::default() };
        let a = doc.add_path(geom::circle_path(Point::new(20.0, 20.0), 10.0), red, 0);
        let b = doc.add_path(geom::rect_path(Rect::new(80.0, 10.0, 100.0, 30.0)), blue, 0);
        let made = blend(&mut doc, &[a, b], 3);
        assert_eq!(made.len(), 1);
        let page = doc.page();
        // the group sits between the two
        assert_eq!(page.shapes.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a, made[0], b]);
        let ShapeKind::Group(steps) = &page.shape(made[0]).unwrap().kind else { panic!("a group") };
        assert_eq!(steps.len(), 3);
        // the middle step: halfway along, halfway in size, purple
        let mid = &steps[1];
        let bb = mid.world_paths()[0].bounding_box();
        assert!((bb.center().x - 55.0).abs() < 0.5 && (bb.center().y - 20.0).abs() < 0.5, "{bb:?}");
        assert!((bb.width() - 20.0).abs() < 0.5, "{}", bb.width());
        assert_eq!(mid.style.fill, Some(Rgba(128, 0, 128, 255)));
        // between a circle's area and the square's
        let area = mid.world_paths()[0].area().abs();
        assert!(area > std::f64::consts::PI * 100.0 && area < 400.0, "{area}");
        // smooth curves, not 160 little lines
        assert!(mid.world_paths()[0].elements().len() < 60);
    }

    #[test]
    fn needs_two_shapes() {
        let mut doc = Document::default();
        let a = doc.add_path(geom::circle_path(Point::new(20.0, 20.0), 10.0), Style::default(), 0);
        assert_eq!(blend(&mut doc, &[a], 3), vec![a]);
        assert_eq!(doc.page().shapes.len(), 1);
    }
}
