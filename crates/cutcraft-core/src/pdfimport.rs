//! PDF and Adobe Illustrator (.ai) import, ported from BurnCraft.
//!
//! Illustrator files since CS are PDF files inside, so both go through the
//! same reader. The first page's drawing operators are interpreted: paths
//! (lines, curves, rectangles) under the current transform, with nested form
//! XObjects, and each path's stroke/fill colour. PDF units are points (1/72
//! inch) with Y up; shapes come out in mm with Y down. Text and embedded images
//! are reported in the notes, not imported.

use crate::doc::{CutStyle, Rgba, Shape, Style};
use anyhow::{Result, anyhow, bail};
use kurbo::{Affine, BezPath, Point};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};

const MM_PER_PT: f64 = 25.4 / 72.0;
/// Deepest nesting of form XObjects followed (guards against loops).
const MAX_DEPTH: usize = 12;

#[derive(Clone)]
struct GraphicsState {
    ctm: Affine,
    stroke: Rgba,
    fill: Rgba,
    line_width: f64,
}

#[derive(Default)]
struct Out {
    shapes: Vec<Shape>,
    text: usize,
    images: usize,
}

/// Import the first page's vector paths; returns the shapes and notes about
/// anything left out.
pub fn import(data: &[u8]) -> Result<(Vec<Shape>, Vec<String>)> {
    if !data.starts_with(b"%PDF") && !data.windows(5).take(1024).any(|w| w == b"%PDF-") {
        if data.starts_with(b"%!PS") {
            bail!("This is an old PostScript-based Illustrator file - open it in Illustrator and save it again as a newer .ai or PDF.");
        }
        bail!("Not a PDF (or PDF-compatible Illustrator) file");
    }
    let doc = Document::load_mem(data).map_err(|e| anyhow!("Couldn't read the PDF: {e}"))?;
    let pages = doc.get_pages();
    let (_, &page_id) = pages.iter().next().ok_or_else(|| anyhow!("The file has no pages"))?;
    let media = media_box(&doc, page_id).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    // page space (points, Y up from the bottom of the page) → mm, Y down
    let to_doc = Affine::scale(MM_PER_PT) * Affine::new([1.0, 0.0, 0.0, -1.0, -media[0], media[3]]);
    let content = doc.get_and_decode_page_content(page_id).map_err(|e| anyhow!("Couldn't read the page drawing: {e}"))?;
    let (resources, _) = doc.get_page_resources(page_id).unwrap_or((None, Vec::new()));
    let mut out = Out::default();
    let gs = GraphicsState { ctm: Affine::IDENTITY, stroke: Rgba::BLACK, fill: Rgba::BLACK, line_width: 1.0 };
    run(&doc, &content.operations, resources, gs, to_doc, &mut out, 0);
    let mut notes = vec![];
    if pages.len() > 1 {
        notes.push(format!("page 1 of {} only", pages.len()));
    }
    if out.text > 0 {
        notes.push("text skipped (convert it to outlines first)".into());
    }
    if out.images > 0 {
        notes.push(format!("{} image(s) skipped", out.images));
    }
    Ok((out.shapes, notes))
}

/// The page's MediaBox (inherited from parent page-tree nodes if needed).
fn media_box(doc: &Document, page_id: ObjectId) -> Option<[f64; 4]> {
    let mut dict = doc.get_object(page_id).ok()?.as_dict().ok()?;
    for _ in 0..32 {
        if let Ok(b) = dict.get(b"MediaBox").and_then(|o| doc.dereference(o).map(|(_, o)| o)).and_then(|o| o.as_array()) {
            let v: Vec<f64> = b.iter().filter_map(number).collect();
            if v.len() == 4 {
                return Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]);
            }
        }
        let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
        dict = doc.get_object(parent).ok()?.as_dict().ok()?;
    }
    None
}

fn number(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn rgb_from(ops: &[Object], space: usize) -> Option<Rgba> {
    let v: Vec<f64> = ops.iter().filter_map(number).collect();
    let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    match (space, v.as_slice()) {
        (1, [g, ..]) => Some(Rgba(to(*g), to(*g), to(*g), 255)),
        (3, [r, g, b, ..]) => Some(Rgba(to(*r), to(*g), to(*b), 255)),
        (4, [c, m, y, k, ..]) => Some(Rgba(to((1.0 - c) * (1.0 - k)), to((1.0 - m) * (1.0 - k)), to((1.0 - y) * (1.0 - k)), 255)),
        _ => None,
    }
}

/// Interprets drawing operators. `to_doc` maps page space to document mm.
fn run(doc: &Document, ops: &[Operation], resources: Option<&Dictionary>, mut gs: GraphicsState, to_doc: Affine, out: &mut Out, depth: usize) {
    let mut stack: Vec<GraphicsState> = Vec::new();
    let mut path = BezPath::new();
    let mut current = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    let n = |o: &Operation, i: usize| o.operands.get(i).and_then(number).unwrap_or(0.0);
    for op in ops {
        let p = |i: usize| Point::new(n(op, i), n(op, i + 1));
        match op.operator.as_str() {
            "q" => stack.push(gs.clone()),
            "Q" => {
                if let Some(s) = stack.pop() {
                    gs = s;
                }
            }
            "cm" => gs.ctm *= Affine::new([n(op, 0), n(op, 1), n(op, 2), n(op, 3), n(op, 4), n(op, 5)]),
            "w" => gs.line_width = n(op, 0),
            "m" => {
                current = p(0);
                start = current;
                path.move_to(current);
            }
            "l" => {
                current = p(0);
                path.line_to(current);
            }
            "c" => {
                path.curve_to(p(0), p(2), p(4));
                current = p(4);
            }
            // first control point is the current point
            "v" => {
                path.curve_to(current, p(0), p(2));
                current = p(2);
            }
            // second control point is the end point
            "y" => {
                path.curve_to(p(0), p(2), p(2));
                current = p(2);
            }
            "h" => {
                path.close_path();
                current = start;
            }
            "re" => {
                let (x, y, w, h) = (n(op, 0), n(op, 1), n(op, 2), n(op, 3));
                path.move_to((x, y));
                path.line_to((x + w, y));
                path.line_to((x + w, y + h));
                path.line_to((x, y + h));
                path.close_path();
                current = Point::new(x, y);
                start = current;
            }
            // painting: stroke and/or fill end the path
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                let o = op.operator.as_str();
                if matches!(o, "s" | "b" | "b*") {
                    path.close_path();
                }
                let stroked = matches!(o, "S" | "s" | "B" | "B*" | "b" | "b*");
                let filled = !matches!(o, "S" | "s");
                if !path.elements().is_empty() {
                    let t = to_doc * gs.ctm;
                    // the line width scaled by the transform (mean of its axes)
                    let k = (t.determinant().abs()).sqrt();
                    let style = Style {
                        fill: filled.then_some(gs.fill),
                        stroke: stroked.then_some(gs.stroke),
                        stroke_width: if stroked { (gs.line_width * k).max(0.05) } else { 0.25 },
                        cut: CutStyle::Cut,
                        no_print: false,
                    };
                    out.shapes.push(Shape::new_path(0, t * std::mem::take(&mut path), style));
                }
                path = BezPath::new();
            }
            // end the path without painting (used after setting a clip)
            "n" => path = BezPath::new(),
            "RG" => gs.stroke = rgb_from(&op.operands, 3).unwrap_or(gs.stroke),
            "rg" => gs.fill = rgb_from(&op.operands, 3).unwrap_or(gs.fill),
            "G" => gs.stroke = rgb_from(&op.operands, 1).unwrap_or(gs.stroke),
            "g" => gs.fill = rgb_from(&op.operands, 1).unwrap_or(gs.fill),
            "K" => gs.stroke = rgb_from(&op.operands, 4).unwrap_or(gs.stroke),
            "k" => gs.fill = rgb_from(&op.operands, 4).unwrap_or(gs.fill),
            // colours set in a named colour space: guess from the number of components
            "SC" | "SCN" => gs.stroke = rgb_from(&op.operands, op.operands.iter().filter(|o| number(o).is_some()).count()).unwrap_or(gs.stroke),
            "sc" | "scn" => gs.fill = rgb_from(&op.operands, op.operands.iter().filter(|o| number(o).is_some()).count()).unwrap_or(gs.fill),
            "BT" => out.text += 1,
            "BI" => out.images += 1,
            "Do" => {
                let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else { continue };
                let Some(xobject) = resources
                    .and_then(|r| r.get(b"XObject").ok())
                    .and_then(|x| doc.dereference(x).ok())
                    .and_then(|(_, x)| x.as_dict().ok())
                    .and_then(|x| x.get(name).ok())
                    .and_then(|o| doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_stream().ok())
                else {
                    continue;
                };
                match xobject.dict.get(b"Subtype").and_then(|s| s.as_name()) {
                    Ok(b"Form") if depth < MAX_DEPTH => {
                        let matrix: Vec<f64> = xobject.dict.get(b"Matrix").and_then(|m| m.as_array()).map(|a| a.iter().filter_map(number).collect()).unwrap_or_default();
                        let m = if matrix.len() == 6 { Affine::new([matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5]]) } else { Affine::IDENTITY };
                        let own = xobject.dict.get(b"Resources").ok().and_then(|r| doc.dereference(r).ok()).and_then(|(_, r)| r.as_dict().ok()).or(resources);
                        let data = xobject.decompressed_content().unwrap_or_else(|_| xobject.content.clone());
                        if let Ok(content) = Content::decode(&data) {
                            let inner = GraphicsState { ctm: gs.ctm * m, ..gs.clone() };
                            run(doc, &content.operations, own, inner, to_doc, out, depth + 1);
                        }
                    }
                    Ok(b"Image") => out.images += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;
    use lopdf::{Stream, dictionary};

    /// A one-page PDF (A4) with `ops` as its content and one form XObject named Fm1.
    fn pdf(ops: &str, form: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let form_id = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()], "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 72.into(), 0.into()] },
            form.as_bytes().to_vec(),
        ));
        let content_id = doc.add_object(Stream::new(dictionary! {}, ops.as_bytes().to_vec()));
        let resources = dictionary! { "XObject" => dictionary! { "Fm1" => form_id } };
        let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()] }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    pub(super) fn pdf_for_demo(ops: &str, form: &str) -> Vec<u8> {
        pdf(ops, form)
    }

    fn bounds(s: &Shape) -> kurbo::Rect {
        s.world_paths()[0].bounding_box()
    }

    #[test]
    fn lines_rects_colours_and_transforms() {
        // a red 1-inch line at the bottom of the page; a blue filled rect drawn at 2× scale
        let ops = "1 0 0 RG 0 0 m 72 0 l S q 2 0 0 2 0 0 cm 0 0 1 rg 36 36 36 18 re f Q BT ET";
        let (shapes, notes) = import(&pdf(ops, "")).unwrap();
        assert_eq!(shapes.len(), 2);
        let line = bounds(&shapes[0]);
        assert!((line.width() - 25.4).abs() < 1e-6, "72 pt = 25.4 mm: {line:?}");
        assert!((line.y0 - 842.0 * MM_PER_PT).abs() < 1e-6, "the page bottom is the bottom in mm (Y down): {line:?}");
        assert_eq!(shapes[0].style.stroke, Some(Rgba(255, 0, 0, 255)));
        assert_eq!(shapes[0].style.fill, None);
        let rect = bounds(&shapes[1]);
        assert!((rect.width() - 25.4).abs() < 1e-6 && (rect.height() - 12.7).abs() < 1e-6, "scaled by cm: {rect:?}");
        assert_eq!(shapes[1].style.fill, Some(Rgba(0, 0, 255, 255)));
        assert!(notes.iter().any(|w| w.contains("text")));
    }

    #[test]
    fn curves_and_forms() {
        // a curve on the page, and a square inside a form placed 1 inch to the right
        let ops = "0 g 0 100 m 0 150 50 200 100 200 c S /Fm1 Do";
        let form = "0 0 0 RG 0 0 10 10 re S";
        let (shapes, _) = import(&pdf(ops, form)).unwrap();
        assert_eq!(shapes.len(), 2);
        assert!(shapes[0].world_paths()[0].elements().iter().any(|e| matches!(e, kurbo::PathEl::CurveTo(..))));
        let sq = bounds(&shapes[1]);
        assert!((sq.x0 - 25.4).abs() < 1e-6, "the form's Matrix moves it: {sq:?}");
    }

    #[test]
    fn reads_cutcrafts_own_print_pdf_back_in_place() {
        let mut d = crate::doc::Document::default();
        let style = Style { fill: Some(Rgba(0, 128, 255, 255)), ..Style::default() };
        d.add_path(crate::geom::rect_path(kurbo::Rect::new(20.0, 30.0, 70.0, 50.0)), style, 0);
        let path = std::env::temp_dir().join(format!("cutcraft-pdf-roundtrip-{}.pdf", std::process::id()));
        crate::io::export_print_pdf(d.page(), &path).unwrap();
        let (shapes, _) = import(&std::fs::read(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(shapes.len(), 1);
        let b = bounds(&shapes[0]);
        assert!((b.x0 - 20.0).abs() < 0.01 && (b.y0 - 30.0).abs() < 0.01 && (b.x1 - 70.0).abs() < 0.01 && (b.y1 - 50.0).abs() < 0.01, "{b:?}");
        assert_eq!(shapes[0].style.fill, Some(Rgba(0, 128, 255, 255)));
    }

    #[test]
    fn rejects_non_pdf() {
        assert!(import(b"%!PS-Adobe-3.0 old illustrator").unwrap_err().to_string().contains("PostScript"));
        assert!(import(b"hello").is_err());
    }
}

#[cfg(test)]
mod demo {
    /// Writes a small PDF with paths, a form and some text to the temp folder (for a visual check).
    #[test]
    #[ignore = "diagnostic"]
    fn write_import_demo() {
        let ops = "0.9 0.1 0.1 RG 2 w 72 600 m 300 600 l 300 760 l h S 0.1 0.5 0.9 rg 100 400 150 100 re f 0 0.6 0.2 rg 360 500 m 360 580 440 620 500 560 c 540 520 480 440 360 500 c f /Fm1 Do BT ET";
        let form = "0.5 0 0.6 RG 3 w 0 0 m 20 60 l 40 0 l -10 40 l 50 40 l h S";
        let bytes = super::tests::pdf_for_demo(ops, form);
        let p = std::env::temp_dir().join("cutcraft-import-demo.pdf");
        std::fs::write(&p, bytes).unwrap();
        println!("wrote {}", p.display());
    }
}
