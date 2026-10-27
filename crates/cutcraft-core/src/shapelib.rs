//! A library of ready-made shapes and borders, all built from geometry (no
//! artwork files), so every one is a clean cut line at any size.
//!
//! Shapes are drawn in a box about 100 units across and scaled when placed.
//! Edge styles (scalloped, wavy, zigzag, pinked, stamp) can be put round any
//! outline, so squares, rectangles, circles, ellipses and hearts all come in
//! those styles. Borders are long strips built for a given length and height.

use crate::curvefit;
use crate::geom::{self, BoolOp};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, Vec2};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::OnceLock;

/// One shape in the library.
pub struct LibShape {
    pub category: &'static str,
    pub name: &'static str,
    /// The outline, about 100 units across (any position).
    pub unit: BezPath,
}

impl LibShape {
    /// The shape with its longer side `size` mm, centred on the origin.
    pub fn path(&self, size: f64) -> BezPath {
        let b = self.unit.bounding_box();
        let s = size / b.width().max(b.height()).max(1e-9);
        Affine::scale(s) * Affine::translate(-b.center().to_vec2()) * self.unit.clone()
    }
}

/// The categories, in the order they're shown.
pub const CATEGORIES: [&str; 10] = ["Square", "Rectangle", "Polygon", "Triangle", "Circle", "Ellipse", "Heart", "Star", "Arrow", "Others"];

/// Every shape (built once).
pub fn shapes() -> &'static [LibShape] {
    static ALL: OnceLock<Vec<LibShape>> = OnceLock::new();
    ALL.get_or_init(build_shapes)
}

// ------------------------------------------------------------------ building blocks

fn poly(pts: &[(f64, f64)]) -> BezPath {
    let p: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    geom::polyline_path(&p, true)
}

/// A polygon with every corner rounded (radius about `r`, less where the
/// sides are short).
fn round_poly(pts: &[(f64, f64)], r: f64) -> BezPath {
    let p: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let n = p.len();
    let mut b = BezPath::new();
    for i in 0..n {
        let (prev, cur, next) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
        let (a, c) = (prev - cur, next - cur);
        let d = r.min(a.hypot() / 2.0).min(c.hypot() / 2.0);
        let start = cur + a.normalize() * d;
        let end = cur + c.normalize() * d;
        if i == 0 {
            b.move_to(start);
        } else {
            b.line_to(start);
        }
        // a cubic close to a circular arc for a right angle, fine for the rest
        let k = 0.55;
        b.curve_to(start + (cur - start) * k, end + (cur - end) * k, end);
    }
    b.close_path();
    b
}

fn rect_pts(w: f64, h: f64) -> Vec<(f64, f64)> {
    vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
}

fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    geom::circle_path(Point::new(cx, cy), r)
}

fn ellipse(w: f64, h: f64) -> BezPath {
    geom::ellipse_path(Rect::new(0.0, 0.0, w, h))
}

fn union(parts: Vec<BezPath>) -> BezPath {
    geom::union_each(&parts)
}

fn minus(a: BezPath, b: BezPath) -> BezPath {
    geom::boolean(&[a], &[b], BoolOp::Difference)
}

fn svg(d: &str) -> BezPath {
    BezPath::from_svg(d).expect("built-in shape data parses")
}

/// A regular polygon or star, as points, `r` round the centre (first point at the top).
fn ring_pts(cx: f64, cy: f64, radii: &[f64], count: usize, turn: f64) -> Vec<(f64, f64)> {
    let k = radii.len();
    (0..count * k)
        .map(|i| {
            let a = -FRAC_PI_2 + turn + i as f64 * TAU / (count * k) as f64;
            let r = radii[i % k];
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// A wedge of a circle (centre `cx, cy`, radius `r`) from angle `a0` to `a1`
/// (radians, clockwise from the right, y down).
fn sector(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> BezPath {
    let n = (((a1 - a0).abs() / TAU) * 96.0).ceil().max(6.0) as usize;
    let mut pts = vec![(cx, cy)];
    for i in 0..=n {
        let a = a0 + (a1 - a0) * i as f64 / n as f64;
        pts.push((cx + r * a.cos(), cy + r * a.sin()));
    }
    smooth_loop(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>())
}

/// Dense points round a loop → smooth curves, keeping its corners sharp.
fn smooth_loop(pts: &[Point]) -> BezPath {
    let b = geom::bbox_points(pts);
    let step = (b.width().max(b.height()) / 300.0).max(0.05);
    let even = curvefit::resample_closed(pts, step);
    curvefit::fit_closed(&even, step * 0.15, 40.0, 3)
}

// ------------------------------------------------------------------ edge styles

/// A decorative edge, as an offset (outward +) at `t` (0..1) across one tooth.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    /// Rounded bumps with sharp dips between.
    Scallop,
    /// A smooth wave.
    Wave,
    /// Sharp teeth.
    Zigzag,
    /// Half-round notches with flat gaps, like a postage stamp.
    Stamp,
}

impl Edge {
    fn offset(self, t: f64) -> f64 {
        match self {
            Edge::Scallop => (PI * t).sin(),
            Edge::Wave => 0.5 + 0.5 * (TAU * t - FRAC_PI_2).sin(),
            Edge::Zigzag => 1.0 - (2.0 * t - 1.0).abs(),
            Edge::Stamp => {
                let u = (t - 0.5) / 0.32;
                if u.abs() < 1.0 { -(1.0 - u * u).sqrt() } else { 0.0 }
            }
        }
    }
    fn sharp(self) -> bool {
        self == Edge::Zigzag
    }
}

/// Outward direction at `d` along a side, for a loop with signed area `area`.
fn outward(d: Vec2, area: f64) -> Vec2 {
    let n = Vec2::new(d.y, -d.x).normalize();
    if area > 0.0 { n } else { -n }
}

fn signed_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].x * pts[(i + 1) % n].y - pts[(i + 1) % n].x * pts[i].y).sum::<f64>() / 2.0
}

/// A polygon with a decorative edge: each side gets a whole number of teeth
/// about `pitch` long, `depth` deep.
fn edged_poly(pts: &[(f64, f64)], edge: Edge, pitch: f64, depth: f64) -> BezPath {
    let p: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let area = signed_area(&p);
    let mut out = vec![];
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        let d = b - a;
        let n = outward(d, area);
        let teeth = (d.hypot() / pitch).round().max(1.0) as usize;
        let per = if edge.sharp() { 2 } else { 16 };
        for k in 0..teeth * per {
            let s = k as f64 / (teeth * per) as f64;
            let t = (s * teeth as f64).fract();
            out.push(a + d * s + n * (depth * edge.offset(t)));
        }
    }
    if edge.sharp() { geom::polyline_path(&out, true) } else { smooth_loop(&out) }
}

/// Any closed outline with a decorative edge of `teeth` teeth, `depth` deep.
fn edged_curve(base: &BezPath, edge: Edge, teeth: usize, depth: f64) -> BezPath {
    let pl = geom::flatten(base, 0.05).into_iter().next().expect("an outline");
    let len: f64 = (0..pl.pts.len()).map(|i| pl.pts[i].distance(pl.pts[(i + 1) % pl.pts.len()])).sum();
    let per = if edge.sharp() { 2 } else { 16 };
    let even = curvefit::resample_closed(&pl.pts, len / (teeth * per) as f64);
    let area = signed_area(&even);
    let n = even.len();
    let out: Vec<Point> = (0..n)
        .map(|i| {
            let d = even[(i + 1) % n] - even[(i + n - 1) % n];
            let t = (i as f64 / n as f64 * teeth as f64).fract();
            even[i] + outward(d, area) * (depth * edge.offset(t))
        })
        .collect();
    if edge.sharp() { geom::polyline_path(&out, true) } else { smooth_loop(&out) }
}

/// A burst: sharp points round an ellipse `w × h`.
fn burst(w: f64, h: f64, points: usize, inner: f64) -> BezPath {
    let pts: Vec<Point> = (0..points * 2)
        .map(|i| {
            let a = -FRAC_PI_2 + i as f64 * PI / points as f64;
            let k = if i % 2 == 0 { 1.0 } else { inner };
            Point::new(w / 2.0 + w / 2.0 * k * a.cos(), h / 2.0 + h / 2.0 * k * a.sin())
        })
        .collect();
    geom::polyline_path(&pts, true)
}

fn heart() -> BezPath {
    svg("M50,96 C34,84 2,62 2,32 C2,14 15,3 29,3 C39,3 46,9 50,18 C54,9 61,3 71,3 C85,3 98,14 98,32 C98,62 66,84 50,96 Z")
}

// ------------------------------------------------------------------ the shapes

fn build_shapes() -> Vec<LibShape> {
    let mut v: Vec<LibShape> = vec![];
    let mut add = |category: &'static str, name: &'static str, unit: BezPath| v.push(LibShape { category, name, unit });

    // squares and rectangles: plain, rounded, cushioned, and every edge style
    for (cat, w, h) in [("Square", 100.0, 100.0), ("Rectangle", 100.0, 62.0)] {
        let r = rect_pts(w, h);
        add(cat, "Plain", poly(&r));
        add(cat, "Rounded", round_poly(&r, 7.0));
        add(cat, "Round", round_poly(&r, 16.0));
        add(cat, "Very round", round_poly(&r, 28.0));
        add(cat, "Cushion", svg(&format!("M0,0 Q{0},-8 {w},0 Q{1},{2} {w},{h} Q{0},{3} 0,{h} Q-8,{2} 0,0 Z", w / 2.0, w + 8.0, h / 2.0, h + 8.0)));
        add(cat, "Scalloped", edged_poly(&r, Edge::Scallop, 16.0, 4.0));
        add(cat, "Fine scallop", edged_poly(&r, Edge::Scallop, 9.0, 2.5));
        add(cat, "Wavy", edged_poly(&r, Edge::Wave, 14.0, 3.5));
        add(cat, "Zigzag", edged_poly(&r, Edge::Zigzag, 10.0, 4.0));
        add(cat, "Pinked", edged_poly(&r, Edge::Zigzag, 5.0, 2.0));
        add(cat, "Stamp", edged_poly(&r, Edge::Stamp, 9.0, 2.6));
        add(cat, "Ticket", minus(poly(&r), union([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].iter().map(|&(x, y)| circle(x, y, 10.0)).collect())));
    }
    add("Rectangle", "Pill", round_poly(&rect_pts(100.0, 40.0), 20.0));
    add("Rectangle", "Slim pill", round_poly(&rect_pts(100.0, 26.0), 13.0));
    add("Rectangle", "Rounded strip", round_poly(&rect_pts(100.0, 30.0), 6.0));
    add("Rectangle", "Bookmark", poly(&[(0.0, 0.0), (36.0, 0.0), (36.0, 100.0), (18.0, 86.0), (0.0, 100.0)]));

    // polygons
    add("Polygon", "Parallelogram", poly(&[(25.0, 0.0), (100.0, 0.0), (75.0, 55.0), (0.0, 55.0)]));
    add("Polygon", "Trapezoid", poly(&[(22.0, 0.0), (78.0, 0.0), (100.0, 55.0), (0.0, 55.0)]));
    add("Polygon", "Wide trapezoid", poly(&[(30.0, 0.0), (70.0, 0.0), (100.0, 30.0), (0.0, 30.0)]));
    add("Polygon", "Rhombus", poly(&[(50.0, 0.0), (100.0, 35.0), (50.0, 70.0), (0.0, 35.0)]));
    add("Polygon", "Kite", poly(&[(30.0, 0.0), (60.0, 30.0), (30.0, 100.0), (0.0, 30.0)]));
    add("Polygon", "Pentagon", poly(&ring_pts(50.0, 52.0, &[50.0], 5, 0.0)));
    add("Polygon", "Shield", poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 55.0), (50.0, 100.0), (0.0, 55.0)]));
    add("Polygon", "Hexagon", poly(&ring_pts(50.0, 50.0, &[50.0], 6, PI / 6.0)));
    add("Polygon", "Long hexagon", poly(&[(20.0, 0.0), (80.0, 0.0), (100.0, 25.0), (80.0, 50.0), (20.0, 50.0), (0.0, 25.0)]));
    add("Polygon", "Octagon", poly(&ring_pts(50.0, 50.0, &[50.0], 8, PI / 8.0)));
    add("Polygon", "Decagon", poly(&ring_pts(50.0, 50.0, &[50.0], 10, 0.0)));

    // triangles
    let tri = [(50.0, 0.0), (100.0, 86.6), (0.0, 86.6)];
    add("Triangle", "Triangle", poly(&tri));
    add("Triangle", "Rounded", round_poly(&tri, 8.0));
    add("Triangle", "Very rounded", round_poly(&tri, 18.0));
    add("Triangle", "Curved sides", svg("M50,0 A100,100 0 0 1 100,86.6 A100,100 0 0 1 0,86.6 A100,100 0 0 1 50,0 Z"));
    add("Triangle", "Right angle", poly(&[(100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]));
    add("Triangle", "Tall", poly(&[(30.0, 0.0), (60.0, 100.0), (0.0, 100.0)]));
    add("Triangle", "Wide", poly(&[(100.0, 0.0), (100.0, 40.0), (0.0, 40.0)]));

    // circles
    add("Circle", "Circle", circle(50.0, 50.0, 50.0));
    add("Circle", "Burst", burst(100.0, 100.0, 12, 0.78));
    add("Circle", "Fine burst", burst(100.0, 100.0, 24, 0.85));
    add("Circle", "Scalloped", edged_curve(&circle(50.0, 50.0, 46.0), Edge::Scallop, 10, 5.0));
    add("Circle", "Many scallops", edged_curve(&circle(50.0, 50.0, 47.0), Edge::Scallop, 18, 3.5));
    add("Circle", "Wavy", edged_curve(&circle(50.0, 50.0, 46.0), Edge::Wave, 12, 4.0));
    add("Circle", "Stamp", edged_curve(&circle(50.0, 50.0, 50.0), Edge::Stamp, 22, 3.0));
    add("Circle", "Bite", sector(50.0, 50.0, 50.0, -FRAC_PI_2 + 0.5, -FRAC_PI_2 - 0.5 + TAU));
    add("Circle", "Wide bite", sector(50.0, 50.0, 50.0, -FRAC_PI_2 + 0.9, -FRAC_PI_2 - 0.9 + TAU));
    add("Circle", "Half", sector(50.0, 50.0, 50.0, PI, TAU));
    add("Circle", "Quarter", sector(0.0, 100.0, 100.0, -FRAC_PI_2, 0.0));
    add("Circle", "Slice", sector(50.0, 100.0, 100.0, -FRAC_PI_2 - PI / 6.0, -FRAC_PI_2 + PI / 6.0));
    add("Circle", "Segment", minus(circle(50.0, 50.0, 50.0), poly(&rect_pts(100.0, 100.0).iter().map(|&(x, y)| (x * 1.3 - 15.0, y + 70.0)).collect::<Vec<_>>())));

    // ellipses
    add("Ellipse", "Ellipse", ellipse(100.0, 70.0));
    add("Ellipse", "Burst", burst(100.0, 70.0, 14, 0.8));
    add("Ellipse", "Scalloped", edged_curve(&ellipse(100.0, 70.0), Edge::Scallop, 12, 4.0));
    add("Ellipse", "Wavy", edged_curve(&ellipse(100.0, 70.0), Edge::Wave, 14, 3.0));
    add("Ellipse", "Flat", ellipse(100.0, 45.0));

    // hearts
    add("Heart", "Heart", heart());
    add("Heart", "Scalloped", edged_curve(&heart(), Edge::Scallop, 14, 3.5));
    add("Heart", "Wavy", edged_curve(&heart(), Edge::Wave, 16, 2.5));
    add("Heart", "Zigzag", edged_curve(&heart(), Edge::Zigzag, 24, 3.0));
    add("Heart", "Plump", svg("M50,94 C28,80 0,62 0,36 C0,14 14,2 30,2 C40,2 47,8 50,16 C53,8 60,2 70,2 C86,2 100,14 100,36 C100,62 72,80 50,94 Z"));
    add("Heart", "Outline", minus(heart(), Affine::translate((50.0, 50.0)) * Affine::scale(0.7) * Affine::translate((-50.0, -48.0)) * heart()));

    // stars
    add("Star", "Star", poly(&ring_pts(50.0, 52.0, &[50.0, 20.0], 5, 0.0)));
    add("Star", "Fat star", poly(&ring_pts(50.0, 52.0, &[50.0, 27.0], 5, 0.0)));
    add("Star", "Thin star", poly(&ring_pts(50.0, 52.0, &[50.0, 14.0], 5, 0.0)));
    add("Star", "Rounded star", round_poly(&ring_pts(50.0, 52.0, &[50.0, 24.0], 5, 0.0), 4.0));
    add("Star", "Four points", poly(&ring_pts(50.0, 50.0, &[50.0, 15.0], 4, 0.0)));
    add("Star", "Six points", poly(&ring_pts(50.0, 50.0, &[50.0, 28.0], 6, 0.0)));
    add("Star", "Eight points", poly(&ring_pts(50.0, 50.0, &[50.0, 20.0], 8, 0.0)));
    add("Star", "Sparkle", svg("M50,0 C53,35 65,47 100,50 C65,53 53,65 50,100 C47,65 35,53 0,50 C35,47 47,35 50,0 Z"));

    // arrows
    add("Arrow", "Arrow", poly(&[(0.0, 30.0), (60.0, 30.0), (60.0, 5.0), (100.0, 50.0), (60.0, 95.0), (60.0, 70.0), (0.0, 70.0)]));
    add("Arrow", "Notched", poly(&[(0.0, 30.0), (60.0, 30.0), (60.0, 5.0), (100.0, 50.0), (60.0, 95.0), (60.0, 70.0), (0.0, 70.0), (14.0, 50.0)]));
    add("Arrow", "Slim", poly(&[(0.0, 42.0), (70.0, 42.0), (70.0, 25.0), (100.0, 50.0), (70.0, 75.0), (70.0, 58.0), (0.0, 58.0)]));
    add("Arrow", "Double", poly(&[(0.0, 50.0), (28.0, 18.0), (28.0, 36.0), (72.0, 36.0), (72.0, 18.0), (100.0, 50.0), (72.0, 82.0), (72.0, 64.0), (28.0, 64.0), (28.0, 82.0)]));
    add("Arrow", "Turn", svg("M0,100 L0,55 A40,40 0 0 1 40,15 L62,15 L62,0 L100,30 L62,60 L62,45 L40,45 A12,12 0 0 0 28,57 L28,100 Z"));
    add("Arrow", "Play", poly(&[(0.0, 0.0), (100.0, 50.0), (0.0, 100.0)]));
    add("Arrow", "Chevron", poly(&[(0.0, 0.0), (55.0, 0.0), (100.0, 50.0), (55.0, 100.0), (0.0, 100.0), (45.0, 50.0)]));
    add("Arrow", "Tag", poly(&[(0.0, 0.0), (70.0, 0.0), (100.0, 50.0), (70.0, 100.0), (0.0, 100.0)]));

    // others
    let spade_top = Affine::translate((0.0, 100.0)) * Affine::scale_non_uniform(1.0, -1.0) * heart();
    add("Others", "Spade", union(vec![Affine::translate((0.0, -8.0)) * Affine::scale_non_uniform(1.0, 0.85) * spade_top, svg("M50,60 C52,80 58,92 66,100 L34,100 C42,92 48,80 50,60 Z")]));
    add("Others", "Club", union(vec![circle(50.0, 26.0, 24.0), circle(25.0, 58.0, 24.0), circle(75.0, 58.0, 24.0), svg("M50,45 C52,75 58,90 66,100 L34,100 C42,90 48,75 50,45 Z")]));
    add("Others", "Diamond", svg("M50,0 C60,20 75,38 90,50 C75,62 60,80 50,100 C40,80 25,62 10,50 C25,38 40,20 50,0 Z"));
    add("Others", "Crescent", minus(circle(50.0, 50.0, 50.0), circle(72.0, 42.0, 42.0)));
    add("Others", "Arch", minus(sector(50.0, 50.0, 50.0, PI, TAU), circle(50.0, 50.0, 26.0)));
    add("Others", "Fan", minus(sector(50.0, 100.0, 100.0, -FRAC_PI_2 - 0.6, -FRAC_PI_2 + 0.6), circle(50.0, 100.0, 45.0)));
    add("Others", "Ring", minus(circle(50.0, 50.0, 50.0), circle(50.0, 50.0, 30.0)));
    add("Others", "Open ring", minus(minus(circle(50.0, 50.0, 50.0), circle(50.0, 50.0, 22.0)), poly(&[(50.0, 50.0), (30.0, -5.0), (70.0, -5.0)])));
    add("Others", "Drop", svg("M50,0 C62,22 85,42 85,64 A35,35 0 0 1 15,64 C15,42 38,22 50,0 Z"));
    add("Others", "Leaf", svg("M50,0 C78,25 78,75 50,100 C22,75 22,25 50,0 Z"));
    add("Others", "Egg", svg("M50,0 C78,0 92,42 92,62 C92,86 73,100 50,100 C27,100 8,86 8,62 C8,42 22,0 50,0 Z"));
    add("Others", "Bow tie", poly(&[(0.0, 0.0), (50.0, 30.0), (100.0, 0.0), (100.0, 70.0), (50.0, 40.0), (0.0, 70.0)]));
    add("Others", "Cross", poly(&[(35.0, 0.0), (65.0, 0.0), (65.0, 35.0), (100.0, 35.0), (100.0, 65.0), (65.0, 65.0), (65.0, 100.0), (35.0, 100.0), (35.0, 65.0), (0.0, 65.0), (0.0, 35.0), (35.0, 35.0)]));
    add("Others", "Round cross", union(vec![round_poly(&[(32.0, 0.0), (68.0, 0.0), (68.0, 100.0), (32.0, 100.0)], 18.0), round_poly(&[(0.0, 32.0), (100.0, 32.0), (100.0, 68.0), (0.0, 68.0)], 18.0)]));
    add("Others", "Trefoil", union(vec![circle(50.0, 30.0, 26.0), circle(28.0, 68.0, 26.0), circle(72.0, 68.0, 26.0), circle(50.0, 55.0, 18.0)]));
    add("Others", "Quatrefoil", union(vec![circle(50.0, 25.0, 25.0), circle(75.0, 50.0, 25.0), circle(50.0, 75.0, 25.0), circle(25.0, 50.0, 25.0), poly(&rect_pts(50.0, 50.0).iter().map(|&(x, y)| (x + 25.0, y + 25.0)).collect::<Vec<_>>())]));
    add("Others", "Flower", union((0..6).map(|i| {
        let a = i as f64 * TAU / 6.0;
        circle(50.0 + 30.0 * a.cos(), 50.0 + 30.0 * a.sin(), 21.0)
    }).chain([circle(50.0, 50.0, 30.0)]).collect()));
    add("Others", "Daisy", union((0..10).map(|i| {
        Affine::rotate_about(i as f64 * TAU / 10.0, Point::new(50.0, 50.0)) * svg("M50,50 C42,35 42,10 50,0 C58,10 58,35 50,50 Z")
    }).chain([circle(50.0, 50.0, 14.0)]).collect()));
    add("Others", "L shape", poly(&[(0.0, 0.0), (40.0, 0.0), (40.0, 60.0), (100.0, 60.0), (100.0, 100.0), (0.0, 100.0)]));
    add("Others", "Pennant", poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (50.0, 70.0), (0.0, 100.0)]));
    add("Others", "Ribbon", poly(&[(0.0, 0.0), (100.0, 0.0), (85.0, 25.0), (100.0, 50.0), (0.0, 50.0), (15.0, 25.0)]));
    add("Others", "Lightning", poly(&[(55.0, 0.0), (15.0, 55.0), (45.0, 55.0), (35.0, 100.0), (85.0, 38.0), (55.0, 38.0), (70.0, 0.0)]));
    add("Others", "Keyhole", union(vec![circle(50.0, 32.0, 28.0), poly(&[(38.0, 40.0), (62.0, 40.0), (75.0, 100.0), (25.0, 100.0)])]));
    add("Others", "Cloud", svg("M25,70 C10,70 0,60 0,48 C0,36 10,27 22,28 C24,14 36,4 50,4 C63,4 73,12 76,24 C90,24 100,34 100,47 C100,60 90,70 77,70 Z"));
    add("Others", "Speech bubble", svg("M15,0 L85,0 C93,0 100,7 100,15 L100,55 C100,63 93,70 85,70 L45,70 L22,92 L27,70 L15,70 C7,70 0,63 0,55 L0,15 C0,7 7,0 15,0 Z"));
    add("Others", "House", poly(&[(50.0, 0.0), (100.0, 45.0), (88.0, 45.0), (88.0, 100.0), (12.0, 100.0), (12.0, 45.0), (0.0, 45.0)]));
    add("Others", "Gift tag", minus(poly(&[(25.0, 0.0), (100.0, 0.0), (100.0, 50.0), (25.0, 50.0), (0.0, 25.0)]), circle(20.0, 25.0, 5.0)));
    add("Others", "Rounded corner", svg("M0,0 L60,0 A40,40 0 0 1 100,40 L100,100 L0,100 Z"));
    add("Others", "Inside corner", svg("M0,0 L60,0 A40,40 0 0 0 100,40 L100,100 L0,100 Z"));
    v
}

// ------------------------------------------------------------------ borders

/// One border style; `build(length, height)` makes it in mm, starting at
/// the origin.
pub struct Border {
    pub name: &'static str,
    pub build: fn(f64, f64) -> BezPath,
}

pub const BORDERS: [Border; 18] = [
    Border { name: "Scallops", build: |l, h| profile_strip(l, h, h * 1.4, &|t| (PI * t).sin(), false) },
    Border { name: "Hanging scallops", build: |l, h| flip(profile_strip(l, h, h * 1.4, &|t| (PI * t).sin(), false), h) },
    Border { name: "Wave", build: |l, h| profile_strip(l, h, h * 1.8, &|t| 0.5 + 0.5 * (TAU * t - FRAC_PI_2).sin(), false) },
    Border { name: "Zigzag", build: |l, h| profile_strip(l, h, h * 1.2, &|t| 1.0 - (2.0 * t - 1.0).abs(), true) },
    Border { name: "Pinked strip", build: |l, h| pinked_strip(l, h) },
    Border { name: "Wavy ribbon", build: |l, h| wavy_ribbon(l, h) },
    Border { name: "Torn paper", build: |l, h| torn(l, h) },
    Border { name: "Grass", build: |l, h| grass(l, h) },
    Border { name: "Ocean waves", build: |l, h| profile_strip(l, h, h * 1.6, &curl, false) },
    Border { name: "Picket fence", build: |l, h| fence(l, h) },
    Border { name: "Castle", build: |l, h| castle(l, h) },
    Border { name: "Bead chain", build: |l, h| chain(l, h, &|cx, cy, s| geom::ellipse_path(Rect::from_center_size(Point::new(cx, cy), (s * 1.3, s)))) },
    Border { name: "Heart chain", build: |l, h| chain(l, h, &|cx, cy, s| fit_in(heart(), cx, cy, s * 1.15)) },
    Border { name: "Star chain", build: |l, h| chain(l, h, &|cx, cy, s| fit_in(poly(&ring_pts(50.0, 52.0, &[50.0, 22.0], 5, 0.0)), cx, cy, s * 1.2)) },
    Border { name: "Diamond lattice", build: |l, h| diamonds(l, h) },
    Border { name: "Bunting", build: |l, h| bunting(l, h) },
    Border { name: "Lace scallops", build: |l, h| lace(l, h) },
    Border { name: "Flower row", build: |l, h| flower_row(l, h) },
];

/// A border made at a sample size, for its preview.
pub fn border_preview(b: &Border) -> &'static BezPath {
    static CACHE: OnceLock<Vec<BezPath>> = OnceLock::new();
    let all = CACHE.get_or_init(|| BORDERS.iter().map(|b| (b.build)(150.0, 18.0)).collect());
    let i = BORDERS.iter().position(|x| x.name == b.name).unwrap_or(0);
    &all[i]
}

/// How many repeats of about `pitch` fit along `l`.
fn repeats(l: f64, pitch: f64) -> usize {
    (l / pitch.max(1e-6)).round().max(1.0) as usize
}

/// A strip `l` long: flat along the bottom, its top edge rising and falling
/// by `f(t)` (0..1 across each repeat) - the band itself is a third of `h`,
/// the pattern the rest.
fn profile_strip(l: f64, h: f64, pitch: f64, f: &dyn Fn(f64) -> f64, sharp: bool) -> BezPath {
    let n = repeats(l, pitch);
    let base = h * 0.35;
    let per = if sharp { 2 } else { 24 };
    let mut pts = vec![Point::new(0.0, h), Point::new(l, h)];
    for k in (0..=n * per).rev() {
        let s = k as f64 / (n * per) as f64;
        let t = (s * n as f64).fract();
        let t = if k == n * per { 1.0 } else { t };
        let y = h - base - (h - base) * f(t);
        pts.push(Point::new(l * s, y));
    }
    if sharp { geom::polyline_path(&pts, true) } else { smooth_loop(&pts) }
}

/// Upside down within its height.
fn flip(p: BezPath, h: f64) -> BezPath {
    Affine::translate((0.0, h)) * Affine::scale_non_uniform(1.0, -1.0) * p
}

/// A breaking wave: a steep front and a long back.
fn curl(t: f64) -> f64 {
    let rise = 0.75;
    if t < rise { (t / rise * FRAC_PI_2).sin().powi(2) } else { ((1.0 - t) / (1.0 - rise) * FRAC_PI_2).sin().powi(2) * 0.6 + 0.4 * (1.0 - (t - rise) / (1.0 - rise)) }
}

/// Fine teeth along both edges.
fn pinked_strip(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 0.4);
    let d = h * 0.15;
    let mut pts = vec![];
    for k in 0..=n * 2 {
        pts.push(Point::new(l * k as f64 / (n * 2) as f64, if k % 2 == 0 { d } else { 0.0 }));
    }
    for k in (0..=n * 2).rev() {
        pts.push(Point::new(l * k as f64 / (n * 2) as f64, if k % 2 == 0 { h - d } else { h }));
    }
    geom::polyline_path(&pts, true)
}

/// A ribbon that waves along its length, both edges together.
fn wavy_ribbon(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 3.0);
    let amp = h * 0.22;
    let band = h - 2.0 * amp;
    let steps = n * 24;
    let y = |s: f64| amp + amp * (TAU * s * n as f64).sin();
    let mut pts: Vec<Point> = (0..=steps).map(|k| {
        let s = k as f64 / steps as f64;
        Point::new(l * s, y(s))
    }).collect();
    pts.extend((0..=steps).rev().map(|k| {
        let s = k as f64 / steps as f64;
        Point::new(l * s, y(s) + band)
    }));
    smooth_loop(&pts)
}

/// A small, repeatable pseudo-random number in 0..1.
fn noise(i: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x % 10_000) as f64 / 10_000.0
}

/// A paper strip with a torn top edge.
fn torn(l: f64, h: f64) -> BezPath {
    let steps = (l / (h * 0.12)).ceil().max(8.0) as usize;
    let mut pts = vec![Point::new(0.0, h), Point::new(l, h)];
    let mut y = h * 0.3;
    for k in (0..=steps).rev() {
        y = (y + (noise(k as u64) - 0.5) * h * 0.25).clamp(h * 0.1, h * 0.5);
        pts.push(Point::new(l * k as f64 / steps as f64, y));
    }
    geom::polyline_path(&pts, true)
}

/// Blades of grass of varying height on a strip.
fn grass(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 0.35);
    let w = l / n as f64;
    let mut pts = vec![Point::new(0.0, h), Point::new(l, h)];
    for k in (0..n).rev() {
        let x = k as f64 * w;
        let tip = h * (0.05 + 0.35 * noise(k as u64 + 7));
        let lean = (noise(k as u64 + 99) - 0.5) * w;
        pts.push(Point::new(x + w, h * 0.7));
        pts.push(Point::new(x + w / 2.0 + lean, tip));
        pts.push(Point::new(x, h * 0.7));
    }
    geom::polyline_path(&pts, true)
}

/// Pointed pickets joined by a rail.
fn fence(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 0.5);
    let w = l / n as f64;
    let mut parts = vec![geom::rect_path(Rect::new(0.0, h * 0.55, l, h * 0.75))];
    for k in 0..n {
        let x = k as f64 * w + w * 0.15;
        let pw = w * 0.7;
        parts.push(poly(&[(x, h * 0.25), (x + pw / 2.0, 0.0), (x + pw, h * 0.25), (x + pw, h), (x, h)]));
    }
    union(parts)
}

/// Battlements along the top.
fn castle(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 0.8);
    let w = l / n as f64;
    let mut pts = vec![Point::new(0.0, h), Point::new(l, h)];
    for k in (0..n).rev() {
        let x = k as f64 * w;
        pts.extend([Point::new(x + w, h * 0.4), Point::new(x + w * 0.75, h * 0.4), Point::new(x + w * 0.75, 0.0), Point::new(x + w * 0.25, 0.0), Point::new(x + w * 0.25, h * 0.4), Point::new(x, h * 0.4)]);
    }
    geom::polyline_path(&pts, true)
}

/// Shapes in a row, overlapping a thin bar so it all cuts as one piece.
fn chain(l: f64, h: f64, shape: &dyn Fn(f64, f64, f64) -> BezPath) -> BezPath {
    let n = repeats(l, h * 1.1);
    let w = l / n as f64;
    let mut parts = vec![geom::rect_path(Rect::new(w * 0.3, h * 0.42, l - w * 0.3, h * 0.58))];
    for k in 0..n {
        parts.push(shape(w * (k as f64 + 0.5), h / 2.0, h.min(w * 0.72)));
    }
    union(parts)
}

/// `p` scaled to fit `s` across, centred on `cx, cy`.
fn fit_in(p: BezPath, cx: f64, cy: f64, s: f64) -> BezPath {
    let b = p.bounding_box();
    let k = s / b.width().max(b.height());
    Affine::translate((cx, cy)) * Affine::scale(k) * Affine::translate(-b.center().to_vec2()) * p
}

/// A band of diamonds, each with a diamond hole.
fn diamonds(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h);
    let w = l / n as f64;
    let mut solid = vec![];
    let mut holes = vec![];
    for k in 0..n {
        let cx = w * (k as f64 + 0.5);
        let d = |s: f64| poly(&[(cx, h / 2.0 - h / 2.0 * s), (cx + w * 0.55 * s, h / 2.0), (cx, h / 2.0 + h / 2.0 * s), (cx - w * 0.55 * s, h / 2.0)]);
        solid.push(d(1.0));
        holes.push(d(0.45));
    }
    minus(union(solid), union(holes))
}

/// Triangle flags hanging from a line.
fn bunting(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 0.9);
    let w = l / n as f64;
    let line = h * 0.08;
    let mut parts = vec![geom::rect_path(Rect::new(0.0, 0.0, l, line))];
    for k in 0..n {
        let x = k as f64 * w;
        parts.push(poly(&[(x + w * 0.08, line * 0.5), (x + w * 0.92, line * 0.5), (x + w / 2.0, h)]));
    }
    union(parts)
}

/// Hanging scallops, each with a round hole, under a solid band.
fn lace(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 1.2);
    let w = l / n as f64;
    let band = flip(profile_strip(l, h, w, &|t| (PI * t).sin(), false), h);
    let holes: Vec<BezPath> = (0..n).map(|k| circle(w * (k as f64 + 0.5), h * 0.55, h.min(w) * 0.16)).collect();
    minus(band, union(holes))
}

/// Little flowers sitting on a strip.
fn flower_row(l: f64, h: f64) -> BezPath {
    let n = repeats(l, h * 1.1);
    let w = l / n as f64;
    let mut parts = vec![geom::rect_path(Rect::new(0.0, h * 0.8, l, h))];
    for k in 0..n {
        let (cx, cy, r) = (w * (k as f64 + 0.5), h * 0.42, h.min(w) * 0.17);
        for i in 0..5 {
            let a = -FRAC_PI_2 + i as f64 * TAU / 5.0;
            parts.push(circle(cx + r * 1.3 * a.cos(), cy + r * 1.3 * a.sin(), r));
        }
        parts.push(circle(cx, cy, r * 1.2));
        parts.push(geom::rect_path(Rect::new(cx - r * 0.25, cy, cx + r * 0.25, h * 0.85)));
    }
    union(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws every shape and border, filled, to `SHAPES_OUT` (a PNG) to look over.
    #[test]
    #[ignore = "diagnostic"]
    fn contact_sheet() {
        let Ok(out) = std::env::var("SHAPES_OUT") else { return };
        let cell = 70u32;
        let cols = 14u32;
        let all = shapes();
        let rows = (all.len() as u32).div_ceil(cols);
        let brows = BORDERS.len() as u32 / 2 + 1;
        let (w, h) = (cols * cell, rows * cell + brows * 50);
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        let mut paint = |p: &BezPath, ox: f64, oy: f64, bw: f64, bh: f64, col: [u8; 4]| {
            let b = p.bounding_box();
            let k = (bw / b.width()).min(bh / b.height());
            let pl = geom::flatten(p, 0.1);
            for y in 0..bh as u32 {
                for x in 0..bw as u32 {
                    let q = Point::new(b.x0 + (x as f64 + 0.5) / k, b.y0 + (y as f64 + 0.5) / k);
                    let mut inside = false;
                    for l in &pl {
                        let n = l.pts.len();
                        for i in 0..n {
                            let (a, c) = (l.pts[i], l.pts[(i + 1) % n]);
                            if (a.y > q.y) != (c.y > q.y) && q.x < a.x + (q.y - a.y) / (c.y - a.y) * (c.x - a.x) {
                                inside = !inside;
                            }
                        }
                    }
                    if inside {
                        img.put_pixel(ox as u32 + x, oy as u32 + y, image::Rgba(col));
                    }
                }
            }
        };
        for (i, s) in all.iter().enumerate() {
            let (cx, cy) = ((i as u32 % cols) * cell, (i as u32 / cols) * cell);
            paint(&s.unit, cx as f64 + 6.0, cy as f64 + 6.0, 58.0, 58.0, [60, 100, 200, 255]);
        }
        for (i, b) in BORDERS.iter().enumerate() {
            let (cx, cy) = ((i as u32 % 2) * (w / 2), rows * cell + (i as u32 / 2) * 50);
            paint(border_preview(b), cx as f64 + 10.0, cy as f64 + 8.0, (w / 2 - 20) as f64, 34.0, [200, 90, 60, 255]);
        }
        img.save(out).unwrap();
    }

    #[test]
    fn every_shape_is_a_closed_sizeable_outline() {
        let all = shapes();
        assert!(all.len() > 80, "{} shapes", all.len());
        for s in all {
            assert!(CATEGORIES.contains(&s.category), "{}", s.category);
            let p = s.path(50.0);
            let b = p.bounding_box();
            assert!((b.width().max(b.height()) - 50.0).abs() < 0.5, "{} / {}: {b:?}", s.category, s.name);
            assert!(b.center().to_vec2().hypot() < 0.5, "{} centred", s.name);
            assert!(p.area().abs() > 50.0, "{} / {} has area {}", s.category, s.name, p.area());
        }
    }

    #[test]
    fn every_border_fills_its_strip() {
        for b in &BORDERS {
            let p = (b.build)(200.0, 20.0);
            let bb = p.bounding_box();
            assert!(bb.width() > 190.0 && bb.width() < 206.0, "{}: {bb:?}", b.name);
            assert!(bb.height() > 12.0 && bb.height() < 21.0, "{}: {bb:?}", b.name);
            assert!(p.area().abs() > 500.0, "{} area {}", b.name, p.area());
        }
    }
}
