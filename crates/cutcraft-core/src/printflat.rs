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

/// The page as it prints, `dpi` dots per inch, on white: what the Print &
/// Cut PDF has - pictures, filled shapes and the registration marks (lines
/// that are only cut, and anything set not to print, are left out). The
/// resolution is lowered for very large pages so it stays under ~20,000 px
/// a side; the returned dpi is what was used.
pub fn render_print(page: &crate::doc::Page, dpi: f64) -> (image::RgbaImage, f64) {
    let area = page.rect();
    let longest = page.width.max(page.height) / 25.4;
    let dpi = dpi.min(20_000.0 / longest.max(1e-9)).max(10.0);
    let k = dpi / 25.4;
    let (w, h) = ((page.width * k).ceil().max(1.0) as usize, (page.height * k).ceil().max(1.0) as usize);
    let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([255, 255, 255, 255]));
    const SS: usize = 2;
    for s in page.shapes.iter().filter(|s| page.is_visible(s)) {
        for leaf in s.leaves() {
            if leaf.hidden || leaf.style.no_print {
                continue;
            }
            match &leaf.kind {
                ShapeKind::Image(d) => draw_image(&mut img, area, &leaf, d),
                _ => {
                    if let Some(c) = leaf.style.fill {
                        let cov = coverage(&leaf.world_paths(), area, w, h, SS);
                        paint(&mut img, &cov, (SS * SS) as u16, [c.0, c.1, c.2, c.3]);
                    }
                }
            }
        }
    }
    let marks = page.regmarks.print_geometry(page.width, page.height);
    if !marks.is_empty() {
        let cov = coverage(&marks, area, w, h, SS);
        paint(&mut img, &cov, (SS * SS) as u16, [0, 0, 0, 255]);
    }
    (img, dpi)
}

/// Nearest-pixel draw of a picture leaf into `img` (covering `area`).
fn draw_image(img: &mut image::RgbaImage, area: Rect, leaf: &Shape, d: &ImageData) {
    let Ok(src) = image::load_from_memory(&d.png) else { return };
    let src = src.to_rgba8();
    let inv = leaf.transform.inverse();
    let (w, h) = (img.width(), img.height());
    let (px_w, px_h) = (area.width() / w as f64, area.height() / h as f64);
    // only the pixels the picture covers
    let corners = [(0.0, 0.0), (d.width_mm, 0.0), (d.width_mm, d.height_mm), (0.0, d.height_mm)].map(|c| leaf.transform * Point::from(c));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        (x0, y0, x1, y1) = (x0.min(c.x), y0.min(c.y), x1.max(c.x), y1.max(c.y));
    }
    // mm from the area's edge → a pixel column or row, kept on the image
    let first = |mm: f64, size: f64, n: u32| ((mm / size).floor().max(0.0) as u32).min(n);
    let last = |mm: f64, size: f64, n: u32| ((mm / size).ceil().max(0.0) as u32).min(n);
    let (cx0, cx1) = (first(x0 - area.x0, px_w, w), last(x1 - area.x0, px_w, w));
    let (cy0, cy1) = (first(y0 - area.y0, px_h, h), last(y1 - area.y0, px_h, h));
    for y in cy0..cy1 {
        for x in cx0..cx1 {
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

/// A shape's outline for the cut line: for a picture with see-through
/// parts (its background removed, or a shape cut out of it), the outline of
/// what's visible rather than its rectangular frame.
fn outline_of(leaf: &Shape) -> Vec<BezPath> {
    if let ShapeKind::Image(d) = &leaf.kind {
        if let Ok(img) = image::load_from_memory(&d.png) {
            let img = img.to_rgba8();
            // a plain white background is blank paper: cut round what's on it
            // (unless that leaves nothing - an all-white picture)
            let cleaned = crate::raster::without_white_backdrop(&img).and_then(|c| crate::trace::visible_outline(&c));
            if let Some(px) = cleaned.or_else(|| crate::trace::visible_outline(&img)) {
                let to_mm = kurbo::Affine::scale_non_uniform(d.width_mm / img.width().max(1) as f64, d.height_mm / img.height().max(1) as f64);
                return vec![leaf.transform * (to_mm * px)];
            }
        }
    }
    leaf.world_paths()
}

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
    let all: Vec<BezPath> = shapes.iter().flat_map(|s| s.leaves()).filter(|l| !l.hidden).flat_map(|l| outline_of(&l)).collect();
    let outline = crate::effects::outer_silhouette(&all);
    let layer = shapes[0].layer;
    let idx = page.shapes.iter().position(|s| sel.contains(&s.id)).unwrap_or(0);
    let picture = image_shape(doc, &img, area, layer);
    let cut_id = doc.alloc_id();
    // the cut line in its layer's colour, like other cut lines
    let colour = doc.page().layer(layer).map(|l| l.color).unwrap_or(Rgba(40, 40, 40, 255));
    let mut cut = Shape::new_path(cut_id, outline, Style { fill: None, stroke: Some(colour), no_print: true, ..Style::default() });
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
    fn printing_renders_fills_and_leaves_cut_lines_out() {
        let mut doc = Document::default();
        {
            let p = doc.page_mut();
            p.width = 100.0;
            p.height = 50.0;
        }
        let red = Style { fill: Some(Rgba(255, 0, 0, 255)), ..Style::default() };
        let line = Style { stroke: Some(Rgba(0, 0, 255, 255)), ..Style::default() };
        doc.add_path(geom::rect_path(Rect::new(10.0, 10.0, 30.0, 20.0)), red, 0);
        doc.add_path(geom::rect_path(Rect::new(50.0, 10.0, 90.0, 40.0)), line, 0);
        let (img, dpi) = render_print(doc.page(), 100.0);
        assert_eq!(dpi, 100.0);
        assert_eq!((img.width(), img.height()), (394, 197));
        let at = |x: f64, y: f64| img.get_pixel((x / 25.4 * dpi) as u32, (y / 25.4 * dpi) as u32).0;
        assert_eq!(at(20.0, 15.0), [255, 0, 0, 255], "the fill prints");
        assert_eq!(at(50.0, 25.0), [255, 255, 255, 255], "a cut-only outline doesn't");
        assert_eq!(at(70.0, 25.0), [255, 255, 255, 255]);
    }

    #[test]
    fn a_white_background_picture_flattens_to_a_cut_line_round_whats_on_it() {
        // a black 40 x 40 px square in the middle of a white 100 x 100 px picture, 40 x 40 mm
        let mut img = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
        for y in 30..70 {
            for x in 30..70 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut pic = crate::io::import_image(&png).unwrap();
        if let ShapeKind::Image(d) = &mut pic.kind {
            (d.width_mm, d.height_mm) = (40.0, 40.0);
        }
        let mut doc = Document::default();
        pic.id = doc.alloc_id();
        let p = pic.id;
        doc.page_mut().shapes.push(pic);
        let g = flatten(&mut doc, &[p]);
        let grp = doc.page().shape(g[0]).unwrap();
        let cut = &grp.leaves()[1];
        // round the 16 x 16 mm square, not the 40 x 40 mm picture
        use geo::Area;
        let area = geom::to_multipolygon(&cut.world_paths()).unsigned_area();
        assert!((area - 256.0).abs() / 256.0 < 0.08, "{area}");
        // in the layer's colour
        assert_eq!(cut.style.stroke, Some(doc.page().layers[0].color));
    }

    #[test]
    fn a_picture_with_a_shape_cut_out_flattens_to_its_visible_outline() {
        // a white 100 x 100 px picture, 40 x 40 mm, with a circle cut out of a corner
        let mut png = vec![];
        image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut pic = crate::io::import_image(&png).unwrap();
        if let ShapeKind::Image(d) = &mut pic.kind {
            (d.width_mm, d.height_mm) = (40.0, 40.0);
        }
        let mut doc = Document::default();
        pic.id = doc.alloc_id();
        let p = pic.id;
        doc.page_mut().shapes.push(pic);
        let c = doc.add_path(geom::circle_path(Point::new(0.0, 0.0), 20.0), Style::default(), 0);
        crate::ops::image_mask(&mut doc, &[p, c], false, true).unwrap();
        let g = flatten(&mut doc, &[p]);
        let grp = doc.page().shape(g[0]).unwrap();
        let cut = &grp.leaves()[1];
        // the square less a quarter circle, not the whole square
        use geo::Area;
        let area = geom::to_multipolygon(&cut.world_paths()).unsigned_area();
        let want = 40.0 * 40.0 - std::f64::consts::PI * 400.0 / 4.0;
        assert!((area - want).abs() / want < 0.03, "{area} vs {want}");
    }

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
