//! Shape → GPU-friendly geometry cache (local coordinates, mm).

use cutcraft_core::doc::{Rgba, Shape, ShapeKind};
use cutcraft_core::geom;
use cutcraft_core::kurbo::{Affine, BezPath, Point};
use egui::Color32;
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LPath;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers};
use std::collections::HashMap;
use std::sync::Arc;

pub struct CachedLeaf {
    /// Transform from leaf-local to shape-local.
    pub fill: Option<(Vec<[f32; 2]>, Vec<u32>, Color32)>,
    pub lines: Vec<(Vec<Point>, bool)>,
    pub stroke: Option<Color32>,
    pub stroke_width: f64,
    pub cuts: bool,
    pub image: Option<(Arc<Vec<u8>>, u32, u32, f64, f64, Affine)>,
}

pub struct CachedShape {
    pub rev: u64,
    pub leaves: Vec<CachedLeaf>,
}

#[derive(Default)]
pub struct RenderCache {
    pub shapes: HashMap<u64, CachedShape>,
}

/// The colour a leaf shape is filled with on screen, if any.
pub fn fill_colour(s: &Shape) -> Option<Rgba> {
    match &s.kind {
        // Stroke-font letters are open lines; filling would draw crescents.
        ShapeKind::Text { data, .. } if data.family == cutcraft_core::singleline::FAMILY => None,
        _ => s.style.fill,
    }
}

/// A closed outline as a stroke without spikes. egui joins the corners of a
/// closed line with unlimited miters, so where an outline doubles back on
/// itself (hand-drawn and decorative fonts are full of these) the corner
/// shoots off across the screen. Its open lines trim sharp corners instead,
/// so the outline is drawn open, overlapping its first segment so the
/// starting corner is joined like the rest.
pub fn closed_outline(mut pts: Vec<egui::Pos2>, stroke: egui::Stroke) -> egui::Shape {
    if pts.len() > 2 {
        pts.push(pts[0]);
        pts.push(pts[1]);
    }
    egui::Shape::line(pts, stroke)
}

pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}

pub fn tessellate(paths: &[BezPath], tol: f64) -> (Vec<[f32; 2]>, Vec<u32>) {
    let mut b = LPath::builder();
    let mut any = false;
    for pl in geom::flatten_all(paths, tol) {
        if pl.pts.len() < 3 {
            continue;
        }
        b.begin(point(pl.pts[0].x as f32, pl.pts[0].y as f32));
        for p in &pl.pts[1..] {
            b.line_to(point(p.x as f32, p.y as f32));
        }
        b.end(true);
        any = true;
    }
    if !any {
        return (vec![], vec![]);
    }
    let path = b.build();
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut t = FillTessellator::new();
    let _ = t.tessellate_path(
        &path,
        &FillOptions::tolerance(tol as f32).with_fill_rule(FillRule::EvenOdd),
        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
    );
    (buffers.vertices, buffers.indices)
}

fn collect(s: &Shape, rel: Affine, out: &mut Vec<CachedLeaf>) {
    if s.hidden {
        return;
    }
    // a united (effect-carrying) group draws as one shape
    if let Some(m) = s.merged() {
        let mut m = m;
        m.effects.clear();
        return collect(&m, rel, out);
    }
    match &s.kind {
        ShapeKind::Group(ch) => {
            for c in ch {
                collect(c, rel * c.transform, out);
            }
        }
        ShapeKind::Image(img) => out.push(CachedLeaf {
            fill: None,
            lines: vec![],
            stroke: None,
            stroke_width: 0.0,
            cuts: false,
            image: Some((img.png.clone(), img.px_w, img.px_h, img.width_mm, img.height_mm, rel)),
        }),
        _ => {
            let paths = geom::transformed(&s.local_paths(), rel);
            let fill = fill_colour(s).map(|f| {
                let (v, i) = tessellate(&paths, 0.05);
                (v, i, c32(f))
            });
            let lines = geom::flatten_all(&paths, 0.05).into_iter().map(|p| (p.pts, p.closed)).collect();
            out.push(CachedLeaf {
                fill,
                lines,
                stroke: s.style.stroke.map(c32),
                stroke_width: s.style.stroke_width,
                cuts: s.style.cut != cutcraft_core::doc::CutStyle::NoCut,
                image: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warped_text_tessellates() {
        let fonts = cutcraft_core::text::FontLibrary::new();
        let data = cutcraft_core::doc::TextData { text: "CutCraft".into(), size: 28.0, ..Default::default() };
        let outline = fonts.layout(&data);
        let mut s = Shape::new_path(1, outline, cutcraft_core::doc::Style::default());
        s.transform = Affine::translate((25.0, 215.0));
        let mut doc = cutcraft_core::doc::Document::default();
        doc.page_mut().shapes.push(s);
        doc.next_id = 2;
        let ids = cutcraft_core::ops::make_warp(&mut doc, &[1], cutcraft_core::warp::WarpMode::Mesh { rows: 4, cols: 4, pts: vec![] });
        let w = doc.page().shape(ids[0]).unwrap();
        let paths = w.local_paths();
        let orig_subs: usize = geom::flatten_all(&[fonts.layout(&data)], 0.05).len();
        let warp_subs = geom::flatten_all(&paths, 0.05).len();
        assert_eq!(orig_subs, warp_subs, "contour count changed");
        let mut b = LPath::builder();
        for pl in geom::flatten_all(&paths, 0.05) {
            b.begin(point(pl.pts[0].x as f32, pl.pts[0].y as f32));
            for p in &pl.pts[1..] {
                b.line_to(point(p.x as f32, p.y as f32));
            }
            b.end(true);
        }
        let path = b.build();
        let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
        let r = FillTessellator::new().tessellate_path(
            &path,
            &FillOptions::tolerance(0.05).with_fill_rule(FillRule::EvenOdd),
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position().to_array()),
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(!buffers.indices.is_empty());
        // compare filled area from the render cache with the true area
        let mut leaves = vec![];
        let mut w2 = w.clone();
        w2.style.fill = Some(Rgba(40, 40, 40, 255));
        collect(&w2, Affine::IDENTITY, &mut leaves);
        let (v, idx, _) = leaves[0].fill.as_ref().expect("fill");
        let mut area = 0.0f64;
        for t in idx.chunks(3) {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            area += (((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) as f64 / 2.0).abs();
        }
        use geo::Area;
        let truth = geom::to_multipolygon(&paths).unsigned_area();
        assert!((area - truth).abs() / truth < 0.05, "mesh area {area} vs {truth}");
    }
}

impl RenderCache {
    pub fn get(&mut self, s: &Shape, rev: u64) -> &CachedShape {
        let stale = self.shapes.get(&s.id).map(|c| c.rev != rev).unwrap_or(true);
        if stale {
            let mut leaves = vec![];
            collect(s, Affine::IDENTITY, &mut leaves);
            self.shapes.insert(s.id, CachedShape { rev, leaves });
        }
        &self.shapes[&s.id]
    }
    pub fn prune(&mut self, live: &[u64]) {
        if self.shapes.len() > live.len() * 2 + 64 {
            self.shapes.retain(|k, _| live.contains(k));
        }
    }
}
