//! Splitting a design bigger than the mat into tiles that lock together: the
//! seams between tiles are cut as dovetails or puzzle knobs instead of straight
//! lines. Ported from BurnCraft's `bc-geom/tiles.rs`.

use crate::geom::polyline_path;
use kurbo::{BezPath, Point, Rect, Vec2};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    Straight,
    Dovetail,
    Puzzle,
}

impl Joint {
    pub const ALL: [Joint; 3] = [Joint::Dovetail, Joint::Puzzle, Joint::Straight];

    pub fn label(self) -> &'static str {
        match self {
            Joint::Straight => "Straight (no joints)",
            Joint::Dovetail => "Dovetail",
            Joint::Puzzle => "Puzzle knob",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileSpec {
    pub cols: usize,
    pub rows: usize,
    pub joint: Joint,
    /// Joint width (mm); its depth is about the same.
    pub size: f64,
    /// Roughly how far apart joints are along a seam (mm).
    pub spacing: f64,
}

/// The joint outline in seam coordinates: `u` along the seam (centred on 0), `v` across it.
fn joint_profile(joint: Joint, s: f64) -> Vec<(f64, f64)> {
    match joint {
        Joint::Straight => Vec::new(),
        Joint::Dovetail => vec![(-0.3 * s, 0.0), (-0.5 * s, 0.7 * s), (0.5 * s, 0.7 * s), (0.3 * s, 0.0)],
        Joint::Puzzle => {
            // A neck 0.4 s wide into a round head 0.8 s across.
            let (r, cy, neck) = (0.4 * s, 0.6 * s, 0.2 * s);
            let meet = cy - (r * r - neck * neck).sqrt();
            let mut pts = vec![(-neck, 0.0), (-neck, meet)];
            // Round the far side of the head, from where the left side of the neck meets it, the
            // long way round (through the tip), to the right side of the neck.
            let a0 = (meet - cy).atan2(-neck);
            let a1 = (meet - cy).atan2(neck);
            let sweep = (a0 - a1).rem_euclid(2.0 * PI);
            let n = 24;
            for i in 1..n {
                let a = a0 - sweep * i as f64 / n as f64;
                pts.push((r * a.cos(), cy + r * a.sin()));
            }
            pts.push((neck, meet));
            pts.push((neck, 0.0));
            pts
        }
    }
}

/// A seam from `a` to `b`, with joints spread along the part between `lo` and `hi` (distances from
/// `a`), alternating which side they stick out to. The same seam is used by the tiles on both sides.
fn seam(a: Point, b: Point, lo: f64, hi: f64, spec: &TileSpec, first_side: f64) -> Vec<Point> {
    let len = (b - a).hypot();
    let mut pts = vec![a];
    if spec.joint == Joint::Straight || len < 1e-9 || hi - lo < spec.size * 1.5 {
        pts.push(b);
        return pts;
    }
    let d = (b - a) / len;
    let n = Vec2::new(-d.y, d.x);
    let count = (((hi - lo) / spec.spacing.max(spec.size * 1.5)).floor() as usize).max(1);
    let profile = joint_profile(spec.joint, spec.size);
    for k in 0..count {
        let centre = lo + (hi - lo) * (k as f64 + 0.5) / count as f64;
        let side = if k % 2 == 0 { first_side } else { -first_side };
        for &(u, v) in &profile {
            pts.push(a + d * (centre + u) + n * (v * side));
        }
    }
    pts.push(b);
    pts
}

/// The tile outlines for a design covering `area`, row by row: each is (column, row, outline).
/// Outer tiles reach well past `area`, so intersecting the design with them keeps everything.
pub fn tile_cells(area: Rect, spec: &TileSpec) -> Vec<(usize, usize, BezPath)> {
    let (cols, rows) = (spec.cols.max(1), spec.rows.max(1));
    let ext = area.width().max(area.height()) + 10.0;
    let xs: Vec<f64> = (0..=cols).map(|i| area.x0 + area.width() * i as f64 / cols as f64).collect();
    let ys: Vec<f64> = (0..=rows).map(|j| area.y0 + area.height() * j as f64 / rows as f64).collect();
    // Grid lines, pushed outwards at the border.
    let gx = |i: usize| if i == 0 { area.x0 - ext } else if i == cols { area.x1 + ext } else { xs[i] };
    let gy = |j: usize| if j == 0 { area.y0 - ext } else if j == rows { area.y1 + ext } else { ys[j] };
    // Vertical seam between columns i-1 and i, in row j, top to bottom; horizontal seam between
    // rows j-1 and j, in column i, left to right. Joints stay on the design's own part of the seam.
    let vseam = |i: usize, j: usize| {
        let (a, b) = (Point::new(xs[i], gy(j)), Point::new(xs[i], gy(j + 1)));
        seam(a, b, ys[j] - gy(j), ys[j + 1] - gy(j), spec, if (i + j).is_multiple_of(2) { 1.0 } else { -1.0 })
    };
    let hseam = |i: usize, j: usize| {
        let (a, b) = (Point::new(gx(i), ys[j]), Point::new(gx(i + 1), ys[j]));
        seam(a, b, xs[i] - gx(i), xs[i + 1] - gx(i), spec, if (i + j).is_multiple_of(2) { -1.0 } else { 1.0 })
    };
    let mut out = Vec::new();
    for j in 0..rows {
        for i in 0..cols {
            let mut ring: Vec<Point> = Vec::new();
            // Top edge, left to right.
            ring.extend(if j == 0 { vec![Point::new(gx(i), gy(0)), Point::new(gx(i + 1), gy(0))] } else { hseam(i, j) });
            // Right edge, top to bottom.
            ring.extend(if i + 1 == cols { vec![Point::new(gx(cols), gy(j)), Point::new(gx(cols), gy(j + 1))] } else { vseam(i + 1, j) });
            // Bottom edge, right to left.
            let mut bottom = if j + 1 == rows { vec![Point::new(gx(i), gy(rows)), Point::new(gx(i + 1), gy(rows))] } else { hseam(i, j + 1) };
            bottom.reverse();
            ring.extend(bottom);
            // Left edge, bottom to top.
            let mut left = if i == 0 { vec![Point::new(gx(0), gy(j)), Point::new(gx(0), gy(j + 1))] } else { vseam(i, j) };
            left.reverse();
            ring.extend(left);
            ring.dedup_by(|a, b| (*a - *b).hypot() < 1e-9);
            if ring.len() > 1 && (ring[0] - ring[ring.len() - 1]).hypot() < 1e-9 {
                ring.pop();
            }
            out.push((i, j, polyline_path(&ring, true)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{self, BoolOp};
    use kurbo::Shape as _;

    #[test]
    fn tiles_cover_the_area_exactly_once() {
        let r = Rect::new(0.0, 0.0, 300.0, 200.0);
        let design = geom::rect_path(r);
        for joint in Joint::ALL {
            let spec = TileSpec { cols: 3, rows: 2, joint, size: 12.0, spacing: 50.0 };
            let cells = tile_cells(r, &spec);
            assert_eq!(cells.len(), 6);
            let pieces: Vec<f64> = cells
                .iter()
                .map(|(_, _, c)| geom::boolean(&[design.clone()], &[c.clone()], BoolOp::Intersection).area().abs())
                .collect();
            let total: f64 = pieces.iter().sum();
            assert!((total - 60_000.0).abs() < 1.0, "{joint:?}: pieces add up to {total}");
            assert!(pieces.iter().all(|a| (a - 10_000.0).abs() < 300.0), "{joint:?}: about equal: {pieces:?}");
            if joint != Joint::Straight {
                assert!(geom::flatten(&cells[1].2, 0.01)[0].pts.len() > 20, "{joint:?} has joints");
            }
        }
    }

    /// Contact sheet: per-corner rounding/chamfer, and a panel split into jointed tiles.
    /// `cargo test -p cutcraft-core preview_tiles_corners -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_tiles_corners() {
        use crate::corners::{CornerCut, set_corner};
        let mut parts: Vec<BezPath> = Vec::new();
        // corners: rounded 8, rounded 15, chamfer 10, sharp — placed at (10, 10)
        let sq = geom::polyline_path(&[Point::new(0.0, 0.0), Point::new(60.0, 0.0), Point::new(60.0, 60.0), Point::new(0.0, 60.0)], true);
        let mut c = set_corner(&sq, Point::new(0.0, 0.0), 8.0, CornerCut::Round);
        c = set_corner(&c, Point::new(60.0, 0.0), 15.0, CornerCut::Round);
        c = set_corner(&c, Point::new(60.0, 60.0), 10.0, CornerCut::Chamfer);
        parts.push(kurbo::Affine::translate((10.0, 10.0)) * c);
        // tiles: a rounded 300 x 200 panel, 3 x 2, dovetail then puzzle, pulled apart
        let panel = geom::rounded_rect_path(Rect::new(0.0, 0.0, 300.0, 200.0), 20.0);
        for (k, joint) in [Joint::Dovetail, Joint::Puzzle].into_iter().enumerate() {
            let spec = TileSpec { cols: 3, rows: 2, joint, size: 16.0, spacing: 45.0 };
            for (i, j, cell) in tile_cells(Rect::new(0.0, 0.0, 300.0, 200.0), &spec) {
                let piece = geom::boolean(&[panel.clone()], &[cell], BoolOp::Intersection);
                let at = kurbo::Affine::translate((90.0 + i as f64 * 10.0 + k as f64 * 340.0, 10.0 + j as f64 * 10.0));
                parts.push(at * piece);
            }
        }
        let (w, h, k) = (790.0, 230.0, 1.6);
        let mut img = image::RgbaImage::from_pixel((w * k) as u32, (h * k) as u32, image::Rgba([255, 255, 255, 255]));
        for p in &parts {
            for pl in geom::flatten(p, 0.05) {
                let mut pts = pl.pts.clone();
                if pl.closed { pts.push(pts[0]); }
                for s in pts.windows(2) {
                    let n = ((s[1] - s[0]).hypot() * k * 2.0).ceil().max(1.0) as usize;
                    for t in 0..=n {
                        let q = s[0].lerp(s[1], t as f64 / n as f64);
                        let (x, y) = ((q.x * k) as u32, (q.y * k) as u32);
                        if x < img.width() && y < img.height() { img.put_pixel(x, y, image::Rgba([20, 20, 20, 255])); }
                    }
                }
            }
        }
        let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/tiles_corners_preview.png");
        img.save(out).unwrap();
        println!("wrote {out}");
    }

    #[test]
    fn puzzle_knob_has_a_round_head() {
        let p = joint_profile(Joint::Puzzle, 10.0);
        let far = p.iter().map(|q| q.1).fold(f64::MIN, f64::max);
        assert!((far - 10.0).abs() < 0.1, "head reaches the joint size: {far}");
        let widest = p.iter().map(|q| q.0.abs()).fold(0.0, f64::max);
        assert!((widest - 4.0).abs() < 0.1 && p.iter().all(|q| q.1 >= -1e-9), "{p:?}");
    }
}
