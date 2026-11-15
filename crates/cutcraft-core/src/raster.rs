//! Raster image processing: grayscale working images, photo adjustments, artistic
//! effects, an eraser, and photo-to-line-art generation.
//!
//! Ported from BurnCraft's `bc-raster` crate (same `image` stack). Gray values are
//! `f32` in 0.0 (black) ..= 1.0 (white). In CutCraft these feed tracing,
//! photo-to-lines and shadow boxes as a non-destructive preprocessing step, so a
//! colour print-and-cut image is never forced to grayscale unless the user asks.

use image::imageops::FilterType;

/// Rubs out parts of an image file: everything within `radius` of the brush path
/// `spots` becomes transparent. Positions are millimetres from the top-left of an
/// image shown `width` × `height` mm. Returns the edited picture as PNG.
pub fn erase(bytes: &[u8], spots: &[(f64, f64)], radius: f64, width: f64, height: f64) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (sx, sy) = (w as f64 / width.max(1e-9), h as f64 / height.max(1e-9));
    let px = |(x, y): (f64, f64)| (x * sx, y * sy);
    let (rx, ry) = ((radius * sx).max(1e-6), (radius * sy).max(1e-6));
    let mut changed = false;
    let pairs: Vec<((f64, f64), (f64, f64))> = match spots {
        [] => Vec::new(),
        [one] => vec![(px(*one), px(*one))],
        _ => spots.windows(2).map(|w| (px(w[0]), px(w[1]))).collect(),
    };
    for (a, b) in pairs {
        let (x0, x1) = ((a.0.min(b.0) - rx).floor().max(0.0) as usize, ((a.0.max(b.0) + rx).ceil() as usize).min(w));
        let (y0, y1) = ((a.1.min(b.1) - ry).floor().max(0.0) as usize, ((a.1.max(b.1) + ry).ceil() as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                // Distance to the segment in brush radii (an ellipse in pixel space).
                let (cx, cy) = ((x as f64 + 0.5 - a.0) / rx, (y as f64 + 0.5 - a.1) / ry);
                let (dx, dy) = ((b.0 - a.0) / rx, (b.1 - a.1) / ry);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 { ((cx * dx + cy * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (ex, ey) = (cx - t * dx, cy - t * dy);
                if ex * ex + ey * ey <= 1.0 {
                    let p = img.get_pixel_mut(x as u32, y as u32);
                    if p.0[3] != 0 {
                        p.0 = [255, 255, 255, 0];
                        changed = true;
                    }
                }
            }
        }
    }
    if !changed {
        return Err("nothing to erase there".into());
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Restore brush: paints `original`'s pixels back along the brush path (the
/// same image size as `bytes`), undoing erasing there.
pub fn restore(bytes: &[u8], original: &[u8], spots: &[(f64, f64)], radius: f64, width: f64, height: f64) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let orig = image::load_from_memory(original).map_err(|e| e.to_string())?.to_rgba8();
    if orig.dimensions() != img.dimensions() {
        return Err("the original picture is a different size".into());
    }
    let mask = brush_mask(img.width() as usize, img.height() as usize, spots, radius, width, height);
    let mut changed = false;
    for (i, on) in mask.iter().enumerate() {
        let (x, y) = ((i % img.width() as usize) as u32, (i / img.width() as usize) as u32);
        if *on && img.get_pixel(x, y) != orig.get_pixel(x, y) {
            img.put_pixel(x, y, *orig.get_pixel(x, y));
            changed = true;
        }
    }
    if !changed {
        return Err("nothing to restore there".into());
    }
    png(&img)
}

/// Which pixels a brush path covers (an ellipse in pixel space per point,
/// swept along the path).
fn brush_mask(w: usize, h: usize, spots: &[(f64, f64)], radius: f64, width: f64, height: f64) -> Vec<bool> {
    let (sx, sy) = (w as f64 / width.max(1e-9), h as f64 / height.max(1e-9));
    let px = |(x, y): (f64, f64)| (x * sx, y * sy);
    let (rx, ry) = ((radius * sx).max(1e-6), (radius * sy).max(1e-6));
    let mut mask = vec![false; w * h];
    let pairs: Vec<((f64, f64), (f64, f64))> = match spots {
        [] => Vec::new(),
        [one] => vec![(px(*one), px(*one))],
        _ => spots.windows(2).map(|w| (px(w[0]), px(w[1]))).collect(),
    };
    for (a, b) in pairs {
        let (x0, x1) = ((a.0.min(b.0) - rx).floor().max(0.0) as usize, ((a.0.max(b.0) + rx).ceil().max(0.0) as usize).min(w));
        let (y0, y1) = ((a.1.min(b.1) - ry).floor().max(0.0) as usize, ((a.1.max(b.1) + ry).ceil().max(0.0) as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                let (cx, cy) = ((x as f64 + 0.5 - a.0) / rx, (y as f64 + 0.5 - a.1) / ry);
                let (dx, dy) = ((b.0 - a.0) / rx, (b.1 - a.1) / ry);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 { ((cx * dx + cy * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (ex, ey) = (cx - t * dx, cy - t * dy);
                if ex * ex + ey * ey <= 1.0 {
                    mask[y * w + x] = true;
                }
            }
        }
    }
    mask
}

fn png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Select & Erase: clears the area of similar colour around `at` (mm within
/// the picture) — the touching area only, or every similar pixel when
/// `everywhere`. `tolerance` 0..100 is how different a colour may be.
/// Returns the new picture and how many pixels were cleared.
pub fn select_erase(bytes: &[u8], at: (f64, f64), tolerance: f64, everywhere: bool, width: f64, height: f64) -> Result<(Vec<u8>, usize), String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (x, y) = ((at.0 / width.max(1e-9) * w as f64) as isize, (at.1 / height.max(1e-9) * h as f64) as isize);
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return Err("click on the picture".into());
    }
    let seed = img.get_pixel(x as u32, y as u32).0;
    if seed[3] == 0 {
        return Err("that part is already clear".into());
    }
    // colour distance, 0..100 (RGB, alpha-weighted)
    let limit = tolerance.clamp(0.0, 100.0) / 100.0 * 441.7;
    let near = |p: [u8; 4]| {
        if p[3] == 0 {
            return false;
        }
        let d = |i: usize| p[i] as f64 - seed[i] as f64;
        (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt() <= limit
    };
    let mut hit = vec![false; w * h];
    if everywhere {
        for (i, p) in img.pixels().enumerate() {
            hit[i] = near(p.0);
        }
    } else {
        let mut stack = vec![(x as usize, y as usize)];
        hit[y as usize * w + x as usize] = true;
        while let Some((px, py)) = stack.pop() {
            for (nx, ny) in [(px.wrapping_sub(1), py), (px + 1, py), (px, py.wrapping_sub(1)), (px, py + 1)] {
                if nx < w && ny < h && !hit[ny * w + nx] && near(img.get_pixel(nx as u32, ny as u32).0) {
                    hit[ny * w + nx] = true;
                    stack.push((nx, ny));
                }
            }
        }
    }
    let mut n = 0;
    for (i, on) in hit.iter().enumerate() {
        if *on {
            img.get_pixel_mut((i % w) as u32, (i / w) as u32).0 = [255, 255, 255, 0];
            n += 1;
        }
    }
    Ok((png(&img)?, n))
}

/// Crop: keeps the part of the picture inside `r` (mm within the picture,
/// clamped to it). Returns the new picture, its pixel size, and the kept
/// rectangle in mm (snapped to whole pixels).
pub fn crop(bytes: &[u8], r: kurbo::Rect, width: f64, height: f64) -> Result<(Vec<u8>, u32, u32, kurbo::Rect), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = (img.width() as f64, img.height() as f64);
    let (sx, sy) = (w / width.max(1e-9), h / height.max(1e-9));
    let x0 = (r.x0.max(0.0) * sx).floor().clamp(0.0, w);
    let y0 = (r.y0.max(0.0) * sy).floor().clamp(0.0, h);
    let x1 = (r.x1.min(width) * sx).ceil().clamp(0.0, w);
    let y1 = (r.y1.min(height) * sy).ceil().clamp(0.0, h);
    if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
        return Err("drag a bigger box over the picture".into());
    }
    let out = image::imageops::crop_imm(&img, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image();
    let kept = kurbo::Rect::new(x0 / sx, y0 / sy, x1 / sx, y1 / sy);
    Ok((png(&out)?, out.width(), out.height(), kept))
}

/// How much of each pixel (0 to 1) lies inside `paths` (the picture's own
/// millimetres, even-odd), for a `w` × `h` pixel picture `width` × `height`
/// mm. Edges are smoothed (4 × 4 samples a pixel).
fn coverage(paths: &[kurbo::BezPath], w: usize, h: usize, width: f64, height: f64) -> Vec<f32> {
    const SUB: usize = 4;
    let (sx, sy) = (w as f64 / width.max(1e-9), h as f64 / height.max(1e-9));
    // edges in pixels
    let mut edges: Vec<(f64, f64, f64, f64)> = vec![];
    for pl in crate::geom::flatten_all(paths, 0.02).into_iter().filter(|p| p.closed && p.pts.len() > 2) {
        let n = pl.pts.len();
        for i in 0..n {
            let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
            edges.push((a.x * sx, a.y * sy, b.x * sx, b.y * sy));
        }
    }
    let mut cov = vec![0f32; w * h];
    let mut xs: Vec<f64> = vec![];
    let step = 1.0 / SUB as f64;
    for y in 0..h {
        let row = &mut cov[y * w..(y + 1) * w];
        for s in 0..SUB {
            let yy = y as f64 + (s as f64 + 0.5) * step;
            xs.clear();
            for &(x0, y0, x1, y1) in &edges {
                if (y0 <= yy) != (y1 <= yy) {
                    xs.push(x0 + (yy - y0) / (y1 - y0) * (x1 - x0));
                }
            }
            xs.sort_by(f64::total_cmp);
            // inside between pairs of crossings: add the covered part of each pixel
            for pair in xs.chunks_exact(2) {
                let (a, b) = (pair[0].clamp(0.0, w as f64), pair[1].clamp(0.0, w as f64));
                if b <= a {
                    continue;
                }
                let (ia, ib) = (a.floor() as usize, (b.ceil() as usize).min(w));
                for (x, c) in row.iter_mut().enumerate().take(ib).skip(ia) {
                    let part = (b.min(x as f64 + 1.0) - a.max(x as f64)).max(0.0);
                    *c += (part * step) as f32;
                }
            }
        }
    }
    cov
}

/// Masks a picture with a shape (`paths`, in the picture's own millimetres):
/// keeps only what's inside it (`keep_inside`), or cuts it out, leaving a
/// see-through hole. Edges are smooth.
pub fn mask(bytes: &[u8], paths: &[kurbo::BezPath], keep_inside: bool, width: f64, height: f64) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let cov = coverage(paths, w, h, width, height);
    if !cov.iter().any(|c| *c > 0.0) {
        return Err("the shape doesn't overlap the picture".into());
    }
    for (p, c) in img.pixels_mut().zip(cov) {
        let k = if keep_inside { c.clamp(0.0, 1.0) } else { 1.0 - c.clamp(0.0, 1.0) };
        p.0[3] = (p.0[3] as f32 * k).round() as u8;
    }
    png(&img)
}

/// A grayscale working image.
#[derive(Debug, Clone, PartialEq)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Gray {
    pub fn new(width: usize, height: usize, value: f32) -> Self {
        Self { width, height, data: vec![value; width * height] }
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// Decodes PNG/JPEG/BMP/etc. Transparent pixels become white.
    pub fn decode(bytes: &[u8]) -> Result<Self, image::ImageError> {
        Ok(Self::from_dynamic(&image::load_from_memory(bytes)?))
    }

    pub fn from_dynamic(img: &image::DynamicImage) -> Self {
        let img = img.to_rgba32f();
        let (w, h) = img.dimensions();
        let data = img
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                (luma * a + (1.0 - a)).clamp(0.0, 1.0)
            })
            .collect();
        Self { width: w as usize, height: h as usize, data }
    }

    /// Back to an opaque RGBA image (for preview, tracing or baking).
    pub fn to_rgba8(&self) -> image::RgbaImage {
        let mut out = image::RgbaImage::new(self.width as u32, self.height as u32);
        for (i, p) in out.pixels_mut().enumerate() {
            let v = (self.data[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            *p = image::Rgba([v, v, v, 255]);
        }
        out
    }

    /// Resamples to `width` × `height` pixels (area-averaging when shrinking).
    pub fn resize(&self, width: usize, height: usize) -> Self {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let buf = image::ImageBuffer::<image::Luma<f32>, Vec<f32>>::from_raw(self.width as u32, self.height as u32, self.data.clone())
            .expect("buffer size matches dimensions");
        let filter = if width < self.width { FilterType::Triangle } else { FilterType::CatmullRom };
        let out = image::imageops::resize(&buf, width.max(1) as u32, height.max(1) as u32, filter);
        Self { width: width.max(1), height: height.max(1), data: out.into_raw().into_iter().map(|v| v.clamp(0.0, 1.0)).collect() }
    }

    /// Shrinks so the long side is at most `max` pixels (keeps aspect).
    pub fn fit(&self, max: usize) -> Self {
        let long = self.width.max(self.height);
        if long <= max || long == 0 {
            return self.clone();
        }
        let k = max as f64 / long as f64;
        self.resize(((self.width as f64 * k).round() as usize).max(1), ((self.height as f64 * k).round() as usize).max(1))
    }

    /// Applies the adjustments in place: background removal, effect, auto levels and
    /// sharpening first, then brightness, contrast, gamma and invert.
    pub fn adjust(&mut self, a: &Adjustments) {
        if a.remove_background > 0.0 {
            self.remove_background(a.remove_background);
        }
        match a.effect {
            Effect::None => {}
            Effect::Sketch => self.sketch(),
            Effect::LineArt => {
                self.sketch();
                for v in &mut self.data {
                    *v = ((*v - 0.55) / 0.4).clamp(0.0, 1.0);
                }
            }
            Effect::Edges => self.edges(),
        }
        if a.auto_level {
            self.auto_level();
        }
        if a.sharpen > 0.0 {
            self.sharpen(a.sharpen);
        }
        let c = a.contrast.clamp(-0.99, 0.99);
        let contrast = (c + 1.0) / (1.0 - c);
        let gamma = a.gamma.clamp(0.1, 5.0);
        for v in &mut self.data {
            let mut x = *v + a.brightness;
            x = (x - 0.5) * contrast + 0.5;
            x = x.clamp(0.0, 1.0).powf(1.0 / gamma);
            if a.invert {
                x = 1.0 - x;
            }
            *v = x;
        }
    }

    /// A soft blur (three box passes ≈ Gaussian) with the given radius in pixels.
    pub fn blur(&self, radius: usize) -> Gray {
        let (w, h) = (self.width, self.height);
        if radius == 0 || w == 0 || h == 0 {
            return self.clone();
        }
        let mut cur = self.data.clone();
        let mut tmp = vec![0.0f32; w * h];
        for _ in 0..3 {
            for y in 0..h {
                let row = &cur[y * w..(y + 1) * w];
                let mut sum: f32 = (0..=radius).map(|i| row[i.min(w - 1)]).sum::<f32>() + radius as f32 * row[0];
                for x in 0..w {
                    tmp[y * w + x] = sum / (2 * radius + 1) as f32;
                    sum += row[(x + radius + 1).min(w - 1)] - row[x.saturating_sub(radius)];
                }
            }
            for x in 0..w {
                let col = |y: usize| tmp[y * w + x];
                let mut sum: f32 = (0..=radius).map(|i| col(i.min(h - 1))).sum::<f32>() + radius as f32 * col(0);
                for y in 0..h {
                    cur[y * w + x] = sum / (2 * radius + 1) as f32;
                    sum += col((y + radius + 1).min(h - 1)) - col(y.saturating_sub(radius));
                }
            }
        }
        Gray { width: w, height: h, data: cur }
    }

    /// Pencil sketch: the image colour-dodged with a blurred negative of itself.
    pub fn sketch(&mut self) {
        let radius = (self.width.min(self.height) / 120).max(2);
        let inverted = Gray { width: self.width, height: self.height, data: self.data.iter().map(|v| 1.0 - v).collect() };
        let blurred = inverted.blur(radius);
        for (v, b) in self.data.iter_mut().zip(&blurred.data) {
            *v = if *b >= 0.999 { 1.0 } else { (*v / (1.0 - b)).clamp(0.0, 1.0) };
        }
    }

    /// Dark lines where brightness changes (Sobel edge strength), on white.
    pub fn edges(&mut self) {
        let (w, h) = (self.width, self.height);
        if w < 3 || h < 3 {
            return;
        }
        let soft = self.blur((w.min(h) / 400).max(1));
        let at = |x: usize, y: usize| soft.data[y * w + x];
        let mut out = vec![1.0f32; w * h];
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                out[y * w + x] = 1.0 - (gx.hypot(gy) * 1.5).clamp(0.0, 1.0);
            }
        }
        self.data = out;
    }

    /// Stretches values so the darkest and lightest 1 % become black and white.
    pub fn auto_level(&mut self) {
        if self.data.len() < 2 {
            return;
        }
        let mut sorted = self.data.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let lo = sorted[sorted.len() / 100];
        let hi = sorted[sorted.len() - 1 - sorted.len() / 100];
        if hi - lo < 1e-3 {
            return;
        }
        for v in &mut self.data {
            *v = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
        }
    }

    /// Unsharp mask: adds `amount` × (pixel − 3×3 blur).
    pub fn sharpen(&mut self, amount: f32) {
        let (w, h) = (self.width, self.height);
        if w < 3 || h < 3 {
            return;
        }
        let src = self.data.clone();
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let xx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                        let yy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                        sum += src[yy * w + xx];
                    }
                }
                let v = src[y * w + x];
                self.data[y * w + x] = (v + amount * (v - sum / 9.0)).clamp(0.0, 1.0);
            }
        }
    }

    /// Whitens a plain backdrop: the region joined to the image edge whose value
    /// stays within `tolerance` of the typical edge value.
    pub fn remove_background(&mut self, tolerance: f32) {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let border: Vec<usize> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1])).collect();
        let mut edge: Vec<f32> = border.iter().map(|&i| self.data[i]).collect();
        edge.sort_by(|a, b| a.total_cmp(b));
        let backdrop = edge[edge.len() / 2];
        let mut seen = vec![false; w * h];
        let mut stack: Vec<usize> = border.into_iter().filter(|&i| (self.data[i] - backdrop).abs() <= tolerance).collect();
        while let Some(i) = stack.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            let (x, y) = (i % w, i / w);
            let mut push = |j: usize| {
                if !seen[j] && (self.data[j] - backdrop).abs() <= tolerance {
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        for (v, bg) in self.data.iter_mut().zip(seen) {
            if bg {
                *v = 1.0;
            }
        }
    }
}

/// Photo adjustments (all neutral by default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjustments {
    /// Added to every value, -1.0..=1.0.
    pub brightness: f32,
    /// -1.0..=1.0; 0 leaves the image unchanged.
    pub contrast: f32,
    /// 1.0 leaves the image unchanged; above 1 lightens mid-tones.
    pub gamma: f32,
    pub invert: bool,
    /// Stretch the darkest and lightest 1 % to black and white.
    pub auto_level: bool,
    /// Unsharp-mask strength, 0 (off) to about 2.
    pub sharpen: f32,
    /// Whiten a plain backdrop joined to the edge (0 = off, up to about 0.5).
    pub remove_background: f32,
    pub effect: Effect,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 0.0, gamma: 1.0, invert: false, auto_level: false, sharpen: 0.0, remove_background: 0.0, effect: Effect::None }
    }
}

impl Adjustments {
    pub fn is_neutral(&self) -> bool {
        *self == Adjustments::default()
    }
}

/// The plain backdrop joined to a picture's edges: a see-through amount per
/// pixel (1 = backdrop, 0 = keep), for colours within `tolerance` (0..1) of
/// the backdrop's colour (the edge's middle colour), with a soft rim.
/// Pixels that are already see-through count as backdrop too.
fn backdrop(img: &image::RgbaImage, tolerance: f32) -> Vec<f32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut out = vec![0.0f32; w * h];
    if w == 0 || h == 0 {
        return out;
    }
    let px = |i: usize| img.as_raw()[i * 4..i * 4 + 4].to_vec();
    let border: Vec<usize> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1])).collect();
    // the backdrop colour: each channel's middle value round the edge
    let mid = |c: usize| {
        let mut v: Vec<u8> = border.iter().map(|&i| px(i)[c]).collect();
        v.sort_unstable();
        v[v.len() / 2] as f32
    };
    let bg = [mid(0), mid(1), mid(2)];
    let distance = |i: usize| {
        let p = px(i);
        if p[3] < 16 {
            return 0.0;
        }
        (0..3).map(|c| (p[c] as f32 - bg[c]).abs() / 255.0).fold(0.0f32, f32::max)
    };
    let tol = tolerance.clamp(0.0, 1.0);
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = border.into_iter().filter(|&i| distance(i) <= tol).collect();
    while let Some(i) = stack.pop() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        out[i] = 1.0;
        let (x, y) = (i % w, i / w);
        let near = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
        for j in near.into_iter().flatten() {
            if !seen[j] && distance(j) <= tol {
                stack.push(j);
            }
        }
    }
    // a soft rim: kept pixels touching the backdrop fade a little by how close they are to it
    let soft = (tol * 0.5).max(0.02);
    let edge: Vec<usize> = (0..w * h)
        .filter(|&i| out[i] == 0.0)
        .filter(|&i| {
            let (x, y) = (i % w, i / w);
            (x > 0 && out[i - 1] == 1.0) || (x + 1 < w && out[i + 1] == 1.0) || (y > 0 && out[i - w] == 1.0) || (y + 1 < h && out[i + w] == 1.0)
        })
        .collect();
    for i in edge {
        out[i] = (1.0 - (distance(i) - tol) / soft).clamp(0.0, 1.0) * 0.8;
    }
    out
}

/// The picture with a plain white (or near-white) backdrop joined to its
/// edges made see-through - None when the backdrop isn't white (a photo, a
/// coloured background), so nothing is guessed. White inside the design,
/// not joined to the edge, stays.
pub fn without_white_backdrop(img: &image::RgbaImage) -> Option<image::RgbaImage> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w < 3 || h < 3 {
        return None;
    }
    let edge = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
    let mut light = 0usize;
    let mut total = 0usize;
    for (x, y) in edge {
        let p = img.get_pixel(x as u32, y as u32).0;
        total += 1;
        if p[3] > 127 && p[0] > 235 && p[1] > 235 && p[2] > 235 {
            light += 1;
        }
    }
    // most of the edge is white
    if light * 10 < total * 7 {
        return None;
    }
    let bg = backdrop(img, 0.06);
    let mut out = img.clone();
    for (p, t) in out.pixels_mut().zip(bg) {
        p.0[3] = (p.0[3] as f32 * (1.0 - t)).round() as u8;
    }
    Some(out)
}

/// A picture with `a` applied, in colour: brightness, contrast, gamma,
/// sharpen, auto levels and invert work on the colours; Remove background
/// makes the plain backdrop see-through; the Sketch, Line art and Edges
/// effects give a black and white picture. What's see-through stays so.
pub fn adjust_rgba(img: &image::RgbaImage, a: &Adjustments) -> image::RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut out = img.clone();
    if w == 0 || h == 0 {
        return out;
    }
    let see_through = if a.remove_background > 0.0 { Some(backdrop(img, a.remove_background)) } else { None };
    let tones = Adjustments { remove_background: 0.0, ..*a };
    if a.effect != Effect::None {
        // black and white effects: on the grey picture
        let mut g = Gray::from_dynamic(&image::DynamicImage::ImageRgba8(img.clone()));
        g.adjust(&tones);
        for (i, p) in out.pixels_mut().enumerate() {
            let v = (g.data[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            p.0 = [v, v, v, p.0[3]];
        }
    } else if !tones.is_neutral() {
        // each colour channel as a grey picture, so sharpen and the tone curve apply to all three
        let channel = |c: usize| Gray { width: w, height: h, data: img.pixels().map(|p| p.0[c] as f32 / 255.0).collect() };
        let mut chans = [channel(0), channel(1), channel(2)];
        if tones.auto_level {
            // one stretch for all three, from the brightness, so colours don't shift
            let mut luma: Vec<f32> = img.pixels().map(|p| (0.2126 * p.0[0] as f32 + 0.7152 * p.0[1] as f32 + 0.0722 * p.0[2] as f32) / 255.0).collect();
            luma.sort_by(f32::total_cmp);
            let (lo, hi) = (luma[luma.len() / 100], luma[(luma.len() * 99 / 100).min(luma.len() - 1)]);
            if hi - lo > 1e-3 {
                for ch in &mut chans {
                    for v in &mut ch.data {
                        *v = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
                    }
                }
            }
        }
        let per_channel = Adjustments { auto_level: false, ..tones };
        for ch in &mut chans {
            ch.adjust(&per_channel);
        }
        for (i, p) in out.pixels_mut().enumerate() {
            for c in 0..3 {
                p.0[c] = (chans[c].data[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    if let Some(bg) = see_through {
        for (p, t) in out.pixels_mut().zip(bg) {
            p.0[3] = (p.0[3] as f32 * (1.0 - t)).round() as u8;
        }
    }
    out
}

/// Artistic effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effect {
    #[default]
    None,
    /// A pencil sketch.
    Sketch,
    /// Only the strong lines of a sketch.
    LineArt,
    /// Dark outlines where the brightness changes.
    Edges,
}

impl Effect {
    pub const ALL: [(Effect, &'static str); 4] =
        [(Effect::None, "None"), (Effect::Sketch, "Sketch"), (Effect::LineArt, "Line art"), (Effect::Edges, "Edges")];
}

// ------------------------------------------------------------------ photo to lines

/// Line-art styles for [`photo_lines`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    /// One continuous spiral from the centre out.
    Spiral,
    /// Horizontal rows of waves.
    Waves,
    /// Dots, more of them where the picture is darker.
    Stipple,
}

impl LineStyle {
    pub const ALL: [(LineStyle, &'static str); 3] =
        [(LineStyle::Spiral, "Spiral"), (LineStyle::Waves, "Waves"), (LineStyle::Stipple, "Stipple")];
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineOptions {
    pub style: LineStyle,
    /// Distance between spiral turns, wave rows or dot grid cells (mm).
    pub spacing: f64,
    /// How far dark areas push the line out, 0..=1.
    pub strength: f64,
    /// Stipple dot diameter (mm).
    pub dot: f64,
    /// Seed for the stipple's random placement.
    pub seed: u64,
}

impl Default for LineOptions {
    fn default() -> Self {
        Self { style: LineStyle::Spiral, spacing: 1.5, strength: 1.0, dot: 0.4, seed: 1 }
    }
}

/// Darker than this counts as "something to draw".
const BLANK: f64 = 0.04;
/// Step along the line (mm).
const STEP: f64 = 0.1;

/// Line art for `image` stretched over a `width` × `height` mm area with its top-left
/// corner at the origin. Returns polylines (closed ones repeat their first point).
/// Ideal for sketch pens: one continuous stroke reads as a drawing.
pub fn photo_lines(image: &Gray, width: f64, height: f64, options: &LineOptions) -> Vec<Vec<(f64, f64)>> {
    if image.width == 0 || image.height == 0 || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let spacing = options.spacing.max(0.2);
    // Darkness at a point (mm), bilinear; outside the picture counts as blank.
    let dark = |x: f64, y: f64| -> f64 {
        if x < 0.0 || y < 0.0 || x > width || y > height {
            return 0.0;
        }
        let fx = (x / width * image.width as f64 - 0.5).clamp(0.0, (image.width - 1) as f64);
        let fy = (y / height * image.height as f64 - 0.5).clamp(0.0, (image.height - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(image.width - 1), (y0 + 1).min(image.height - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let g = |x, y| image.get(x, y) as f64;
        let top = g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx;
        let bottom = g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx;
        1.0 - (top * (1.0 - ty) + bottom * ty)
    };
    let amplitude = |d: f64| d * spacing * 0.48 * options.strength.clamp(0.0, 1.0);
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    // Adds a point of a running line, or ends the line where the picture is blank.
    let mut emit = |p: Option<(f64, f64)>, out: &mut Vec<Vec<(f64, f64)>>| match p {
        Some(p) => current.push(p),
        None => {
            if current.len() >= 2 {
                out.push(std::mem::take(&mut current));
            }
            current.clear();
        }
    };
    match options.style {
        LineStyle::Spiral => {
            let (cx, cy) = (width / 2.0, height / 2.0);
            let reach = cx.hypot(cy);
            let wavelength = spacing * 0.6;
            let (mut theta, mut phase) = (0.0f64, 0.0f64);
            loop {
                let r = spacing * theta / std::f64::consts::TAU;
                if r > reach {
                    break;
                }
                let (c, s) = (theta.cos(), theta.sin());
                let (x, y) = (cx + r * c, cy + r * s);
                let d = dark(x, y);
                let wiggle = phase.sin() * amplitude(d);
                emit((d > BLANK).then_some((x + wiggle * c, y + wiggle * s)), &mut out);
                let ds_dtheta = r.hypot(spacing / std::f64::consts::TAU).max(1e-6);
                theta += STEP / ds_dtheta;
                phase += STEP / wavelength * std::f64::consts::TAU;
            }
            emit(None, &mut out);
        }
        LineStyle::Waves => {
            let rows = (height / spacing).ceil() as usize;
            for row in 0..rows {
                let y0 = (row as f64 + 0.5) * spacing;
                let mut phase = 0.0f64;
                let steps = (width / STEP).ceil() as usize;
                let xs: Vec<usize> = if row % 2 == 0 { (0..=steps).collect() } else { (0..=steps).rev().collect() };
                for i in xs {
                    let x = (i as f64 * STEP).min(width);
                    let d = dark(x, y0);
                    let wavelength = spacing * (1.4 - 0.9 * d);
                    phase += STEP / wavelength * std::f64::consts::TAU;
                    emit((d > BLANK).then_some((x, y0 + phase.sin() * amplitude(d))), &mut out);
                }
                emit(None, &mut out);
            }
        }
        LineStyle::Stipple => {
            let mut state = options.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut random = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 11) as f64 / (1u64 << 53) as f64
            };
            let r = (options.dot / 2.0).max(0.02);
            let (cols, rows) = ((width / spacing).ceil() as usize, (height / spacing).ceil() as usize);
            for row in 0..rows {
                let order: Vec<usize> = if row % 2 == 0 { (0..cols).collect() } else { (0..cols).rev().collect() };
                for col in order {
                    let x = (col as f64 + 0.5 + (random() - 0.5) * 0.7) * spacing;
                    let y = (row as f64 + 0.5 + (random() - 0.5) * 0.7) * spacing;
                    let d = dark(x, y);
                    if d > BLANK && random() < d {
                        let ring: Vec<(f64, f64)> = (0..=8)
                            .map(|k| {
                                let a = k as f64 / 8.0 * std::f64::consts::TAU;
                                (x + r * a.cos(), y + r * a.sin())
                            })
                            .collect();
                        out.push(ring);
                    }
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ CutCraft additions

/// Polylines from [`photo_lines`] as one path (local mm coordinates).
pub fn lines_to_path(lines: &[Vec<(f64, f64)>]) -> kurbo::BezPath {
    let parts: Vec<kurbo::BezPath> = lines
        .iter()
        .filter(|l| l.len() >= 2)
        .map(|l| crate::geom::polyline_path(&l.iter().map(|&(x, y)| kurbo::Point::new(x, y)).collect::<Vec<_>>(), false))
        .collect();
    crate::geom::merge_paths(&parts)
}

/// Shadow box (layered papercut): split `gray` into `plates` stacked plates by
/// brightness. Plates are in local mm with the image's top-left at the origin,
/// spanning `w_mm` × `h_mm`, front plate first. Each is a frame `border` mm wide
/// plus cut-outs: the front plate has the largest opening, each one behind it a
/// smaller one, and the back plate is solid. Lit from behind, the stack reads as
/// a 3D scene.
pub fn shadow_box_plates(gray: &Gray, w_mm: f64, h_mm: f64, plates: usize, border: f64, smooth: f64) -> Vec<kurbo::BezPath> {
    use crate::trace::{self, TraceMode, TraceParams};
    let n = plates.clamp(2, 8);
    let g = gray.fit(600);
    if g.width == 0 || g.height == 0 {
        return Vec::new();
    }
    let rgba = g.to_rgba8();
    let (sx, sy) = (w_mm / g.width as f64, h_mm / g.height as f64);
    let b = border.max(0.0);
    let frame = crate::geom::rect_path(kurbo::Rect::new(-b, -b, w_mm + b, h_mm + b));
    // Drop specks too small to cut cleanly from paper.
    let min_blob = ((g.width * g.height) / 1500).max(8) as u32;
    let mut out = Vec::with_capacity(n);
    for k in 0..n - 1 {
        // Front plate: everything darker than (1 - 1/n) is open; further back, less.
        let thr = (1.0 - (k + 1) as f32 / n as f32).max(0.02);
        let p = TraceParams {
            mode: TraceMode::Threshold,
            threshold: (thr * 255.0).round() as u8,
            despeckle: min_blob,
            smooth: true,
            tolerance: smooth.max(0.3),
            ..Default::default()
        };
        let mut holes = trace::trace(&rgba, &p);
        holes.apply_affine(kurbo::Affine::scale_non_uniform(sx, sy));
        let mut plate = frame.clone();
        plate.extend(holes.elements().iter().copied());
        out.push(plate);
    }
    out.push(frame); // solid back plate
    out
}
