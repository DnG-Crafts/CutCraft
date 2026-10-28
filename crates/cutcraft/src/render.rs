//! Drawing the design. Two caches keep big designs (traces with thousands of
//! shapes) quick:
//! - each shape's geometry, prepared once in its own coordinates (mm) and
//!   only again when the shape itself changes - not when it moves;
//! - the finished picture: ready-to-draw triangle meshes for the page at the
//!   current zoom, built on all cores for a margin around the window, and
//!   reused (just shifted when panning) until something changes.

use cutcraft_core::doc::{Rgba, Shape, ShapeKind};
use cutcraft_core::geom;
use cutcraft_core::kurbo::{Affine, BezPath, Point, Rect};
use egui::Color32;
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LPath;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
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
    /// The shape as it was prepared (compared with the shape now to see
    /// whether it needs preparing again; its position doesn't matter).
    look: Shape,
    pub leaves: Vec<CachedLeaf>,
    /// Bounds of everything it draws, in its own coordinates.
    bounds: Option<Rect>,
}

/// One piece of the finished picture, in drawing order.
pub enum Part {
    /// Triangles in screen points (as placed when the picture was built),
    /// with an id of its own (for keeping it on the graphics card).
    Mesh { id: u64, mesh: Arc<egui::Mesh> },
    /// A picture: its PNG, size (mm) and placement on the page.
    Image { png: Arc<Vec<u8>>, w: f64, h: f64, at: Affine },
}

/// The finished picture and what it was built for.
struct Scene {
    key: u64,
    zoom: f32,
    /// Where the page origin was on screen.
    origin: egui::Pos2,
    /// The screen area it covers, there.
    area: egui::Rect,
    parts: Vec<Part>,
}

#[derive(Default)]
pub struct RenderCache {
    shapes: HashMap<u64, CachedShape>,
    /// Goes up whenever a shape is prepared again.
    generation: u64,
    scene: Option<Scene>,
    /// Counts the pictures handed out (one per canvas frame).
    frame: u64,
}

/// Ids for meshes, never reused.
static NEXT_MESH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What the canvas is showing, for [`RenderCache::picture`].
pub struct Frame<'a> {
    pub page: &'a cutcraft_core::doc::Page,
    pub zoom: f32,
    /// The page origin on screen.
    pub origin: egui::Pos2,
    /// The canvas area on screen.
    pub canvas: egui::Rect,
    pub cut_only: bool,
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

    /// Where the time goes drawing a big design (run by hand, in release:
    /// cargo test --release -p cutcraft draw_timing -- --ignored --nocapture).
    /// "picture" is CutCraft's own work each frame; the total adds egui
    /// copying the meshes, as when there's no GPU drawing (see gpu.rs) -
    /// with it, that copy is replaced by one upload when a picture is built.
    #[test]
    #[ignore = "timing, run by hand"]
    fn draw_timing() {
        use std::time::Instant;
        // a trace-sized design: 20,000 small curvy filled shapes with outlines
        let mut doc = cutcraft_core::doc::Document::default();
        for i in 0..20_000 {
            let (x, y) = ((i % 200) as f64 * 1.5, (i / 200) as f64 * 1.5);
            let mut p = BezPath::new();
            p.move_to((x, y));
            p.curve_to((x + 0.5, y - 0.3), (x + 1.0, y + 0.3), (x + 1.2, y + 0.6));
            p.curve_to((x + 1.0, y + 1.2), (x + 0.4, y + 1.0), (x + 0.1, y + 1.1));
            p.curve_to((x - 0.2, y + 0.8), (x + 0.2, y + 0.4), (x, y));
            p.close_path();
            let style = cutcraft_core::doc::Style { fill: Some(Rgba(30, 30, 30, 255)), ..Default::default() };
            doc.add_path(p, style, 0);
        }
        let shapes = &doc.page().shapes;
        let points: usize = shapes.iter().map(|s| geom::flatten_all(&s.local_paths(), 0.05).iter().map(|p| p.pts.len()).sum::<usize>()).sum();

        println!("{} shapes, {points} outline points, {} cores", shapes.len(), rayon::current_num_threads());

        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        let mut cache = RenderCache::default();
        // what the canvas does each frame: get the picture, hand egui the
        // meshes (shifted if panned), egui turns them into GPU triangles
        let frame = |cache: &mut RenderCache, doc: &cutcraft_core::doc::Document, zoom: f32, origin: egui::Pos2| {
            let t = Instant::now();
            let f = Frame { page: doc.page(), zoom, origin, canvas, cut_only: false };
            let (parts, shift, _) = cache.picture(&ctx, &f);
            let got = t.elapsed();
            let verts: usize = parts.iter().map(|p| if let Part::Mesh { mesh, .. } = p { mesh.vertices.len() } else { 0 }).sum();
            let shapes: Vec<egui::epaint::ClippedShape> = parts
                .iter()
                .filter_map(|p| match p {
                    Part::Mesh { mesh, .. } if shift == egui::Vec2::ZERO => Some(egui::Shape::Mesh(mesh.clone())),
                    Part::Mesh { mesh, .. } => {
                        let mut m = (**mesh).clone();
                        m.translate(shift);
                        Some(egui::Shape::mesh(m))
                    }
                    Part::Image { .. } => None,
                })
                .map(|shape| egui::epaint::ClippedShape { clip_rect: canvas, shape })
                .collect();
            let _ = ctx.tessellate(shapes, 1.5);
            format!("{:?} (picture {got:?}, {verts} vertices)", t.elapsed())
        };
        let o = egui::pos2(20.0, 20.0);
        println!("first draw (prepare all):      {:?}", frame(&mut cache, &doc, 3.0, o));
        println!("next frame, nothing changed:   {:?}", frame(&mut cache, &doc, 3.0, o));
        let id = doc.page().shapes[500].id;
        doc.page_mut().shape_mut(id).unwrap().style.fill = Some(Rgba(200, 0, 0, 255));
        println!("one shape recoloured:          {:?}", frame(&mut cache, &doc, 3.0, o));
        let id = doc.page().shapes[900].id;
        doc.page_mut().shape_mut(id).unwrap().transform = Affine::translate((5.0, 5.0));
        println!("one shape moved:               {:?}", frame(&mut cache, &doc, 3.0, o));
        println!("panned 50 px:                  {:?}", frame(&mut cache, &doc, 3.0, o + egui::vec2(50.0, 30.0)));
        println!("zoomed 10%:                    {:?}", frame(&mut cache, &doc, 3.3, o));
        println!("zoomed in 4x (most off screen): {:?}", frame(&mut cache, &doc, 12.0, o));
    }
}

/// Whether two shapes draw the same, wherever they are.
fn same_look(a: &Shape, b: &Shape) -> bool {
    a.hidden == b.hidden && a.style == b.style && a.kind == b.kind && a.effects == b.effects
}

fn prepare(s: &Shape) -> CachedShape {
    let mut leaves = vec![];
    collect(s, Affine::IDENTITY, &mut leaves);
    let mut bounds: Option<Rect> = None;
    let mut add = |p: Point| bounds = Some(bounds.map_or(Rect::from_points(p, p), |b| b.union_pt(p)));
    for l in &leaves {
        if let Some((v, _, _)) = &l.fill {
            v.iter().for_each(|p| add(Point::new(p[0] as f64, p[1] as f64)));
        }
        l.lines.iter().flat_map(|(pts, _)| pts).for_each(|p| add(*p));
        if let Some((_, _, _, w, h, rel)) = &l.image {
            [(0.0, 0.0), (*w, 0.0), (*w, *h), (0.0, *h)].into_iter().for_each(|c| add(*rel * Point::from(c)));
        }
    }
    let mut look = s.clone();
    look.transform = Affine::IDENTITY;
    CachedShape { look, leaves, bounds }
}

impl RenderCache {
    /// Prepares every new or changed shape of `shapes`, on all cores, and
    /// forgets shapes that are gone.
    fn refresh(&mut self, shapes: &[Shape]) {
        let stale: Vec<&Shape> = shapes.iter().filter(|s| self.shapes.get(&s.id).is_none_or(|c| !same_look(&c.look, s))).collect();
        if !stale.is_empty() {
            let fresh: Vec<(u64, CachedShape)> = stale.par_iter().map(|s| (s.id, prepare(s))).collect();
            self.shapes.extend(fresh);
            self.generation += 1;
        }
        if self.shapes.len() > shapes.len() * 2 + 64 {
            let live: HashSet<u64> = shapes.iter().map(|s| s.id).collect();
            self.shapes.retain(|k, _| live.contains(k));
        }
    }

    /// The page's shapes ready to draw, in order; the shift to give each
    /// mesh (they're placed for where the page origin was when built); and
    /// this frame's number.
    pub fn picture(&mut self, ctx: &egui::Context, f: &Frame) -> (&[Part], egui::Vec2, u64) {
        let page = f.page;
        self.frame += 1;
        self.refresh(&page.shapes);
        // everything the picture depends on apart from panning
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.generation, f.cut_only, f.zoom.to_bits(), ctx.pixels_per_point().to_bits()).hash(&mut h);
        for s in page.shapes.iter().filter(|s| page.is_visible(s)) {
            s.id.hash(&mut h);
            s.transform.as_coeffs().map(f64::to_bits).hash(&mut h);
            page.layer(s.layer).map(|l| (l.color.0, l.color.1, l.color.2, l.color.3)).hash(&mut h);
        }
        let key = h.finish();
        let usable = self.scene.as_ref().is_some_and(|sc| sc.key == key && sc.zoom == f.zoom && sc.area.translate(f.origin - sc.origin).contains_rect(f.canvas));
        if !usable {
            // half a window spare all round, so panning reuses it
            let area = f.canvas.expand2(f.canvas.size() * 0.5);
            let parts = self.build(ctx, f, area);
            self.scene = Some(Scene { key, zoom: f.zoom, origin: f.origin, area, parts });
        }
        let sc = self.scene.as_ref().expect("built above");
        (&sc.parts, f.origin - sc.origin, self.frame)
    }

    /// Builds the picture of the visible shapes that reach into `area`.
    fn build(&self, ctx: &egui::Context, f: &Frame, area: egui::Rect) -> Vec<Part> {
        enum Op<'a> {
            Vector(&'a CachedLeaf, Affine, Option<Color32>),
            Image(&'a CachedLeaf, Affine),
        }
        let (z, origin) = (f.zoom, f.origin);
        let screen = |p: Point| origin + egui::vec2(p.x as f32, p.y as f32) * z;
        let mut ops = vec![];
        for s in f.page.shapes.iter().filter(|s| f.page.is_visible(s)) {
            let Some(c) = self.shapes.get(&s.id) else { continue };
            let a = s.transform;
            // off screen: skip it
            let Some(b) = c.bounds else { continue };
            let corners = [b.origin(), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)].map(|p| screen(a * p));
            if !area.intersects(egui::Rect::from_points(&corners).expand(4.0)) {
                continue;
            }
            let layer_col = f.page.layer(s.layer).map(|l| c32(l.color));
            for leaf in &c.leaves {
                ops.push(if leaf.image.is_some() { Op::Image(leaf, a) } else { Op::Vector(leaf, a, layer_col) });
            }
        }
        let options = ctx.tessellation_options(|o| *o);
        let ppp = ctx.pixels_per_point();
        let font_size = ctx.fonts(|f| f.font_image_size());
        // vector runs between pictures, meshed in chunks on all cores
        let mesh_run = |run: &[Op]| -> Vec<Part> {
            run.par_chunks(512)
                .map(|chunk| {
                    let mut t = egui::epaint::Tessellator::new(ppp, options, font_size, vec![]);
                    let mut mesh = egui::Mesh::default();
                    for op in chunk {
                        let Op::Vector(leaf, a, layer_col) = op else { continue };
                        if !f.cut_only {
                            if let Some((v, idx, col)) = &leaf.fill {
                                let base = mesh.vertices.len() as u32;
                                for p in v {
                                    mesh.colored_vertex(screen(*a * Point::new(p[0] as f64, p[1] as f64)), *col);
                                }
                                mesh.indices.extend(idx.iter().map(|i| i + base));
                            }
                        }
                        let (col, w) = if f.cut_only {
                            if !leaf.cuts {
                                continue;
                            }
                            (Color32::from_rgb(230, 40, 40), 1.5)
                        } else {
                            match leaf.stroke {
                                Some(c) => (c, ((leaf.stroke_width * a.determinant().abs().sqrt()) as f32 * z).max(1.0)),
                                None if leaf.fill.is_none() => (layer_col.unwrap_or(Color32::GRAY), 1.0),
                                None => continue,
                            }
                        };
                        for (pts, closed) in &leaf.lines {
                            let sp: Vec<egui::Pos2> = pts.iter().map(|p| screen(*a * *p)).collect();
                            let line = if *closed { closed_outline(sp, egui::Stroke::new(w, col)) } else { egui::Shape::line(sp, egui::Stroke::new(w, col)) };
                            t.tessellate_shape(line, &mut mesh);
                        }
                    }
                    Part::Mesh { id: NEXT_MESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed), mesh: Arc::new(mesh) }
                })
                .collect()
        };
        let mut parts = vec![];
        let mut start = 0;
        for (i, op) in ops.iter().enumerate() {
            if let Op::Image(leaf, a) = op {
                parts.extend(mesh_run(&ops[start..i]));
                if !f.cut_only {
                    if let Some((png, _, _, w, h, rel)) = &leaf.image {
                        parts.push(Part::Image { png: png.clone(), w: *w, h: *h, at: *a * *rel });
                    }
                }
                start = i + 1;
            }
        }
        parts.extend(mesh_run(&ops[start..]));
        parts.retain(|p| !matches!(p, Part::Mesh { mesh, .. } if mesh.is_empty()));
        parts
    }
}
