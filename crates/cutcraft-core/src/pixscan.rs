//! PixScan-style alignment: photograph material on a sheet/mat with four corner
//! markers, find the markers, remove perspective and place the image on the page
//! at true size so you can trace or cut around it.
//!
//! CutCraft uses its own printable marker sheet (four solid squares). Any mat
//! works with manual corner picking.

use crate::geom;
use kurbo::{BezPath, Point, Rect};

/// Marker layout for a page: square size and centre inset from each edge (mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerSheet {
    pub size: f64,
    pub inset: f64,
}

impl Default for MarkerSheet {
    fn default() -> Self {
        MarkerSheet { size: 12.0, inset: 12.0 }
    }
}

impl MarkerSheet {
    /// Marker squares (page coordinates), TL, TR, BR, BL.
    pub fn markers(&self, w: f64, h: f64) -> Vec<BezPath> {
        self.centres(w, h)
            .iter()
            .map(|c| geom::rect_path(Rect::from_center_size(*c, (self.size, self.size))))
            .collect()
    }
    pub fn centres(&self, w: f64, h: f64) -> [Point; 4] {
        let i = self.inset;
        [Point::new(i, i), Point::new(w - i, i), Point::new(w - i, h - i), Point::new(i, h - i)]
    }
}

/// 3×3 homography (row-major) mapping `src[i]` → `dst[i]`.
pub fn homography(src: &[Point; 4], dst: &[Point; 4]) -> Option<[f64; 9]> {
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i].x, src[i].y);
        let (u, v) = (dst[i].x, dst[i].y);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // Gaussian elimination with partial pivoting on the 8×8 system
    for c in 0..8 {
        let piv = (c..8).max_by(|r1, r2| a[*r1][c].abs().total_cmp(&a[*r2][c].abs()))?;
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        for r in 0..8 {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..9 {
                    a[r][k] -= f * a[c][k];
                }
            }
        }
    }
    let mut h = [0.0; 9];
    for i in 0..8 {
        h[i] = a[i][8] / a[i][i];
    }
    h[8] = 1.0;
    Some(h)
}

pub fn apply(h: &[f64; 9], p: Point) -> Point {
    let w = h[6] * p.x + h[7] * p.y + h[8];
    Point::new((h[0] * p.x + h[1] * p.y + h[2]) / w, (h[3] * p.x + h[4] * p.y + h[5]) / w)
}

/// Resample the photo so that `corners` (image px, TL TR BR BL) become a
/// `w_mm × h_mm` rectangle at `px_per_mm` resolution.
pub fn rectify(img: &image::RgbaImage, corners: &[Point; 4], w_mm: f64, h_mm: f64, px_per_mm: f64) -> Option<image::RgbaImage> {
    let (ow, oh) = ((w_mm * px_per_mm).round().max(1.0) as u32, (h_mm * px_per_mm).round().max(1.0) as u32);
    let dst = [Point::new(0.0, 0.0), Point::new(ow as f64, 0.0), Point::new(ow as f64, oh as f64), Point::new(0.0, oh as f64)];
    // map output pixel → source pixel
    let h = homography(&dst, corners)?;
    let mut out = image::RgbaImage::new(ow, oh);
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    for y in 0..oh {
        for x in 0..ow {
            let s = apply(&h, Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let (sx, sy) = (s.x - 0.5, s.y - 0.5);
            if sx < 0.0 || sy < 0.0 || sx >= iw - 1.0 || sy >= ih - 1.0 {
                out.put_pixel(x, y, image::Rgba([255, 255, 255, 0]));
                continue;
            }
            let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
            let (fx, fy) = (sx - x0 as f64, sy - y0 as f64);
            let mut px = [0u8; 4];
            for c in 0..4 {
                let g = |xx: u32, yy: u32| img.get_pixel(xx, yy)[c] as f64;
                let v = g(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + g(x0 + 1, y0) * fx * (1.0 - fy)
                    + g(x0, y0 + 1) * (1.0 - fx) * fy
                    + g(x0 + 1, y0 + 1) * fx * fy;
                px[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(x, y, image::Rgba(px));
        }
    }
    Some(out)
}

/// Find four dark square markers near the image corners. Returns centres (px) TL, TR, BR, BL.
pub fn detect_markers(img: &image::RgbaImage) -> Option<[Point; 4]> {
    // work on a downscaled grey copy
    let scale = (900.0 / img.width().max(img.height()) as f64).min(1.0);
    let small = image::imageops::resize(
        img,
        ((img.width() as f64 * scale) as u32).max(1),
        ((img.height() as f64 * scale) as u32).max(1),
        image::imageops::FilterType::Triangle,
    );
    let (w, h) = (small.width() as usize, small.height() as usize);
    let lum: Vec<f32> =
        small.pixels().map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect();
    // adaptive-ish threshold: halfway between the darkest 2% and the median
    let mut sorted = lum.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let dark = sorted[sorted.len() / 50];
    let mid = sorted[sorted.len() / 2];
    let thr = (dark + mid) / 2.0;
    let mask: Vec<bool> = lum.iter().map(|l| *l < thr).collect();
    let mut seen = vec![false; w * h];
    let mut blobs: Vec<(Point, f64)> = vec![]; // centroid, area
    let min_area = (w * h) as f64 * 0.0002;
    for start in 0..w * h {
        if seen[start] || !mask[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut n, mut sx, mut sy) = (0usize, 0.0, 0.0);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            sx += x as f64;
            sy += y as f64;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut push = |j: usize| {
                if !seen[j] && mask[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 { push(i - 1) }
            if x + 1 < w { push(i + 1) }
            if y > 0 { push(i - w) }
            if y + 1 < h { push(i + w) }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
        let fill = n as f64 / (bw * bh);
        let aspect = bw / bh;
        if (n as f64) >= min_area && fill > 0.75 && (0.6..1.7).contains(&aspect) {
            blobs.push((Point::new(sx / n as f64, sy / n as f64), n as f64));
        }
    }
    if blobs.len() < 4 {
        return None;
    }
    let corners = [Point::new(0.0, 0.0), Point::new(w as f64, 0.0), Point::new(w as f64, h as f64), Point::new(0.0, h as f64)];
    let mut out = [Point::ZERO; 4];
    let mut used = vec![false; blobs.len()];
    for (k, c) in corners.iter().enumerate() {
        let (bi, _) = blobs
            .iter()
            .enumerate()
            .filter(|(i, _)| !used[*i])
            .map(|(i, b)| (i, b.0.distance(*c)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        used[bi] = true;
        out[k] = Point::new(blobs[bi].0.x / scale, blobs[bi].0.y / scale);
    }
    // sanity: the quad must be convex and reasonably large
    let area = geom::Polyline { pts: out.to_vec(), closed: true }.signed_area().abs();
    if area < (img.width() as f64 * img.height() as f64) * 0.1 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_roundtrip() {
        let src = [Point::new(10.0, 20.0), Point::new(300.0, 5.0), Point::new(320.0, 260.0), Point::new(0.0, 240.0)];
        let dst = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 80.0), Point::new(0.0, 80.0)];
        let h = homography(&src, &dst).unwrap();
        for i in 0..4 {
            assert!(apply(&h, src[i]).distance(dst[i]) < 1e-6);
        }
    }

    #[test]
    fn detects_markers_and_rectifies() {
        let mut img = image::RgbaImage::from_pixel(400, 300, image::Rgba([240, 235, 230, 255]));
        let centres = [(30, 30), (370, 40), (360, 270), (40, 260)];
        for (cx, cy) in centres {
            for y in cy - 8..cy + 8 {
                for x in cx - 8..cx + 8 {
                    img.put_pixel(x, y, image::Rgba([10, 10, 10, 255]));
                }
            }
        }
        let m = detect_markers(&img).expect("markers");
        for (k, (cx, cy)) in centres.iter().enumerate() {
            assert!(m[k].distance(Point::new(*cx as f64 - 0.5, *cy as f64 - 0.5)) < 2.0, "{k}: {:?}", m[k]);
        }
        let r = rectify(&img, &m, 100.0, 80.0, 2.0).unwrap();
        assert_eq!((r.width(), r.height()), (200, 160));
    }
}
