//! Turning vector artwork into print images: Flatten (several shapes become
//! one printed picture with a single outer cut line, for Print Then Cut) and
//! Pattern fill (a shape filled with a repeating picture), as in Cricut Design
//! Space. Both are built on a small scan-line renderer.

use crate::doc::{CutStyle, Document, ImageData, Rgba, Shape, ShapeId, ShapeKind, Style};
use crate::geom;
use crate::ops::Sel;
use kurbo::{Affine, BezPath, Point, Rect};
use std::sync::Arc;

/// Resolution of flattened / pattern-filled pictures.
pub const DPI: f64 = 200.0;

/// Even-odd coverage of `paths` (mm) over a `w`×`h` pixel grid covering
/// `area`, sampled `ss`×`ss` times per pixel: 0..=ss² per pixel.
fn coverage(paths: &[BezPath], area: Rect, w: usize, h: usize, ss: usize) -> Vec<u16> {
    let mut cov = vec![0u16; w * h];
    let pls = geom::flatten_all(paths, 0.05);
    let (px_w, px_h) = (area.width() / w as f64, area.height() / h as f64);
    let mut xs: Vec<f64> = Vec::new();
    for row in 0..h * ss {
        let y = area.y0 + (row as f64 + 0.5) * px_h / ss as f64;
        xs.clear();
        for pl in &pls {
            let n = pl.pts.len();
            if n < 2 {
                continue;
            }
            // every polyline counts as closed for filling
            for i in 0..n {
                let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
                if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                    xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        let py = row / ss;
        for pair in xs.chunks(2) {
            let [x0, x1] = pair else { continue };
            // sub-columns whose centre is inside [x0, x1)
            let c0 = (((x0 - area.x0) / px_w * ss as f64) - 0.5).ceil().max(0.0) as usize;
            let c1 = (((x1 - area.x0) / px_w * ss as f64) - 0.5).ceil().max(0.0) as usize;
            for c in c0..c1.min(w * ss) {
                cov[py * w + c / ss] += 1;
            }
        }
    }
    cov
}

/// Blends `colour` into `img` where `cov` says (alpha = coverage).
fn paint(img: &mut image::RgbaImage, cov: &[u16], full: u16, colour: [u8; 4]) {
    for (i, c) in cov.iter().enumerate() {
        if *c == 0 {
            continue;
        }
        let a = (*c as f32 / full as f32) * colour[3] as f32 / 255.0;
        let p = img.get_pixel_mut((i % img.width() as usize) as u32, (i / img.width() as usize) as u32);
        for k in 0..3 {
            p.0[k] = (p.0[k] as f32 * (1.0 - a) + colour[k] as f32 * a).round() as u8;
        }
        p.0[3] = (p.0[3] as f32 + (255.0 - p.0[3] as f32) * a).round() as u8;
    }
}

/// Pixel size for `area` at [`DPI`], capped so pictures stay a sensible size.
fn pixels(area: Rect) -> (usize, usize) {
    let k = DPI / 25.4;
    let (mut w, mut h) = ((area.width() * k).ceil().max(1.0), (area.height() * k).ceil().max(1.0));
    let cap = 6000.0;
    if w.max(h) > cap {
        let s = cap / w.max(h);
        (w, h) = ((w * s).ceil(), (h * s).ceil());
    }
    (w as usize, h as usize)
}

/// Renders `shapes` (page space) over `area` as they'd print: fills, and
/// outline-only closed shapes as solid in their line colour; pictures drawn
/// in place. Transparent elsewhere.
pub fn render(shapes: &[Shape], area: Rect) -> image::RgbaImage {
    let (w, h) = pixels(area);
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([255, 255, 255, 0]));
    const SS: usize = 3;
    for s in shapes {
        for leaf in s.leaves() {
            if leaf.hidden || leaf.style.no_print {
                continue;
            }
            match &leaf.kind {
                ShapeKind::Image(d) => draw_image(&mut img, area, &leaf, d),
                _ => {
                    let colour = leaf.style.fill.or(leaf.style.stroke).unwrap_or(Rgba::BLACK);
                    let cov = coverage(&leaf.world_paths(), area, w, h, SS);
                    paint(&mut img, &cov, (SS * SS) as u16, [colour.0, colour.1, colour.2, colour.3]);
                }
            }
        }
    }
    img
}

/// Nearest-pixel draw of a picture leaf into `img` (covering `area`).
fn draw_image(img: &mut image::RgbaImage, area: Rect, leaf: &Shape, d: &ImageData) {
    let Ok(src) = image::load_from_memory(&d.png) else { return };
    let src = src.to_rgba8();
    let inv = leaf.transform.inverse();
    let (w, h) = (img.width(), img.height());
    let (px_w, px_h) = (area.width() / w as f64, area.height() / h as f64);
    for y in 0..h {
        for x in 0..w {
            let p = inv * Point::new(area.x0 + (x as f64 + 0.5) * px_w, area.y0 + (y as f64 + 0.5) * px_h);
            if p.x < 0.0 || p.y < 0.0 || p.x >= d.width_mm || p.y >= d.height_mm {
                continue;
            }
            let (sx, sy) = ((p.x / d.width_mm * src.width() as f64) as u32, (p.y / d.height_mm * src.height() as f64) as u32);
            let c = src.get_pixel(sx.min(src.width() - 1), sy.min(src.height() - 1)).0;
            if c[3] == 0 {
                continue;
            }
            let a = c[3] as f32 / 255.0;
            let q = img.get_pixel_mut(x, y);
            for k in 0..3 {
                q.0[k] = (q.0[k] as f32 * (1.0 - a) + c[k] as f32 * a).round() as u8;
            }
            q.0[3] = (q.0[3] as f32 + (255.0 - q.0[3] as f32) * a).round() as u8;
        }
    }
}

fn png(img: &image::RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

/// A picture shape covering `area` (page mm).
fn image_shape(doc: &mut Document, img: &image::RgbaImage, area: Rect, layer: u32) -> Shape {
    let id = doc.alloc_id();
    let mut s = Shape::new_path(id, BezPath::new(), Style { fill: None, stroke: None, stroke_width: 0.0, cut: CutStyle::NoCut, no_print: false });
    s.kind = ShapeKind::Image(ImageData { png: Arc::new(png(img)), px_w: img.width(), px_h: img.height(), width_mm: area.width(), height_mm: area.height(), original: None });
    s.transform = Affine::translate(area.origin().to_vec2());
    s.layer = layer;
    s
}

/// The name a flattened group carries (Unflatten looks for it).
pub const FLATTENED: &str = "Flattened";

/// Flatten: the selection becomes one printed picture plus one cut line round
/// its outer edge, grouped. The original shapes are kept, hidden, inside the
/// group so [`unflatten`] can bring them back.
pub fn flatten(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let page = doc.page();
    let shapes: Vec<Shape> = page.shapes.iter().filter(|s| sel.contains(&s.id)).cloned().collect();
    if shapes.is_empty() {
        return vec![];
    }
    let Some(bb) = shapes.iter().filter_map(|s| s.bbox()).reduce(|a, b| a.union(b)) else { return vec![] };
    let area = bb.inflate(0.5, 0.5);
    let img = render(&shapes, area);
    let all: Vec<BezPath> = shapes.iter().flat_map(|s| s.leaves()).filter(|l| !l.hidden).flat_map(|l| l.world_paths()).collect();
    let outline = crate::effects::outer_silhouette(&all);
    let layer = shapes[0].layer;
    let idx = page.shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
    let picture = image_shape(doc, &img, area, layer);
    let cut_id = doc.alloc_id();
    let mut cut = Shape::new_path(cut_id, outline, Style { fill: None, stroke: Some(Rgba(40, 40, 40, 255)), no_print: true, ..Style::default() });
    cut.layer = layer;
    let mut children = vec![picture, cut];
    for mut s in shapes {
        s.hidden = true;
        children.push(s);
    }
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    let gid = doc.alloc_id();
    let mut g = Shape::new_path(gid, BezPath::new(), Style::default());
    g.kind = ShapeKind::Group(children);
    g.name = FLATTENED.into();
    g.layer = layer;
    let page = doc.page_mut();
    page.shapes.insert(idx.min(page.shapes.len()), g);
    vec![gid]
}

/// Unflatten: the original shapes of a flattened group back, editable.
pub fn unflatten(doc: &mut Document, sel: &[ShapeId]) -> Sel {
    let mut out = vec![];
    let page = doc.page_mut();
    let mut i = 0;
    while i < page.shapes.len() {
        let s = &page.shapes[i];
        if sel.contains(&s.id) && s.name == FLATTENED {
            if let ShapeKind::Group(ch) = s.kind.clone() {
                let t = s.transform;
                page.shapes.remove(i);
                for mut c in ch.into_iter().filter(|c| c.hidden) {
                    c.hidden = false;
                    c.transform = t * c.transform;
                    out.push(c.id);
                    page.shapes.insert(i, c);
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Pattern fill: `id` keeps its outline as the cut line and gets a printed
/// picture of `pattern` repeated across it (tiles `tile_mm` wide, turned by
/// `angle` degrees), grouped with it.
pub fn pattern_fill(doc: &mut Document, id: ShapeId, pattern: &[u8], tile_mm: f64, angle: f64) -> Result<Sel, String> {
    let pat = image::load_from_memory(pattern).map_err(|e| e.to_string())?.to_rgba8();
    let Some(s) = doc.page().shape(id).cloned() else { return Err("no shape".into()) };
    let Some(bb) = s.bbox() else { return Err("the shape is empty".into()) };
    let area = bb;
    let (w, h) = pixels(area);
    const SS: usize = 2;
    let paths = s.world_paths();
    let cov = coverage(&paths, area, w, h, SS);
    let tile_w = tile_mm.max(1.0);
    let tile_h = tile_w * pat.height() as f64 / pat.width().max(1) as f64;
    let rot = Affine::rotate(-angle.to_radians());
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([255, 255, 255, 0]));
    let (px_w, px_h) = (area.width() / w as f64, area.height() / h as f64);
    for y in 0..h {
        for x in 0..w {
            let c = cov[y * w + x];
            if c == 0 {
                continue;
            }
            let p = rot * Point::new((x as f64 + 0.5) * px_w, (y as f64 + 0.5) * px_h);
            let u = (p.x / tile_w).rem_euclid(1.0);
            let v = (p.y / tile_h).rem_euclid(1.0);
            let src = pat.get_pixel(((u * pat.width() as f64) as u32).min(pat.width() - 1), ((v * pat.height() as f64) as u32).min(pat.height() - 1)).0;
            let a = (c as f32 / (SS * SS) as f32) * src[3] as f32 / 255.0;
            img.put_pixel(x as u32, y as u32, image::Rgba([src[0], src[1], src[2], (a * 255.0).round() as u8]));
        }
    }
    let layer = s.layer;
    let picture = image_shape(doc, &img, area, layer);
    let page = doc.page_mut();
    let idx = page.shapes.iter().position(|x| x.id == id).unwrap_or(0);
    let mut outline = page.shapes.remove(idx);
    outline.style.fill = None;
    outline.style.no_print = true;
    let gid = doc.alloc_id();
    let mut g = Shape::new_path(gid, BezPath::new(), Style::default());
    g.kind = ShapeKind::Group(vec![picture, outline]);
    g.name = "Pattern fill".into();
    g.layer = layer;
    let page = doc.page_mut();
    page.shapes.insert(idx.min(page.shapes.len()), g);
    Ok(vec![gid])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;

    #[test]
    fn flatten_prints_the_colours_cuts_the_outside_and_unflattens() {
        let mut d = Document::default();
        let red = Style { fill: Some(Rgba(220, 0, 0, 255)), ..Style::default() };
        let blue = Style { fill: Some(Rgba(0, 0, 220, 255)), ..Style::default() };
        let a = d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 50.0, 30.0)), red, 0);
        let b = d.add_path(geom::rect_path(Rect::new(40.0, 20.0, 70.0, 40.0)), blue, 0);
        let g = flatten(&mut d, &[a, b]);
        assert_eq!(g.len(), 1);
        let grp = d.page().shape(g[0]).unwrap();
        assert_eq!(grp.name, FLATTENED);
        // what's cut: one outer outline, not the two rectangles
        let cut: Vec<_> = grp.leaves().into_iter().filter(|l| !l.hidden && l.style.cut != CutStyle::NoCut).collect();
        assert_eq!(cut.len(), 1);
        assert_eq!(geom::flatten_all(&cut[0].world_paths(), 0.1).len(), 1);
        // what's printed: red on the left, blue where blue was on top
        let ShapeKind::Image(img) = &grp.leaves()[0].kind else { panic!("picture first") };
        let pic = image::load_from_memory(&img.png).unwrap().to_rgba8();
        let at = |x: f64, y: f64| *pic.get_pixel(((x - 9.5) / 25.4 * DPI) as u32, ((y - 9.5) / 25.4 * DPI) as u32);
        assert_eq!(at(15.0, 15.0).0, [220, 0, 0, 255]);
        assert_eq!(at(60.0, 35.0).0, [0, 0, 220, 255]);
        assert_eq!(at(60.0, 12.0).0[3], 0, "outside both: clear");
        // and back
        let back = unflatten(&mut d, &g);
        assert_eq!(back.len(), 2);
        assert!(d.page().shapes.iter().all(|s| !s.hidden));
    }

    #[test]
    fn pattern_fill_tiles_inside_the_shape_only() {
        let mut d = Document::default();
        let id = d.add_path(geom::circle_path(Point::new(50.0, 50.0), 20.0), Style { fill: Some(Rgba::BLACK), ..Style::default() }, 0);
        let mut tile = image::RgbaImage::new(2, 2);
        tile.put_pixel(0, 0, image::Rgba([0, 200, 0, 255]));
        tile.put_pixel(1, 0, image::Rgba([0, 200, 0, 255]));
        tile.put_pixel(0, 1, image::Rgba([200, 0, 0, 255]));
        tile.put_pixel(1, 1, image::Rgba([200, 0, 0, 255]));
        let g = pattern_fill(&mut d, id, &png(&tile), 10.0, 0.0).unwrap();
        let grp = d.page().shape(g[0]).unwrap();
        let ShapeKind::Image(img) = &grp.leaves()[0].kind else { panic!() };
        let pic = image::load_from_memory(&img.png).unwrap().to_rgba8();
        let px = |mm: f64| (mm / 25.4 * DPI) as u32;
        assert_eq!(pic.get_pixel(px(20.0), px(2.0)).0, [0, 200, 0, 255], "top of a tile");
        assert_eq!(pic.get_pixel(px(20.0), px(7.0)).0, [200, 0, 0, 255], "bottom of a tile");
        assert_eq!(pic.get_pixel(1, 1).0[3], 0, "corner outside the circle");
        let outline = &grp.leaves()[1];
        assert_eq!(outline.style.fill, None);
        assert_ne!(outline.style.cut, CutStyle::NoCut, "the outline is still cut");
    }
}
