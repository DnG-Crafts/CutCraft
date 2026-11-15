//! Silhouette Studio documents (`.studio3`, the `silhouette05;` format of
//! Studio 3 and later): their outlines and pictures.
//!
//! The format isn't published; this reads what's been worked out from real
//! files. Each outline is a point list - `size: u32, closed: u8, count: u32`,
//! then `count + 1` points of `x, y: f32` (mm) each followed by an in- and an
//! out-handle (`flag: u8`, then `dx, dy: f32` relative to the point when the
//! flag is 1) and 4 more bytes, `size` covering all of it - which makes a
//! signature that ordinary data doesn't match by chance. Pictures are kept
//! whole (JPEG or PNG) after their length as a u64; a picture's frame on the
//! mat is a four-point outline of the same proportions, which places it.
//! The preview thumbnail has no length before it, and is left out. Text,
//! colours and cut settings aren't read.

use crate::doc::{CutStyle, Rgba, Shape, Style};
use anyhow::{Result, bail};
use kurbo::{Affine, BezPath, Point, Vec2};

const SIGNATURE: &[u8] = b"silhouette05;";

fn f32_at(d: &[u8], o: usize) -> Option<f64> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    let v = f32::from_le_bytes(b) as f64;
    (v.is_finite() && v.abs() < 10_000.0).then_some(v)
}

fn u32_at(d: &[u8], o: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize)
}

struct Node {
    at: Point,
    handle_in: Option<Vec2>,
    handle_out: Option<Vec2>,
}

/// The point list at `o` (its size field), and where it ends.
fn point_list(d: &[u8], o: usize) -> Option<(Vec<Node>, bool, usize)> {
    let size = u32_at(d, o)?;
    let closed = match d.get(o + 4)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let count = u32_at(d, o + 5)?.checked_add(1)?;
    if !(2..=200_000).contains(&count) || size < 9 || o + 4 + size > d.len() {
        return None;
    }
    let mut p = o + 9;
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let at = Point::new(f32_at(d, p)?, f32_at(d, p + 4)?);
        p += 8;
        let mut handle = || -> Option<Option<Vec2>> {
            match *d.get(p)? {
                0 => {
                    p += 1;
                    Some(None)
                }
                1 => {
                    let v = Vec2::new(f32_at(d, p + 1)?, f32_at(d, p + 5)?);
                    p += 9;
                    Some(Some(v))
                }
                _ => None,
            }
        };
        let handle_in = handle()?;
        let handle_out = handle()?;
        nodes.push(Node { at, handle_in, handle_out });
        if p > o + 4 + size {
            return None;
        }
    }
    // the size covers the points and 4 bytes after them, exactly
    (p + 4 == o + 4 + size).then_some((nodes, closed, p + 4))
}

fn to_path(nodes: &[Node], closed: bool) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(nodes[0].at);
    let segment = |path: &mut BezPath, a: &Node, b: &Node| {
        if a.handle_out.is_none() && b.handle_in.is_none() {
            path.line_to(b.at);
        } else {
            let c1 = a.at + a.handle_out.unwrap_or_default();
            let c2 = b.at + b.handle_in.unwrap_or_default();
            path.curve_to(c1, c2, b.at);
        }
    };
    for w in nodes.windows(2) {
        segment(&mut path, &w[0], &w[1]);
    }
    if closed {
        segment(&mut path, &nodes[nodes.len() - 1], &nodes[0]);
        path.close_path();
    }
    path
}

/// Pictures kept whole in the file: (start, end, pixel size).
fn pictures(d: &[u8]) -> Vec<(usize, usize, (u32, u32))> {
    let mut out = vec![];
    let mut i = 8;
    while i + 8 < d.len() {
        let jpeg = d[i..].starts_with(&[0xFF, 0xD8, 0xFF]);
        let png = d[i..].starts_with(b"\x89PNG\r\n\x1a\n");
        if jpeg || png {
            let len = u64::from_le_bytes(d[i - 8..i].try_into().unwrap_or_default()) as usize;
            if len > 100 && i + len <= d.len() {
                if let Ok(size) = image::ImageReader::new(std::io::Cursor::new(&d[i..i + len])).with_guessed_format().map_err(|_| ()).and_then(|r| r.into_dimensions().map_err(|_| ())) {
                    out.push((i, i + len, size));
                    i += len;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Reads a Silhouette Studio document's outlines and pictures (mm), with
/// notes about what's left out.
pub fn import(d: &[u8]) -> Result<(Vec<Shape>, Vec<String>)> {
    if !d.starts_with(SIGNATURE) {
        bail!("this isn't a Silhouette Studio 3+ document");
    }
    let pics = pictures(d);
    let inside_picture = |o: usize| pics.iter().any(|(a, b, _)| (*a..*b).contains(&o));
    let mut lists = vec![];
    let mut o = 0;
    while o + 13 < d.len() {
        if inside_picture(o) {
            o += 1;
            continue;
        }
        match point_list(d, o) {
            Some((nodes, closed, end)) => {
                lists.push((nodes, closed));
                o = end;
            }
            None => o += 1,
        }
    }
    let mut shapes = vec![];
    let mut used = vec![false; pics.len()];
    let cut = Style { stroke: Some(Rgba(255, 0, 0, 255)), stroke_width: 0.25, cut: CutStyle::Cut, ..Style::default() };
    for (nodes, closed) in &lists {
        // a picture's frame: four corners, straight sides, the picture's proportions
        if nodes.len() == 4 && *closed && nodes.iter().all(|n| n.handle_in.is_none() && n.handle_out.is_none()) {
            let (o, x, y) = (nodes[0].at, nodes[3].at - nodes[0].at, nodes[1].at - nodes[0].at);
            let (w, h) = (x.hypot(), y.hypot());
            let frame = pics.iter().enumerate().position(|(k, (_, _, (pw, ph)))| {
                !used[k] && w > 0.0 && h > 0.0 && ((w / h) / (*pw as f64 / *ph as f64) - 1.0).abs() < 0.03
            });
            if let Some(k) = frame {
                used[k] = true;
                let (a, b, _) = pics[k];
                let mut s = crate::io::import_image(&d[a..b])?;
                if let crate::doc::ShapeKind::Image(img) = &mut s.kind {
                    img.width_mm = w;
                    img.height_mm = h;
                }
                // picture mm → page: along the frame's top and left sides
                s.transform = Affine::new([x.x / w, x.y / w, y.x / h, y.y / h, o.x, o.y]);
                shapes.push(s);
                continue;
            }
        }
        shapes.push(Shape::new_path(0, to_path(nodes, *closed), cut.clone()));
    }
    let mut notes = vec![];
    let stray = used.iter().filter(|u| !**u).count();
    if stray > 0 && shapes.iter().all(|s| !s.is_image()) {
        notes.push(format!("{stray} picture(s) without a place on the mat left out"));
    }
    if shapes.is_empty() {
        bail!("no shapes found in this Silhouette Studio document (text isn't read yet - convert it to a path in Silhouette Studio, or export SVG/DXF)");
    }
    notes.push("read from Silhouette Studio's own format - check sizes, and colours/cut settings aren't brought over".into());
    Ok((shapes, notes))
}
