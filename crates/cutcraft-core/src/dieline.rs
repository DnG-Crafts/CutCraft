//! Parametric packaging dielines: tuck-end cartons, pillow boxes and envelopes.
//!
//! Each generator returns a [`DielineGeom`] with a `cut` outline (the union of
//! all panels and flaps) and a `score` set of fold lines, both in local
//! coordinates with the top-left of the net at the origin.

use crate::geom::{self, BoolOp};
use kurbo::{BezPath, Point};

/// The two line sets that make up a dieline.
pub struct DielineGeom {
    /// Outer cut outline (and any cut-outs).
    pub cut: BezPath,
    /// Fold / score lines.
    pub score: BezPath,
    /// Overall net size (width, height) in mm.
    pub size: (f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DielineKind {
    TuckBox,
    PillowBox,
    Envelope,
}

impl DielineKind {
    pub const ALL: [(DielineKind, &'static str); 3] = [
        (DielineKind::TuckBox, "Tuck-end box"),
        (DielineKind::PillowBox, "Pillow box"),
        (DielineKind::Envelope, "Envelope"),
    ];
}

/// Parameters shared by the generators (unused fields are ignored per kind).
#[derive(Clone, Copy, Debug)]
pub struct DielineParams {
    /// Width / front face (mm).
    pub w: f64,
    /// Depth / gusset (mm).
    pub d: f64,
    /// Height / length (mm).
    pub h: f64,
    /// Glue-tab width (mm).
    pub glue: f64,
    /// Tuck-flap depth (mm).
    pub tuck: f64,
}

impl Default for DielineParams {
    fn default() -> Self {
        DielineParams { w: 60.0, d: 30.0, h: 90.0, glue: 12.0, tuck: 18.0 }
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    geom::rect_path(kurbo::Rect::new(x0, y0, x1, y1))
}

fn poly(pts: &[(f64, f64)]) -> BezPath {
    geom::polyline_path(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true)
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    geom::polyline_path(&[Point::new(x0, y0), Point::new(x1, y1)], false)
}

pub fn generate(kind: DielineKind, p: &DielineParams) -> DielineGeom {
    match kind {
        DielineKind::TuckBox => tuck_box(p),
        DielineKind::PillowBox => pillow_box(p),
        DielineKind::Envelope => envelope(p),
    }
}

/// Straight tuck-end carton: four walls (front/side/back/side), a glue tab, top
/// and bottom closing panels with tuck flaps on the back column and chamfered
/// dust flaps on the side columns.
pub fn tuck_box(p: &DielineParams) -> DielineGeom {
    let (w, d, h) = (p.w.max(5.0), p.d.max(3.0), p.h.max(5.0));
    let g = p.glue.max(4.0);
    let tuck = p.tuck.max(4.0).min(d * 1.2);
    let flap = d; // closing-panel depth
    let dust = d * 0.9; // dust-flap height
    let ch = (d * 0.3).min(6.0); // chamfer
    let inset = 2.0; // tuck insertion clearance

    // Column x boundaries: glue | front(W) | side(D) | back(W) | side(D)
    let x_glue = 0.0;
    let x_front = g;
    let x_sidel = x_front + w;
    let x_back = x_sidel + d;
    let x_sider = x_back + w;
    let x_end = x_sider + d;

    let body_top = flap + tuck; // leave room for top panel + tuck above
    let body_bot = body_top + h;
    let net_h = body_bot + flap + tuck;

    let mut regions: Vec<BezPath> = Vec::new();
    // Body (the four walls) + glue tab.
    regions.push(rect(x_front, body_top, x_end, body_bot));
    regions.push(rect(x_glue, body_top + ch, x_front, body_bot - ch)); // chamfered glue tab

    // Top closing panel over the back column + tuck flap.
    regions.push(rect(x_back, body_top - flap, x_sider, body_top));
    regions.push(poly(&[
        (x_back + inset, body_top - flap),
        (x_back + inset + ch, body_top - flap - tuck),
        (x_sider - inset - ch, body_top - flap - tuck),
        (x_sider - inset, body_top - flap),
    ]));
    // Top dust flaps over the side columns (chamfered).
    for (a, b) in [(x_sidel, x_back), (x_sider, x_end)] {
        regions.push(poly(&[(a, body_top), (a + ch, body_top - dust), (b - ch, body_top - dust), (b, body_top)]));
    }
    // Bottom closing panel + tuck (mirror of top).
    regions.push(rect(x_back, body_bot, x_sider, body_bot + flap));
    regions.push(poly(&[
        (x_back + inset, body_bot + flap),
        (x_back + inset + ch, body_bot + flap + tuck),
        (x_sider - inset - ch, body_bot + flap + tuck),
        (x_sider - inset, body_bot + flap),
    ]));
    for (a, b) in [(x_sidel, x_back), (x_sider, x_end)] {
        regions.push(poly(&[(a, body_bot), (a + ch, body_bot + dust), (b - ch, body_bot + dust), (b, body_bot)]));
    }

    let cut = geom::boolean(&regions, &[], BoolOp::Union);

    // Fold lines: vertical panel creases + the two horizontal body creases +
    // the closing-panel-to-tuck creases.
    let mut folds: Vec<BezPath> = Vec::new();
    for x in [x_front, x_sidel, x_back, x_sider] {
        folds.push(line(x, body_top, x, body_bot));
    }
    folds.push(line(x_front, body_top, x_end, body_top));
    folds.push(line(x_front, body_bot, x_end, body_bot));
    folds.push(line(x_back, body_top - flap, x_sider, body_top - flap));
    folds.push(line(x_back, body_bot + flap, x_sider, body_bot + flap));

    DielineGeom { cut, score: geom::merge_paths(&folds), size: (x_end, net_h) }
}

/// Pillow box: panels W, D, W, D around the body, a glue tab, and curved end
/// caps on the two wide panels.
pub fn pillow_box(p: &DielineParams) -> DielineGeom {
    let (w, d, l) = (p.w.max(10.0), p.d.max(5.0), p.h.max(10.0));
    let g = p.glue.max(4.0);
    let cap = (d * 0.6).max(4.0); // curved cap height

    // Columns: glue | p1(W) | p2(D) | p3(W) | p4(D)
    let x0 = g;
    let xs = [x0, x0 + w, x0 + w + d, x0 + 2.0 * w + d, x0 + 2.0 * w + 2.0 * d];
    let x_end = xs[4];
    let (body_top, body_bot) = (cap, cap + l);
    let net_h = body_bot + cap;

    let mut regions: Vec<BezPath> = Vec::new();
    regions.push(rect(0.0, body_top, x_end, body_bot));

    // Half-ellipse caps on the two wide panels (p1 at xs[0..1], p3 at xs[2..3]).
    let arc = |xa: f64, xb: f64, base: f64, peak: f64| -> BezPath {
        let mid = (xa + xb) * 0.5;
        let rx = (xb - xa) * 0.5;
        let steps = 24;
        let mut pts = vec![Point::new(xa, base)];
        for i in 0..=steps {
            let t = std::f64::consts::PI * i as f64 / steps as f64; // 0..PI
            let x = mid - rx * t.cos();
            let y = base + (peak - base) * t.sin();
            pts.push(Point::new(x, y));
        }
        pts.push(Point::new(xb, base));
        geom::polyline_path(&pts, true)
    };
    for (a, b) in [(xs[0], xs[1]), (xs[2], xs[3])] {
        regions.push(arc(a, b, body_top, 0.0)); // top cap
        regions.push(arc(a, b, body_bot, net_h)); // bottom cap
    }

    let cut = geom::boolean(&regions, &[], BoolOp::Union);

    let mut folds: Vec<BezPath> = Vec::new();
    for &x in &xs[..4] {
        folds.push(line(x, body_top, x, body_bot));
    }
    // Cap base creases across the wide panels.
    for (a, b) in [(xs[0], xs[1]), (xs[2], xs[3])] {
        folds.push(line(a, body_top, b, body_top));
        folds.push(line(a, body_bot, b, body_bot));
    }
    DielineGeom { cut, score: geom::merge_paths(&folds), size: (x_end, net_h) }
}

/// Standard pocket envelope: a central rectangle with four chamfered flaps
/// (top flap is the seal). `w`×`h` is the pocket; `tuck` sizes the seal flap.
pub fn envelope(p: &DielineParams) -> DielineGeom {
    let (w, h) = (p.w.max(20.0), p.h.max(20.0));
    let top_flap = p.tuck.max(h * 0.45);
    let bot_flap = h * 0.35;
    let side_flap = w * 0.28;
    let ch = (w.min(h) * 0.12).min(12.0);

    // Pocket occupies [side_flap, top_flap] .. origin at net top-left.
    let px0 = side_flap;
    let px1 = side_flap + w;
    let py0 = top_flap;
    let py1 = top_flap + h;

    let mut regions: Vec<BezPath> = Vec::new();
    regions.push(rect(px0, py0, px1, py1));
    // Top seal flap.
    regions.push(poly(&[(px0, py0), (px0 + ch, py0 - top_flap), (px1 - ch, py0 - top_flap), (px1, py0)]));
    // Bottom flap.
    regions.push(poly(&[(px0, py1), (px0 + ch, py1 + bot_flap), (px1 - ch, py1 + bot_flap), (px1, py1)]));
    // Left / right flaps.
    regions.push(poly(&[(px0, py0), (px0 - side_flap, py0 + ch), (px0 - side_flap, py1 - ch), (px0, py1)]));
    regions.push(poly(&[(px1, py0), (px1 + side_flap, py0 + ch), (px1 + side_flap, py1 - ch), (px1, py1)]));

    let cut = geom::boolean(&regions, &[], BoolOp::Union);

    let folds = geom::merge_paths(&[
        line(px0, py0, px1, py0),
        line(px0, py1, px1, py1),
        line(px0, py0, px0, py1),
        line(px1, py0, px1, py1),
    ]);
    DielineGeom { cut, score: folds, size: (px1 + side_flap, py1 + bot_flap) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    #[test]
    fn tuck_box_has_outline_and_folds() {
        let g = tuck_box(&DielineParams::default());
        assert!(!g.cut.elements().is_empty());
        assert!(geom::subpaths(&g.score).len() >= 6);
        // the net is wider than a single wall and taller than the body
        assert!(g.size.0 > 60.0 && g.size.1 > 90.0);
        // cut area is positive and sensible
        assert!(g.cut.bounding_box().area() > 1000.0);
    }

    /// Writes an HTML preview of all three dielines (cut = red, fold = blue).
    /// `cargo test -p cutcraft-core preview_dielines -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_dielines() {
        let mut svgs = String::new();
        for (kind, name) in DielineKind::ALL {
            let g = generate(kind, &DielineParams::default());
            svgs += &format!(
                "<figure><figcaption>{name}</figcaption><svg viewBox='-5 -5 {} {}' width='320'>\
                 <path d='{}' fill='none' stroke='#d33' stroke-width='0.5'/>\
                 <path d='{}' fill='none' stroke='#36c' stroke-width='0.5' stroke-dasharray='2 2'/></svg></figure>",
                g.size.0 + 10.0,
                g.size.1 + 10.0,
                g.cut.to_svg(),
                g.score.to_svg(),
            );
        }
        let html = format!("<!doctype html><meta charset=utf8><body style='font-family:sans-serif;display:flex;gap:24px;flex-wrap:wrap'>{svgs}</body>");
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dielines_preview.html");
        std::fs::write(&out, html).unwrap();
        println!("wrote {}", out.display());
    }

    #[test]
    fn pillow_and_envelope_generate() {
        let pb = pillow_box(&DielineParams::default());
        assert!(!pb.cut.elements().is_empty());
        assert!(geom::subpaths(&pb.score).len() >= 4);
        let ev = envelope(&DielineParams::default());
        assert!(!ev.cut.elements().is_empty());
        assert_eq!(geom::subpaths(&ev.score).len(), 4);
    }
}
