//! Text layout: system fonts and CutCraft user fonts → glyph outlines (mm),
//! with spacing, alignment, vertical text and text-on-path.

use crate::doc::{Align, TextData};
use crate::geom;
use crate::userfont::{UPM, UserFont};
use kurbo::{Affine, BezPath, Point, Vec2};
use std::collections::BTreeSet;

pub struct FontLibrary {
    db: fontdb::Database,
    families: Vec<String>,
    pub user: Vec<UserFont>,
}

struct Outliner {
    path: BezPath,
    a: Affine,
}

impl ttf_parser::OutlineBuilder for Outliner {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(self.a * Point::new(x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.a * Point::new(x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(self.a * Point::new(x1 as f64, y1 as f64), self.a * Point::new(x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.curve_to(
            self.a * Point::new(x1 as f64, y1 as f64),
            self.a * Point::new(x2 as f64, y2 as f64),
            self.a * Point::new(x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// One glyph: outline with origin at the baseline start, y down, mm.
struct Glyph {
    outline: BezPath,
    advance: f64,
}

/// Vertical metrics in mm: ascender (positive), descender (negative), line gap.
struct Metrics {
    asc: f64,
    desc: f64,
    gap: f64,
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl FontLibrary {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // fonts added in CutCraft (from the web or a file), kept with its settings
        db.load_fonts_dir(Self::added_dir());
        let mut lib = FontLibrary { db, families: vec![], user: UserFont::load_all() };
        lib.refresh_families();
        lib
    }

    pub fn refresh_families(&mut self) {
        let mut set = BTreeSet::new();
        for f in self.db.faces() {
            if let Some((name, _)) = f.families.first() {
                set.insert(name.clone());
            }
        }
        for u in &self.user {
            set.insert(u.name.clone());
        }
        // The built-in stroke font goes first, so pen users find it straight away.
        set.remove(crate::singleline::FAMILY);
        self.families = std::iter::once(crate::singleline::FAMILY.to_string()).chain(set).collect();
    }

    /// Where fonts added in CutCraft are kept.
    pub fn added_dir() -> std::path::PathBuf {
        crate::userfont::UserFont::dir()
    }

    /// Adds a font file (TTF or OTF) to CutCraft - copied to its fonts
    /// folder, so it's there next time too, without installing it in the
    /// system. Returns the font's family names.
    pub fn add_font(&mut self, file_name: &str, data: Vec<u8>) -> anyhow::Result<Vec<String>> {
        ttf_parser::Face::parse(&data, 0).map_err(|_| anyhow::anyhow!("that isn't a font file CutCraft can read (TTF or OTF)"))?;
        let safe: String = file_name.chars().map(|c| if c.is_alphanumeric() || ".-_ ".contains(c) { c } else { '_' }).collect();
        let dir = Self::added_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(&safe);
        std::fs::write(&path, &data)?;
        let before: std::collections::HashSet<fontdb::ID> = self.db.faces().map(|f| f.id).collect();
        self.db.load_font_data(data);
        let mut names: Vec<String> = self.db.faces().filter(|f| !before.contains(&f.id)).filter_map(|f| f.families.first().map(|n| n.0.clone())).collect();
        names.dedup();
        self.refresh_families();
        Ok(names)
    }

    pub fn reload_user_fonts(&mut self) {
        self.user = UserFont::load_all();
        self.refresh_families();
    }

    pub fn families(&self) -> &[String] {
        &self.families
    }

    pub fn is_user_font(&self, family: &str) -> bool {
        self.user.iter().any(|u| u.name == family)
    }

    pub fn database(&self) -> &fontdb::Database {
        &self.db
    }

    fn face_id(&self, family: &str, bold: bool, italic: bool) -> Option<fontdb::ID> {
        let weight = if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL };
        let style = if italic { fontdb::Style::Italic } else { fontdb::Style::Normal };
        let query = |fams: &[fontdb::Family]| {
            self.db.query(&fontdb::Query { families: fams, weight, style, stretch: fontdb::Stretch::Normal })
        };
        query(&[fontdb::Family::Name(family)])
            .or_else(|| query(&[fontdb::Family::SansSerif]))
            .or_else(|| self.db.faces().next().map(|f| f.id))
    }

    /// Lay out text into a single path (local coordinates, top-left at origin, mm).
    pub fn layout(&self, t: &TextData) -> BezPath {
        self.with_glyphs(t, |t, m, g| layout_lines(t, m, g)).unwrap_or_default()
    }

    /// Where each letter of `t` sits: for every non-blank character, the
    /// transform that puts a one-letter text (same font and size) exactly over
    /// that letter in `t`'s layout ("ungroup to letters").
    pub fn letter_placements(&self, t: &TextData) -> Vec<(char, Affine)> {
        self.with_glyphs(t, |t, m, g| {
            placed_glyphs(t, m, g)
                .into_iter()
                .filter(|p| !p.ch.is_whitespace())
                .map(|p| (p.ch, p.at * Affine::translate((0.0, -m.asc))))
                .collect()
        })
        .unwrap_or_default()
    }

    /// Each line of a plain horizontal text, with the transform that puts a
    /// one-line text (same settings) over it ("ungroup to lines"). Empty for
    /// text on a path, vertical or curved text.
    pub fn line_placements(&self, t: &TextData) -> Vec<(String, Affine)> {
        if t.on_path.is_some() || t.vertical || t.curve.abs() > 1e-9 {
            return vec![];
        }
        self.with_glyphs(t, |t, m, g| {
            let line_h = (m.asc - m.desc + m.gap) * t.line_spacing.max(0.1);
            let lines: Vec<&str> = t.text.split('\n').collect();
            let widths: Vec<f64> = lines.iter().map(|l| g(l).iter().map(|g| g.advance).sum()).collect();
            let max_w = widths.iter().copied().fold(0.0, f64::max);
            lines
                .iter()
                .zip(&widths)
                .enumerate()
                .filter(|(_, (l, _))| !l.trim().is_empty())
                .map(|(li, (l, w))| {
                    let x = match t.align {
                        Align::Left => 0.0,
                        Align::Center => (max_w - w) / 2.0,
                        Align::Right => max_w - w,
                    };
                    (l.to_string(), Affine::translate((x, li as f64 * line_h)))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Every character the font has a glyph for (for the glyph picker), in
    /// code-point order, controls and spaces left out.
    pub fn glyph_chars(&self, family: &str, bold: bool, italic: bool) -> Vec<char> {
        let keep = |c: &char| !c.is_control() && !c.is_whitespace();
        if family == crate::singleline::FAMILY {
            return (' '..='~').filter(keep).filter(|c| crate::singleline::glyph(*c).is_some()).collect();
        }
        if let Some(u) = self.user.iter().find(|u| u.name == family) {
            let mut v: Vec<char> = u.glyphs.keys().copied().filter(keep).collect();
            v.sort();
            return v;
        }
        let Some(id) = self.face_id(family, bold, italic) else { return vec![] };
        self.db
            .with_face_data(id, |data, index| {
                let face = ttf_parser::Face::parse(data, index).ok()?;
                let mut set = std::collections::BTreeSet::new();
                for st in face.tables().cmap?.subtables {
                    if st.is_unicode() {
                        st.codepoints(|cp| {
                            if let Some(c) = char::from_u32(cp) {
                                if keep(&c) && st.glyph_index(cp).is_some_and(|g| g.0 != 0) {
                                    set.insert(c);
                                }
                            }
                        });
                    }
                }
                Some(set.into_iter().collect())
            })
            .flatten()
            .unwrap_or_default()
    }

    /// Runs `f` with the font metrics and a glyph source for `t`'s font.
    fn with_glyphs<R>(&self, t: &TextData, f: impl Fn(&TextData, &Metrics, &dyn Fn(&str) -> Vec<Glyph>) -> R) -> Option<R> {
        if t.family == crate::singleline::FAMILY {
            use crate::singleline::{CAP, LINE};
            // `size` is the em size; capitals are drawn ~0.7 em tall like typical fonts.
            let k = 0.7 * t.size / CAP;
            let m = Metrics { asc: CAP * k, desc: -3.5 * k, gap: (LINE - CAP - 3.5) * k };
            let glyphs = |line: &str| -> Vec<Glyph> {
                line.chars()
                    .map(|ch| match crate::singleline::glyph(ch) {
                        Some((mut o, adv)) => {
                            o.apply_affine(italic(t.italic) * Affine::scale(k));
                            Glyph { outline: o, advance: adv * k + t.char_spacing / 100.0 * t.size }
                        }
                        None => Glyph { outline: BezPath::new(), advance: 5.0 * k },
                    })
                    .collect()
            };
            return Some(f(t, &m, &glyphs));
        }
        if let Some(u) = self.user.iter().find(|u| u.name == t.family) {
            let k = t.size / UPM;
            let m = Metrics { asc: u.ascender * k, desc: u.descender * k, gap: 0.0 };
            let synth_italic = t.italic;
            let glyphs = |line: &str| -> Vec<Glyph> {
                line.chars()
                    .map(|ch| match u.glyphs.get(&ch) {
                        Some(g) => {
                            let mut o = g.path.clone();
                            o.apply_affine(italic(synth_italic) * Affine::scale(k));
                            Glyph { outline: o, advance: g.advance * k + t.char_spacing / 100.0 * t.size }
                        }
                        None => Glyph { outline: BezPath::new(), advance: u.space_advance * k + t.char_spacing / 100.0 * t.size },
                    })
                    .collect()
            };
            return Some(f(t, &m, &glyphs));
        }
        let id = self.face_id(&t.family, t.bold, t.italic)?;
        let res = self.db.with_face_data(id, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok()?;
            let scale = t.size / face.units_per_em() as f64;
            let m = Metrics {
                asc: face.ascender() as f64 * scale,
                desc: face.descender() as f64 * scale,
                gap: face.line_gap() as f64 * scale,
            };
            let glyphs = |line: &str| glyphs_for_line(&face, line, t, scale);
            Some(f(t, &m, &glyphs))
        });
        res.flatten()
    }
}

fn italic(on: bool) -> Affine {
    if on { Affine::new([1.0, 0.0, -0.2, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY }
}

fn glyphs_for_line(face: &ttf_parser::Face, line: &str, t: &TextData, scale: f64) -> Vec<Glyph> {
    let synth_italic = t.italic && !face.is_italic();
    let a = italic(synth_italic) * Affine::scale_non_uniform(scale, -scale);
    let mut out: Vec<Glyph> = Vec::new();
    let mut prev: Option<ttf_parser::GlyphId> = None;
    for ch in line.chars() {
        let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
        let mut b = Outliner { path: BezPath::new(), a };
        face.outline_glyph(gid, &mut b);
        let mut adv = face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale;
        if let (Some(p), Some(kern)) = (prev, face.tables().kern) {
            for st in kern.subtables {
                if st.horizontal {
                    if let Some(k) = st.glyphs_kerning(p, gid) {
                        if let Some(last) = out.last_mut() {
                            last.advance += k as f64 * scale;
                        }
                        break;
                    }
                }
            }
        }
        adv += t.char_spacing / 100.0 * t.size;
        prev = Some(gid);
        out.push(Glyph { outline: b.path, advance: adv });
    }
    out
}

/// A glyph placed in the layout: its character and the transform from its
/// own outline (origin on the baseline at its left edge) to the text's.
struct Placed {
    ch: char,
    at: Affine,
    outline: BezPath,
}

fn layout_lines(t: &TextData, m: &Metrics, glyphs_for: &dyn Fn(&str) -> Vec<Glyph>) -> BezPath {
    let pieces = placed_glyphs(t, m, glyphs_for)
        .into_iter()
        .map(|p| {
            let mut o = p.outline;
            o.apply_affine(p.at);
            o
        })
        .collect();
    finish(t, pieces)
}

fn placed_glyphs(t: &TextData, m: &Metrics, glyphs_for: &dyn Fn(&str) -> Vec<Glyph>) -> Vec<Placed> {
    let line_h = (m.asc - m.desc + m.gap) * t.line_spacing.max(0.1);
    let mut out: Vec<Placed> = Vec::new();

    if let Some(path) = &t.on_path {
        // text on path: single line, glyphs centred on their advance along the path
        let line = t.text.replace('\n', " ");
        let mut s = t.path_offset;
        for (i, (ch, g)) in line.chars().zip(glyphs_for(&line)).enumerate() {
            if let Some((p, tan)) = geom::point_at_length(path, s + g.advance / 2.0) {
                let ang = tan.y.atan2(tan.x);
                let at = Affine::translate(p.to_vec2()) * Affine::rotate(ang) * Affine::translate((-g.advance / 2.0, 0.0));
                out.push(Placed { ch, at, outline: g.outline });
            }
            s += g.advance + t.kern_after(i);
        }
        return out;
    }

    let lines: Vec<&str> = t.text.split('\n').collect();
    if t.vertical {
        for (li, line) in lines.iter().enumerate() {
            let x = li as f64 * line_h;
            let mut y = m.asc;
            for (ch, g) in line.chars().zip(glyphs_for(line)) {
                let at = Affine::translate((x + (t.size - g.advance) / 2.0, y));
                out.push(Placed { ch, at, outline: g.outline });
                y += t.size * (1.0 + t.char_spacing / 100.0);
            }
        }
        return out;
    }

    // index of each line's first character in the whole text
    let starts: Vec<usize> = lines.iter().scan(0usize, |acc, l| {
        let s = *acc;
        *acc += l.chars().count() + 1;
        Some(s)
    }).collect();
    let laid: Vec<(&str, Vec<Glyph>, f64)> = lines
        .iter()
        .zip(&starts)
        .map(|(l, s0)| {
            let g = glyphs_for(l);
            let w: f64 = g.iter().map(|g| g.advance).sum::<f64>() + (0..l.chars().count()).map(|i| t.kern_after(s0 + i)).sum::<f64>();
            (*l, g, w)
        })
        .collect();
    let max_w = laid.iter().map(|l| l.2).fold(0.0, f64::max);
    // curved text: each line bends round a circle spanning `curve` degrees of
    // the widest line, centred under (arch) or over (smile) the text
    let k = if t.curve.abs() > 1e-6 && max_w > 1e-9 { t.curve.clamp(-360.0, 360.0).to_radians() / max_w } else { 0.0 };
    for (li, (line, glyphs, w)) in laid.into_iter().enumerate() {
        let mut x = match t.align {
            Align::Left => 0.0,
            Align::Center => (max_w - w) / 2.0,
            Align::Right => max_w - w,
        };
        let y = m.asc + li as f64 * line_h;
        for (ci, (ch, g)) in line.chars().zip(glyphs).enumerate() {
            let adv = g.advance + t.kern_after(starts[li] + ci);
            let at = if k == 0.0 {
                Affine::translate((x, y))
            } else {
                // the glyph's middle lands on the circle; it turns to follow it
                let r = 1.0 / k;
                let theta = (x + g.advance / 2.0 - max_w / 2.0) * k;
                let c = Point::new(max_w / 2.0, y + r);
                let p = c + Vec2::new(r * theta.sin(), -r * theta.cos());
                Affine::translate(p.to_vec2()) * Affine::rotate(theta) * Affine::translate((-g.advance / 2.0, 0.0))
            };
            out.push(Placed { ch, at, outline: g.outline });
            x += adv;
        }
    }
    out
}

/// Joins placed glyphs into one path — welded (overlaps merged) when asked.
/// Stroke-font text is never welded: its letters are open lines, not regions.
fn finish(t: &TextData, pieces: Vec<BezPath>) -> BezPath {
    if t.weld && t.family != crate::singleline::FAMILY {
        return geom::union_each(&pieces);
    }
    let mut out = BezPath::new();
    for p in pieces {
        out.extend(p.elements().iter().copied());
    }
    out
}



#[cfg(test)]
mod tests {
    /// Saves a design with "Text" in `FONT_FAMILY`, curved and scaled up, to
    /// `DESIGN_OUT` - to look at on the canvas.
    #[test]
    #[ignore = "diagnostic"]
    fn write_curved_text_design() {
        let (Ok(fam), Ok(out)) = (std::env::var("FONT_FAMILY"), std::env::var("DESIGN_OUT")) else { return };
        let lib = FontLibrary::new();
        let data = TextData { text: "Text".into(), family: fam, size: 20.0, curve: 110.0, ..TextData::default() };
        let outline = lib.layout(&data);
        let mut d = crate::doc::Document::default();
        let s = crate::doc::Shape {
            id: 0,
            name: String::new(),
            layer: 0,
            transform: Affine::translate((20.0, 60.0)) * Affine::scale(6.0),
            style: crate::doc::Style::default(),
            kind: crate::doc::ShapeKind::Text { data, outline },
            locked: false,
            hidden: false,
            effects: Vec::new(),
        };
        d.add_shape(s, 0);
        crate::io::save_native(&d, std::path::Path::new(&out)).unwrap();
    }

    /// Lays out "Text" in the font family `FONT_FAMILY`, curved, and reports far-out points.
    #[test]
    #[ignore = "diagnostic"]
    fn inspect_curved_layout() {
        use kurbo::Shape as _;
        let Ok(fam) = std::env::var("FONT_FAMILY") else { return };
        let lib = FontLibrary::new();
        for curve in [0.0, 110.0] {
            let t = TextData { text: "Text".into(), family: fam.clone(), size: 20.0, curve, ..TextData::default() };
            let p = lib.layout(&t);
            let els = p.elements();
            let mut no_move = 0;
            let mut after_close = false;
            for e in els {
                match e {
                    kurbo::PathEl::MoveTo(_) => after_close = false,
                    kurbo::PathEl::ClosePath => after_close = true,
                    _ if after_close => { no_move += 1; after_close = false; }
                    _ => {}
                }
            }
            println!("curve {curve}: bbox {:?}, {} elements, {} subpaths, {no_move} segments after a close without a move", p.bounding_box(), els.len(), crate::geom::subpaths(&p).len());
        }
    }

    /// Prints each glyph's outline facts for the font file in `FONT_FILE`.
    #[test]
    #[ignore = "diagnostic"]
    fn inspect_font_glyphs() {
        let Ok(f) = std::env::var("FONT_FILE") else { return };
        let data = std::fs::read(f).unwrap();
        let face = ttf_parser::Face::parse(&data, 0).unwrap();
        for ch in "Text".chars() {
            let gid = face.glyph_index(ch).unwrap();
            let mut b = Outliner { path: BezPath::new(), a: Affine::IDENTITY };
            let bb = face.outline_glyph(gid, &mut b);
            let els = b.path.elements();
            let moves = els.iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
            let closes = els.iter().filter(|e| matches!(e, kurbo::PathEl::ClosePath)).count();
            let mut longest: f64 = 0.0;
            let mut last = Point::ZERO;
            let mut start = Point::ZERO;
            let mut first_el = None;
            for (k, e) in els.iter().enumerate() {
                if k == 0 { first_el = Some(format!("{e:?}")); }
                match *e {
                    kurbo::PathEl::MoveTo(p) => { last = p; start = p; }
                    kurbo::PathEl::LineTo(p) => { longest = longest.max(last.distance(p)); last = p; }
                    kurbo::PathEl::QuadTo(_, p) | kurbo::PathEl::CurveTo(_, _, p) => { longest = longest.max(last.distance(p)); last = p; }
                    kurbo::PathEl::ClosePath => { longest = longest.max(last.distance(start)); last = start; }
                }
            }
            println!("{ch}: font bbox {bb:?}, path bbox {:?}, {} elements, {moves} moves, {closes} closes, longest segment {longest:.0}, first {first_el:?}", kurbo::Shape::bounding_box(&b.path), els.len());
        }
    }

    #[test]
    fn kerning_moves_the_letters_after_it() {
        use kurbo::Shape as _;
        let lib = FontLibrary::new();
        let t = TextData { text: "AVA".into(), family: crate::singleline::FAMILY.into(), size: 20.0, ..TextData::default() };
        let before = lib.letter_placements(&t);
        let mut k = t.clone();
        k.set_kern(0, -3.0);
        let after = lib.letter_placements(&k);
        let dx = |i: usize| after[i].1.translation().x - before[i].1.translation().x;
        assert!(dx(0).abs() < 1e-9 && (dx(1) + 3.0).abs() < 1e-9 && (dx(2) + 3.0).abs() < 1e-9);
        assert!((lib.layout(&k).bounding_box().width() - lib.layout(&t).bounding_box().width() + 3.0).abs() < 1e-6);
        k.set_kern(0, 0.0);
        assert!(k.kern.is_empty());
        assert!(lib.glyph_chars(crate::singleline::FAMILY, false, false).contains(&'A'));
    }

    #[test]
    fn curved_text_bends_and_letters_split_back_in_place() {
        use kurbo::Shape as _;
        let lib = FontLibrary::new();
        let t = TextData { text: "CURVED".into(), family: crate::singleline::FAMILY.into(), size: 20.0, ..TextData::default() };
        let straight = lib.layout(&t).bounding_box();
        let arched = lib.layout(&TextData { curve: 120.0, ..t.clone() }).bounding_box();
        assert!(arched.height() > straight.height() + 5.0, "the ends drop: {straight:?} -> {arched:?}");
        // an arch drops its ends; a negative curve lifts them (a smile)
        let y_of = |curve: f64, i: usize| lib.letter_placements(&TextData { curve, ..t.clone() })[i].1.translation().y;
        assert!(y_of(120.0, 0) > y_of(120.0, 2) + 1.0 && y_of(120.0, 5) > y_of(120.0, 3) + 1.0);
        assert!(y_of(-120.0, 0) < y_of(-120.0, 2) - 1.0);
        // ungroup to letters: each one-letter text, placed, lands on its letter
        for curve in [0.0, 120.0] {
            let t = TextData { curve, ..t.clone() };
            let letters = lib.letter_placements(&t);
            assert_eq!(letters.iter().map(|(c, _)| *c).collect::<String>(), "CURVED");
            let whole = lib.layout(&t).bounding_box();
            let mut union = kurbo::Rect::ZERO;
            for (i, (ch, a)) in letters.iter().enumerate() {
                let one = TextData { text: ch.to_string(), curve: 0.0, ..t.clone() };
                let b = (*a * lib.layout(&one)).bounding_box();
                union = if i == 0 { b } else { union.union(b) };
            }
            assert!((union.x0 - whole.x0).abs() < 0.05 && (union.y1 - whole.y1).abs() < 0.05 && (union.x1 - whole.x1).abs() < 0.05, "{curve}: {union:?} vs {whole:?}");
        }
        // ungroup to lines
        let two = TextData { text: "AB\n\nCD".into(), ..t.clone() };
        let lines = lib.line_placements(&two);
        assert_eq!(lines.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), vec!["AB", "CD"]);
        assert!(lines[1].1.translation().y > lines[0].1.translation().y);
    }

    use super::*;
    #[test]
    fn lays_out_text_with_system_font() {
        let lib = FontLibrary::new();
        if lib.families().is_empty() {
            return; // no fonts in CI
        }
        let p = lib.layout(&TextData::default());
        assert!(!p.elements().is_empty());
        let bb = kurbo::Shape::bounding_box(&p);
        assert!(bb.width() > 5.0 && bb.height() > 5.0);
    }

    #[test]
    fn lays_out_with_user_font() {
        let mut lib = FontLibrary { db: fontdb::Database::new(), families: vec![], user: vec![] };
        let mut f = UserFont::new("Mine");
        f.set_glyph('A', &[geom::rect_path(kurbo::Rect::new(0.0, -7.0, 5.0, 0.0))], 10.0, Some(0.0), 0.5).unwrap();
        lib.user.push(f);
        lib.refresh_families();
        let t = TextData { text: "AA".into(), family: "Mine".into(), size: 10.0, ..Default::default() };
        let bb = kurbo::Shape::bounding_box(&lib.layout(&t));
        assert!((bb.width() - 11.0).abs() < 1e-6, "{bb:?}"); // 5 + 1 bearing + 5
    }

    /// Contact sheet: single-line alphabet + a script font unwelded vs welded.
    /// `cargo test -p cutcraft-core preview_text -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_text() {
        let lib = FontLibrary::new();
        let (w, row) = (1200u32, 170u32);
        let mut img = image::RgbaImage::from_pixel(w, row * 4, image::Rgba([255, 255, 255, 255]));
        // even-odd fill of closed outlines
        let fill = |img: &mut image::RgbaImage, p: &BezPath, oy: f64, k: f64, col: [u8; 3]| {
            let polys = geom::flatten(p, 0.02);
            for py in 0..row {
                let y = (py as f64 + 0.5) / k;
                let mut xs: Vec<f64> = Vec::new();
                for pl in polys.iter().filter(|pl| pl.closed) {
                    let n = pl.pts.len();
                    for i in 0..n {
                        let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
                        if (a.y > y) != (b.y > y) {
                            xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                        }
                    }
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for c in xs.chunks(2) {
                    if c.len() == 2 {
                        for px in ((c[0] * k).max(0.0) as u32)..((c[1] * k).min(w as f64) as u32) {
                            img.put_pixel(px, (oy + py as f64) as u32, image::Rgba([col[0], col[1], col[2], 255]));
                        }
                    }
                }
            }
        };
        let stroke = |img: &mut image::RgbaImage, p: &BezPath, oy: f64, k: f64| {
            for pl in geom::flatten(p, 0.02) {
                for s in pl.pts.windows(2) {
                    let n = ((s[1] - s[0]).hypot() * k).ceil().max(1.0) as usize;
                    for j in 0..=n {
                        let q = s[0].lerp(s[1], j as f64 / n as f64);
                        for d in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let (x, y) = ((q.x * k) as i64 + d.0, (oy + q.y * k) as i64 + d.1);
                            if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < img.height() {
                                img.put_pixel(x as u32, y as u32, image::Rgba([30, 30, 160, 255]));
                            }
                        }
                    }
                }
            }
        };
        let k = 4.0; // px per mm
        let at = |p: BezPath, dx: f64, dy: f64| { let mut p = p; p.apply_affine(Affine::translate((dx, dy))); p };
        let sl = |t: &str| lib.layout(&TextData { text: t.into(), family: crate::singleline::FAMILY.into(), size: 12.0, ..Default::default() });
        stroke(&mut img, &at(sl("ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789"), 4.0, 6.0), 0.0, k);
        stroke(&mut img, &at(sl("abcdefghijklmnopqrstuvwxyz  !?&@#%"), 4.0, 22.0), 0.0, k);
        let script = TextData { text: "Welcome".into(), family: "Dancing Script".into(), size: 32.0, ..Default::default() };
        fill(&mut img, &at(lib.layout(&script), 4.0, 2.0), row as f64 * 2.0, k, [200, 40, 40]);
        fill(&mut img, &at(lib.layout(&TextData { weld: true, ..script }), 4.0, 2.0), row as f64 * 3.0, k, [20, 120, 40]);
        let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/text_preview.png");
        img.save(out).unwrap();
        println!("wrote {out}");
    }

    #[test]
    #[ignore]
    fn measure_script_weld() {
        let lib = FontLibrary::new();
        for fam in ["Dancing Script", "Kaushan Script"] {
            let t = TextData { text: "Welcome".into(), family: fam.into(), size: 32.0, ..Default::default() };
            let loose = lib.layout(&t);
            let welded = lib.layout(&TextData { weld: true, ..t });
            // area under even-odd (what an unwelded cut would produce) vs true union
            let eo = geom::from_multipolygon(&geom::to_multipolygon(&[loose.clone()]));
            use kurbo::Shape as _;
            let area = |p: &BezPath| geom::subpaths(p).iter().map(|s| s.area()).sum::<f64>().abs();
            println!("{fam}: subpaths loose={} welded={} | area even-odd={:.1} welded={:.1} mm2",
                geom::subpaths(&loose).len(), geom::subpaths(&welded).len(), area(&eo), area(&welded));
        }
    }

    fn lib_with_block_font() -> FontLibrary {
        let mut lib = FontLibrary { db: fontdb::Database::new(), families: vec![], user: vec![] };
        let mut f = UserFont::new("Block");
        f.set_glyph('A', &[geom::rect_path(kurbo::Rect::new(0.0, -7.0, 5.0, 0.0))], 10.0, Some(0.0), 0.5).unwrap();
        lib.user.push(f);
        lib.refresh_families();
        lib
    }

    #[test]
    fn weld_merges_overlapping_letters() {
        let lib = lib_with_block_font();
        // negative spacing makes the 5 mm letters overlap by ~2 mm
        let t = TextData { text: "AAA".into(), family: "Block".into(), size: 10.0, char_spacing: -30.0, ..Default::default() };
        let loose = lib.layout(&t);
        assert_eq!(geom::subpaths(&loose).len(), 3, "unwelded: one outline per letter");
        let welded = lib.layout(&TextData { weld: true, ..t });
        assert_eq!(geom::subpaths(&welded).len(), 1, "welded: one merged outline");
    }

    #[test]
    fn union_each_keeps_a_letters_own_hole() {
        // an "o": outer ring + inner hole in ONE path must stay a ring
        let o = geom::boolean(
            &[geom::rect_path(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0))],
            &[geom::rect_path(kurbo::Rect::new(3.0, 3.0, 7.0, 7.0))],
            geom::BoolOp::Difference,
        );
        assert_eq!(geom::subpaths(&geom::union_each(&[o])).len(), 2);
    }

    #[test]
    fn single_line_font_lays_out_open_strokes() {
        let lib = FontLibrary { db: fontdb::Database::new(), families: vec![], user: vec![] };
        let t = TextData { text: "HI".into(), family: crate::singleline::FAMILY.into(), size: 10.0, ..Default::default() };
        let p = lib.layout(&t);
        let bb = kurbo::Shape::bounding_box(&p);
        assert!((bb.height() - 7.0).abs() < 1e-6, "capitals are 0.7 em tall: {bb:?}");
        assert!(!p.elements().iter().any(|e| matches!(e, kurbo::PathEl::ClosePath)), "strokes stay open");
        // welding is ignored for strokes (it would otherwise erase them)
        let w = lib.layout(&TextData { weld: true, ..t });
        assert_eq!(geom::subpaths(&w).len(), geom::subpaths(&p).len());
    }

    #[test]
    fn single_line_family_is_listed_first() {
        let mut lib = FontLibrary { db: fontdb::Database::new(), families: vec![], user: vec![] };
        lib.refresh_families();
        assert_eq!(lib.families().first().map(String::as_str), Some(crate::singleline::FAMILY));
    }
}
