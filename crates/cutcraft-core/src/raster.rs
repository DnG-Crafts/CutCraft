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

#[cfg(test)]
mod tests {
    #[test]
    fn select_erase_restore_and_crop() {
        // left half red, right half blue, 20 x 10 px, as 20 x 10 mm
        let mut img = image::RgbaImage::new(20, 10);
        for (x, _, p) in img.enumerate_pixels_mut() {
            p.0 = if x < 10 { [220, 30, 30, 255] } else { [30, 30, 220, 255] };
        }
        let bytes = super::png(&img).unwrap();
        let (erased, n) = select_erase(&bytes, (2.0, 2.0), 10.0, false, 20.0, 10.0).unwrap();
        assert_eq!(n, 100, "the red half only");
        let e = image::load_from_memory(&erased).unwrap().to_rgba8();
        assert_eq!(e.get_pixel(3, 3).0[3], 0);
        assert_eq!(e.get_pixel(15, 3).0[3], 255);
        // restore a dab back from the original
        let back = restore(&erased, &bytes, &[(3.0, 5.0)], 1.5, 20.0, 10.0).unwrap();
        assert_eq!(image::load_from_memory(&back).unwrap().to_rgba8().get_pixel(3, 5).0, [220, 30, 30, 255]);
        assert!(restore(&bytes, &bytes, &[(3.0, 5.0)], 1.5, 20.0, 10.0).is_err(), "nothing to restore");
        // crop to the blue half
        let (c, w, h, kept) = crop(&bytes, kurbo::Rect::new(10.0, -5.0, 40.0, 8.0), 20.0, 10.0).unwrap();
        assert_eq!((w, h), (10, 8));
        assert_eq!(kept, kurbo::Rect::new(10.0, 0.0, 20.0, 8.0));
        assert_eq!(image::load_from_memory(&c).unwrap().to_rgba8().get_pixel(0, 0).0, [30, 30, 220, 255]);
    }

    use super::*;

    #[test]
    fn shadow_box_layers_open_progressively() {
        // Left third black, middle grey, right white.
        let mut g = Gray::new(60, 60, 1.0);
        for y in 0..60 {
            for x in 0..40 {
                g.data[y * 60 + x] = if x < 20 { 0.0 } else { 0.5 };
            }
        }
        let plates = shadow_box_plates(&g, 60.0, 60.0, 3, 5.0, 1.0);
        assert_eq!(plates.len(), 3);
        let n = |p: &kurbo::BezPath| crate::geom::subpaths(p).len();
        assert_eq!(n(&plates[2]), 1, "back plate is a solid frame");
        assert!(n(&plates[0]) >= 2 && n(&plates[1]) >= 2, "front plates have openings");
        // The front opening is larger than the next one.
        use kurbo::Shape as _;
        let hole_area = |p: &kurbo::BezPath| crate::geom::subpaths(p)[1..].iter().map(|s| s.area().abs()).sum::<f64>();
        assert!(hole_area(&plates[0]) > hole_area(&plates[1]) * 1.5);
    }

    /// Renders photo-to-lines (3 styles) and a shadow box for the app icon to HTML.
    /// `cargo test -p cutcraft-core preview_image_tools -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_image_tools() {
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../cutcraft/assets/cutcraft.png")).unwrap();
        let g = Gray::decode(&png).unwrap();
        let (w, h) = (80.0, 80.0);
        let mut body = String::new();
        let svg = |title: &str, paths: &[kurbo::BezPath], vb: (f64, f64, f64, f64), colour: &str| {
            let d: String = paths.iter().map(|p| format!("<path d='{}' fill='none' stroke='{colour}' stroke-width='0.25'/>", p.to_svg())).collect();
            format!("<figure><figcaption>{title}</figcaption><svg viewBox='{} {} {} {}' width='260' style='background:#fff;border:1px solid #ccc'>{d}</svg></figure>", vb.0, vb.1, vb.2, vb.3)
        };
        for (style, name) in LineStyle::ALL {
            let lines = photo_lines(&g, w, h, &LineOptions { style, spacing: 1.4, ..Default::default() });
            body += &svg(name, &[lines_to_path(&lines)], (0.0, 0.0, w, h), "#1e1ea0");
        }
        for (k, p) in shadow_box_plates(&g, w, h, 4, 6.0, 1.5).iter().enumerate() {
            body += &svg(&format!("Shadow plate {}", k + 1), &[p.clone()], (-8.0, -8.0, w + 16.0, h + 16.0), "#d33");
        }
        let html = format!("<!doctype html><meta charset=utf8><body style='font-family:sans-serif;display:flex;flex-wrap:wrap;gap:12px'>{body}</body>");
        let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/image_tools_preview.html");
        std::fs::write(out, html).unwrap();
        println!("wrote {out}");
    }

    /// Same as `preview_image_tools` but rasterised to a PNG contact sheet.
    #[test]
    #[ignore]
    fn preview_image_tools_png() {
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../cutcraft/assets/cutcraft.png")).unwrap();
        let g = Gray::decode(&png).unwrap();
        let (w, h, tile) = (80.0, 80.0, 300u32);
        let mut tiles: Vec<(Vec<kurbo::BezPath>, (f64, f64, f64), [u8; 3])> = Vec::new();
        // the source picture itself (as an outline grid) is shown as tile 0
        for (style, _) in LineStyle::ALL {
            let lines = photo_lines(&g, w, h, &LineOptions { style, spacing: 1.4, ..Default::default() });
            tiles.push((vec![lines_to_path(&lines)], (0.0, 0.0, w), [30, 30, 160]));
        }
        for p in shadow_box_plates(&g, w, h, 4, 6.0, 1.5) {
            tiles.push((vec![p], (-8.0, -8.0, w + 16.0), [210, 40, 40]));
        }
        let cols = 4u32;
        let rows = (tiles.len() as u32 + cols - 1) / cols;
        let mut img = image::RgbaImage::from_pixel(cols * tile, rows * tile, image::Rgba([255, 255, 255, 255]));
        for (i, (paths, (ox, oy, span), col)) in tiles.iter().enumerate() {
            let (tx, ty) = ((i as u32 % cols) * tile, (i as u32 / cols) * tile);
            let k = (tile as f64 - 10.0) / span;
            let to = |p: kurbo::Point| (tx as f64 + 5.0 + (p.x - ox) * k, ty as f64 + 5.0 + (p.y - oy) * k);
            for path in paths {
                for pl in crate::geom::flatten(path, 0.05) {
                    let mut pts = pl.pts.clone();
                    if pl.closed { pts.push(pts[0]); }
                    for seg in pts.windows(2) {
                        let (a, b) = (to(seg[0]), to(seg[1]));
                        let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
                        for j in 0..=n {
                            let t = j as f64 / n as f64;
                            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                            if x >= 0.0 && y >= 0.0 && (x as u32) < img.width() && (y as u32) < img.height() {
                                img.put_pixel(x as u32, y as u32, image::Rgba([col[0], col[1], col[2], 255]));
                            }
                        }
                    }
                }
            }
        }
        // the original image, scaled, in the last free tile
        let thumb = image::imageops::resize(&image::load_from_memory(&png).unwrap().to_rgba8(), tile - 10, tile - 10, image::imageops::FilterType::Triangle);
        let last = tiles.len() as u32;
        image::imageops::overlay(&mut img, &thumb, ((last % cols) * tile + 5) as i64, ((last / cols) * tile + 5) as i64);
        let out = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/image_tools_preview.png");
        img.save(out).unwrap();
        println!("wrote {out}");
    }

    #[test]
    fn lines_become_a_path() {
        let lines = photo_lines(&Gray::new(10, 10, 0.0), 20.0, 4.0, &LineOptions { style: LineStyle::Waves, spacing: 2.0, ..Default::default() });
        let p = lines_to_path(&lines);
        assert_eq!(crate::geom::subpaths(&p).len(), lines.len());
    }

    /// Left half black, right half white.
    fn half_dark() -> Gray {
        let mut g = Gray::new(100, 100, 1.0);
        for y in 0..100 {
            for x in 0..50 {
                g.data[y * 100 + x] = 0.0;
            }
        }
        g
    }

    #[test]
    fn lines_only_where_dark() {
        for style in [LineStyle::Spiral, LineStyle::Waves, LineStyle::Stipple] {
            let lines = photo_lines(&half_dark(), 50.0, 50.0, &LineOptions { style, ..Default::default() });
            assert!(!lines.is_empty(), "{style:?} draws something");
            let wiggle = 1.5 * 0.5 + 0.3;
            assert!(lines.iter().flatten().all(|(x, _)| *x < 25.0 + wiggle), "{style:?} stays in the dark half");
        }
    }

    #[test]
    fn stipple_density_follows_darkness() {
        let o = LineOptions { style: LineStyle::Stipple, spacing: 1.0, ..Default::default() };
        let count = |v: f32| photo_lines(&Gray::new(10, 10, v), 40.0, 40.0, &o).len();
        let (dark, mid) = (count(0.0), count(0.5));
        assert!(dark > 1500 && (600..1000).contains(&mid), "{dark} {mid}");
        assert_eq!(count(1.0), 0);
    }

    #[test]
    fn adjustments_are_neutral_by_default() {
        let mut g = half_dark();
        let before = g.clone();
        g.adjust(&Adjustments::default());
        assert_eq!(g, before);
        assert!(Adjustments::default().is_neutral());
    }

    #[test]
    fn invert_and_remove_background() {
        let mut g = Gray::new(20, 20, 0.9); // light grey backdrop
        for y in 8..12 {
            for x in 8..12 {
                g.data[y * 20 + x] = 0.0; // dark subject
            }
        }
        g.adjust(&Adjustments { remove_background: 0.1, ..Default::default() });
        assert_eq!(g.get(0, 0), 1.0, "backdrop whitened");
        assert_eq!(g.get(10, 10), 0.0, "subject kept");
        g.adjust(&Adjustments { invert: true, ..Default::default() });
        assert_eq!(g.get(10, 10), 1.0);
    }

    #[test]
    fn rgba_roundtrip_and_fit() {
        let g = half_dark();
        let img = g.to_rgba8();
        assert_eq!((img.width(), img.height()), (100, 100));
        let back = Gray::from_dynamic(&image::DynamicImage::ImageRgba8(img));
        assert!((back.get(10, 10) - 0.0).abs() < 0.01 && (back.get(90, 10) - 1.0).abs() < 0.01);
        let small = g.fit(25);
        assert_eq!((small.width, small.height), (25, 25));
    }
}

#[cfg(test)]
mod image_demo {
    /// Writes a design holding one red/blue picture to the temp folder (for checking the image tools).
    #[test]
    #[ignore = "diagnostic"]
    fn write_image_demo() {
        let mut img = image::RgbaImage::new(200, 100);
        for (x, y, p) in img.enumerate_pixels_mut() {
            p.0 = if (x as i32 - 150).pow(2) + (y as i32 - 50).pow(2) < 900 { [250, 220, 0, 255] } else if x < 100 { [220, 40, 40, 255] } else { [40, 80, 220, 255] };
        }
        let bytes = super::png(&img).unwrap();
        let mut d = crate::doc::Document::default();
        let mut s = crate::io::import_image(&bytes).unwrap();
        if let crate::doc::ShapeKind::Image(i) = &mut s.kind {
            (i.width_mm, i.height_mm) = (160.0, 80.0);
        }
        s.transform = kurbo::Affine::translate((40.0, 60.0));
        d.add_shape(s, 0);
        let p = std::env::temp_dir().join("cutcraft-image-demo.cutcraft");
        crate::io::save_native(&d, &p).unwrap();
        println!("wrote {}", p.display());
    }
}
