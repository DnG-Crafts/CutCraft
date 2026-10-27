//! Document model: Document → Page → (Layers, Shapes).

use crate::geom::{self, affine_serde, svg_path_serde};
use kurbo::{Affine, BezPath, Point, Rect};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type ShapeId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const BLACK: Rgba = Rgba(0, 0, 0, 255);
    pub const RED: Rgba = Rgba(220, 30, 30, 255);
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);
    /// `#rrggbb`, or `#rrggbbaa` when it isn't fully opaque.
    pub fn hex(&self) -> String {
        if self.3 == 255 { self.rgb_hex() } else { format!("{}{:02x}", self.rgb_hex(), self.3) }
    }
    /// `#rrggbb`, leaving out the opacity.
    pub fn rgb_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
    pub fn from_hex(s: &str) -> Option<Rgba> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 && s.len() != 8 {
            return None;
        }
        let v = u32::from_str_radix(&s[..6], 16).ok()?;
        let a = if s.len() == 8 { u8::from_str_radix(&s[6..], 16).ok()? } else { 255 };
        Some(Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, a))
    }
    pub fn is_transparent(&self) -> bool {
        self.3 == 0
    }
}

/// How a shape is turned into cutter motion.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CutStyle {
    NoCut,
    /// Cut every line of the shape.
    Cut,
    /// Cut only the outer edge (union of the shape's regions).
    CutEdge,
    /// Perforated line: `on` mm cut / `off` mm uncut.
    Perforate { on: f64, off: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: f64,
    pub cut: CutStyle,
    /// Hide from print & cut output.
    pub no_print: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style { fill: None, stroke: Some(Rgba::RED), stroke_width: 0.25, cut: CutStyle::Cut, no_print: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextData {
    pub text: String,
    pub family: String,
    /// Em size in mm.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// Extra character spacing, % of em.
    pub char_spacing: f64,
    /// Line spacing multiplier (1.0 = font default).
    pub line_spacing: f64,
    pub vertical: bool,
    /// Optional path (page coordinates) the text follows.
    #[serde(default, with = "opt_path")]
    pub on_path: Option<BezPath>,
    /// Distance along path to start, mm.
    #[serde(default)]
    pub path_offset: f64,
    /// Fuse overlapping letters into one outline (script fonts), so the blade
    /// doesn't cut every letter separately where they touch.
    #[serde(default)]
    pub weld: bool,
    /// Bend a horizontal line of text round a circle: how many degrees of the
    /// circle it spans (positive arches up like the top of a badge, negative
    /// curves down like a smile, 0 straight).
    #[serde(default)]
    pub curve: f64,
    /// Kerning: extra space (mm, may be negative) after the character at
    /// each index (counting every character of `text`, newlines included).
    #[serde(default)]
    pub kern: Vec<(usize, f64)>,
}

impl TextData {
    /// Extra space after the character at `i`, mm.
    pub fn kern_after(&self, i: usize) -> f64 {
        self.kern.iter().filter(|(k, _)| *k == i).map(|(_, v)| *v).sum()
    }
    /// Sets the extra space after the character at `i` (0 removes it).
    pub fn set_kern(&mut self, i: usize, mm: f64) {
        self.kern.retain(|(k, _)| *k != i);
        if mm.abs() > 1e-6 {
            self.kern.push((i, mm));
            self.kern.sort_by_key(|(k, _)| *k);
        }
    }
}

impl Default for TextData {
    fn default() -> Self {
        TextData {
            text: "Text".into(),
            family: "Arial".into(),
            size: 20.0,
            bold: false,
            italic: false,
            align: Align::Left,
            char_spacing: 0.0,
            line_spacing: 1.0,
            vertical: false,
            on_path: None,
            path_offset: 0.0,
            weld: false,
            curve: 0.0,
            kern: Vec::new(),
        }
    }
}

mod opt_path {
    use kurbo::BezPath;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(p: &Option<BezPath>, s: S) -> Result<S::Ok, S::Error> {
        match p {
            Some(p) => s.serialize_some(&p.to_svg()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<BezPath>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        s.map(|s| BezPath::from_svg(&s).map_err(serde::de::Error::custom)).transpose()
    }
}

/// Raster image. Placed in local coords at (0,0)-(width_mm,height_mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    /// Encoded PNG bytes.
    #[serde(with = "b64")]
    pub png: Arc<Vec<u8>>,
    pub px_w: u32,
    pub px_h: u32,
    pub width_mm: f64,
    pub height_mm: f64,
    /// The picture before it was first erased (for the Restore brush).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_b64")]
    pub original: Option<Arc<Vec<u8>>>,
}

mod opt_b64 {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::sync::Arc;
    pub fn serialize<S: Serializer>(d: &Option<Arc<Vec<u8>>>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => super::b64::serialize(d, s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Arc<Vec<u8>>>, D::Error> {
        Option::<String>::deserialize(d)?.map(|s| super::b64::decode(&s).map(Arc::new).map_err(serde::de::Error::custom)).transpose()
    }
}

pub mod b64 {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::sync::Arc;
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    pub fn encode(d: &[u8]) -> String {
        let mut s = String::with_capacity(d.len() * 4 / 3 + 4);
        for c in d.chunks(3) {
            let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(s.len() * 3 / 4);
        let mut buf = 0u32;
        let mut bits = 0;
        for ch in s.bytes() {
            let v = match ch {
                b'A'..=b'Z' => ch - b'A',
                b'a'..=b'z' => ch - b'a' + 26,
                b'0'..=b'9' => ch - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' | b'\n' | b'\r' => continue,
                _ => return Err("bad base64".into()),
            } as u32;
            buf = buf << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Ok(out)
    }
    pub fn serialize<S: Serializer>(d: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(d))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).map(Arc::new).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ShapeKind {
    Path(#[serde(with = "svg_path_serde")] BezPath),
    /// Text with its cached outline (local coordinates).
    Text {
        data: TextData,
        #[serde(with = "svg_path_serde")]
        outline: BezPath,
    },
    Image(ImageData),
    Group(Vec<Shape>),
    /// Live warp of other artwork (mesh envelope or conical).
    Warp(crate::warp::WarpData),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: ShapeId,
    pub name: String,
    pub layer: u32,
    #[serde(with = "affine_serde")]
    pub transform: Affine,
    pub style: Style,
    pub kind: ShapeKind,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Non-destructive effect stack, applied in order to the base geometry.
    #[serde(default)]
    pub effects: Vec<crate::effectstack::Effect>,
}

impl Shape {
    pub fn new_path(id: ShapeId, path: BezPath, style: Style) -> Shape {
        Shape {
            id,
            name: String::new(),
            layer: 0,
            transform: Affine::IDENTITY,
            style,
            kind: ShapeKind::Path(path),
            locked: false,
            hidden: false,
            effects: Vec::new(),
        }
    }

    pub fn display_name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match &self.kind {
            ShapeKind::Path(_) => format!("Path {}", self.id),
            ShapeKind::Text { data, .. } => format!("Text \"{}\"", data.text.lines().next().unwrap_or("")),
            ShapeKind::Image(_) => format!("Image {}", self.id),
            ShapeKind::Group(c) => format!("Group ({})", c.len()),
            ShapeKind::Warp(w) => match w.mode {
                crate::warp::WarpMode::Mesh { .. } => "Warp".into(),
                crate::warp::WarpMode::Conical(_) => "Conical warp".into(),
            },
        }
    }

    /// Base geometry for this shape's kind, before the effect stack.
    fn base_local_paths(&self) -> Vec<BezPath> {
        match &self.kind {
            ShapeKind::Path(p) => vec![p.clone()],
            ShapeKind::Text { outline, .. } => vec![outline.clone()],
            ShapeKind::Image(img) => vec![geom::rect_path(Rect::new(0.0, 0.0, img.width_mm, img.height_mm))],
            ShapeKind::Group(ch) => ch.iter().flat_map(|c| geom::transformed(&c.local_paths(), c.transform)).collect(),
            ShapeKind::Warp(w) => w.warped_paths(),
        }
    }

    /// Local-space geometry (before this shape's transform), with the
    /// non-destructive effect stack applied.
    pub fn local_paths(&self) -> Vec<BezPath> {
        let base = self.base_local_paths();
        if self.effects.is_empty() {
            base
        } else {
            crate::effectstack::apply_all(&self.effects, base)
        }
    }

    /// Page-space geometry.
    pub fn world_paths(&self) -> Vec<BezPath> {
        geom::transformed(&self.local_paths(), self.transform)
    }

    pub fn bbox(&self) -> Option<Rect> {
        geom::paths_bbox(&self.world_paths())
    }

    /// A group carrying effects (e.g. Unite) is drawn and cut as one shape:
    /// its parts combined, the effects applied, in the first part's style.
    /// None for other shapes, and for groups holding pictures.
    pub fn merged(&self) -> Option<Shape> {
        let ShapeKind::Group(ch) = &self.kind else { return None };
        if self.effects.is_empty() || self.leaves_raw().iter().any(|l| l.is_image()) {
            return None;
        }
        let style = ch.iter().flat_map(|c| c.leaves_raw()).find(|l| !l.hidden).map(|l| l.style.clone()).unwrap_or_default();
        let mut s = Shape::new_path(self.id, geom::merge_paths(&self.local_paths()), style);
        s.transform = self.transform;
        s.layer = self.layer;
        s.hidden = self.hidden;
        s.locked = self.locked;
        s.name = self.name.clone();
        Some(s)
    }

    fn leaves_raw(&self) -> Vec<Shape> {
        match &self.kind {
            ShapeKind::Group(ch) => ch
                .iter()
                .flat_map(|c| {
                    let mut c = c.clone();
                    c.transform = self.transform * c.transform;
                    c.leaves_raw()
                })
                .collect(),
            _ => vec![self.clone()],
        }
    }

    /// Leaf shapes with transforms flattened to page space.
    pub fn leaves(&self) -> Vec<Shape> {
        if let Some(m) = self.merged() {
            return vec![m];
        }
        match &self.kind {
            ShapeKind::Group(ch) => ch
                .iter()
                .flat_map(|c| {
                    let mut c = c.clone();
                    c.transform = self.transform * c.transform;
                    c.leaves()
                })
                .collect(),
            _ => vec![self.clone()],
        }
    }

    pub fn is_image(&self) -> bool {
        matches!(self.kind, ShapeKind::Image(_))
    }

    /// Lines that should be cut, in page space, honoring the cut style.
    pub fn cut_paths(&self) -> Vec<BezPath> {
        let mut out = Vec::new();
        for leaf in self.leaves() {
            if leaf.hidden || leaf.is_image() {
                continue;
            }
            match leaf.style.cut {
                CutStyle::NoCut => {}
                CutStyle::Cut | CutStyle::Perforate { .. } => out.extend(leaf.world_paths()),
                CutStyle::CutEdge => {
                    let w = leaf.world_paths();
                    let e = geom::boolean(&w, &[], geom::BoolOp::Union);
                    out.push(e);
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: u32,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub color: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatKind {
    None,
    Mat12x12,
    Mat12x24,
    Mat8x12,
    Mat15x15,
    Mat24x24,
}

impl MatKind {
    pub const ALL: [MatKind; 6] =
        [MatKind::None, MatKind::Mat8x12, MatKind::Mat12x12, MatKind::Mat12x24, MatKind::Mat15x15, MatKind::Mat24x24];
    pub fn size_mm(&self) -> Option<(f64, f64)> {
        let i = 25.4;
        match self {
            MatKind::None => None,
            MatKind::Mat8x12 => Some((8.5 * i, 12.0 * i)),
            MatKind::Mat12x12 => Some((12.0 * i, 12.0 * i)),
            MatKind::Mat12x24 => Some((12.0 * i, 24.0 * i)),
            MatKind::Mat15x15 => Some((15.0 * i, 15.0 * i)),
            MatKind::Mat24x24 => Some((24.0 * i, 24.0 * i)),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            MatKind::None => "None (roll / matless)",
            MatKind::Mat8x12 => "8.5 × 12 in mat",
            MatKind::Mat12x12 => "12 × 12 in mat",
            MatKind::Mat12x24 => "12 × 24 in mat",
            MatKind::Mat15x15 => "15 × 15 in mat",
            MatKind::Mat24x24 => "24 × 24 in mat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub visible: bool,
    pub snap: bool,
    pub spacing: f64,
    pub subdivisions: u32,
}

impl Default for Grid {
    fn default() -> Self {
        Grid { visible: true, snap: false, spacing: 25.4, subdivisions: 4 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub name: String,
    pub width: f64,
    pub height: f64,
    pub mat: MatKind,
    pub media_color: Rgba,
    pub layers: Vec<Layer>,
    pub shapes: Vec<Shape>,
    pub grid: Grid,
    pub regmarks: crate::regmarks::RegMarks,
    /// Cut border inset from the media edge, mm.
    pub cut_border: f64,
    /// Ruler guides (drawing aids, never cut or exported).
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// How many copies of this page's design the Make it screen lays out
    /// (on top of the job's own copies) - set by the production queue.
    #[serde(default = "one")]
    pub copies: u32,
}

fn one() -> u32 {
    1
}

/// A ruler guide: a vertical line at x = `pos` or a horizontal one at y = `pos` (mm).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub vertical: bool,
    pub pos: f64,
}

impl Page {
    pub fn new(name: &str, w: f64, h: f64) -> Page {
        Page {
            name: name.into(),
            width: w,
            height: h,
            mat: MatKind::Mat12x12,
            media_color: Rgba::WHITE,
            layers: vec![Layer { id: 0, name: "Layer 1".into(), visible: true, locked: false, color: Rgba::RED }],
            shapes: Vec::new(),
            grid: Grid::default(),
            regmarks: Default::default(),
            cut_border: 0.0,
            guides: Vec::new(),
            copies: 1,
        }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width, self.height)
    }

    pub fn layer(&self, id: u32) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn shape_index(&self, id: ShapeId) -> Option<usize> {
        self.shapes.iter().position(|s| s.id == id)
    }

    pub fn shape(&self, id: ShapeId) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }

    pub fn shape_mut(&mut self, id: ShapeId) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    pub fn is_editable(&self, s: &Shape) -> bool {
        !s.locked && self.layer(s.layer).map(|l| l.visible && !l.locked).unwrap_or(true)
    }

    pub fn is_visible(&self, s: &Shape) -> bool {
        !s.hidden && self.layer(s.layer).map(|l| l.visible).unwrap_or(true)
    }

    /// Topmost visible shape under point (tolerance in mm for strokes).
    pub fn hit_test(&self, p: Point, tol: f64) -> Option<ShapeId> {
        for s in self.shapes.iter().rev() {
            if !self.is_visible(s) {
                continue;
            }
            let w = s.world_paths();
            let filled = s.style.fill.is_some() || s.is_image() || matches!(s.kind, ShapeKind::Text { .. });
            if let Some(bb) = geom::paths_bbox(&w) {
                if !bb.inflate(tol, tol).contains(p) {
                    continue;
                }
            }
            if (filled && geom::contains(&w, p)) || geom::distance_to_outline(&w, p) <= tol {
                return Some(s.id);
            }
            if matches!(s.kind, ShapeKind::Group(_)) && geom::contains(&w, p) {
                return Some(s.id);
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub pages: Vec<Page>,
    pub active: usize,
    pub next_id: ShapeId,
    pub machine: String,
    /// Named parameters and locked sizes/distances (Parameters panel).
    #[serde(default)]
    pub params: crate::params::Parametric,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            pages: vec![Page::new("Page 1", 12.0 * 25.4, 12.0 * 25.4)],
            active: 0,
            next_id: 1,
            machine: "cameo5".into(),
            params: Default::default(),
        }
    }
}

impl Document {
    pub fn page(&self) -> &Page {
        &self.pages[self.active]
    }
    pub fn page_mut(&mut self) -> &mut Page {
        &mut self.pages[self.active]
    }
    pub fn alloc_id(&mut self) -> ShapeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    /// Give a shape (and group children) fresh ids.
    pub fn reid(&mut self, s: &mut Shape) {
        s.id = self.alloc_id();
        if let ShapeKind::Group(ch) = &mut s.kind {
            for c in ch {
                self.reid(c);
            }
        }
    }
    pub fn add_shape(&mut self, mut s: Shape, layer: u32) -> ShapeId {
        if s.id == 0 {
            s.id = self.alloc_id();
        }
        s.layer = layer;
        let id = s.id;
        self.page_mut().shapes.push(s);
        id
    }
    pub fn add_path(&mut self, path: BezPath, style: Style, layer: u32) -> ShapeId {
        let id = self.alloc_id();
        self.add_shape(Shape::new_path(id, path, style), layer)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn colour_codes_keep_transparency() {
        use super::Rgba;
        assert_eq!(Rgba(255, 0, 0, 255).hex(), "#ff0000");
        assert_eq!(Rgba(0, 0, 0, 0).hex(), "#00000000");
        assert_ne!(Rgba(0, 0, 0, 0).hex(), Rgba(0, 0, 0, 255).hex(), "transparent isn't black");
        for c in [Rgba(12, 34, 56, 255), Rgba(12, 34, 56, 128), Rgba(0, 0, 0, 0)] {
            assert_eq!(Rgba::from_hex(&c.hex()), Some(c));
        }
    }

    use super::*;
    #[test]
    fn base64_roundtrip() {
        for n in 0..10 {
            let d: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(b64::decode(&b64::encode(&d)).unwrap(), d);
        }
    }
    #[test]
    fn hit_test_finds_rect() {
        let mut d = Document::default();
        let id = d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 20.0, 20.0)), Style::default(), 0);
        assert_eq!(d.page().hit_test(Point::new(10.0, 15.0), 0.5), Some(id));
        assert_eq!(d.page().hit_test(Point::new(15.0, 15.0), 0.5), None);
    }

    #[test]
    fn guides_round_trip_and_older_files_still_load() {
        let mut d = Document::default();
        d.page_mut().guides.push(Guide { vertical: true, pos: 25.4 });
        let json = serde_json::to_value(&d).unwrap();
        let back: Document = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back.page().guides, d.page().guides);
        let mut old = json;
        old["pages"][0].as_object_mut().unwrap().remove("guides");
        let back: Document = serde_json::from_value(old).unwrap();
        assert!(back.page().guides.is_empty());
    }
}
