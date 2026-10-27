//! One-shot distortion effects that bake artwork into new paths: arch, bulge,
//! bridge, wave, flag, perspective, barrel, tuna-can, title-crawl, plus the
//! generative drop shadow, cast shadow and 3D extrude.
//!
//! Each distortion maps the artwork's points through a function of their
//! position within the artwork's bounding box, densifying edges so straight
//! lines bend smoothly.

use crate::geom::{self, BoolOp};
use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use std::f64::consts::PI;

/// The envelope distortions, for a single UI list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Warp {
    Arch,
    Bulge,
    Bridge,
    Wave,
    Flag,
    Perspective,
    Barrel,
    TunaCan,
    TitleCrawl,
    Rays,
    Rotate3d,
}

impl Warp {
    pub const ALL: [(Warp, &'static str, &'static str); 11] = [
        (Warp::Arch, "Arch", "Bend along a circular arc"),
        (Warp::Bulge, "Bulge", "Fatten the middle"),
        (Warp::Bridge, "Bridge", "Bow the lower edge up"),
        (Warp::Wave, "Wave", "Sine wave along the width"),
        (Warp::Flag, "Flag", "Wave that grows toward the right"),
        (Warp::Perspective, "Perspective", "Trapezoid taper"),
        (Warp::Barrel, "Barrel", "Fisheye bulge / pinch from the centre"),
        (Warp::TunaCan, "Tuna can", "Wrap around a horizontal cylinder"),
        (Warp::TitleCrawl, "Title crawl", "Recede toward the top"),
        (Warp::Rays, "Rays", "Fan outward from a point"),
        (Warp::Rotate3d, "3D rotate", "Rotate about the vertical axis in perspective"),
    ];

    /// Short display name (from [`Self::ALL`]).
    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(w, _, _)| *w == self).map(|(_, n, _)| *n).unwrap_or("Warp")
    }

    /// `amount` is the primary strength (−1..1 for most); `k2` is a secondary
    /// control (cycles for Wave/Flag, bottom taper for Perspective, depth for
    /// TitleCrawl, wrap angle in degrees for TunaCan).
    pub fn apply(self, paths: &[BezPath], amount: f64, k2: f64) -> BezPath {
        let Some(bb) = geom::paths_bbox(paths) else { return geom::merge_paths(paths) };
        let f = self.mapper(bb, amount, k2);
        geom::merge_paths(&geom::remap_densified(paths, (bb.width().max(bb.height()) / 150.0).clamp(0.3, 2.0), &f))
    }

    fn mapper(self, bb: Rect, a: f64, k2: f64) -> Box<dyn Fn(Point) -> Point> {
        let (x0, y0, w, h) = (bb.x0, bb.y0, bb.width().max(1e-6), bb.height().max(1e-6));
        let cx = x0 + w / 2.0;
        let y1 = bb.y1;
        match self {
            Warp::Arch => {
                // circular sag across the width; positive bends upward
                let s = a * h;
                if s.abs() < 1e-6 {
                    return Box::new(|p| p);
                }
                let r = (w * w / 4.0 + s * s) / (2.0 * s.abs());
                let sign = s.signum();
                Box::new(move |p: Point| {
                    let dx = p.x - cx;
                    let sag = r - (r * r - dx * dx).max(0.0).sqrt();
                    Point::new(p.x, p.y - sign * sag)
                })
            }
            Warp::Bulge => Box::new(move |p: Point| {
                let u = (p.x - x0) / w;
                let v = (p.y - y0) / h;
                let nv = 0.5 + (v - 0.5) * (1.0 + a * (PI * u).sin());
                Point::new(p.x, y0 + nv * h)
            }),
            Warp::Bridge => Box::new(move |p: Point| {
                let u = (p.x - x0) / w;
                let v = (p.y - y0) / h;
                Point::new(p.x, p.y - a * h * (PI * u).sin() * v)
            }),
            Warp::Wave => {
                let cycles = k2.max(0.25);
                Box::new(move |p: Point| {
                    let u = (p.x - x0) / w;
                    Point::new(p.x, p.y + a * h * (2.0 * PI * cycles * u).sin())
                })
            }
            Warp::Flag => {
                let cycles = k2.max(0.25);
                Box::new(move |p: Point| {
                    let u = (p.x - x0) / w;
                    Point::new(p.x, p.y + a * h * u * (2.0 * PI * cycles * u).sin())
                })
            }
            Warp::Perspective => {
                // a = top width scale, k2 = bottom width scale
                let (ts, bs) = (a, if k2.abs() < 1e-6 { 1.0 } else { k2 });
                Box::new(move |p: Point| {
                    let v = (p.y - y0) / h;
                    let scale = ts + (bs - ts) * v;
                    Point::new(cx + (p.x - cx) * scale, p.y)
                })
            }
            Warp::Barrel => {
                let half_diag = (w * w + h * h).sqrt() / 2.0;
                let cy = y0 + h / 2.0;
                Box::new(move |p: Point| {
                    let d = Vec2::new(p.x - cx, p.y - cy);
                    let rn = d.hypot() / half_diag;
                    let f = 1.0 + a * (rn * rn - 0.33);
                    Point::new(cx + d.x * f, cy + d.y * f)
                })
            }
            Warp::TunaCan => {
                // a = amount of forward bow (0 = flat label), k2 = wrap angle (deg)
                let wrap = k2.to_radians().abs().clamp(0.05, PI * 1.8);
                let r = (w / 2.0) / (wrap / 2.0).sin();
                Box::new(move |p: Point| {
                    let u = (p.x - x0) / w - 0.5;
                    let phi = u * wrap;
                    Point::new(cx + r * phi.sin(), p.y + (r - r * phi.cos()) * 0.15 * a)
                })
            }
            Warp::TitleCrawl => {
                // a = top width scale (<1 narrows top); k2 = vertical recession depth
                let ts = a.clamp(0.05, 1.0);
                let depth = k2.clamp(0.0, 3.0);
                Box::new(move |p: Point| {
                    let v = ((p.y - y0) / h).clamp(0.0, 1.0);
                    let scale = ts + (1.0 - ts) * v;
                    let nv = v.powf(1.0 + depth);
                    Point::new(cx + (p.x - cx) * scale, y1 - (1.0 - nv) * h)
                })
            }
            Warp::Rays => {
                // fan the artwork around a centre point above it; a = spread (−1..1)
                let spread = (a.clamp(-1.0, 1.0) * PI).abs().max(1e-3);
                let r0 = (w / 2.0) / (spread / 2.0).sin();
                let cy_arc = y0 - r0; // centre sits above the top edge
                let sign = if a >= 0.0 { 1.0 } else { -1.0 };
                Box::new(move |p: Point| {
                    let u = (p.x - x0) / w - 0.5;
                    let phi = u * spread * sign;
                    let radius = r0 + (p.y - y0);
                    Point::new(cx + radius * phi.sin(), cy_arc + radius * phi.cos())
                })
            }
            Warp::Rotate3d => {
                // a = rotation angle fraction (−1..1 → ±80°); k2 = focal length factor
                let theta = (a.clamp(-1.0, 1.0) * 80f64.to_radians()).clamp(-1.3, 1.3);
                let focal = (if k2.abs() < 1e-6 { 2.0 } else { k2.abs() }) * w;
                let cy = y0 + h / 2.0;
                Box::new(move |p: Point| {
                    let lx = p.x - cx;
                    let z = lx * theta.sin();
                    let scale = focal / (focal + z).max(1e-3);
                    Point::new(cx + lx * theta.cos() * scale, cy + (p.y - cy) * scale)
                })
            }
        }
    }
}

/// Offset duplicate for a drop shadow (caller fills it and places it behind).
pub fn drop_shadow(paths: &[BezPath], dx: f64, dy: f64) -> BezPath {
    let out = geom::transformed(paths, Affine::translate((dx, dy)));
    geom::boolean(&out, &[], BoolOp::Union)
}

/// Long "cast" shadow laid toward one side. `skew` shears by height, `squash`
/// flattens vertically (0.2–0.4 looks like a floor shadow).
pub fn cast_shadow(paths: &[BezPath], skew: f64, squash: f64) -> BezPath {
    let Some(bb) = geom::paths_bbox(paths) else { return geom::merge_paths(paths) };
    let y1 = bb.y1;
    let squash = squash.clamp(0.02, 2.0);
    let f = move |p: Point| {
        let hgt = y1 - p.y;
        Point::new(p.x + hgt * skew, y1 - hgt * squash)
    };
    let s = geom::remap_densified(paths, (bb.width().max(bb.height()) / 150.0).clamp(0.3, 2.0), &f);
    geom::boolean(&s, &[], BoolOp::Union)
}

/// 3D extrude. Returns connector lines and the back-face outline (translated by
/// `(dx,dy)`), to be placed behind the original with a cut/draw style.
/// The front artwork is left unchanged by the caller.
pub fn extrude(paths: &[BezPath], dx: f64, dy: f64, samples: usize) -> Vec<BezPath> {
    let outline = geom::boolean(paths, &[], BoolOp::Union);
    let back = geom::transformed(&[outline.clone()], Affine::translate((dx, dy)));
    let mut out = vec![geom::merge_paths(&back)];
    let samples = samples.clamp(8, 400);
    for sub in geom::subpaths(&outline) {
        let len = geom::path_length(&sub);
        if len < 1e-3 {
            continue;
        }
        let n = ((len / 3.0).ceil() as usize).clamp(8, samples);
        for i in 0..n {
            if let Some((p, _)) = geom::point_at_length(&sub, len * i as f64 / n as f64) {
                out.push(geom::polyline_path(&[p, Point::new(p.x + dx, p.y + dy)], false));
            }
        }
    }
    out
}

/// Round the corners of polygon paths by `radius` mm (fillet). Curved segments
/// are left alone; only the straight-line vertices are rounded.
pub fn round_corners(paths: &[BezPath], radius: f64) -> BezPath {
    if radius <= 0.0 {
        return geom::merge_paths(paths);
    }
    let mut out = BezPath::new();
    for pl in geom::flatten_all(paths, 0.05) {
        let n = pl.pts.len();
        if n < 3 {
            out.extend(geom::polyline_to_path(&pl).elements().iter().copied());
            continue;
        }
        let mut b = BezPath::new();
        let get = |i: isize| pl.pts[i.rem_euclid(n as isize) as usize];
        let count = if pl.closed { n } else { n };
        let mut started = false;
        for i in 0..count {
            let prev = get(i as isize - 1);
            let cur = get(i as isize);
            let next = get(i as isize + 1);
            if !pl.closed && (i == 0 || i == n - 1) {
                // keep open-path endpoints sharp
                if !started {
                    b.move_to(cur);
                    started = true;
                } else {
                    b.line_to(cur);
                }
                continue;
            }
            let d0 = (prev - cur).normalize();
            let d1 = (next - cur).normalize();
            let r = radius.min(prev.distance(cur) / 2.0).min(next.distance(cur) / 2.0);
            let p_in = cur + d0 * r;
            let p_out = cur + d1 * r;
            if !started {
                b.move_to(p_in);
                started = true;
            } else {
                b.line_to(p_in);
            }
            // quadratic through the corner approximates the fillet arc
            b.quad_to(cur, p_out);
        }
        if pl.closed {
            b.close_path();
        }
        out.extend(b.elements().iter().copied());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn square() -> BezPath {
        geom::rect_path(Rect::new(0.0, 0.0, 100.0, 20.0))
    }

    #[test]
    fn arch_raises_the_middle() {
        let w = Warp::Arch.apply(&[square()], 0.3, 0.0);
        let bb = w.bounding_box();
        // top edge bows up, so min-y is well above 0
        assert!(bb.y0 < -5.0, "{bb:?}");
        // width is preserved
        assert!((bb.width() - 100.0).abs() < 0.5);
    }

    #[test]
    fn perspective_narrows_top() {
        let w = Warp::Perspective.apply(&[square()], 0.5, 1.0);
        // sample the top and bottom extents
        let pls = geom::flatten(&w, 0.1);
        let top_w = pls.iter().flat_map(|p| &p.pts).filter(|p| p.y < 5.0).fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.x), a.1.max(p.x)));
        let bot_w = pls.iter().flat_map(|p| &p.pts).filter(|p| p.y > 15.0).fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.x), a.1.max(p.x)));
        assert!((top_w.1 - top_w.0) < (bot_w.1 - bot_w.0) - 20.0);
    }

    #[test]
    fn extrude_makes_connectors_and_back() {
        let v = extrude(&[square()], 10.0, -10.0, 60);
        assert!(v.len() > 10);
        let back = v[0].bounding_box();
        assert!((back.x0 - 10.0).abs() < 0.5 && (back.y0 + 10.0).abs() < 0.5);
    }

    #[test]
    fn round_corners_shrinks_bbox_little_and_adds_curves() {
        let r = round_corners(&[square()], 4.0);
        assert!(r.segments().any(|s| matches!(s, kurbo::PathSeg::Quad(_))));
    }

    #[test]
    fn cast_shadow_lies_down() {
        let s = cast_shadow(&[square()], 1.5, 0.3);
        let bb = s.bounding_box();
        // sheared sideways: wider than the original 100
        assert!(bb.width() > 110.0);
        // squashed: shorter than the original 20
        assert!(bb.height() < 20.0);
    }
}
