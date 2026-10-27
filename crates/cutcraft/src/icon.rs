//! Procedurally drawn application icon (no external asset needed).
//!
//! A rounded "app tile" with a teal→blue gradient and a white craft knife over a
//! dashed cut line. Rendered at 4× and downsampled for anti-aliasing. Used for
//! the window/taskbar icon at runtime and to generate the `.exe` icon.

use image::{Rgba, RgbaImage};

type Pt = (f32, f32);

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Signed-ish test: is (px,py) inside a rounded rectangle?
fn in_rounded_rect(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if px < x0 || px > x1 || py < y0 || py > y1 {
        return false;
    }
    // corner circles
    let cx = px.clamp(x0 + r, x1 - r);
    let cy = py.clamp(y0 + r, y1 - r);
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= r * r
}

/// Even-odd point-in-polygon test.
fn in_poly(px: f32, py: f32, v: &[Pt]) -> bool {
    let mut inside = false;
    let mut j = v.len() - 1;
    for i in 0..v.len() {
        let (xi, yi) = v[i];
        let (xj, yj) = v[j];
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from point to a line segment.
fn dist_seg(px: f32, py: f32, a: Pt, b: Pt) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (px - a.0, py - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + vx * t, a.1 + vy * t);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

fn rotate(p: Pt, c: Pt, ang: f32) -> Pt {
    let (s, co) = ang.sin_cos();
    let (dx, dy) = (p.0 - c.0, p.1 - c.1);
    (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co)
}

/// Render the icon at `size` px (square), anti-aliased via 4× supersampling.
pub fn rgba(size: u32) -> RgbaImage {
    const SS: u32 = 4;
    let big = size * SS;
    let n = big as f32;
    let mut img = RgbaImage::new(big, big);

    let teal = [45.0, 212.0, 191.0];
    let blue = [37.0, 99.0, 235.0];
    let steel = [226.0, 232.0, 240.0]; // blade
    let graphite = [51.0, 65.0, 85.0]; // handle

    // Tile geometry (in 0..n space).
    let pad = n * 0.055;
    let (x0, y0, x1, y1) = (pad, pad, n - pad, n - pad);
    let radius = n * 0.20;

    let c = (n * 0.5, n * 0.5);
    let ang = -0.785398; // -45°, knife points up-right
    // Knife handle + blade defined horizontally, tip to the right, then rotated.
    let s = n / 256.0;
    let handle: Vec<Pt> = [(-96.0, -11.0), (44.0, -11.0), (44.0, 11.0), (-96.0, 11.0)]
        .iter()
        .map(|&(x, y)| rotate((c.0 + x * s, c.1 + y * s), c, ang))
        .collect();
    let blade: Vec<Pt> = [(40.0, -15.0), (96.0, -3.0), (104.0, 2.0), (40.0, 15.0)]
        .iter()
        .map(|&(x, y)| rotate((c.0 + x * s, c.1 + y * s), c, ang))
        .collect();
    // Cutting edge highlight (bottom edge of the blade).
    let edge_a = rotate((c.0 + 40.0 * s, c.1 + 15.0 * s), c, ang);
    let edge_b = rotate((c.0 + 104.0 * s, c.1 + 2.0 * s), c, ang);
    // Dashed cut line across the tile (same angle as the knife).
    let line_a = rotate((c.0 - 150.0 * s, c.1 + 50.0 * s), c, ang);
    let line_b = rotate((c.0 + 150.0 * s, c.1 + 50.0 * s), c, ang);
    let dash = 16.0 * s;

    for y in 0..big {
        for x in 0..big {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut px = [0.0, 0.0, 0.0];
            let mut a = 0.0f32;

            if in_rounded_rect(fx, fy, x0, y0, x1, y1, radius) {
                // diagonal gradient
                let t = ((fx - x0) + (fy - y0)) / ((x1 - x0) + (y1 - y0));
                px = mix(teal, blue, t.clamp(0.0, 1.0));
                a = 1.0;

                // dashed cut line (under the knife)
                if dist_seg(fx, fy, line_a, line_b) < 3.0 * s {
                    let d = dist_seg(fx, fy, line_a, (fx, fy)); // param along not trivial; use projection length
                    let _ = d;
                    // dash pattern by projecting onto the line direction
                    let (dx, dy) = (line_b.0 - line_a.0, line_b.1 - line_a.1);
                    let len = (dx * dx + dy * dy).sqrt();
                    let proj = ((fx - line_a.0) * dx + (fy - line_a.1) * dy) / len;
                    if (proj / dash).floor() as i32 % 2 == 0 {
                        px = mix(px, [255.0, 255.0, 255.0], 0.55);
                    }
                }

                // knife
                if in_poly(fx, fy, &handle) {
                    px = graphite;
                } else if in_poly(fx, fy, &blade) {
                    px = steel;
                    // bevel highlight near the cutting edge
                    let de = dist_seg(fx, fy, edge_a, edge_b);
                    if de < 7.0 * s {
                        px = mix(px, [255.0, 255.0, 255.0], 1.0 - de / (7.0 * s));
                    }
                }
            }

            img.put_pixel(x, y, Rgba([px[0] as u8, px[1] as u8, px[2] as u8, (a * 255.0) as u8]));
        }
    }

    image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3)
}

/// The window/taskbar icon for eframe.
pub fn icon_data() -> egui::IconData {
    let img = rgba(256);
    let (width, height) = (img.width(), img.height());
    egui::IconData { rgba: img.into_raw(), width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates the committed icon assets (PNG preview + `.exe` `.ico`).
    /// Run with `cargo test -p cutcraft regenerate_icon_assets -- --ignored`.
    #[test]
    #[ignore]
    fn regenerate_icon_assets() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let assets = manifest.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let img = rgba(256);
        img.save(assets.join("cutcraft.png")).unwrap();
        // Encode a 256×256 ICO for the Windows executable.
        use image::ImageEncoder;
        let mut ico = std::io::BufWriter::new(std::fs::File::create(assets.join("cutcraft.ico")).unwrap());
        image::codecs::ico::IcoEncoder::new(&mut ico)
            .write_image(img.as_raw(), 256, 256, image::ExtendedColorType::Rgba8)
            .unwrap();
        println!("wrote {}", assets.display());
    }
}
