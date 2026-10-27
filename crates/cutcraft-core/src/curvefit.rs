//! Fitting smooth Bézier curves to dense outline points (from tracing), with
//! sharp corners kept sharp: the points are split at corners, and each run
//! between them is fitted with as few cubic curves as stay within a
//! tolerance (Schneider's method, "An Algorithm for Automatically Fitting
//! Digitized Curves", Graphics Gems, 1990).

use kurbo::{BezPath, Point, Vec2};

/// Evenly spaced points (`step` apart) along a closed loop.
pub fn resample_closed(pts: &[Point], step: f64) -> Vec<Point> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let len: f64 = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).sum();
    let count = ((len / step).round() as usize).max(8);
    let step = len / count as f64;
    let mut out = Vec::with_capacity(count);
    let (mut i, mut along) = (0usize, 0.0);
    for k in 0..count {
        let target = k as f64 * step;
        loop {
            let seg = pts[i % n].distance(pts[(i + 1) % n]);
            if along + seg >= target || i >= n * 2 {
                let t = if seg > 0.0 { (target - along) / seg } else { 0.0 };
                out.push(pts[i % n].lerp(pts[(i + 1) % n], t.clamp(0.0, 1.0)));
                break;
            }
            along += seg;
            i += 1;
        }
    }
    out
}

/// Indices of the corners of a closed loop of evenly spaced points: where
/// the outline turns by more than `corner_deg` within `reach` points either
/// side, taking the sharpest point of each bend.
pub fn corners(pts: &[Point], reach: usize, corner_deg: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 2 * reach + 3 {
        return vec![];
    }
    let turn: Vec<f64> = (0..n)
        .map(|i| {
            let a = pts[(i + n - reach) % n];
            let b = pts[i];
            let c = pts[(i + reach) % n];
            let (u, v) = (b - a, c - b);
            if u.hypot() < 1e-9 || v.hypot() < 1e-9 {
                return 0.0;
            }
            u.cross(v).atan2(u.dot(v)).abs().to_degrees()
        })
        .collect();
    (0..n)
        .filter(|&i| turn[i] > corner_deg && (1..=reach).all(|d| turn[i] >= turn[(i + d) % n] && turn[i] > turn[(i + n - d) % n]))
        .collect()
}

/// Evens out small wobbles in a closed loop of evenly spaced points: each
/// point moves to a weighted average of its neighbours (`sigma` points
/// wide), never averaging across a corner, and corners stay put.
pub fn smooth_closed(pts: &[Point], corners: &[usize], sigma: f64) -> Vec<Point> {
    let n = pts.len();
    if n < 5 || sigma <= 0.0 {
        return pts.to_vec();
    }
    let reach = (sigma * 3.0).ceil() as usize;
    let is_corner = |i: usize| corners.contains(&(i % n));
    (0..n)
        .map(|i| {
            if is_corner(i) {
                return pts[i];
            }
            let (mut sum, mut wsum) = (pts[i].to_vec2(), 1.0);
            for dir in [1isize, -1] {
                for d in 1..=reach {
                    let j = (i as isize + dir * d as isize).rem_euclid(n as isize) as usize;
                    if is_corner(j) {
                        break;
                    }
                    let w = (-((d * d) as f64) / (2.0 * sigma * sigma)).exp();
                    sum += pts[j].to_vec2() * w;
                    wsum += w;
                }
            }
            (sum / wsum).to_point()
        })
        .collect()
}

/// A closed loop as smooth curves within `tol`, sharp at bends sharper than
/// `corner_deg`. The loop's points should be evenly spaced.
pub fn fit_closed(pts: &[Point], tol: f64, corner_deg: f64, reach: usize) -> BezPath {
    let n = pts.len();
    let mut out = BezPath::new();
    if n < 3 {
        return out;
    }
    let cs = corners(pts, reach, corner_deg);
    if cs.is_empty() {
        // one smooth run all the way round, with matching tangents where it closes
        let mut run: Vec<Point> = pts.to_vec();
        run.push(pts[0]);
        let t = (pts[1] - pts[n - 1]).normalize();
        out.move_to(pts[0]);
        fit_run(&run, t, -t, tol, &mut out);
        out.close_path();
        return out;
    }
    out.move_to(pts[cs[0]]);
    for k in 0..cs.len() {
        let (a, b) = (cs[k], cs[(k + 1) % cs.len()]);
        let len = if b > a { b - a } else { b + n - a };
        let run: Vec<Point> = (0..=len).map(|j| pts[(a + j) % n]).collect();
        let t1 = start_tangent(&run);
        let t2 = end_tangent(&run);
        fit_run(&run, t1, t2, tol, &mut out);
    }
    out.close_path();
    out
}

/// An open run of points as smooth curves within `tol` (no corners looked for).
pub fn fit_open(pts: &[Point], tol: f64) -> BezPath {
    let mut out = BezPath::new();
    if pts.len() < 2 {
        return out;
    }
    out.move_to(pts[0]);
    fit_run(pts, start_tangent(pts), end_tangent(pts), tol, &mut out);
    out
}

fn start_tangent(run: &[Point]) -> Vec2 {
    let k = (run.len() - 1).min(3);
    norm(run[k] - run[0])
}

fn end_tangent(run: &[Point]) -> Vec2 {
    let n = run.len();
    let k = (n - 1).min(3);
    norm(run[n - 1 - k] - run[n - 1])
}

fn norm(v: Vec2) -> Vec2 {
    let l = v.hypot();
    if l > 1e-12 { v / l } else { Vec2::new(1.0, 0.0) }
}

/// Appends curves through `run` (its first point is where the path already is).
/// `t1` points out of the start, `t2` points back from the end.
fn fit_run(run: &[Point], t1: Vec2, t2: Vec2, tol: f64, out: &mut BezPath) {
    fit_cubic(run, t1, t2, tol * tol, out, 0);
}

fn fit_cubic(d: &[Point], t1: Vec2, t2: Vec2, err2: f64, out: &mut BezPath, depth: u32) {
    let n = d.len();
    if n == 2 {
        let dist = d[0].distance(d[1]) / 3.0;
        out.curve_to(d[0] + t1 * dist, d[1] + t2 * dist, d[1]);
        return;
    }
    let mut u = chord_params(d);
    let mut bez = generate(d, &u, t1, t2);
    let (mut max, mut split) = max_error(d, &bez, &u);
    if max < err2 || depth > 24 {
        out.curve_to(bez[1], bez[2], bez[3]);
        return;
    }
    // close: improve the fit by reparameterising before giving up on one curve
    if max < err2 * 16.0 {
        for _ in 0..6 {
            u = reparameterise(d, &u, &bez);
            bez = generate(d, &u, t1, t2);
            (max, split) = max_error(d, &bez, &u);
            if max < err2 {
                out.curve_to(bez[1], bez[2], bez[3]);
                return;
            }
        }
    }
    let split = split.clamp(1, n - 2);
    let tc = norm(d[split - 1] - d[split + 1]);
    fit_cubic(&d[..=split], t1, tc, err2, out, depth + 1);
    fit_cubic(&d[split..], -tc, t2, err2, out, depth + 1);
}

fn chord_params(d: &[Point]) -> Vec<f64> {
    let mut u = vec![0.0; d.len()];
    for i in 1..d.len() {
        u[i] = u[i - 1] + d[i].distance(d[i - 1]);
    }
    let total = *u.last().unwrap();
    if total > 0.0 {
        u.iter_mut().for_each(|x| *x /= total);
    }
    u
}

fn bezier(b: &[Point; 4], t: f64) -> Point {
    let mt = 1.0 - t;
    let v = b[0].to_vec2() * (mt * mt * mt) + b[1].to_vec2() * (3.0 * mt * mt * t) + b[2].to_vec2() * (3.0 * mt * t * t) + b[3].to_vec2() * (t * t * t);
    v.to_point()
}

/// The least-squares curve with the given end tangents.
fn generate(d: &[Point], u: &[f64], t1: Vec2, t2: Vec2) -> [Point; 4] {
    let (first, last) = (d[0], d[d.len() - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, &t) in d.iter().zip(u) {
        let mt = 1.0 - t;
        let (b0, b1, b2, b3) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        let a1 = t1 * b1;
        let a2 = t2 * b2;
        c00 += a1.dot(a1);
        c01 += a1.dot(a2);
        c11 += a2.dot(a2);
        let tmp = p.to_vec2() - (first.to_vec2() * (b0 + b1) + last.to_vec2() * (b2 + b3));
        x0 += a1.dot(tmp);
        x1 += a2.dot(tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let (mut al, mut ar) = if det.abs() > 1e-12 { ((x0 * c11 - c01 * x1) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    let seg = first.distance(last);
    let eps = 1e-6 * seg;
    if al < eps || ar < eps {
        // the fit failed: fall back to a third of the chord
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    [first, first + t1 * al, last + t2 * ar, last]
}

fn max_error(d: &[Point], b: &[Point; 4], u: &[f64]) -> (f64, usize) {
    let (mut max, mut at) = (0.0, d.len() / 2);
    for i in 1..d.len() - 1 {
        let e = (bezier(b, u[i]) - d[i]).hypot2();
        if e >= max {
            max = e;
            at = i;
        }
    }
    (max, at)
}

/// One Newton step per point towards the parameter nearest it on the curve.
fn reparameterise(d: &[Point], u: &[f64], b: &[Point; 4]) -> Vec<f64> {
    let q1 = [(b[1] - b[0]) * 3.0, (b[2] - b[1]) * 3.0, (b[3] - b[2]) * 3.0];
    let q2 = [(q1[1] - q1[0]) * 2.0, (q1[2] - q1[1]) * 2.0];
    d.iter()
        .zip(u)
        .map(|(p, &t)| {
            let mt = 1.0 - t;
            let q = bezier(b, t);
            let d1 = q1[0] * (mt * mt) + q1[1] * (2.0 * mt * t) + q1[2] * (t * t);
            let d2 = q2[0] * mt + q2[1] * t;
            let diff = q - *p;
            let num = diff.dot(d1);
            let den = d1.dot(d1) + diff.dot(d2);
            if den.abs() < 1e-12 { t } else { (t - num / den).clamp(0.0, 1.0) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn a_circle_fits_in_a_few_smooth_curves() {
        let pts: Vec<Point> = (0..200).map(|i| {
            let a = i as f64 / 200.0 * std::f64::consts::TAU;
            Point::new(50.0 + 30.0 * a.cos(), 50.0 + 30.0 * a.sin())
        }).collect();
        let p = fit_closed(&pts, 0.2, 50.0, 4);
        let curves = p.elements().iter().filter(|e| matches!(e, kurbo::PathEl::CurveTo(..))).count();
        assert!((1..=8).contains(&curves), "{curves} curves");
        assert!((p.area().abs() - std::f64::consts::PI * 900.0).abs() < 30.0);
    }

    #[test]
    fn a_square_keeps_its_four_corners() {
        let mut pts = vec![];
        for (a, b) in [((0.0, 0.0), (40.0, 0.0)), ((40.0, 0.0), (40.0, 40.0)), ((40.0, 40.0), (0.0, 40.0)), ((0.0, 40.0), (0.0, 0.0))] {
            for i in 0..40 {
                let t = i as f64 / 40.0;
                pts.push(Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        assert_eq!(corners(&pts, 4, 50.0).len(), 4);
        let p = fit_closed(&pts, 0.2, 50.0, 4);
        assert!((p.area().abs() - 1600.0).abs() < 5.0, "{}", p.area());
        // every corner of the square is on the path
        let bb = p.bounding_box();
        assert!((bb.width() - 40.0).abs() < 0.3 && (bb.height() - 40.0).abs() < 0.3, "{bb:?}");
    }
}
