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
