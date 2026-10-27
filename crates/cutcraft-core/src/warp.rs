//! Live warps: envelope/mesh warp with presets, and conical (tumbler) warp.
//!
//! A warp shape keeps its original artwork so it can be edited, reverted or
//! released to plain paths at any time.

use crate::doc::Shape;
use crate::geom::{self, Polyline};
use kurbo::{BezPath, Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConeParams {
    /// Diameter of the top rim, mm.
    pub top_d: f64,
    /// Diameter of the bottom, mm.
    pub bottom_d: f64,
    /// Slant height (measured along the side), mm.
    pub slant: f64,
    /// Distance from the top rim to the top of the design, mm.
    pub offset_from_top: f64,
}

impl Default for ConeParams {
    fn default() -> Self {
        ConeParams { top_d: 90.0, bottom_d: 70.0, slant: 150.0, offset_from_top: 30.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WarpMode {
    /// (rows+1)×(cols+1) control points, row-major, in the warp's local space.
    Mesh { rows: usize, cols: usize, pts: Vec<(f64, f64)> },
    Conical(ConeParams),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpData {
    pub original: Box<Shape>,
    /// Bounding box of the original artwork (warp-local).
    pub src: (f64, f64, f64, f64),
    pub mode: WarpMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    None,
    ArcUp,
    ArcDown,
    ArchTop,
    Bulge,
    Pinch,
    Wave,
    Flag,
    Rise,
    Perspective,
}

impl Preset {
    pub const ALL: [(Preset, &'static str); 10] = [
        (Preset::None, "Reset"),
        (Preset::ArcUp, "Arc up"),
        (Preset::ArcDown, "Arc down"),
        (Preset::ArchTop, "Arch"),
        (Preset::Bulge, "Bulge"),
        (Preset::Pinch, "Pinch"),
        (Preset::Wave, "Wave"),
        (Preset::Flag, "Flag"),
        (Preset::Rise, "Rise"),
        (Preset::Perspective, "Perspective"),
    ];

    /// Map unit coordinates (u,v) ∈ [0,1]² through the preset with strength k.
    fn apply(&self, u: f64, v: f64, k: f64) -> (f64, f64) {
        use std::f64::consts::PI;
        let s = (PI * u).sin();
        match self {
            Preset::None => (u, v),
            Preset::ArcUp => (u, v - k * s),
            Preset::ArcDown => (u, v + k * s),
            Preset::ArchTop => (u, v - k * s * (1.0 - v)),
            Preset::Bulge => (u, 0.5 + (v - 0.5) * (1.0 + k * s)),
            Preset::Pinch => (u, 0.5 + (v - 0.5) * (1.0 - k * s)),
            Preset::Wave => (u, v + k * 0.5 * (2.0 * PI * u).sin()),
            Preset::Flag => (u, v + k * 0.5 * (2.0 * PI * u).sin() * u),
            Preset::Rise => (u, v - k * u),
            Preset::Perspective => (0.5 + (u - 0.5) * (1.0 - k * (1.0 - v)), v),
        }
    }
}

impl WarpData {
    pub fn new_mesh(original: Shape, rows: usize, cols: usize) -> WarpData {
        let bb = geom::paths_bbox(&original.world_paths()).unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
        let mut w = WarpData { original: Box::new(original), src: (bb.x0, bb.y0, bb.x1, bb.y1), mode: WarpMode::Conical(ConeParams::default()) };
        w.set_preset(Preset::None, 0.0, rows, cols);
        w
    }

    pub fn new_conical(original: Shape, p: ConeParams) -> WarpData {
        let bb = geom::paths_bbox(&original.world_paths()).unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
        WarpData { original: Box::new(original), src: (bb.x0, bb.y0, bb.x1, bb.y1), mode: WarpMode::Conical(p) }
    }

    pub fn src_rect(&self) -> Rect {
        Rect::new(self.src.0, self.src.1, self.src.2, self.src.3)
    }

    pub fn set_preset(&mut self, p: Preset, k: f64, rows: usize, cols: usize) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let r = self.src_rect();
        let mut pts = Vec::with_capacity((rows + 1) * (cols + 1));
        for j in 0..=rows {
            for i in 0..=cols {
                let (u, v) = p.apply(i as f64 / cols as f64, j as f64 / rows as f64, k);
                pts.push((r.x0 + u * r.width(), r.y0 + v * r.height()));
            }
        }
        self.mode = WarpMode::Mesh { rows, cols, pts };
    }

    /// Map a point of the original artwork to its warped position.
    pub fn map(&self, p: Point) -> Point {
        let r = self.src_rect();
        match &self.mode {
            WarpMode::Mesh { rows, cols, pts } => {
                let u = ((p.x - r.x0) / r.width().max(1e-9)).clamp(0.0, 1.0) * *cols as f64;
                let v = ((p.y - r.y0) / r.height().max(1e-9)).clamp(0.0, 1.0) * *rows as f64;
                catmull_rom_patch(pts, *rows, *cols, u, v)
            }
            WarpMode::Conical(c) => cone_map(c, r, p),
        }
    }

    pub fn warped_paths(&self) -> Vec<BezPath> {
        let mut out = vec![];
        for path in self.original.world_paths() {
            let pls: Vec<Polyline> = geom::flatten(&path, 0.05)
                .into_iter()
                .map(|pl| {
                    // densify long segments so straight lines bend smoothly
                    let mut dense = vec![];
                    let n = pl.pts.len();
                    let segs = if pl.closed { n } else { n.saturating_sub(1) };
                    for i in 0..segs {
                        let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
                        let steps = (a.distance(b) / 1.0).ceil().max(1.0) as usize;
                        for s in 0..steps {
                            dense.push(self.map(a.lerp(b, s as f64 / steps as f64)));
                        }
                    }
                    if !pl.closed {
                        if let Some(l) = pl.pts.last() {
                            dense.push(self.map(*l));
                        }
                    }
                    Polyline { pts: dense, closed: pl.closed }
                })
                .collect();
            out.push(geom::polylines_to_path(&pls));
        }
        out
    }
}

fn catmull_rom_1d(p: [Point; 4], t: f64) -> Point {
    let t2 = t * t;
    let t3 = t2 * t;
    let f = |a: f64, b: f64, c: f64, d: f64| {
        0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    Point::new(f(p[0].x, p[1].x, p[2].x, p[3].x), f(p[0].y, p[1].y, p[2].y, p[3].y))
}

/// Interpolating tensor-product Catmull–Rom surface through the grid.
fn catmull_rom_patch(pts: &[(f64, f64)], rows: usize, cols: usize, u: f64, v: f64) -> Point {
    let get = |i: isize, j: isize| -> Point {
        // linear extrapolation beyond the border keeps edges straight-ish
        let ci = i.clamp(0, cols as isize) as usize;
        let cj = j.clamp(0, rows as isize) as usize;
        let at = |ci: usize, cj: usize| {
            let p = pts[cj * (cols + 1) + ci];
            Point::new(p.0, p.1)
        };
        let base = at(ci, cj);
        let mut q = base;
        if i < 0 || i > cols as isize {
            let inner = if i < 0 { 1 } else { cols - 1 };
            q += base - at(inner.min(cols), cj);
        }
        if j < 0 || j > rows as isize {
            let inner = if j < 0 { 1 } else { rows - 1 };
            q += base - at(ci, inner.min(rows));
        }
        q
    };
    let i = (u.floor() as isize).min(cols as isize - 1).max(0);
    let j = (v.floor() as isize).min(rows as isize - 1).max(0);
    let (tu, tv) = (u - i as f64, v - j as f64);
    let mut col = [Point::ZERO; 4];
    for (k, dj) in (-1..=2).enumerate() {
        let row = [get(i - 1, j + dj), get(i, j + dj), get(i + 1, j + dj), get(i + 2, j + dj)];
        col[k] = catmull_rom_1d(row, tu);
    }
    catmull_rom_1d(col, tv)
}

/// Unrolled-cone geometry: (apex radius of top edge, apex radius of bottom edge, k = angle factor).
fn cone_geometry(c: &ConeParams) -> Option<(f64, f64, f64, bool)> {
    let (ra, rb) = (c.top_d / 2.0, c.bottom_d / 2.0);
    if (ra - rb).abs() < 1e-6 || c.slant <= 0.0 {
        return None; // cylinder: no warp needed
    }
    let top_big = ra > rb;
    let (big, small) = if top_big { (ra, rb) } else { (rb, ra) };
    let l_big = c.slant * big / (big - small);
    let l_small = l_big - c.slant;
    Some((l_big, l_small, big / l_big, top_big))
}

fn cone_map(c: &ConeParams, src: Rect, p: Point) -> Point {
    let Some((l_big, l_small, k, top_big)) = cone_geometry(c) else { return p };
    let (ra, rb) = (c.top_d / 2.0, c.bottom_d / 2.0);
    // design top sits `offset_from_top` below the rim
    let h = c.offset_from_top + (p.y - src.y0);
    let h_ref = c.offset_from_top + src.height() / 2.0;
    let r_ref = ra + (rb - ra) * (h_ref / c.slant);
    let cx = src.center().x;
    let alpha = (p.x - cx) / r_ref; // angle around the cup
    let phi = alpha * k;
    if top_big {
        let rho = l_big - h;
        let apex_y = src.y0 - c.offset_from_top + l_big;
        Point::new(cx + rho * phi.sin(), apex_y - rho * phi.cos())
    } else {
        let rho = l_small + h;
        let apex_y = src.y0 - c.offset_from_top - l_small;
        Point::new(cx + rho * phi.sin(), apex_y + rho * phi.cos())
    }
}

/// Outline of the whole unrolled cup wrap (for cutting a test template).
pub fn cone_template(c: &ConeParams, src: Rect) -> Option<BezPath> {
    cone_geometry(c)?;
    let (ra, rb) = (c.top_d / 2.0, c.bottom_d / 2.0);
    let h_ref = c.offset_from_top + src.height() / 2.0;
    let r_ref = ra + (rb - ra) * (h_ref / c.slant);
    let half = std::f64::consts::PI * r_ref; // half circumference at the reference height
    let cx = src.center().x;
    let n = 120;
    let x_at = |i: usize| cx - half + 2.0 * half * i as f64 / n as f64;
    // the rim is at h = 0, i.e. p.y = src.y0 - offset_from_top
    let rim_y = src.y0 - c.offset_from_top;
    let top: Vec<Point> = (0..=n).map(|i| cone_map(c, src, Point::new(x_at(i), rim_y))).collect();
    let bottom: Vec<Point> = (0..=n).rev().map(|i| cone_map(c, src, Point::new(x_at(i), rim_y + c.slant))).collect();
    let mut all = top;
    all.extend(bottom);
    Some(geom::polyline_path(&all, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Style;

    fn rect_shape() -> Shape {
        Shape::new_path(1, geom::rect_path(Rect::new(0.0, 0.0, 100.0, 50.0)), Style::default())
    }

    #[test]
    fn identity_mesh_preserves_geometry() {
        let w = WarpData::new_mesh(rect_shape(), 3, 3);
        for p in [Point::new(10.0, 10.0), Point::new(55.0, 31.0), Point::new(100.0, 50.0)] {
            assert!(w.map(p).distance(p) < 1e-6, "{p:?} -> {:?}", w.map(p));
        }
    }

    #[test]
    fn arc_up_lifts_middle() {
        let mut w = WarpData::new_mesh(rect_shape(), 4, 4);
        w.set_preset(Preset::ArcUp, 0.3, 4, 4);
        let mid = w.map(Point::new(50.0, 0.0));
        assert!(mid.y < -10.0);
        let edge = w.map(Point::new(0.0, 0.0));
        assert!(edge.y.abs() < 1e-6);
    }

    #[test]
    fn conical_keeps_centre_and_curves_edges() {
        let w = WarpData::new_conical(rect_shape(), ConeParams::default());
        let c = w.map(Point::new(50.0, 0.0));
        assert!((c.x - 50.0).abs() < 1e-9 && c.y.abs() < 1e-9);
        let e = w.map(Point::new(0.0, 0.0));
        assert!(e.y > 0.5, "edges bend down for a top-wide cup: {e:?}");
        assert!(cone_template(&ConeParams::default(), w.src_rect()).is_some());
    }
}
