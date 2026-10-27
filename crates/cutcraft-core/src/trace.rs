//! Bitmap → vector tracing.
//!
//! Pipeline: optional blur (low-pass) → threshold / colour match → despeckle
//! (remove small blobs) → sub-pixel outlines (marching squares on the lightly
//! softened mask) → Bézier fit with sharp corners kept (see `curvefit`).

use crate::geom::{self, Polyline};
use kurbo::{BezPath, Point};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceMode {
    /// Pixels darker than the threshold.
    Threshold,
    /// Pixels within tolerance of a colour.
    Colour([u8; 3]),
    /// Isolate the subject from its background: flood from the image borders and
    /// keep everything the background doesn't reach. Best for a photo of an
    /// object on a reasonably plain backdrop. `threshold` is the colour tolerance.
    Subject,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceParams {
    pub mode: TraceMode,
    /// 0..=255 luminance threshold, or colour distance tolerance.
    pub threshold: u8,
    /// Box blur radius in pixels (0 = off).
    pub blur: u32,
    /// Remove blobs smaller than this many pixels.
    pub despeckle: u32,
    /// Only keep outermost contours.
    pub outer_only: bool,
    /// Simplification tolerance in pixels.
    pub tolerance: f64,
    pub smooth: bool,
    pub invert: bool,
    /// Trace the medial axis (single-stroke skeleton) instead of outlines —
    /// for sketch pens, foil, and engraving. Output paths are open.
    pub centerline: bool,
}

impl Default for TraceParams {
    fn default() -> Self {
        TraceParams {
            mode: TraceMode::Threshold,
            threshold: 128,
            blur: 0,
            despeckle: 16,
            outer_only: false,
            tolerance: 1.0,
            smooth: true,
            invert: false,
            centerline: false,
        }
    }
}

/// Otsu's method: pick the luminance threshold that best separates the image
/// into two classes. Handy as a one-click starting point for `Threshold` mode.
pub fn otsu_threshold(img: &image::RgbaImage) -> u8 {
    let mut hist = [0u32; 256];
    for px in img.pixels() {
        let a = px[3] as f32 / 255.0;
        let c = [0usize, 1, 2].map(|i| px[i] as f32 * a + 255.0 * (1.0 - a));
        let lum = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]).round() as usize;
        hist[lum.min(255)] += 1;
    }
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum_all: f32 = (0..256).map(|t| t as f32 * hist[t] as f32).sum();
    let mut var = [0f32; 256];
    let (mut w_b, mut sum_b) = (0f32, 0f32);
    for t in 0..256 {
        w_b += hist[t] as f32;
        let w_f = total as f32 - w_b;
        if w_b == 0.0 || w_f == 0.0 {
            continue;
        }
        sum_b += t as f32 * hist[t] as f32;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        var[t] = w_b * w_f * (m_b - m_f) * (m_b - m_f);
    }
    let max_var = var.iter().cloned().fold(0f32, f32::max);
    if max_var <= 0.0 {
        return 128;
    }
    // Average all thresholds that hit the maximum (midpoint of any flat plateau).
    let (mut sum_t, mut n) = (0usize, 0usize);
    for (t, v) in var.iter().enumerate() {
        if *v >= max_var * 0.999 {
            sum_t += t;
            n += 1;
        }
    }
    (sum_t / n.max(1)) as u8
}

/// Binary mask, row-major.
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub px: Vec<bool>,
}

impl Mask {
    fn get(&self, x: isize, y: isize) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.px[y as usize * self.w + x as usize]
    }
}

pub fn make_mask(img: &image::RgbaImage, p: &TraceParams) -> Mask {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut lum = vec![0f32; w * h];
    let mut rgb = vec![[0f32; 3]; w * h];
    for (x, y, px) in img.enumerate_pixels() {
        let a = px[3] as f32 / 255.0;
        // composite on white
        let c = [0, 1, 2].map(|i| px[i] as f32 * a + 255.0 * (1.0 - a));
        let i = y as usize * w + x as usize;
        rgb[i] = c;
        lum[i] = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    }
    if p.blur > 0 {
        lum = box_blur(&lum, w, h, p.blur as usize);
    }
    let mut px: Vec<bool> = match p.mode {
        TraceMode::Threshold => lum.iter().map(|l| *l < p.threshold as f32).collect(),
        TraceMode::Colour(c) => rgb
            .iter()
            .map(|q| {
                let d = ((q[0] - c[0] as f32).powi(2) + (q[1] - c[1] as f32).powi(2) + (q[2] - c[2] as f32).powi(2)).sqrt();
                d <= p.threshold as f32
            })
            .collect(),
        TraceMode::Subject => subject_mask(&rgb, w, h, p.threshold as f32),
    };
    if p.invert {
        px.iter_mut().for_each(|b| *b = !*b);
    }
    let mut m = Mask { w, h, px };
    if p.despeckle > 0 {
        despeckle(&mut m, p.despeckle as usize);
    }
    m
}

/// Foreground mask by background subtraction: flood-fill inward from the image
/// borders, treating any pixel within `tol` colour distance of the border colour
/// as background. Everything the fill can't reach is the subject.
fn subject_mask(rgb: &[[f32; 3]], w: usize, h: usize, tol: f32) -> Vec<bool> {
    let dist2 = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    // Average border colour as the background reference.
    let mut sum = [0f32; 3];
    let mut n = 0f32;
    let mut add = |i: usize| {
        for k in 0..3 {
            sum[k] += rgb[i][k];
        }
        n += 1.0;
    };
    for x in 0..w {
        add(x);
        add((h - 1) * w + x);
    }
    for y in 0..h {
        add(y * w);
        add(y * w + w - 1);
    }
    let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
    let tol2 = tol * tol;

    let mut bg = vec![false; w * h];
    let mut stack = Vec::new();
    let seed = |i: usize, bg: &mut Vec<bool>, stack: &mut Vec<usize>| {
        if !bg[i] && dist2(rgb[i], mean) <= tol2 {
            bg[i] = true;
            stack.push(i);
        }
    };
    for x in 0..w {
        seed(x, &mut bg, &mut stack);
        seed((h - 1) * w + x, &mut bg, &mut stack);
    }
    for y in 0..h {
        seed(y * w, &mut bg, &mut stack);
        seed(y * w + w - 1, &mut bg, &mut stack);
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let go = |j: usize, bg: &mut Vec<bool>, stack: &mut Vec<usize>| {
            if !bg[j] && dist2(rgb[j], mean) <= tol2 {
                bg[j] = true;
                stack.push(j);
            }
        };
        if x > 0 { go(i - 1, &mut bg, &mut stack) }
        if x + 1 < w { go(i + 1, &mut bg, &mut stack) }
        if y > 0 { go(i - w, &mut bg, &mut stack) }
        if y + 1 < h { go(i + w, &mut bg, &mut stack) }
    }
    bg.iter().map(|b| !b).collect()
}

fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; w * h];
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            let s: f32 = (a..=b).map(|i| src[y * w + i]).sum();
            tmp[y * w + x] = s / (b - a + 1) as f32;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            let s: f32 = (a..=b).map(|j| tmp[j * w + x]).sum();
            out[y * w + x] = s / (b - a + 1) as f32;
        }
    }
    out
}

/// Remove connected components (of both colours) smaller than `min` pixels.
fn despeckle(m: &mut Mask, min: usize) {
    for target in [true, false] {
        let mut seen = vec![false; m.w * m.h];
        for start in 0..m.px.len() {
            if seen[start] || m.px[start] != target {
                continue;
            }
            let mut comp = vec![start];
            let mut stack = vec![start];
            seen[start] = true;
            let mut touches_border = false;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % m.w, i / m.w);
                if x == 0 || y == 0 || x == m.w - 1 || y == m.h - 1 {
                    touches_border = true;
                }
                let mut push = |j: usize| {
                    if !seen[j] && m.px[j] == target {
                        seen[j] = true;
                        stack.push(j);
                        comp.push(j);
                    }
                };
                if x > 0 { push(i - 1) }
                if x + 1 < m.w { push(i + 1) }
                if y > 0 { push(i - m.w) }
                if y + 1 < m.h { push(i + m.w) }
            }
            if comp.len() < min && !(target == false && touches_border) {
                for i in comp {
                    m.px[i] = !target;
                }
            }
        }
    }
}

/// Follow pixel boundaries; returns closed loops in pixel-corner coordinates.
/// Loops keep foreground on the right-hand side (clockwise outer contours in y-down space).
pub fn contours(m: &Mask) -> Vec<Vec<(i32, i32)>> {
    // directed edges: from corner -> to corner
    let mut next: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for y in 0..m.h as isize {
        for x in 0..m.w as isize {
            if !m.get(x, y) {
                continue;
            }
            let (xi, yi) = (x as i32, y as i32);
            if !m.get(x, y - 1) {
                next.entry((xi, yi)).or_default().push((xi + 1, yi));
            }
            if !m.get(x + 1, y) {
                next.entry((xi + 1, yi)).or_default().push((xi + 1, yi + 1));
            }
            if !m.get(x, y + 1) {
                next.entry((xi + 1, yi + 1)).or_default().push((xi, yi + 1));
            }
            if !m.get(x - 1, y) {
                next.entry((xi, yi + 1)).or_default().push((xi, yi));
            }
        }
    }
    let mut loops = Vec::new();
    let mut keys: Vec<(i32, i32)> = next.keys().copied().collect();
    keys.sort();
    for k in keys {
        while next.get(&k).map(|v| !v.is_empty()).unwrap_or(false) {
            let mut lp = vec![k];
            let mut prev = k;
            let mut cur = next.get_mut(&k).unwrap().pop().unwrap();
            let mut guard = 0;
            while cur != k && guard < 10_000_000 {
                guard += 1;
                lp.push(cur);
                let outs = next.get_mut(&cur).unwrap();
                let pick = if outs.len() > 1 {
                    // saddle: prefer turning right relative to incoming direction
                    let din = (cur.0 - prev.0, cur.1 - prev.1);
                    let right = (-din.1, din.0);
                    outs.iter().position(|o| (o.0 - cur.0, o.1 - cur.1) == right).unwrap_or(0)
                } else {
                    0
                };
                let n = outs.swap_remove(pick);
                prev = cur;
                cur = n;
            }
            loops.push(lp);
        }
    }
    loops
}

/// Zhang-Suen thinning: erode the mask to a 1-pixel-wide skeleton (medial axis).
fn thin(m: &Mask) -> Vec<bool> {
    let (w, h) = (m.w, m.h);
    let mut g = m.px.clone();
    // p2..p9 clockwise from North (y-down): N, NE, E, SE, S, SW, W, NW.
    let offs: [(isize, isize); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
    loop {
        let mut changed = false;
        for step in 0..2 {
            let mut remove = Vec::new();
            for y in 1..h as isize - 1 {
                for x in 1..w as isize - 1 {
                    let i = y as usize * w + x as usize;
                    if !g[i] {
                        continue;
                    }
                    let p: [bool; 8] = std::array::from_fn(|k| {
                        let (dx, dy) = offs[k];
                        g[(y + dy) as usize * w + (x + dx) as usize]
                    });
                    let b = p.iter().filter(|v| **v).count();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    // A = number of 0->1 transitions in p2,p3,…,p9,p2
                    let a = (0..8).filter(|&k| !p[k] && p[(k + 1) % 8]).count();
                    if a != 1 {
                        continue;
                    }
                    let (p2, p4, p6, p8) = (p[0], p[2], p[4], p[6]);
                    if step == 0 {
                        if p2 && p4 && p6 {
                            continue;
                        }
                        if p4 && p6 && p8 {
                            continue;
                        }
                    } else {
                        if p2 && p4 && p8 {
                            continue;
                        }
                        if p2 && p6 && p8 {
                            continue;
                        }
                    }
                    remove.push(i);
                }
            }
            if !remove.is_empty() {
                changed = true;
                for i in remove {
                    g[i] = false;
                }
            }
        }
        if !changed {
            break;
        }
    }
    g
}

/// Turn a 1-pixel skeleton into open polylines by walking chains between nodes
/// (endpoints and junctions). Pixel coordinates, centred on each pixel.
fn skeleton_paths(g: &[bool], w: usize, h: usize) -> Vec<Vec<Point>> {
    let nbrs = |i: usize| -> Vec<usize> {
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        let mut v = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    let j = ny as usize * w + nx as usize;
                    if g[j] {
                        v.push(j);
                    }
                }
            }
        }
        v
    };
    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let edge = |a: usize, b: usize| if a < b { (a, b) } else { (b, a) };
    let pt = |i: usize| Point::new((i % w) as f64, (i / w) as f64);
    let mut paths = Vec::new();

    let walk = |start: usize, first: usize, used: &mut std::collections::HashSet<(usize, usize)>| -> Vec<Point> {
        let mut path = vec![pt(start)];
        let (mut prev, mut cur) = (start, first);
        loop {
            path.push(pt(cur));
            let ns = nbrs(cur);
            if ns.len() != 2 {
                break; // reached a node (endpoint or junction)
            }
            let next = ns.into_iter().find(|&n| n != prev);
            let Some(next) = next else { break };
            if !used.insert(edge(cur, next)) {
                break;
            }
            prev = cur;
            cur = next;
        }
        path
    };

    // Start chains from every node (degree != 2).
    for i in 0..g.len() {
        if !g[i] {
            continue;
        }
        let ns = nbrs(i);
        if ns.len() == 2 {
            continue;
        }
        for n in ns {
            if used.insert(edge(i, n)) {
                paths.push(walk(i, n, &mut used));
            }
        }
    }
    // Pure loops (all degree-2) left over.
    for i in 0..g.len() {
        if !g[i] {
            continue;
        }
        let ns = nbrs(i);
        if ns.len() != 2 {
            continue;
        }
        for n in ns {
            if used.insert(edge(i, n)) {
                paths.push(walk(i, n, &mut used));
            }
        }
    }
    paths
}

/// Centerline (single-stroke) trace: skeletonise the mask and vectorise it into
/// open paths. Great for pens, foil and engraving.
pub fn centerline(img: &image::RgbaImage, p: &TraceParams) -> BezPath {
    let m = make_mask(img, p);
    let skel = thin(&m);
    let mut out = BezPath::new();
    for raw in skeleton_paths(&skel, m.w, m.h) {
        if raw.len() < 3 {
            continue; // drop tiny spurs
        }
        let s = geom::rdp(&raw, p.tolerance.max(0.5));
        if s.len() < 2 {
            continue;
        }
        let pl = Polyline { pts: s, closed: false };
        let path = if p.smooth { geom::smooth_polyline(&pl, 60.0) } else { geom::polyline_to_path(&pl) };
        out.extend(path.elements().iter().copied());
    }
    out
}

/// Trace an image. Output is in pixel coordinates; map with a transform afterwards.
///
/// The mask is softened very slightly and its outlines found at sub-pixel
/// precision (marching squares), so they run smoothly instead of stepping
/// along pixel edges; then they're fitted with curves, keeping real corners
/// sharp (or simplified to straight lines when `smooth` is off).
pub fn trace(img: &image::RgbaImage, p: &TraceParams) -> BezPath {
    if p.centerline {
        return centerline(img, p);
    }
    let m = make_mask(img, p);
    let ink = ink_levels(img, p, &m);
    let (field, fw, fh) = soften(&m, &ink);
    let mut out = BezPath::new();
    for lp in iso_loops(&field, fw, fh) {
        if lp.len() < 4 {
            continue;
        }
        let mut pl = Polyline { pts: lp, closed: true };
        let outer = encloses_ink(&pl.pts, &field, fw, fh);
        // outlines go one way round and holes the other, as before
        if (pl.signed_area() > 0.0) != outer {
            pl.pts.reverse();
        }
        if p.outer_only && !outer {
            continue;
        }
        if pl.signed_area().abs() < 1.0 {
            continue;
        }
        let even = crate::curvefit::resample_closed(&pl.pts, 0.5);
        let path = if p.smooth {
            // even out wobble from noise in the picture, but not across corners
            let cs = crate::curvefit::corners(&even, 4, 55.0);
            let calm = crate::curvefit::smooth_closed(&even, &cs, 1.5 + p.tolerance);
            crate::curvefit::fit_closed(&calm, p.tolerance.max(0.3) * 0.8, 55.0, 4)
        } else {
            let mut pts = even.clone();
            pts.push(pts[0]);
            let mut s = geom::rdp(&pts, p.tolerance.max(0.3));
            s.pop();
            geom::polyline_to_path(&Polyline { pts: s, closed: true })
        };
        out.extend(path.elements().iter().copied());
    }
    out
}

/// How much each pixel is ink, 0..1, with 0.5 at the threshold: from the
/// picture's own shading where it has some (the soft, anti-aliased edges
/// say exactly where an edge runs), but always on the mask's side of 0.5, so
/// despeckling and inverting still count.
fn ink_levels(img: &image::RgbaImage, p: &TraceParams, m: &Mask) -> Vec<f32> {
    const SPAN: f32 = 96.0;
    let (w, h) = (m.w, m.h);
    let mut lum = vec![0f32; w * h];
    let mut rgb = vec![[0f32; 3]; w * h];
    for (x, y, px) in img.enumerate_pixels() {
        let a = px[3] as f32 / 255.0;
        let c = [0, 1, 2].map(|i| px[i] as f32 * a + 255.0 * (1.0 - a));
        let i = y as usize * w + x as usize;
        rgb[i] = c;
        lum[i] = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    }
    if p.blur > 0 {
        lum = box_blur(&lum, w, h, p.blur as usize);
    }
    (0..w * h)
        .map(|i| {
            let raw = match p.mode {
                TraceMode::Threshold => 0.5 + (p.threshold as f32 - lum[i]) / SPAN,
                TraceMode::Colour(c) => {
                    let q = rgb[i];
                    let d = ((q[0] - c[0] as f32).powi(2) + (q[1] - c[1] as f32).powi(2) + (q[2] - c[2] as f32).powi(2)).sqrt();
                    0.5 + (p.threshold as f32 - d) / SPAN
                }
                // a flood fill has no shading to go on
                TraceMode::Subject => if m.px[i] { 1.0 } else { 0.0 },
            };
            let v = if p.invert && p.mode != TraceMode::Subject { 1.0 - raw } else { raw };
            if m.px[i] { v.clamp(0.51, 1.0) } else { v.clamp(0.0, 0.49) }
        })
        .collect()
}

/// The ink levels lightly blurred (so any stair-steps left become slopes,
/// while a one-pixel line still survives) and with a blank border so every
/// outline closes. Value `i, j` is the middle of pixel `i - 1, j - 1`.
fn soften(m: &Mask, ink: &[f32]) -> (Vec<f32>, usize, usize) {
    let (w, h) = (m.w + 2, m.h + 2);
    let mut f = vec![0f32; w * h];
    for y in 0..m.h {
        for x in 0..m.w {
            f[(y + 1) * w + x + 1] = ink[y * m.w + x];
        }
    }
    // [1 4 1] / 6 across, then down
    let k = [1.0 / 6.0, 4.0 / 6.0, 1.0 / 6.0];
    let mut t = vec![0f32; w * h];
    for y in 0..h {
        for x in 1..w - 1 {
            t[y * w + x] = k[0] * f[y * w + x - 1] + k[1] * f[y * w + x] + k[2] * f[y * w + x + 1];
        }
    }
    let mut g = vec![0f32; w * h];
    for y in 1..h - 1 {
        for x in 0..w {
            g[y * w + x] = k[0] * t[(y - 1) * w + x] + k[1] * t[y * w + x] + k[2] * t[(y + 1) * w + x];
        }
    }
    (g, w, h)
}

/// Closed outlines where `f` crosses one half (marching squares), in pixel
/// coordinates.
fn iso_loops(f: &[f32], w: usize, h: usize) -> Vec<Vec<Point>> {
    const ISO: f32 = 0.5;
    let at = |i: usize, j: usize| f[j * w + i];
    // edge keys: horizontal edge from grid point (i, j) to (i + 1, j) is 2n,
    // vertical from (i, j) to (i, j + 1) is 2n + 1, with n = j * w + i
    let hk = |i: usize, j: usize| ((j * w + i) * 2) as u64;
    let vk = |i: usize, j: usize| ((j * w + i) * 2 + 1) as u64;
    let mut point: HashMap<u64, Point> = HashMap::new();
    let mut link: HashMap<u64, Vec<u64>> = HashMap::new();
    let cross = |a: f32, b: f32| ((ISO - a) / (b - a)).clamp(0.0, 1.0) as f64;
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let (tl, tr, br, bl) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            let case = ((tl > ISO) as u8) << 3 | ((tr > ISO) as u8) << 2 | ((br > ISO) as u8) << 1 | (bl > ISO) as u8;
            if case == 0 || case == 15 {
                continue;
            }
            let (x, y) = (i as f64 - 0.5, j as f64 - 0.5);
            let top = hk(i, j);
            let bottom = hk(i, j + 1);
            let left = vk(i, j);
            let right = vk(i + 1, j);
            let mut edge = |k: u64, p: Point| {
                point.entry(k).or_insert(p);
            };
            if (tl > ISO) != (tr > ISO) {
                edge(top, Point::new(x + cross(tl, tr), y));
            }
            if (bl > ISO) != (br > ISO) {
                edge(bottom, Point::new(x + cross(bl, br), y + 1.0));
            }
            if (tl > ISO) != (bl > ISO) {
                edge(left, Point::new(x, y + cross(tl, bl)));
            }
            if (tr > ISO) != (br > ISO) {
                edge(right, Point::new(x + 1.0, y + cross(tr, br)));
            }
            let centre = (tl + tr + br + bl) / 4.0 > ISO;
            let segs: &[(u64, u64)] = &match case {
                1 | 14 => vec![(left, bottom)],
                2 | 13 => vec![(bottom, right)],
                3 | 12 => vec![(left, right)],
                4 | 11 => vec![(top, right)],
                6 | 9 => vec![(top, bottom)],
                7 | 8 => vec![(left, top)],
                // saddles: the middle decides which corners join
                5 if centre => vec![(left, top), (bottom, right)],
                5 => vec![(top, right), (left, bottom)],
                10 if centre => vec![(top, right), (left, bottom)],
                10 => vec![(left, top), (bottom, right)],
                _ => vec![],
            };
            for &(a, b) in segs {
                link.entry(a).or_default().push(b);
                link.entry(b).or_default().push(a);
            }
        }
    }
    let mut loops = vec![];
    let mut keys: Vec<u64> = link.keys().copied().collect();
    keys.sort_unstable();
    let mut seen: std::collections::HashSet<u64> = Default::default();
    for start in keys {
        if seen.contains(&start) {
            continue;
        }
        let mut lp = vec![];
        let (mut prev, mut cur) = (u64::MAX, start);
        loop {
            seen.insert(cur);
            lp.push(point[&cur]);
            let next = link[&cur].iter().copied().find(|&n| n != prev && !seen.contains(&n));
            match next {
                Some(n) => {
                    prev = cur;
                    cur = n;
                }
                None => break,
            }
        }
        loops.push(lp);
    }
    loops
}

/// Whether the ink is inside this outline (an outer edge) rather than
/// outside it (a hole): look just to each side of one of its edges.
fn encloses_ink(pts: &[Point], f: &[f32], w: usize, h: usize) -> bool {
    let sample = |p: Point| {
        // grid point (i, j) sits at pixel (i - 0.5, j - 0.5)
        let (gx, gy) = (p.x + 0.5, p.y + 0.5);
        let (i, j) = (gx.floor().clamp(0.0, (w - 2) as f64) as usize, gy.floor().clamp(0.0, (h - 2) as f64) as usize);
        let (tx, ty) = ((gx - i as f64).clamp(0.0, 1.0) as f32, (gy - j as f64).clamp(0.0, 1.0) as f32);
        let v = |a: usize, b: usize| f[b * w + a];
        let top = v(i, j) * (1.0 - tx) + v(i + 1, j) * tx;
        let bot = v(i, j + 1) * (1.0 - tx) + v(i + 1, j + 1) * tx;
        top * (1.0 - ty) + bot * ty
    };
    // the longest edge gives the clearest reading
    let n = pts.len();
    let k = (0..n).max_by(|&a, &b| pts[a].distance(pts[(a + 1) % n]).total_cmp(&pts[b].distance(pts[(b + 1) % n]))).unwrap_or(0);
    let (a, b) = (pts[k], pts[(k + 1) % n]);
    let mid = a.midpoint(b);
    let d = b - a;
    let len = d.hypot().max(1e-9);
    let nrm = kurbo::Vec2::new(-d.y / len, d.x / len) * 0.35;
    let ink_side = if sample(mid + nrm) >= sample(mid - nrm) { mid + nrm } else { mid - nrm };
    point_in(pts, ink_side)
}

fn point_in(pts: &[Point], p: Point) -> bool {
    let n = pts.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Traces the picture in `TRACE_IMG` and draws the cut lines, zoomed, to
    /// `TRACE_OUT` (a PNG), for checking how smooth they are by eye.
    #[test]
    #[ignore = "diagnostic"]
    fn draw_a_trace() {
        let (Ok(src), Ok(dst)) = (std::env::var("TRACE_IMG"), std::env::var("TRACE_OUT")) else { return };
        let img = image::open(src).unwrap().to_rgba8();
        let path = trace(&img, &TraceParams::default());
        let zoom = 3.0;
        let (w, h) = ((img.width() as f64 * zoom) as u32, (img.height() as f64 * zoom) as u32);
        let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        for pl in geom::flatten(&path, 0.02) {
            let n = pl.pts.len();
            let segs = if pl.closed { n } else { n - 1 };
            for i in 0..segs {
                let (a, b) = (pl.pts[i], pl.pts[(i + 1) % n]);
                let steps = (a.distance(b) * zoom * 2.0).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let q = a.lerp(b, s as f64 / steps as f64);
                    let (x, y) = ((q.x * zoom) as i64, (q.y * zoom) as i64);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (x, y) = (x + dx, y + dy);
                        if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                            out.put_pixel(x as u32, y as u32, image::Rgba([220, 30, 30, 255]));
                        }
                    }
                }
            }
        }
        out.save(dst).unwrap();
        let curves = path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::CurveTo(..))).count();
        println!("{} outlines, {curves} curves", geom::subpaths(&path).len());
    }
    #[test]
    fn traces_square_with_hole() {
        let mut img = image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 255, 255, 255]));
        for y in 5..35 {
            for x in 5..35 {
                if !(15..25).contains(&x) || !(15..25).contains(&y) {
                    img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        let p = TraceParams { smooth: false, despeckle: 0, ..Default::default() };
        let m = make_mask(&img, &p);
        assert_eq!(contours(&m).len(), 2);
        let path = trace(&img, &p);
        assert_eq!(geom::subpaths(&path).len(), 2);
        let outer = trace(&img, &TraceParams { outer_only: true, ..p });
        assert_eq!(geom::subpaths(&outer).len(), 1);
    }

    #[test]
    fn otsu_splits_black_and_white() {
        let mut img = image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 255, 255, 255]));
        for y in 0..20 {
            for x in 0..10 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let t = otsu_threshold(&img);
        assert!(t > 20 && t < 235, "threshold {t} should fall between the two peaks");
    }

    #[test]
    fn subject_isolates_object_from_border() {
        // white background, red object in the middle
        let mut img = image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 255, 255, 255]));
        for y in 12..28 {
            for x in 12..28 {
                img.put_pixel(x, y, image::Rgba([200, 20, 20, 255]));
            }
        }
        let p = TraceParams { mode: TraceMode::Subject, threshold: 60, despeckle: 0, smooth: false, ..Default::default() };
        let m = make_mask(&img, &p);
        // corners are background, centre is foreground
        assert!(!m.px[0]);
        assert!(m.px[20 * 40 + 20]);
        assert_eq!(geom::subpaths(&trace(&img, &p)).len(), 1);
    }

    #[test]
    fn centerline_reduces_bar_to_a_stroke() {
        // a thick horizontal black bar should skeletonise to a single open stroke
        let mut img = image::RgbaImage::from_pixel(60, 20, image::Rgba([255, 255, 255, 255]));
        for y in 7..13 {
            for x in 5..55 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let p = TraceParams { centerline: true, despeckle: 0, smooth: false, ..Default::default() };
        let path = trace(&img, &p);
        let subs = geom::subpaths(&path);
        assert_eq!(subs.len(), 1, "one centreline stroke");
        // the stroke should be open (not closed back to its start)
        let pts: Vec<_> = subs[0].elements().iter().filter_map(|e| match e {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => Some(*p),
            _ => None,
        }).collect();
        assert!(pts.len() >= 2);
    }
}
