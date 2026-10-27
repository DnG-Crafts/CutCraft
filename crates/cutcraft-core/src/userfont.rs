//! Font creation: user-drawn glyphs stored as CutCraft fonts (JSON) and
//! exportable as a TrueType (.ttf) file installable in any program.

use crate::geom::{self, svg_path_serde};
use anyhow::{Result, bail};
use kurbo::{Affine, BezPath, CubicBez, PathEl, Point, Shape as _};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const UPM: f64 = 1000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserGlyph {
    /// Outline in font units, baseline at y = 0, y grows downwards.
    #[serde(with = "svg_path_serde")]
    pub path: BezPath,
    pub advance: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserFont {
    pub name: String,
    pub glyphs: BTreeMap<char, UserGlyph>,
    pub ascender: f64,
    pub descender: f64,
    pub space_advance: f64,
}

impl UserFont {
    pub fn new(name: &str) -> UserFont {
        UserFont { name: name.into(), glyphs: BTreeMap::new(), ascender: 800.0, descender: -200.0, space_advance: 300.0 }
    }

    pub fn dir() -> PathBuf {
        crate::prefs::config_dir().join("fonts")
    }

    pub fn file_name(&self) -> String {
        let safe: String = self.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        format!("{safe}.ccfont.json")
    }

    pub fn save(&self) -> Result<PathBuf> {
        let d = Self::dir();
        std::fs::create_dir_all(&d)?;
        let p = d.join(self.file_name());
        std::fs::write(&p, serde_json::to_vec_pretty(self)?)?;
        Ok(p)
    }

    pub fn load_all() -> Vec<UserFont> {
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(Self::dir()) {
            for e in rd.flatten() {
                if e.path().to_string_lossy().ends_with(".ccfont.json") {
                    if let Ok(f) = std::fs::read(e.path()).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                        v.push(f);
                    }
                }
            }
        }
        v
    }

    pub fn delete(&self) -> Result<()> {
        std::fs::remove_file(Self::dir().join(self.file_name()))?;
        Ok(())
    }

    /// Create a glyph from page-space paths. `em_mm` is the size the letter was drawn at
    /// (cap height ≈ 0.7 em); `baseline_y` defaults to the bottom of the artwork.
    pub fn set_glyph(&mut self, ch: char, paths: &[BezPath], em_mm: f64, baseline_y: Option<f64>, side_bearing_mm: f64) -> Result<()> {
        let Some(bb) = geom::paths_bbox(paths) else { bail!("Nothing selected for '{ch}'") };
        if em_mm <= 0.0 {
            bail!("Em size must be positive");
        }
        let base = baseline_y.unwrap_or(bb.y1);
        let k = UPM / em_mm;
        let a = Affine::scale(k) * Affine::translate((-bb.x0 + side_bearing_mm, -base));
        let mut path = geom::merge_paths(paths);
        path.apply_affine(a);
        let advance = (bb.width() + 2.0 * side_bearing_mm) * k;
        self.glyphs.insert(ch, UserGlyph { path, advance });
        Ok(())
    }

    // ------------------------------------------------------------ TrueType export

    pub fn to_ttf(&self) -> Result<Vec<u8>> {
        let chars: Vec<char> = self.glyphs.keys().copied().filter(|c| (*c as u32) < 0xFFFF).collect();
        if chars.is_empty() {
            bail!("The font has no glyphs");
        }
        let mut has_space = chars.contains(&' ');
        let mut order: Vec<Option<char>> = vec![None]; // .notdef
        order.extend(chars.iter().map(|c| Some(*c)));
        if !has_space {
            order.push(Some(' '));
            has_space = true;
        }
        let _ = has_space;

        struct G {
            data: Vec<u8>,
            adv: u16,
            xmin: i16,
            bbox: [i16; 4],
            points: usize,
            contours: usize,
        }
        let mut glyphs: Vec<G> = vec![];
        for c in &order {
            let (path, adv) = match c {
                Some(' ') if !self.glyphs.contains_key(&' ') => (BezPath::new(), self.space_advance),
                Some(c) => {
                    let g = &self.glyphs[c];
                    (g.path.clone(), g.advance)
                }
                None => (BezPath::new(), UPM / 2.0),
            };
            let contours = tt_contours(&path);
            let (data, bbox, pts) = encode_glyph(&contours);
            glyphs.push(G { data, adv: adv.round().clamp(0.0, 65535.0) as u16, xmin: bbox[0], bbox, points: pts, contours: contours.len() });
        }
        let n = glyphs.len() as u16;
        let fb = glyphs.iter().filter(|g| !g.data.is_empty()).fold([i16::MAX, i16::MAX, i16::MIN, i16::MIN], |a, g| {
            [a[0].min(g.bbox[0]), a[1].min(g.bbox[1]), a[2].max(g.bbox[2]), a[3].max(g.bbox[3])]
        });
        let fb = if fb[0] == i16::MAX { [0, 0, 0, 0] } else { fb };
        let asc = self.ascender.round() as i16;
        let desc = self.descender.round() as i16;

        // glyf + loca
        let mut glyf = vec![];
        let mut loca = vec![];
        for g in &glyphs {
            loca.extend((glyf.len() as u32).to_be_bytes());
            glyf.extend(&g.data);
            while glyf.len() % 4 != 0 {
                glyf.push(0);
            }
        }
        loca.extend((glyf.len() as u32).to_be_bytes());

        // hmtx
        let mut hmtx = vec![];
        for g in &glyphs {
            hmtx.extend(g.adv.to_be_bytes());
            hmtx.extend(if g.data.is_empty() { 0i16 } else { g.xmin }.to_be_bytes());
        }

        // head
        let mut head = vec![];
        head.extend(0x0001_0000u32.to_be_bytes());
        head.extend(0x0001_0000u32.to_be_bytes()); // fontRevision 1.0
        head.extend(0u32.to_be_bytes()); // checkSumAdjustment (patched later)
        head.extend(0x5F0F_3CF5u32.to_be_bytes());
        head.extend(0x000Bu16.to_be_bytes());
        head.extend((UPM as u16).to_be_bytes());
        head.extend(0i64.to_be_bytes());
        head.extend(0i64.to_be_bytes());
        for v in fb {
            head.extend(v.to_be_bytes());
        }
        head.extend(0u16.to_be_bytes()); // macStyle
        head.extend(8u16.to_be_bytes()); // lowestRecPPEM
        head.extend(2i16.to_be_bytes()); // fontDirectionHint
        head.extend(1i16.to_be_bytes()); // indexToLocFormat: long
        head.extend(0i16.to_be_bytes());

        // hhea
        let adv_max = glyphs.iter().map(|g| g.adv).max().unwrap_or(0);
        let min_lsb = glyphs.iter().filter(|g| !g.data.is_empty()).map(|g| g.xmin).min().unwrap_or(0);
        let min_rsb = glyphs.iter().filter(|g| !g.data.is_empty()).map(|g| g.adv as i32 - g.bbox[2] as i32).min().unwrap_or(0) as i16;
        let mut hhea = vec![];
        hhea.extend(0x0001_0000u32.to_be_bytes());
        hhea.extend(asc.to_be_bytes());
        hhea.extend(desc.to_be_bytes());
        hhea.extend(0i16.to_be_bytes());
        hhea.extend(adv_max.to_be_bytes());
        hhea.extend(min_lsb.to_be_bytes());
        hhea.extend(min_rsb.to_be_bytes());
        hhea.extend(fb[2].to_be_bytes());
        hhea.extend(1i16.to_be_bytes());
        hhea.extend(0i16.to_be_bytes());
        hhea.extend(0i16.to_be_bytes());
        hhea.extend([0u8; 8]);
        hhea.extend(0i16.to_be_bytes());
        hhea.extend(n.to_be_bytes());

        // maxp
        let mut maxp = vec![];
        maxp.extend(0x0001_0000u32.to_be_bytes());
        maxp.extend(n.to_be_bytes());
        maxp.extend((glyphs.iter().map(|g| g.points).max().unwrap_or(0) as u16).to_be_bytes());
        maxp.extend((glyphs.iter().map(|g| g.contours).max().unwrap_or(0) as u16).to_be_bytes());
        maxp.extend(0u16.to_be_bytes());
        maxp.extend(0u16.to_be_bytes());
        maxp.extend(2u16.to_be_bytes());
        for _ in 0..8 {
            maxp.extend(0u16.to_be_bytes());
        }

        // cmap format 4, one segment per character
        let mut segs: Vec<(u16, u16)> = order.iter().enumerate().filter_map(|(gid, c)| c.map(|c| (c as u16, gid as u16))).collect();
        segs.sort();
        segs.dedup_by_key(|s| s.0);
        let seg_count = segs.len() as u16 + 1;
        let mut sub = vec![];
        let p2 = 2u16.pow((seg_count as f64).log2().floor() as u32);
        sub.extend(4u16.to_be_bytes());
        sub.extend((16 + 8 * seg_count).to_be_bytes());
        sub.extend(0u16.to_be_bytes());
        sub.extend((seg_count * 2).to_be_bytes());
        sub.extend((p2 * 2).to_be_bytes());
        sub.extend(((p2 as f64).log2() as u16).to_be_bytes());
        sub.extend((seg_count * 2 - p2 * 2).to_be_bytes());
        for (c, _) in &segs {
            sub.extend(c.to_be_bytes());
        }
        sub.extend(0xFFFFu16.to_be_bytes());
        sub.extend(0u16.to_be_bytes());
        for (c, _) in &segs {
            sub.extend(c.to_be_bytes());
        }
        sub.extend(0xFFFFu16.to_be_bytes());
        for (c, g) in &segs {
            sub.extend(g.wrapping_sub(*c).to_be_bytes());
        }
        sub.extend(1u16.to_be_bytes());
        for _ in 0..seg_count {
            sub.extend(0u16.to_be_bytes());
        }
        let mut cmap = vec![];
        cmap.extend(0u16.to_be_bytes());
        cmap.extend(1u16.to_be_bytes());
        cmap.extend(3u16.to_be_bytes());
        cmap.extend(1u16.to_be_bytes());
        cmap.extend(12u32.to_be_bytes());
        cmap.extend(sub);

        // name
        let names: [(u16, String); 5] = [
            (1, self.name.clone()),
            (2, "Regular".into()),
            (3, format!("CutCraft:{}", self.name)),
            (4, self.name.clone()),
            (6, self.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().max_len_or("CutCraftFont")),
        ];
        let mut strings = vec![];
        let mut records = vec![];
        for (id, s) in &names {
            let enc: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            records.extend(3u16.to_be_bytes());
            records.extend(1u16.to_be_bytes());
            records.extend(0x0409u16.to_be_bytes());
            records.extend(id.to_be_bytes());
            records.extend((enc.len() as u16).to_be_bytes());
            records.extend((strings.len() as u16).to_be_bytes());
            strings.extend(enc);
        }
        let mut name = vec![];
        name.extend(0u16.to_be_bytes());
        name.extend((names.len() as u16).to_be_bytes());
        name.extend((6 + 12 * names.len() as u16).to_be_bytes());
        name.extend(records);
        name.extend(strings);

        // post v3
        let mut post = vec![];
        post.extend(0x0003_0000u32.to_be_bytes());
        post.extend(0u32.to_be_bytes());
        post.extend((-100i16).to_be_bytes());
        post.extend(50i16.to_be_bytes());
        post.extend([0u8; 20]);

        // OS/2 v4
        let first = segs.first().map(|s| s.0).unwrap_or(32);
        let last = segs.last().map(|s| s.0).unwrap_or(32);
        let avg = (glyphs.iter().map(|g| g.adv as u32).sum::<u32>() / n.max(1) as u32) as i16;
        let mut os2 = vec![];
        os2.extend(4u16.to_be_bytes());
        os2.extend(avg.to_be_bytes());
        os2.extend(400u16.to_be_bytes());
        os2.extend(5u16.to_be_bytes());
        os2.extend(0u16.to_be_bytes()); // fsType: installable
        for v in [650i16, 600, 0, 75, 650, 600, 0, 350, 50, 250] {
            os2.extend(v.to_be_bytes());
        }
        os2.extend(0i16.to_be_bytes());
        os2.extend([0u8; 10]); // panose
        os2.extend(1u32.to_be_bytes());
        os2.extend([0u8; 12]);
        os2.extend(*b"CCFT");
        os2.extend(0x0040u16.to_be_bytes());
        os2.extend(first.to_be_bytes());
        os2.extend(last.to_be_bytes());
        os2.extend(asc.to_be_bytes());
        os2.extend(desc.to_be_bytes());
        os2.extend(0i16.to_be_bytes());
        os2.extend((asc.max(fb[3]) as u16).to_be_bytes());
        os2.extend(((-desc).max(-fb[1]) as u16).to_be_bytes());
        os2.extend(1u32.to_be_bytes());
        os2.extend(0u32.to_be_bytes());
        os2.extend(500i16.to_be_bytes());
        os2.extend(700i16.to_be_bytes());
        os2.extend(0u16.to_be_bytes());
        os2.extend(32u16.to_be_bytes());
        os2.extend(0u16.to_be_bytes());

        let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
            (b"OS/2", os2),
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"loca", loca),
            (b"maxp", maxp),
            (b"name", name),
            (b"post", post),
        ];
        let nt = tables.len() as u16;
        let p2 = 2u16.pow((nt as f64).log2().floor() as u32);
        let mut out = vec![];
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend(nt.to_be_bytes());
        out.extend((p2 * 16).to_be_bytes());
        out.extend(((p2 as f64).log2() as u16).to_be_bytes());
        out.extend((nt * 16 - p2 * 16).to_be_bytes());
        let mut offset = 12 + 16 * tables.len();
        let mut body = vec![];
        let mut head_off = 0;
        for (tag, data) in &tables {
            out.extend(*tag);
            out.extend(checksum(data).to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            if *tag == b"head" {
                head_off = offset;
            }
            let mut d = data.clone();
            while d.len() % 4 != 0 {
                d.push(0);
            }
            offset += d.len();
            body.extend(d);
        }
        out.extend(body);
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[head_off + 8..head_off + 12].copy_from_slice(&adj.to_be_bytes());
        Ok(out)
    }

    pub fn export_ttf(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_ttf()?)?;
        Ok(())
    }
}

trait OrDefault {
    fn max_len_or(self, d: &str) -> String;
}
impl OrDefault for String {
    fn max_len_or(self, d: &str) -> String {
        if self.is_empty() { d.into() } else { self.chars().take(60).collect() }
    }
}

fn checksum(d: &[u8]) -> u32 {
    let mut s = 0u32;
    for c in d.chunks(4) {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        s = s.wrapping_add(u32::from_be_bytes(b));
    }
    s
}

/// TrueType contours: (point, on_curve), y-up font units.
fn tt_contours(path: &BezPath) -> Vec<Vec<(Point, bool)>> {
    let flip = |p: Point| Point::new(p.x.round(), (-p.y).round());
    let mut out = vec![];
    let mut cur: Vec<(Point, bool)> = vec![];
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    let finish = |cur: &mut Vec<(Point, bool)>, out: &mut Vec<Vec<(Point, bool)>>| {
        if cur.len() > 1 && cur.last().unwrap().1 && cur.last().unwrap().0 == cur[0].0 {
            cur.pop();
        }
        if cur.len() >= 3 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut cur, &mut out);
                cur.push((flip(p), true));
                last = p;
                start = p;
            }
            PathEl::LineTo(p) => {
                cur.push((flip(p), true));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                cur.push((flip(c), false));
                cur.push((flip(p), true));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                for (_, _, q) in CubicBez::new(last, a, b, p).to_quads(1.0) {
                    cur.push((flip(q.p1), false));
                    cur.push((flip(q.p2), true));
                }
                last = p;
            }
            PathEl::ClosePath => {
                let _ = start;
                finish(&mut cur, &mut out);
            }
        }
    }
    finish(&mut cur, &mut out);
    out
}

fn encode_glyph(contours: &[Vec<(Point, bool)>]) -> (Vec<u8>, [i16; 4], usize) {
    if contours.is_empty() {
        return (vec![], [0; 4], 0);
    }
    let pts: Vec<(Point, bool)> = contours.iter().flatten().copied().collect();
    let r = geom::bbox_points(&pts.iter().map(|p| p.0).collect::<Vec<_>>());
    let bbox = [r.x0 as i16, r.y0 as i16, r.x1 as i16, r.y1 as i16];
    let mut d = vec![];
    d.extend((contours.len() as i16).to_be_bytes());
    for v in bbox {
        d.extend(v.to_be_bytes());
    }
    let mut end = 0usize;
    for c in contours {
        end += c.len();
        d.extend(((end - 1) as u16).to_be_bytes());
    }
    d.extend(0u16.to_be_bytes()); // no instructions
    for (_, on) in &pts {
        d.push(if *on { 1 } else { 0 }); // int16 deltas for x and y
    }
    let mut prev = 0i32;
    for (p, _) in &pts {
        let x = p.x as i32;
        d.extend(((x - prev) as i16).to_be_bytes());
        prev = x;
    }
    prev = 0;
    for (p, _) in &pts {
        let y = p.y as i32;
        d.extend(((y - prev) as i16).to_be_bytes());
        prev = y;
    }
    (d, bbox, pts.len())
}

/// Bounding-box helper so callers can preview glyph sizes.
pub fn glyph_bounds(g: &UserGlyph) -> kurbo::Rect {
    g.path.bounding_box()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;

    #[test]
    fn ttf_roundtrip_with_ttf_parser() {
        let mut f = UserFont::new("Test Hand");
        let a = geom::rect_path(Rect::new(0.0, -7.0, 5.0, 0.0));
        let o = geom::circle_path(Point::new(3.0, -3.5), 3.0);
        f.set_glyph('A', &[a], 10.0, Some(0.0), 0.5).unwrap();
        f.set_glyph('o', &[o], 10.0, Some(0.0), 0.5).unwrap();
        let ttf = f.to_ttf().unwrap();
        let face = ttf_parser::Face::parse(&ttf, 0).expect("valid ttf");
        assert_eq!(face.units_per_em(), 1000);
        let gid = face.glyph_index('A').unwrap();
        let bb = face.glyph_bounding_box(gid).unwrap();
        assert_eq!((bb.x_min, bb.y_min, bb.x_max, bb.y_max), (50, 0, 550, 700));
        assert_eq!(face.glyph_hor_advance(gid), Some(600));
        let go = face.glyph_index('o').unwrap();
        assert!(face.glyph_bounding_box(go).is_some());
        assert!(face.glyph_index(' ').is_some());
        assert_eq!(checksum(&ttf), 0xB1B0_AFBA);
    }
}
