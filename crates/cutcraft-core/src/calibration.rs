//! Print & Cut calibration: a test sheet that shows how far the cutter's
//! lines land from the print, measured from a photo (or read off printed
//! rulers), and the correction that cancels it out.
//!
//! The sheet has registration marks and five printed black dots (near the
//! corners of the safe area, and the middle). The cutter cuts, or draws with a
//! pen, a square around each dot. In a photo of the result the four corner
//! dots fix the perspective; each square's centre, against where its dot
//! says it should be, is that spot's error. Fine grey rulers across the
//! squares' left and top edges let the same error be read by eye.

use crate::doc::{CutStyle, Document, Page, Rgba, Style};
use crate::{geom, pixscan};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _, Vec2};
use serde::{Deserialize, Serialize};

/// Printed dot radius, mm.
pub const DOT_R: f64 = 2.5;
/// Side of the square cut around each dot, mm.
pub const BOX: f64 = 20.0;
/// How far each ruler reads either side of zero, mm.
pub const RULER: f64 = 3.0;
pub const PAGE_NAME: &str = "Print & Cut calibration";
/// Names of the targets, in [`targets`] order.
pub const TARGET_NAMES: [&str; 5] = ["Top left", "Top right", "Bottom right", "Bottom left", "Middle"];

/// Where the dots go on `page`: top left, top right, bottom right, bottom
/// left, middle (mm).
pub fn targets(page: &Page) -> [Point; 5] {
    let safe = page.regmarks.safe_area(page.width, page.height);
    // keep the squares and their rulers inside the safe area
    let m = BOX / 2.0 + RULER + 4.0;
    let r = safe.inflate(-m, -m);
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1), r.center()]
}

/// Adds the calibration sheet as a new page (the current page's size and
/// mat) and makes it the active page. Returns its index.
pub fn add_sheet(doc: &mut Document) -> usize {
    let cur = doc.page();
    let mut page = Page::new(PAGE_NAME, cur.width, cur.height);
    page.mat = cur.mat;
    page.regmarks.enabled = true;
    doc.pages.push(page);
    doc.active = doc.pages.len() - 1;
    let ink = |c: Rgba| Style { fill: Some(c), stroke: None, stroke_width: 0.0, cut: CutStyle::NoCut, no_print: false };
    let grey = Rgba(150, 150, 150, 255);
    let cut = Style { fill: None, stroke: Some(Rgba::RED), stroke_width: 0.25, cut: CutStyle::Cut, no_print: true };
    for t in targets(doc.page()) {
        doc.add_path(kurbo::Circle::new(t, DOT_R).to_path(0.01), ink(Rgba::BLACK), 0);
        for p in ruler(t) {
            doc.add_path(p, ink(grey), 0);
        }
        doc.add_path(geom::rect_path(Rect::from_center_size(t, (BOX, BOX))), cut.clone(), 0);
    }
    doc.active
}

/// The two rulers of the target at `t`: one across the square's left edge
/// (read the x error where the line crosses it) and one across its top edge
/// (the y error). Ticks every 0.5 mm, longer each whole mm and at zero.
fn ruler(t: Point) -> Vec<BezPath> {
    let thin = 0.12;
    let mut v = vec![];
    let (lx, ty) = (t.x - BOX / 2.0, t.y - BOX / 2.0);
    // a horizontal ruler through the middle of the left edge
    v.push(geom::rect_path(Rect::new(lx - RULER, t.y - thin / 2.0, lx + RULER, t.y + thin / 2.0)));
    // a vertical one through the middle of the top edge
    v.push(geom::rect_path(Rect::new(t.x - thin / 2.0, ty - RULER, t.x + thin / 2.0, ty + RULER)));
    let steps = (RULER / 0.5) as i32;
    for i in -steps..=steps {
        let d = i as f64 * 0.5;
        let len = if i == 0 { 2.4 } else if i % 2 == 0 { 1.6 } else { 0.9 };
        v.push(geom::rect_path(Rect::new(lx + d - thin / 2.0, t.y - len, lx + d + thin / 2.0, t.y)));
        v.push(geom::rect_path(Rect::new(t.x - len, ty + d - thin / 2.0, t.x, ty + d + thin / 2.0)));
    }
    v
}

/// A correction applied to Print & Cut lines (page mm) before they're sent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub coeffs: [f64; 6],
}

impl Calibration {
    pub fn affine(&self) -> Affine {
        Affine::new(self.coeffs)
    }

    /// How the cuts were off before correcting: shift at the page centre (mm,
    /// right and down positive), rotation (degrees) and size (% too big).
    pub fn describe(&self, page: &Page) -> String {
        let err = self.affine().inverse();
        let c = Point::new(page.width / 2.0, page.height / 2.0);
        let shift = err * c - c;
        let [a, b, ..] = err.as_coeffs();
        let rot = b.atan2(a).to_degrees();
        let scale = ((a * a + b * b).sqrt() - 1.0) * 100.0;
        format!(
            "cuts were {:.2} mm {}, {:.2} mm {}, turned {:.2}°, {:+.2}% in size",
            shift.x.abs(),
            if shift.x >= 0.0 { "right" } else { "left" },
            shift.y.abs(),
            if shift.y >= 0.0 { "low" } else { "high" },
            rot,
            scale
        )
    }
}

/// The correction that cancels the error seen at the targets: `expected[i]`
/// is where a square should have been and `actual[i]` where it landed.
/// Three or more spots give the full shift, turn, size and skew; fewer give
/// only the average shift.
pub fn fit(expected: &[Point], actual: &[Point]) -> Option<Calibration> {
    let n = expected.len().min(actual.len());
    if n == 0 {
        return None;
    }
    let err = affine_fit(&expected[..n], &actual[..n]).unwrap_or_else(|| {
        let d = (0..n).map(|i| actual[i] - expected[i]).fold(Vec2::ZERO, |a, b| a + b) / n as f64;
        Affine::translate(d)
    });
    if err.determinant().abs() < 1e-9 {
        return None;
    }
    Some(Calibration { coeffs: err.inverse().as_coeffs() })
}

/// Least-squares affine taking `from` to `to`; `None` if the points don't
/// span an area.
fn affine_fit(from: &[Point], to: &[Point]) -> Option<Affine> {
    if from.len() < 3 {
        return None;
    }
    // normal equations: [x y 1] · (a c e)ᵀ = u, likewise for v
    let mut m = [[0.0f64; 3]; 3];
    let (mut bu, mut bv) = ([0.0f64; 3], [0.0f64; 3]);
    for (p, q) in from.iter().zip(to) {
        let r = [p.x, p.y, 1.0];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += r[i] * r[j];
            }
            bu[i] += r[i] * q.x;
            bv[i] += r[i] * q.y;
        }
    }
    let u = solve3(m, bu)?;
    let v = solve3(m, bv)?;
    // kurbo order: [a b c d e f] = x' = a x + c y + e, y' = b x + d y + f
    Some(Affine::new([u[0], v[0], u[1], v[1], u[2], v[2]]))
}

fn solve3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-9 * (1.0 + m[0][0].abs() * m[1][1].abs()) {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for i in 0..3 {
            mk[i][k] = b[i];
        }
        *o = det(&mk) / d;
    }
    Some(out)
}

/// Moves every point of `job` by the correction.
pub fn correct_job(job: &mut crate::cutjob::CutJob, c: &Calibration) {
    let a = c.affine();
    for pass in &mut job.passes {
        for pl in &mut pass.paths {
            for p in &mut pl.pts {
                *p = a * *p;
            }
        }
    }
}

// ------------------------------------------------------------------ photo

/// Where each square landed (page mm, [`targets`] order), found in a photo or
/// scan of the cut sheet; `None` for a square that couldn't be found.
pub fn measure_photo(img: &image::RgbaImage, page: &Page) -> Result<[Option<Point>; 5], String> {
    let scale = (1800.0 / img.width().max(img.height()) as f64).min(1.0);
    let small = if scale < 1.0 {
        image::imageops::resize(img, ((img.width() as f64 * scale) as u32).max(1), ((img.height() as f64 * scale) as u32).max(1), image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let (w, h) = (small.width() as usize, small.height() as usize);
    let lum: Vec<f32> = small.pixels().map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect();
    let mask = dark_mask(&lum, w, h);
    let blobs = components(&mask, w, h);

    let min_dot = (w * h) as f64 * 2e-5;
    let dots: Vec<&Blob> = blobs.iter().filter(|b| b.n as f64 >= min_dot && b.fill() > 0.6 && (0.6..1.7).contains(&b.aspect())).collect();
    let squares: Vec<&Blob> = blobs.iter().filter(|b| b.fill() < 0.35 && (0.6..1.7).contains(&b.aspect())).collect();
    // a target is a dot with a square around it, the dot near the square's middle
    let mut pairs: Vec<(Point, &Blob)> = vec![];
    for d in &dots {
        let c = d.centroid();
        let around = squares.iter().filter(|s| {
            let r = s.rect();
            r.contains(c) && r.width() > 2.5 * d.rect().width() && (c - r.center()).hypot() < r.width() * 0.3
        });
        if let Some(s) = around.min_by(|a, b| a.rect().area().total_cmp(&b.rect().area())) {
            pairs.push((c, s));
        }
    }
    if pairs.len() < 4 {
        return Err(format!(
            "Found {} of the squares - make sure the whole sheet is in the photo, evenly lit, and the squares were drawn with a pen (cut lines barely show in a photo). Or read the rulers and type the numbers in.",
            pairs.len()
        ));
    }
    // the corner targets are the extreme ones
    let pick = |f: &dyn Fn(Point) -> f64| pairs.iter().enumerate().max_by(|a, b| f(a.1.0).total_cmp(&f(b.1.0))).map(|(i, _)| i).unwrap();
    let corners = [pick(&|p| -p.x - p.y), pick(&|p| p.x - p.y), pick(&|p| p.x + p.y), pick(&|p| -p.x + p.y)];
    let mut uniq = corners.to_vec();
    uniq.sort();
    uniq.dedup();
    if uniq.len() < 4 {
        return Err("Couldn't tell the four corner squares apart - photograph the sheet straight on.".into());
    }
    let want = targets(page);
    let src = corners.map(|i| pairs[i].0);
    let dst = [want[0], want[1], want[2], want[3]];
    let hm = pixscan::homography(&src, &dst).ok_or("Couldn't work out the photo's perspective")?;
    let mut out = [None; 5];
    let place = |s: &Blob| -> Option<Point> {
        // the square's pixels on the page; its middle is where it was cut
        let mut r: Option<Rect> = None;
        for &(x, y) in &s.px {
            let p = pixscan::apply(&hm, Point::new(x as f64 + 0.5, y as f64 + 0.5));
            r = Some(r.map_or(Rect::from_points(p, p), |r| r.union_pt(p)));
        }
        let r = r?;
        // its line has some width: the middle of the outline is what counts
        ((r.width() - BOX).abs() < 2.0 && (r.height() - BOX).abs() < 2.0).then(|| r.center())
    };
    for (k, &i) in corners.iter().enumerate() {
        out[k] = place(pairs[i].1);
    }
    // the middle one: the pair whose dot lands nearest the middle target
    let middle = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| !corners.contains(i))
        .map(|(_, (c, s))| (pixscan::apply(&hm, *c), s))
        .min_by(|a, b| (a.0 - want[4]).hypot().total_cmp(&(b.0 - want[4]).hypot()));
    if let Some((c, s)) = middle {
        if (c - want[4]).hypot() < BOX / 2.0 {
            out[4] = place(s);
        }
    }
    if out.iter().take(4).any(|o| o.is_none()) {
        return Err("Some squares came out the wrong size in the photo - photograph the sheet straight on, filling the frame.".into());
    }
    Ok(out)
}

/// Pixels clearly darker than their surroundings: ink and pen lines, but not
/// the grey rulers or shading across the paper.
fn dark_mask(lum: &[f32], w: usize, h: usize) -> Vec<bool> {
    // integral image for the local average
    let mut sum = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0;
        for x in 0..w {
            row += lum[y * w + x] as f64;
            sum[(y + 1) * (w + 1) + x + 1] = sum[y * (w + 1) + x + 1] + row;
        }
    }
    let r = (w.max(h) / 25).max(4);
    let mut mask = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sum[y1 * (w + 1) + x1] - sum[y0 * (w + 1) + x1] - sum[y1 * (w + 1) + x0] + sum[y0 * (w + 1) + x0];
            let mean = s / ((y1 - y0) * (x1 - x0)) as f64;
            mask[y * w + x] = (lum[y * w + x] as f64) < mean * 0.6;
        }
    }
    mask
}

struct Blob {
    n: usize,
    px: Vec<(u32, u32)>,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    sx: f64,
    sy: f64,
}

impl Blob {
    fn rect(&self) -> Rect {
        Rect::new(self.x0 as f64, self.y0 as f64, self.x1 as f64 + 1.0, self.y1 as f64 + 1.0)
    }
    fn fill(&self) -> f64 {
        self.n as f64 / self.rect().area()
    }
    fn aspect(&self) -> f64 {
        self.rect().width() / self.rect().height()
    }
    fn centroid(&self) -> Point {
        Point::new(self.sx / self.n as f64 + 0.5, self.sy / self.n as f64 + 0.5)
    }
}

/// 8-connected groups of dark pixels (the tiny ones dropped).
fn components(mask: &[bool], w: usize, h: usize) -> Vec<Blob> {
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    for start in 0..w * h {
        if seen[start] || !mask[start] {
            continue;
        }
        let mut b = Blob { n: 0, px: vec![], x0: usize::MAX, y0: usize::MAX, x1: 0, y1: 0, sx: 0.0, sy: 0.0 };
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            b.n += 1;
            b.px.push((x as u32, y as u32));
            b.sx += x as f64;
            b.sy += y as f64;
            (b.x0, b.y0, b.x1, b.y1) = (b.x0.min(x), b.y0.min(y), b.x1.max(x), b.y1.max(y));
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let j = ny as usize * w + nx as usize;
                        if !seen[j] && mask[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
        }
        if b.n >= 12 {
            out.push(b);
        }
    }
    out
}

/// Whether `page` is a calibration sheet made by [`add_sheet`].
pub fn is_sheet(page: &Page) -> bool {
    page.name == PAGE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter() -> Page {
        let mut p = Page::new("x", 215.9, 279.4);
        p.regmarks.enabled = true;
        p
    }

    #[test]
    fn fit_undoes_a_shift_and_turn() {
        let page = letter();
        let want: Vec<Point> = targets(&page).to_vec();
        let err = Affine::translate((0.6, -0.4)) * Affine::rotate_about(0.3f64.to_radians(), Point::new(100.0, 140.0));
        let got: Vec<Point> = want.iter().map(|p| err * *p).collect();
        let c = fit(&want, &got).unwrap();
        for p in &want {
            let back = c.affine() * (err * *p);
            assert!((back - *p).hypot() < 1e-6, "{back:?} vs {p:?}");
        }
        // one spot: just the shift
        let c = fit(&want[..1], &[want[0] + Vec2::new(1.0, 2.0)]).unwrap();
        assert!(((c.affine() * Point::new(50.0, 50.0)) - Point::new(49.0, 48.0)).hypot() < 1e-9);
    }

    /// Draws the cut sheet as a camera would see it: the dots where they were
    /// printed, the squares where the cutter put them (moved by `err`), the
    /// grey rulers, all seen at a slant.
    fn photo(page: &Page, err: Affine) -> image::RgbaImage {
        let (w, h) = (1500u32, 1900u32);
        // page mm -> photo px: scaled, turned a little and offset
        let view = Affine::translate((90.0, 60.0)) * Affine::rotate(4f64.to_radians()) * Affine::scale(6.2);
        let inv = view.inverse();
        let t = targets(page);
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([235, 232, 225, 255]));
        // the rulers are all rectangles
        let rulers: Vec<Rect> = t.iter().flat_map(|p| ruler(*p)).map(|r| r.bounding_box()).collect();
        for y in 0..h {
            for x in 0..w {
                let p = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let dot = t.iter().any(|c| (p - *c).hypot() <= DOT_R);
                // the square's line, 0.4 mm wide, where the cutter drew it
                let line = t.iter().any(|c| {
                    let q = err.inverse() * p;
                    let (dx, dy) = ((q.x - c.x).abs(), (q.y - c.y).abs());
                    let half = BOX / 2.0;
                    (dx <= half + 0.2 && dy <= half + 0.2) && (dx >= half - 0.2 || dy >= half - 0.2)
                });
                let grey = !dot && !line && rulers.iter().any(|r| r.contains(p));

                let px = if dot { [20, 20, 20, 255] } else if line { [30, 40, 160, 255] } else if grey { [150, 150, 150, 255] } else { continue };
                img.put_pixel(x, y, image::Rgba(px));
            }
        }
        img
    }

    #[test]
    fn measures_a_photo_of_the_sheet() {
        let page = letter();
        let err = Affine::translate((0.8, -0.5)) * Affine::rotate_about(0.2f64.to_radians(), Point::new(108.0, 140.0));
        let found = measure_photo(&photo(&page, err), &page).unwrap();
        let want = targets(&page);
        for (i, f) in found.iter().enumerate() {
            let f = f.unwrap_or_else(|| panic!("{} not found", TARGET_NAMES[i]));
            let truth = err * want[i];
            assert!((f - truth).hypot() < 0.15, "{}: {f:?} vs {truth:?}", TARGET_NAMES[i]);
        }
        let actual: Vec<Point> = found.iter().flatten().copied().collect();
        let c = fit(&want, &actual).unwrap();
        for p in want {
            assert!(((c.affine() * (err * p)) - p).hypot() < 0.15);
        }
    }

    #[test]
    fn the_sheet_has_dots_rulers_and_cut_squares() {
        let mut d = Document::default();
        let i = add_sheet(&mut d);
        assert_eq!(d.active, i);
        let page = d.page();
        assert!(is_sheet(page) && page.regmarks.enabled);
        let cuts = page.shapes.iter().filter(|s| s.style.cut == CutStyle::Cut).count();
        assert_eq!(cuts, 5);
        assert!(page.shapes.iter().filter(|s| s.style.cut == CutStyle::Cut).all(|s| s.style.no_print));
        let safe = page.regmarks.safe_area(page.width, page.height);
        for s in &page.shapes {
            let b = s.bbox().unwrap();
            assert!(safe.contains(b.origin()) && safe.contains(Point::new(b.x1, b.y1)), "{b:?} outside {safe:?}");
        }
    }
}
