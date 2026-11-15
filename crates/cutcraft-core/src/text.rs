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
