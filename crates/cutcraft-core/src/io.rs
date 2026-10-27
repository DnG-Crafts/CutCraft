//! File import / export.
//!
//! - `.cutcraft` native: gzip-compressed JSON of the Document.
//! - SVG import (usvg) and export.
//! - DXF import (LINE, LWPOLYLINE, POLYLINE, CIRCLE, ARC, SPLINE control polygon) and export (R12).
//! - PDF export for Print & Cut (vector, with registration marks).
//! - PNG/JPEG/GIF/BMP/TIFF import as image shapes.
//! - `.studio3` files are detected; their object stream is not decoded yet.

use crate::doc::{Document, ImageData, Page, Rgba, Shape, ShapeKind, Style};
use crate::geom::{self, Polyline};
use anyhow::{Context, Result, bail};
use kurbo::{Affine, BezPath, PathEl, Point, Shape as _};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

pub const NATIVE_EXT: &str = "cutcraft";

/// Saves safely: writes a temporary file beside `path`, then swaps it into
/// place, so a crash or full disk mid-save never damages the existing project.
pub fn save_native(doc: &Document, path: &Path) -> Result<()> {
    let json = serde_json::to_vec(doc)?;
    let tmp = path.with_extension("cutcraft.tmp");
    {
        let f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        gz.write_all(&json)?;
        gz.finish()?.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn load_native(path: &Path) -> Result<Document> {
    let f = std::fs::File::open(path)?;
    let mut s = Vec::new();
    flate2::read::GzDecoder::new(f).read_to_end(&mut s)?;
    let mut doc: Document = serde_json::from_slice(&s)?;
    if doc.active >= doc.pages.len() {
        doc.active = 0;
    }
    Ok(doc)
}

/// What kind of file a path is, by extension (and magic for studio files).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Native,
    Svg,
    Dxf,
    /// PDF, or Illustrator .ai (PDF inside).
    Pdf,
    /// HP-GL plotter file.
    Hpgl,
    /// Brother ScanNCut / CanvasWorkspace cut file.
    Fcm,
    Image,
    Studio3,
    Unknown,
}

pub fn kind_of(path: &Path) -> FileKind {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        NATIVE_EXT => FileKind::Native,
        "svg" | "svgz" => FileKind::Svg,
        "dxf" => FileKind::Dxf,
        "pdf" | "ai" => FileKind::Pdf,
        "plt" | "hpgl" | "hgl" => FileKind::Hpgl,
        "fcm" => FileKind::Fcm,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" => FileKind::Image,
        "studio3" | "studio" => FileKind::Studio3,
        _ => FileKind::Unknown,
    }
}

/// File extensions `import` reads (for open/import dialogs).
pub const IMPORT_EXTS: [&str; 18] = ["svg", "svgz", "dxf", "pdf", "ai", "plt", "hpgl", "hgl", "fcm", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "studio3"];

/// Import a file as shapes (page coordinates, mm). Shapes have id 0 (caller assigns).
pub fn import(path: &Path, fontdb: Option<Arc<fontdb::Database>>) -> Result<Vec<Shape>> {
    Ok(import_with_notes(path, fontdb)?.0)
}

/// Like [`import`], plus notes about anything left out (e.g. a PDF's text).
pub fn import_with_notes(path: &Path, fontdb: Option<Arc<fontdb::Database>>) -> Result<(Vec<Shape>, Vec<String>)> {
    match kind_of(path) {
        FileKind::Pdf => crate::pdfimport::import(&std::fs::read(path)?),
        FileKind::Hpgl => Ok((crate::hpglimport::import(&std::fs::read(path)?)?, vec![])),
        FileKind::Fcm => crate::fcm::import(&std::fs::read(path)?),
        _ => Ok((import_plain(path, fontdb)?, vec![])),
    }
}

fn import_plain(path: &Path, fontdb: Option<Arc<fontdb::Database>>) -> Result<Vec<Shape>> {
    match kind_of(path) {
        FileKind::Svg => import_svg(&std::fs::read(path)?, fontdb),
        FileKind::Dxf => import_dxf(&std::fs::read_to_string(path)?),
        FileKind::Image => Ok(vec![import_image(&std::fs::read(path)?)?]),
        FileKind::Studio3 => {
            let head = std::fs::read(path)?;
            if head.starts_with(b"silhouette05;") {
                bail!("This is a Silhouette Studio V3+ document. Its binary object stream is not supported yet - export it as SVG/DXF from Silhouette Studio and import that.");
            }
            bail!("Unrecognised Silhouette file")
        }
        FileKind::Native => Ok(load_native(path)?.page().shapes.clone()),
        FileKind::Pdf | FileKind::Hpgl | FileKind::Fcm => unreachable!("handled by import_with_notes"),
        FileKind::Unknown => bail!("Unsupported file type"),
    }
}

// ------------------------------------------------------------------ images

pub fn import_image(bytes: &[u8]) -> Result<Shape> {
    let img = image::load_from_memory(bytes)?;
    let (w, h) = (img.width(), img.height());
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    // assume 150 dpi for sizing, capped to 200 mm
    let mut wmm = w as f64 / 150.0 * 25.4;
    let mut hmm = h as f64 / 150.0 * 25.4;
    let k = (200.0 / wmm.max(hmm)).min(1.0);
    wmm *= k;
    hmm *= k;
    Ok(Shape {
        id: 0,
        name: String::new(),
        layer: 0,
        transform: Affine::IDENTITY,
        style: Style { fill: None, stroke: None, stroke_width: 0.0, cut: crate::doc::CutStyle::NoCut, no_print: false },
        kind: ShapeKind::Image(ImageData { png: Arc::new(png), px_w: w, px_h: h, width_mm: wmm, height_mm: hmm, original: None }),
        locked: false,
        hidden: false,
        effects: Vec::new(),
    })
}

// ------------------------------------------------------------------ SVG

const PX_TO_MM: f64 = 25.4 / 96.0;

pub fn import_svg(data: &[u8], fontdb: Option<Arc<fontdb::Database>>) -> Result<Vec<Shape>> {
    let mut opt = usvg::Options::default();
    if let Some(db) = fontdb {
        opt.fontdb = db;
    }
    let tree = usvg::Tree::from_data(data, &opt)?;
    let mut out = vec![];
    collect_svg(tree.root(), &mut out);
    Ok(out.into_iter().map(|p| Shape::new_path(0, p.path, p.style)).collect())
}

/// An SVG for cutting, such as an icon from the web: scaled so its longer
/// side is `size` mm, with stroked lines turned into outlines (an outline-
/// style icon then cuts as shapes, not as single lines down the middle of
/// each stroke). Filled parts stay as they are.
pub fn import_svg_for_cutting(data: &[u8], size: f64) -> Result<Vec<Shape>> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
    let mut pieces = vec![];
    collect_svg(tree.root(), &mut pieces);
    // the drawing's extent, strokes included
    let mut bb: Option<kurbo::Rect> = None;
    for p in &pieces {
        let half = p.stroke.map(|s| s.0 / 2.0).unwrap_or(0.0);
        bb = geom::union_rect(bb, p.path.bounding_box().inflate(half, half));
    }
    let Some(bb) = bb else { bail!("there's nothing to cut in that SVG") };
    let k = size / bb.width().max(bb.height()).max(1e-9);
    let a = Affine::scale(k) * Affine::translate(-bb.origin().to_vec2());
    let mut out = vec![];
    for p in pieces {
        let path = a * p.path;
        let colour = p.style.fill.or(p.style.stroke).unwrap_or(Rgba::BLACK);
        let mut region = if p.style.fill.is_some() { geom::union_each(&[path.clone()]) } else { BezPath::new() };
        if let Some((w, round_join, round_cap)) = p.stroke {
            let lines = geom::stroke_outline(&[path], w * k, round_join, round_cap);
            region = if region.elements().is_empty() { lines } else { geom::union_each(&[region, lines]) };
        }
        if region.elements().is_empty() {
            continue;
        }
        let style = Style { fill: Some(colour), stroke: None, stroke_width: 0.25, cut: crate::doc::CutStyle::Cut, no_print: false };
        out.push(Shape::new_path(0, region, style));
    }
    if out.is_empty() {
        bail!("there's nothing to cut in that SVG");
    }
    Ok(out)
}

/// One path from an SVG: its outline (mm), how it looks, and its stroke
/// (width in mm, round joins, round ends) if it has one.
struct SvgPiece {
    path: BezPath,
    style: Style,
    stroke: Option<(f64, bool, bool)>,
}

fn color_of(p: &usvg::Paint) -> Option<Rgba> {
    match p {
        usvg::Paint::Color(c) => Some(Rgba(c.red, c.green, c.blue, 255)),
        _ => Some(Rgba(128, 128, 128, 255)),
    }
}

fn collect_svg(g: &usvg::Group, out: &mut Vec<SvgPiece>) {
    for n in g.children() {
        match n {
            usvg::Node::Group(g) => collect_svg(g, out),
            usvg::Node::Text(t) => collect_svg(t.flattened(), out),
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let t = p.abs_transform();
                let a = Affine::scale(PX_TO_MM)
                    * Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64]);
                let mut b = BezPath::new();
                let pt = |p: usvg::tiny_skia_path::Point| Point::new(p.x as f64, p.y as f64);
                for seg in p.data().segments() {
                    use usvg::tiny_skia_path::PathSegment as S;
                    match seg {
                        S::MoveTo(p) => b.move_to(pt(p)),
                        S::LineTo(p) => b.line_to(pt(p)),
                        S::QuadTo(c, p) => b.quad_to(pt(c), pt(p)),
                        S::CubicTo(c1, c2, p) => b.curve_to(pt(c1), pt(c2), pt(p)),
                        S::Close => b.close_path(),
                    }
                }
                b.apply_affine(a);
                let fill = p.fill().and_then(|f| color_of(f.paint()));
                let stroke = p.stroke().and_then(|s| color_of(s.paint()));
                let style = Style {
                    fill,
                    stroke: stroke.or(if fill.is_none() { Some(Rgba::BLACK) } else { None }),
                    stroke_width: p.stroke().map(|s| s.width().get() as f64 * PX_TO_MM).unwrap_or(0.25),
                    cut: crate::doc::CutStyle::Cut,
                    no_print: false,
                };
                // the stroke width scales with the shape's transform
                let stroke = p.stroke().map(|s| {
                    let scale = (a.determinant().abs()).sqrt();
                    (s.width().get() as f64 * scale, s.linejoin() == usvg::LineJoin::Round, s.linecap() == usvg::LineCap::Round)
                });
                out.push(SvgPiece { path: b, style, stroke });
            }
            usvg::Node::Image(_) => {}
        }
    }
}

fn svg_d(paths: &[BezPath]) -> String {
    paths.iter().map(|p| p.to_svg()).collect::<Vec<_>>().join(" ")
}

pub fn export_svg(page: &Page, only_cut_lines: bool) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n",
        w = page.width,
        h = page.height
    );
    for shape in &page.shapes {
        if !page.is_visible(shape) {
            continue;
        }
        for leaf in shape.leaves() {
            if leaf.hidden {
                continue;
            }
            if let ShapeKind::Image(img) = &leaf.kind {
                if only_cut_lines {
                    continue;
                }
                let c = leaf.transform.as_coeffs();
                s += &format!(
                    "  <image transform=\"matrix({} {} {} {} {} {})\" width=\"{}\" height=\"{}\" xlink:href=\"data:image/png;base64,{}\"/>\n",
                    c[0], c[1], c[2], c[3], c[4], c[5], img.width_mm, img.height_mm, b64(&img.png)
                );
                continue;
            }
            let paths = if only_cut_lines { leaf.cut_paths() } else { leaf.world_paths() };
            if paths.is_empty() {
                continue;
            }
            // colour, plus an opacity attribute when it isn't opaque
            let paint = |c: Option<crate::doc::Rgba>, what: &str| match c {
                None => "\"none\"".to_string(),
                Some(c) if c.3 == 255 => format!("\"{}\"", c.rgb_hex()),
                Some(c) => format!("\"{}\" {what}-opacity=\"{:.3}\"", c.rgb_hex(), c.3 as f64 / 255.0),
            };
            let fill = if only_cut_lines { paint(None, "fill") } else { paint(leaf.style.fill, "fill") };
            let stroke = paint(leaf.style.stroke, "stroke");
            s += &format!(
                "  <path d=\"{}\" fill={} fill-rule=\"evenodd\" stroke={} stroke-width=\"{}\"/>\n",
                svg_d(&paths),
                fill,
                stroke,
                leaf.style.stroke_width.max(0.05)
            );
        }
    }
    s += "</svg>\n";
    s
}

fn b64(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in d.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

// ------------------------------------------------------------------ DXF

/// Minimal ASCII DXF reader (entities section only).
pub fn import_dxf(text: &str) -> Result<Vec<Shape>> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let mut pairs = vec![];
    let mut i = 0;
    while i + 1 < lines.len() {
        if let Ok(code) = lines[i].parse::<i32>() {
            pairs.push((code, lines[i + 1].to_string()));
        }
        i += 2;
    }
    // detect units ($INSUNITS 1 = inch, 4 = mm)
    let mut scale = 1.0;
    for w in pairs.windows(2) {
        if w[0].1 == "$INSUNITS" && w[1].0 == 70 && w[1].1.trim() == "1" {
            scale = 25.4;
        }
    }
    let mut paths: Vec<BezPath> = vec![];
    let mut k = 0;
    let get = |ent: &[(i32, String)], code: i32| -> Vec<f64> {
        ent.iter().filter(|p| p.0 == code).filter_map(|p| p.1.trim().parse().ok()).collect()
    };
    let mut in_entities = false;
    while k < pairs.len() {
        let (code, val) = &pairs[k];
        if *code == 2 && val == "ENTITIES" {
            in_entities = true;
        }
        if *code == 0 && val == "ENDSEC" {
            in_entities = false;
        }
        if *code != 0 || !in_entities {
            k += 1;
            continue;
        }
        let kind = val.clone();
        let start = k + 1;
        let mut end = start;
        while end < pairs.len() && pairs[end].0 != 0 {
            end += 1;
        }
        let ent = &pairs[start..end];
        match kind.as_str() {
            "LINE" => {
                let (x1, y1, x2, y2) = (get(ent, 10), get(ent, 20), get(ent, 11), get(ent, 21));
                if !x1.is_empty() && !y1.is_empty() && !x2.is_empty() && !y2.is_empty() {
                    paths.push(geom::polyline_path(&[Point::new(x1[0], y1[0]), Point::new(x2[0], y2[0])], false));
                }
            }
            "LWPOLYLINE" => {
                let (xs, ys) = (get(ent, 10), get(ent, 20));
                let closed = get(ent, 70).first().map(|f| (*f as i32) & 1 == 1).unwrap_or(false);
                let pts: Vec<Point> = xs.iter().zip(ys.iter()).map(|(x, y)| Point::new(*x, *y)).collect();
                if pts.len() > 1 {
                    paths.push(geom::polyline_path(&pts, closed));
                }
            }
            "POLYLINE" => {
                let closed = get(ent, 70).first().map(|f| (*f as i32) & 1 == 1).unwrap_or(false);
                let mut pts = vec![];
                let mut j = end;
                while j < pairs.len() {
                    if pairs[j].0 == 0 && pairs[j].1 == "VERTEX" {
                        let mut e2 = j + 1;
                        while e2 < pairs.len() && pairs[e2].0 != 0 {
                            e2 += 1;
                        }
                        let v = &pairs[j + 1..e2];
                        if let (Some(x), Some(y)) = (get(v, 10).first().copied(), get(v, 20).first().copied()) {
                            pts.push(Point::new(x, y));
                        }
                        j = e2;
                    } else if pairs[j].0 == 0 && pairs[j].1 == "SEQEND" {
                        break;
                    } else {
                        j += 1;
                    }
                }
                end = j;
                if pts.len() > 1 {
                    paths.push(geom::polyline_path(&pts, closed));
                }
            }
            "CIRCLE" => {
                if let (Some(x), Some(y), Some(r)) =
                    (get(ent, 10).first().copied(), get(ent, 20).first().copied(), get(ent, 40).first().copied())
                {
                    paths.push(geom::circle_path(Point::new(x, y), r));
                }
            }
            "ARC" => {
                if let (Some(x), Some(y), Some(r), Some(a0), Some(a1)) = (
                    get(ent, 10).first().copied(),
                    get(ent, 20).first().copied(),
                    get(ent, 40).first().copied(),
                    get(ent, 50).first().copied(),
                    get(ent, 51).first().copied(),
                ) {
                    let mut sweep = a1 - a0;
                    if sweep <= 0.0 {
                        sweep += 360.0;
                    }
                    let arc = kurbo::Arc::new(Point::new(x, y), (r, r), a0.to_radians(), sweep.to_radians(), 0.0);
                    paths.push(arc.to_path(0.01));
                }
            }
            "SPLINE" => {
                let (xs, ys) = (get(ent, 10), get(ent, 20));
                let pts: Vec<Point> = xs.iter().zip(ys.iter()).map(|(x, y)| Point::new(*x, *y)).collect();
                if pts.len() > 1 {
                    let pl = Polyline { pts, closed: false };
                    paths.push(geom::smooth_polyline(&pl, 180.0));
                }
            }
            _ => {}
        }
        k = end;
    }
    if paths.is_empty() {
        bail!("No supported entities found in DXF");
    }
    // DXF is y-up: flip into page space and move to origin
    let bb = geom::paths_bbox(&paths).unwrap();
    let a = Affine::scale(scale) * Affine::new([1.0, 0.0, 0.0, -1.0, -bb.x0, bb.y1]);
    Ok(paths
        .into_iter()
        .map(|mut p| {
            p.apply_affine(a);
            Shape::new_path(0, p, Style { stroke: Some(Rgba::BLACK), ..Style::default() })
        })
        .collect())
}

pub fn export_dxf(page: &Page) -> String {
    let mut s = String::from("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for shape in &page.shapes {
        if !page.is_visible(shape) {
            continue;
        }
        for leaf in shape.leaves() {
            if leaf.is_image() {
                continue;
            }
            for pl in geom::flatten_all(&leaf.world_paths(), 0.02) {
                s += "0\nPOLYLINE\n8\n0\n66\n1\n70\n";
                s += if pl.closed { "1\n" } else { "0\n" };
                for p in &pl.pts {
                    s += &format!("0\nVERTEX\n8\n0\n10\n{:.4}\n20\n{:.4}\n", p.x, page.height - p.y);
                }
                s += "0\nSEQEND\n";
            }
        }
    }
    s += "0\nENDSEC\n0\nEOF\n";
    s
}

// ------------------------------------------------------------------ PDF (print & cut)

/// Write a single-page vector PDF of the printable artwork plus registration marks.
pub fn export_print_pdf(page: &Page, path: &Path) -> Result<()> {
    let mm = 72.0 / 25.4;
    let (w, h) = (page.width * mm, page.height * mm);
    let mut content = String::new();
    let mut images: Vec<(String, Vec<u8>, u32, u32)> = vec![];
    // PDF y-up: flip
    content += &format!("1 0 0 -1 0 {h} cm {mm} 0 0 {mm} 0 0 cm\n");
    let path_ops = |paths: &[BezPath]| -> String {
        let mut o = String::new();
        for p in paths {
            for el in p.elements() {
                match el {
                    PathEl::MoveTo(p) => o += &format!("{:.3} {:.3} m\n", p.x, p.y),
                    PathEl::LineTo(p) => o += &format!("{:.3} {:.3} l\n", p.x, p.y),
                    PathEl::QuadTo(c, p) => o += &format!("{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n", c.x, c.y, c.x, c.y, p.x, p.y),
                    PathEl::CurveTo(a, b, p) => o += &format!("{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c\n", a.x, a.y, b.x, b.y, p.x, p.y),
                    PathEl::ClosePath => o += "h\n",
                }
            }
        }
        o
    };
    for shape in &page.shapes {
        if !page.is_visible(shape) {
            continue;
        }
        for leaf in shape.leaves() {
            if leaf.hidden || leaf.style.no_print {
                continue;
            }
            if let ShapeKind::Image(img) = &leaf.kind {
                let dec = image::load_from_memory(&img.png)?.to_rgb8();
                let name = format!("Im{}", images.len());
                let c = leaf.transform.as_coeffs();
                content += &format!(
                    "q {} {} {} {} {} {} cm {} 0 0 {} 0 0 cm 1 0 0 -1 0 1 cm /{} Do Q\n",
                    c[0], c[1], c[2], c[3], c[4], c[5], img.width_mm, img.height_mm, name
                );
                images.push((name, dec.into_raw(), img.px_w, img.px_h));
                continue;
            }
            let paths = leaf.world_paths();
            if let Some(f) = leaf.style.fill {
                content += &format!("{:.3} {:.3} {:.3} rg\n", f.0 as f64 / 255.0, f.1 as f64 / 255.0, f.2 as f64 / 255.0);
                content += &path_ops(&paths);
                content += "f*\n";
            }
        }
    }
    let marks = page.regmarks.print_geometry(page.width, page.height);
    if !marks.is_empty() {
        content += "0 0 0 rg\n";
        content += &path_ops(&marks);
        content += "f\n";
    }

    // assemble objects
    let mut objs: Vec<Vec<u8>> = vec![];
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objs.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
    let mut xobjs = String::new();
    for (i, (name, _, _, _)) in images.iter().enumerate() {
        xobjs += &format!("/{} {} 0 R ", name, 5 + i);
    }
    objs.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Contents 4 0 R /Resources << /XObject << {xobjs}>> >> >>"
        )
        .into_bytes(),
    );
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend(content.as_bytes());
    stream.extend(b"\nendstream");
    objs.push(stream);
    for (_, rgb, pw, ph) in &images {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(rgb)?;
        let data = z.finish()?;
        let mut o = format!(
            "<< /Type /XObject /Subtype /Image /Width {pw} /Height {ph} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        o.extend(data);
        o.extend(b"\nendstream");
        objs.push(o);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes());
    std::fs::write(path, out)?;
    Ok(())
}

// ------------------------------------------------------------------ G-code

#[derive(Clone, Debug, PartialEq)]
pub struct GcodeOptions {
    /// Z height for travel moves (pen up), mm.
    pub travel_z: f64,
    /// Z height for cutting / drawing (pen down), mm.
    pub cut_z: f64,
    /// Cutting feed rate, mm/min.
    pub feed: f64,
    /// Travel (pen-up) feed rate, mm/min.
    pub travel_feed: f64,
    /// Use M3/M5 (spindle/laser on-off) instead of Z moves.
    pub use_spindle: bool,
    /// Spindle speed / laser power for M3 S….
    pub spindle_power: f64,
    /// Flatten curves to this tolerance, mm.
    pub tolerance: f64,
}

impl Default for GcodeOptions {
    fn default() -> Self {
        GcodeOptions {
            travel_z: 3.0,
            cut_z: -1.0,
            feed: 800.0,
            travel_feed: 2000.0,
            use_spindle: false,
            spindle_power: 1000.0,
            tolerance: 0.05,
        }
    }
}

/// Export the page's cut lines as G-code. Y is measured from the bottom-left.
pub fn export_gcode(page: &Page, opt: &GcodeOptions) -> String {
    let mut g = String::new();
    g += "; CutCraft G-code\n";
    g += "G21 ; mm\nG90 ; absolute\nG17 ; XY plane\n";
    let up = |g: &mut String| {
        if opt.use_spindle {
            *g += "M5 ; tool up\n";
        } else {
            *g += &format!("G0 Z{:.3} F{:.0}\n", opt.travel_z, opt.travel_feed);
        }
    };
    let down = |g: &mut String| {
        if opt.use_spindle {
            *g += &format!("M3 S{:.0} ; tool down\n", opt.spindle_power);
        } else {
            *g += &format!("G1 Z{:.3} F{:.0}\n", opt.cut_z, opt.feed);
        }
    };
    up(&mut g);
    let y = |p: Point| page.height - p.y;
    for shape in &page.shapes {
        if !page.is_visible(shape) {
            continue;
        }
        for leaf in shape.leaves() {
            if leaf.hidden || leaf.is_image() {
                continue;
            }
            for pl in geom::flatten_all(&leaf.cut_paths(), opt.tolerance) {
                if pl.pts.is_empty() {
                    continue;
                }
                let mut pts = pl.pts.clone();
                if pl.closed {
                    pts.push(pts[0]);
                }
                g += &format!("G0 X{:.3} Y{:.3} F{:.0}\n", pts[0].x, y(pts[0]), opt.travel_feed);
                down(&mut g);
                for p in &pts[1..] {
                    g += &format!("G1 X{:.3} Y{:.3} F{:.0}\n", p.x, y(*p), opt.feed);
                }
                up(&mut g);
            }
        }
    }
    g += "G0 X0 Y0\nM2 ; end\n";
    g
}

#[cfg(test)]
mod tests {
    #[test]
    fn stroked_svg_lines_become_cuttable_outlines() {
        use kurbo::Shape as _;
        // a circle drawn as a 2-unit stroke (radius 10) in a 24-unit icon
        let ring = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="none" stroke="black" stroke-width="2"/></svg>"#;
        let s = import_svg_for_cutting(ring, 24.0).unwrap();
        assert_eq!(s.len(), 1);
        let area: f64 = s[0].world_paths().iter().map(|p| p.area()).sum::<f64>().abs();
        // scaled so the outer edge (radius 11) fills 24 mm: the ring is 24/22 × 2 mm wide
        let k: f64 = 24.0 / 22.0;
        let want = std::f64::consts::PI * ((11.0 * k).powi(2) - (9.0 * k).powi(2));
        assert!((area - want).abs() / want < 0.03, "{area} vs {want}");
        // a filled shape stays filled
        let dot = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"#;
        let s = import_svg_for_cutting(dot, 50.0).unwrap();
        let area: f64 = s[0].world_paths().iter().map(|p| p.area()).sum::<f64>().abs();
        assert!((area - 2500.0).abs() < 5.0, "{area}");
    }

    use super::*;
    use kurbo::Rect;

    #[test]
    fn gcode_has_moves() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 30.0, 30.0)), Style::default(), 0);
        let g = export_gcode(d.page(), &GcodeOptions::default());
        assert!(g.contains("G21"));
        assert!(g.matches("G1 X").count() >= 4);
        assert!(g.contains("M2"));
    }

    #[test]
    fn native_roundtrip() {
        let mut d = Document::default();
        d.add_path(geom::ellipse_path(Rect::new(0.0, 0.0, 30.0, 20.0)), Style::default(), 0);
        let p = std::env::temp_dir().join("cc_test.cutcraft");
        save_native(&d, &p).unwrap();
        let d2 = load_native(&p).unwrap();
        assert_eq!(d2.page().shapes.len(), 1);
        let a = d.page().shapes[0].bbox().unwrap();
        let b = d2.page().shapes[0].bbox().unwrap();
        assert!((a.width() - b.width()).abs() < 1e-6);
    }

    #[test]
    fn svg_roundtrip() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 60.0, 40.0)), Style::default(), 0);
        let svg = export_svg(d.page(), false);
        let shapes = import_svg(svg.as_bytes(), None).unwrap();
        assert_eq!(shapes.len(), 1);
        let bb = shapes[0].bbox().unwrap();
        assert!((bb.width() - 50.0).abs() < 0.05, "{bb:?}");
    }

    #[test]
    fn dxf_roundtrip() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 60.0, 40.0)), Style::default(), 0);
        let s = export_dxf(d.page());
        let shapes = import_dxf(&s).unwrap();
        let bb = shapes[0].bbox().unwrap();
        assert!((bb.width() - 50.0).abs() < 0.01 && (bb.height() - 30.0).abs() < 0.01);
    }

    #[test]
    fn pdf_writes() {
        let mut d = Document::default();
        let mut st = Style::default();
        st.fill = Some(Rgba(0, 128, 255, 255));
        d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 60.0, 40.0)), st, 0);
        d.page_mut().regmarks.enabled = true;
        let p = std::env::temp_dir().join("cc_test.pdf");
        export_print_pdf(d.page(), &p).unwrap();
        assert!(std::fs::read(&p).unwrap().starts_with(b"%PDF"));
    }
}
