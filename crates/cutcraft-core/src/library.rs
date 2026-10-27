//! Ready-made art: a shape library (heart, banner, speech bubble…), blank
//! templates (coasters, tags, bookmark…) and sign outlines. Everything is
//! returned centred on the origin in millimetres.
//!
//! Shape and template data ported from BurnCraft's `bc-geom/makers.rs` and
//! `burncraft/art.rs`.

use crate::geom;
use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use std::f64::consts::TAU;

fn centred_rect(w: f64, h: f64, r: f64) -> BezPath {
    let rect = Rect::new(-w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
    if r > 0.0 { geom::rounded_rect_path(rect, r.min(w.min(h) / 2.0)) } else { geom::rect_path(rect) }
}

fn circle(r: f64) -> BezPath {
    geom::circle_path(Point::ZERO, r)
}

fn at(x: f64, y: f64, p: BezPath) -> BezPath {
    Affine::translate((x, y)) * p
}

/// A circle with `bumps` rounded scallops around its edge.
fn scalloped(bumps: u32, radius: f64, depth: f64) -> BezPath {
    let n = bumps.max(3) as usize * 16;
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * TAU;
            let r = radius - depth + depth * (bumps as f64 * a / 2.0).sin().abs();
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    geom::polyline_path(&pts, true)
}

/// A star with many short points.
fn starburst(points: usize, outer: f64, inner: f64) -> BezPath {
    let pts: Vec<Point> = (0..points * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = std::f64::consts::PI * i as f64 / points as f64 - std::f64::consts::FRAC_PI_2;
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    geom::polyline_path(&pts, true)
}

// ------------------------------------------------------------------ shape library

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryShape {
    Heart,
    Arrow,
    SpeechBubble,
    Badge,
    Banner,
    Starburst,
    Cloud,
    Shield,
    Tag,
    House,
}

impl LibraryShape {
    pub const ALL: [LibraryShape; 10] = [
        LibraryShape::Heart,
        LibraryShape::Arrow,
        LibraryShape::SpeechBubble,
        LibraryShape::Badge,
        LibraryShape::Banner,
        LibraryShape::Starburst,
        LibraryShape::Cloud,
        LibraryShape::Shield,
        LibraryShape::Tag,
        LibraryShape::House,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LibraryShape::Heart => "Heart",
            LibraryShape::Arrow => "Arrow",
            LibraryShape::SpeechBubble => "Speech bubble",
            LibraryShape::Badge => "Badge",
            LibraryShape::Banner => "Banner",
            LibraryShape::Starburst => "Starburst",
            LibraryShape::Cloud => "Cloud",
            LibraryShape::Shield => "Shield",
            LibraryShape::Tag => "Gift tag",
            LibraryShape::House => "House",
        }
    }

    /// The shape, about `size` mm across, centred on the origin.
    pub fn path(self, size: f64) -> BezPath {
        let unit = self.unit();
        let b = unit.bounding_box();
        let s = size / b.width().max(b.height()).max(1e-9);
        Affine::scale(s) * Affine::translate(-b.center().to_vec2()) * unit
    }

    fn unit(self) -> BezPath {
        let svg = match self {
            LibraryShape::Heart => "M50,90 C20,70 0,50 0,28 C0,10 14,0 27,0 C38,0 46,7 50,16 C54,7 62,0 73,0 C86,0 100,10 100,28 C100,50 80,70 50,90 Z",
            LibraryShape::Arrow => "M0,35 L60,35 L60,10 L100,50 L60,90 L60,65 L0,65 Z",
            LibraryShape::SpeechBubble => "M15,0 L85,0 C93,0 100,7 100,15 L100,55 C100,63 93,70 85,70 L45,70 L22,92 L27,70 L15,70 C7,70 0,63 0,55 L0,15 C0,7 7,0 15,0 Z",
            LibraryShape::Banner => "M0,20 L15,20 L15,10 L85,10 L85,20 L100,20 L90,37 L100,54 L80,54 L80,44 L20,44 L20,54 L0,54 L10,37 Z M15,20 L15,44 L20,44 M85,20 L85,44 L80,44",
            LibraryShape::Cloud => "M25,70 C10,70 0,60 0,48 C0,36 10,27 22,28 C24,14 36,4 50,4 C63,4 73,12 76,24 C90,24 100,34 100,47 C100,60 90,70 77,70 Z",
            LibraryShape::Shield => "M50,0 C62,8 80,10 95,10 L95,45 C95,72 75,88 50,100 C25,88 5,72 5,45 L5,10 C20,10 38,8 50,0 Z",
            LibraryShape::Tag => "M25,0 L100,0 L100,50 L25,50 L0,25 Z M18,21 A4,4 0 1 0 18,29 A4,4 0 1 0 18,21 Z",
            LibraryShape::House => "M50,0 L100,45 L88,45 L88,100 L12,100 L12,45 L0,45 Z",
            LibraryShape::Badge => return scalloped(20, 50.0, 6.0),
            LibraryShape::Starburst => return starburst(24, 50.0, 40.0),
        };
        BezPath::from_svg(svg).expect("built-in shape data parses")
    }
}

// ------------------------------------------------------------------ templates

/// A ready-made blank: outline plus any holes, as one compound path.
pub struct Template {
    pub name: &'static str,
    pub build: fn() -> BezPath,
}

fn compound(parts: Vec<BezPath>) -> BezPath {
    geom::merge_paths(&parts)
}

pub const TEMPLATES: [Template; 9] = [
    Template { name: "Round coaster Ø90", build: || circle(45.0) },
    Template { name: "Square coaster 90", build: || centred_rect(90.0, 90.0, 8.0) },
    Template {
        name: "Hexagon coaster 95",
        build: || Affine::rotate(std::f64::consts::FRAC_PI_6) * geom::polygon_path(Rect::new(-47.5, -47.5, 47.5, 47.5), 6),
    },
    Template { name: "Ornament Ø75", build: || compound(vec![circle(37.5), at(0.0, -31.5, circle(2.0))]) },
    Template { name: "Keychain 50×25", build: || compound(vec![centred_rect(50.0, 25.0, 5.0), at(-19.0, 0.0, circle(2.5))]) },
    Template { name: "Dog tag 50×28", build: || compound(vec![centred_rect(50.0, 28.0, 14.0), at(-18.0, 0.0, circle(2.0))]) },
    Template {
        name: "Luggage tag 100×55",
        build: || compound(vec![centred_rect(100.0, 55.0, 5.0), at(-40.0, 0.0, centred_rect(4.0, 14.0, 2.0))]),
    },
    Template { name: "Bookmark 50×150", build: || compound(vec![centred_rect(50.0, 150.0, 5.0), at(0.0, -65.0, circle(2.5))]) },
    Template { name: "Name plate 200×60", build: || centred_rect(200.0, 60.0, 6.0) },
];

// ------------------------------------------------------------------ signs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignShape {
    Rectangle,
    Rounded,
    Oval,
    /// Corners cut inward as quarter circles.
    Plaque,
}

impl SignShape {
    pub const ALL: [(SignShape, &'static str); 4] =
        [(SignShape::Rectangle, "Rectangle"), (SignShape::Rounded, "Rounded"), (SignShape::Oval, "Oval"), (SignShape::Plaque, "Plaque")];
}

/// A sign blank `width` × `height` centred on the origin.
pub fn sign_outline(shape: SignShape, width: f64, height: f64, corner: f64) -> BezPath {
    let c = corner.clamp(0.0, width.min(height) / 2.0);
    match shape {
        SignShape::Rectangle => centred_rect(width, height, 0.0),
        SignShape::Rounded => centred_rect(width, height, c),
        SignShape::Oval => geom::ellipse_path(Rect::new(-width / 2.0, -height / 2.0, width / 2.0, height / 2.0)),
        SignShape::Plaque => {
            let (w, h) = (width / 2.0, height / 2.0);
            let mut p = BezPath::new();
            // Each corner is a quarter circle bitten out of the rectangle.
            p.move_to((-w + c, -h));
            p.line_to((w - c, -h));
            p.quad_to((w - c, -h + c), (w, -h + c));
            p.line_to((w, h - c));
            p.quad_to((w - c, h - c), (w - c, h));
            p.line_to((-w + c, h));
            p.quad_to((-w + c, h - c), (-w, h - c));
            p.line_to((-w, -h + c));
            p.quad_to((-w + c, -h + c), (-w + c, -h));
            p.close_path();
            p
        }
    }
}

/// Mounting holes: `count` holes of `diameter`, `inset` mm in from the edges of a
/// `width` × `height` sign centred on the origin (2 at the top, or 4 in the
/// corners; an oval gets them left and right of centre).
pub fn mounting_holes(count: u32, width: f64, height: f64, diameter: f64, inset: f64, oval: bool) -> BezPath {
    let r = diameter / 2.0;
    let (x, y) = (width / 2.0 - inset - r, height / 2.0 - inset - r);
    let centres: Vec<(f64, f64)> = match (count, oval) {
        (0, _) => vec![],
        (_, true) => vec![(-x, 0.0), (x, 0.0)],
        (1, _) => vec![(0.0, -y)],
        (2 | 3, _) => vec![(-x, -y), (x, -y)],
        _ => vec![(-x, -y), (x, -y), (x, y), (-x, y)],
    };
    compound(centres.into_iter().map(|(cx, cy)| at(cx, cy, circle(r))).collect())
}

// ------------------------------------------------------------------ monogram

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonogramFrame {
    None,
    Circle,
    Ring,
    Diamond,
}

impl MonogramFrame {
    pub const ALL: [(MonogramFrame, &'static str); 4] =
        [(MonogramFrame::None, "None"), (MonogramFrame::Circle, "Circle"), (MonogramFrame::Ring, "Ring"), (MonogramFrame::Diamond, "Diamond")];
}

/// A classic three-letter monogram centred on the origin: the large surname
/// initial in the middle, first and middle initials (60 % size) either side, and
/// an optional frame. Letters stay editable text. Ported from BurnCraft.
pub fn monogram(
    fonts: &crate::text::FontLibrary,
    [first, last, middle]: [&str; 3],
    family: &str,
    size: f64,
    frame: MonogramFrame,
) -> Option<Vec<crate::doc::Shape>> {
    use crate::doc::{Shape, ShapeKind, Style, TextData};
    // A text shape whose glyph box is centred on (x, 0); returns it and its width.
    let letter = |text: &str, size: f64, x: f64| -> Option<(Shape, f64)> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        let data = TextData { text: t.into(), family: family.into(), size, ..Default::default() };
        let outline = fonts.layout(&data);
        let b = outline.bounding_box();
        if !(b.width() > 0.0) {
            return None;
        }
        let mut s = Shape::new_path(0, BezPath::new(), Style { fill: Some(crate::doc::Rgba(40, 40, 40, 255)), ..Style::default() });
        s.transform = Affine::translate((x - b.center().x, -b.center().y));
        s.kind = ShapeKind::Text { data, outline };
        s.name = format!("Monogram {t}");
        Some((s, b.width()))
    };
    let (big, big_w) = letter(last, size, 0.0)?;
    let mut shapes = vec![big];
    let small = size * 0.6;
    let gap = size * 0.12;
    let mut reach = big_w / 2.0;
    for (text, side) in [(first, -1.0), (middle, 1.0)] {
        // lay out once to learn the width, then place beside the big letter
        if let Some((_, w)) = letter(text, small, 0.0) {
            let x = side * (big_w / 2.0 + gap + w / 2.0);
            reach = reach.max(x.abs() + w / 2.0);
            shapes.extend(letter(text, small, x).map(|(s, _)| s));
        }
    }
    let r = reach.max(size / 2.0) + size * 0.25;
    let frame_path = match frame {
        MonogramFrame::None => None,
        MonogramFrame::Circle => Some(circle(r)),
        MonogramFrame::Ring => Some(compound(vec![circle(r + size * 0.08), circle(r)])),
        MonogramFrame::Diamond => Some(Affine::rotate(std::f64::consts::FRAC_PI_4) * centred_rect(r * 1.35 * std::f64::consts::SQRT_2, r * 1.35 * std::f64::consts::SQRT_2, 0.0)),
    };
    if let Some(p) = frame_path {
        let mut f = crate::doc::Shape::new_path(0, p, Style::default());
        f.name = "Monogram frame".into();
        shapes.push(f);
    }
    Some(shapes)
}

/// Outlines scaled into a 0..1 box (keeping proportions), for preview tiles.
// ------------------------------------------------------------------ product templates

/// A product blank shown on the canvas to size and place a design on (as in
/// Cricut's Templates): its outline (mm, top-left at the origin), plus a
/// dashed "design area" where artwork usually goes. Never cut or printed.
pub struct Product {
    pub name: &'static str,
    /// Size choices: (label, width, height) in mm.
    pub sizes: &'static [(&'static str, f64, f64)],
    pub build: fn(f64, f64) -> (BezPath, Rect),
}

/// Cubic-free polyline outline helper.
fn outline(pts: &[(f64, f64)], w: f64, h: f64) -> BezPath {
    let v: Vec<Point> = pts.iter().map(|(x, y)| Point::new(x * w, y * h)).collect();
    geom::polyline_path(&v, true)
}

fn tshirt(w: f64, h: f64) -> (BezPath, Rect) {
    let body = outline(&[(0.36, 0.0), (0.42, 0.05), (0.5, 0.07), (0.58, 0.05), (0.64, 0.0), (0.86, 0.06), (1.0, 0.27), (0.85, 0.36), (0.8, 0.28), (0.8, 1.0), (0.2, 1.0), (0.2, 0.28), (0.15, 0.36), (0.0, 0.27), (0.14, 0.06)], w, h);
    (body, Rect::new(0.3 * w, 0.16 * h, 0.7 * w, 0.6 * h))
}

fn onesie(w: f64, h: f64) -> (BezPath, Rect) {
    let body = outline(&[(0.35, 0.0), (0.5, 0.06), (0.65, 0.0), (0.88, 0.08), (1.0, 0.3), (0.84, 0.36), (0.8, 0.3), (0.8, 0.8), (0.62, 1.0), (0.38, 1.0), (0.2, 0.8), (0.2, 0.3), (0.16, 0.36), (0.0, 0.3), (0.12, 0.08)], w, h);
    (body, Rect::new(0.3 * w, 0.18 * h, 0.7 * w, 0.62 * h))
}

fn mug(w: f64, h: f64) -> (BezPath, Rect) {
    // the printable wrap; the handle sits at the left/right ends
    (geom::rect_path(Rect::new(0.0, 0.0, w, h)), Rect::new(0.08 * w, 0.1 * h, 0.92 * w, 0.9 * h))
}

fn tote(w: f64, h: f64) -> (BezPath, Rect) {
    let mut p = geom::rect_path(Rect::new(0.0, 0.25 * h, w, h));
    for x in [0.22, 0.62] {
        p.extend(geom::rect_path(Rect::new(x * w, 0.0, (x + 0.16) * w, 0.25 * h)).elements().iter().copied());
    }
    (p, Rect::new(0.15 * w, 0.37 * h, 0.85 * w, 0.88 * h))
}

fn card(w: f64, h: f64) -> (BezPath, Rect) {
    // a folded card shown open flat: the right half is the front
    let mut p = geom::rect_path(Rect::new(0.0, 0.0, w, h));
    p.move_to((w / 2.0, 0.0));
    p.line_to((w / 2.0, h));
    (p, Rect::new(w / 2.0 + 0.08 * w, 0.08 * h, w - 0.08 * w, 0.92 * h))
}

fn pillow(w: f64, h: f64) -> (BezPath, Rect) {
    (Affine::translate((w / 2.0, h / 2.0)) * centred_rect(w, h, 0.06 * w.min(h)), Rect::new(0.15 * w, 0.15 * h, 0.85 * w, 0.85 * h))
}

pub const PRODUCTS: [Product; 6] = [
    Product { name: "T-shirt", sizes: &[("Youth", 420.0, 560.0), ("Adult S", 460.0, 680.0), ("Adult M", 510.0, 710.0), ("Adult L", 560.0, 740.0), ("Adult XL", 610.0, 760.0)], build: tshirt },
    Product { name: "Baby bodysuit", sizes: &[("0–3 m", 220.0, 330.0), ("3–6 m", 240.0, 360.0), ("6–12 m", 260.0, 390.0)], build: onesie },
    Product { name: "Mug wrap", sizes: &[("11 oz", 216.0, 89.0), ("15 oz", 222.0, 102.0), ("Tumbler 20 oz", 240.0, 165.0)], build: mug },
    Product { name: "Tote bag", sizes: &[("Small", 330.0, 450.0), ("Standard", 380.0, 520.0)], build: tote },
    Product { name: "Greeting card (open)", sizes: &[("A2 4¼×5½ in", 216.0, 140.0), ("A7 5×7 in", 254.0, 178.0), ("A6", 210.0, 148.0)], build: card },
    Product { name: "Cushion", sizes: &[("40 cm", 400.0, 400.0), ("45 cm", 450.0, 450.0)], build: pillow },
];

pub fn thumbnail(path: &BezPath) -> Vec<Vec<Point>> {
    let lines: Vec<Vec<Point>> = geom::flatten(path, 0.3)
        .into_iter()
        .map(|l| {
            let mut pts = l.pts;
            if l.closed {
                if let Some(&f) = pts.first() {
                    pts.push(f);
                }
            }
            pts
        })
        .filter(|l| l.len() >= 2)
        .collect();
    let b = path.bounding_box();
    let size = b.width().max(b.height());
    if !(size.is_finite() && size > 0.0) {
        return Vec::new();
    }
    let (ox, oy) = ((size - b.width()) / 2.0, (size - b.height()) / 2.0);
    lines.into_iter().map(|l| l.into_iter().map(|p| Point::new((p.x - b.x0 + ox) / size, (p.y - b.y0 + oy) / size)).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_shapes_are_centred_and_sized() {
        for s in LibraryShape::ALL {
            let p = s.path(60.0);
            let b = p.bounding_box();
            assert!((b.width().max(b.height()) - 60.0).abs() < 0.5, "{}: {b:?}", s.label());
            assert!(b.center().to_vec2().hypot() < 0.5, "{} centred", s.label());
        }
    }

    #[test]
    fn templates_are_sensible_blanks() {
        for t in &TEMPLATES {
            let p = (t.build)();
            let b = p.bounding_box();
            assert!(b.width() > 20.0 && b.height() > 20.0, "{}: {b:?}", t.name);
        }
        // the keychain has its outline plus a hole
        assert_eq!(geom::subpaths(&(TEMPLATES[4].build)()).len(), 2);
    }

    #[test]
    fn signs_and_holes() {
        for (s, n) in SignShape::ALL {
            let b = sign_outline(s, 200.0, 80.0, 15.0).bounding_box();
            assert!((b.width() - 200.0).abs() < 0.5 && (b.height() - 80.0).abs() < 0.5, "{n}: {b:?}");
        }
        assert_eq!(geom::subpaths(&mounting_holes(4, 200.0, 80.0, 5.0, 8.0, false)).len(), 4);
        assert_eq!(geom::subpaths(&mounting_holes(2, 200.0, 80.0, 5.0, 8.0, true)).len(), 2);
    }

    #[test]
    fn monogram_places_initials_and_frame() {
        let mut fonts = crate::text::FontLibrary::default();
        let mut f = crate::userfont::UserFont::new("Blocky");
        for c in ['A', 'B', 'C'] {
            f.set_glyph(c, &[geom::rect_path(Rect::new(0.0, -7.0, 5.0, 0.0))], 10.0, Some(0.0), 0.5).unwrap();
        }
        fonts.user.push(f);
        fonts.refresh_families();
        let shapes = monogram(&fonts, ["A", "B", "C"], "Blocky", 30.0, MonogramFrame::Ring).unwrap();
        assert_eq!(shapes.len(), 4, "three letters + frame");
        let centres: Vec<f64> = shapes[..3].iter().map(|s| s.bbox().unwrap().center().x).collect();
        assert!(centres[0].abs() < 1e-6, "surname initial centred");
        assert!(centres[1] < -1.0 && centres[2] > 1.0, "first left, middle right: {centres:?}");
        let big = shapes[0].bbox().unwrap().height();
        let small = shapes[1].bbox().unwrap().height();
        assert!((small / big - 0.6).abs() < 0.01, "side letters at 60 %");
        assert_eq!(geom::subpaths(&shapes[3].world_paths()[0]).len(), 2, "ring = two circles");
        // the frame encloses every letter
        let fr = shapes[3].bbox().unwrap();
        assert!(shapes[..3].iter().all(|s| fr.contains_rect(s.bbox().unwrap())));
    }

    /// Contact sheet of every shape, template, sign style and a monogram.
    /// `cargo test -p cutcraft-core preview_library -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_library() {
        let fonts = crate::text::FontLibrary::new();
        let mut items: Vec<(String, BezPath)> = Vec::new();
        for s in LibraryShape::ALL {
            items.push((s.label().into(), s.path(60.0)));
        }
        for t in &TEMPLATES {
            items.push((t.name.into(), (t.build)()));
        }
        for (s, n) in SignShape::ALL {
            let mut p = sign_outline(s, 200.0, 80.0, 16.0);
            p.extend(mounting_holes(4, 200.0, 80.0, 6.0, 8.0, s == SignShape::Oval).elements().iter().copied());
            items.push((format!("Sign: {n}"), p));
        }
        if let Some(m) = monogram(&fonts, ["J", "S", "A"], "Times New Roman", 40.0, MonogramFrame::Ring) {
            let parts: Vec<BezPath> = m.iter().flat_map(|s| s.world_paths()).collect();
            items.push(("Monogram".into(), geom::merge_paths(&parts)));
        }
        let (tile, cols) = (180u32, 6u32);
        let rows = (items.len() as u32).div_ceil(cols);
        let mut img = image::RgbaImage::from_pixel(cols * tile, rows * tile, image::Rgba([255, 255, 255, 255]));
        for (i, (_, path)) in items.iter().enumerate() {
            let (tx, ty) = ((i as u32 % cols) * tile, (i as u32 / cols) * tile);
            for l in thumbnail(path) {
                for seg in l.windows(2) {
                    let n = 200;
                    for j in 0..=n {
                        let q = seg[0].lerp(seg[1], j as f64 / n as f64);
                        let (x, y) = (tx + 10 + (q.x * (tile - 20) as f64) as u32, ty + 10 + (q.y * (tile - 20) as f64) as u32);
                        img.put_pixel(x.min(img.width() - 1), y.min(img.height() - 1), image::Rgba([30, 30, 30, 255]));
                    }
                }
            }
        }
        let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/library_preview.png");
        img.save(out).unwrap();
        for (i, (n, _)) in items.iter().enumerate() {
            print!("{i}:{n}  ");
        }
        println!("
wrote {out}");
    }

    #[test]
    fn thumbnail_fits_unit_box() {
        let t = thumbnail(&LibraryShape::Heart.path(50.0));
        assert!(t.iter().flatten().all(|p| (-1e-6..=1.0 + 1e-6).contains(&p.x) && (-1e-6..=1.0 + 1e-6).contains(&p.y)));
    }
}
