//! Barcode generators: Code 39, Code 128 (set B) and QR codes, as filled paths (mm).

use crate::geom;
use anyhow::{Result, bail};
use kurbo::{BezPath, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbology {
    Code39,
    Code128,
    Qr,
}

impl Symbology {
    pub const ALL: [(Symbology, &'static str); 3] =
        [(Symbology::Code39, "Code 39"), (Symbology::Code128, "Code 128"), (Symbology::Qr, "QR code")];
}

const CODE39: &[(char, &str)] = &[
    ('0', "000110100"), ('1', "100100001"), ('2', "001100001"), ('3', "101100000"), ('4', "000110001"),
    ('5', "100110000"), ('6', "001110000"), ('7', "000100101"), ('8', "100100100"), ('9', "001100100"),
    ('A', "100001001"), ('B', "001001001"), ('C', "101001000"), ('D', "000011001"), ('E', "100011000"),
    ('F', "001011000"), ('G', "000001101"), ('H', "100001100"), ('I', "001001100"), ('J', "000011100"),
    ('K', "100000011"), ('L', "001000011"), ('M', "101000010"), ('N', "000010011"), ('O', "100010010"),
    ('P', "001010010"), ('Q', "000000111"), ('R', "100000110"), ('S', "001000110"), ('T', "000010110"),
    ('U', "110000001"), ('V', "011000001"), ('W', "111000000"), ('X', "010010001"), ('Y', "110010000"),
    ('Z', "011010000"), ('-', "010000101"), ('.', "110000100"), (' ', "011000100"), ('$', "010101000"),
    ('/', "010100010"), ('+', "010001010"), ('%', "000101010"), ('*', "010010100"),
];

const CODE128: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213",
    "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221", "223211", "221132",
    "221231", "213212", "223112", "312131", "311222", "321122", "321221", "312212", "322112", "322211",
    "212123", "212321", "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313",
    "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121", "313121", "211331",
    "231131", "213113", "213311", "213131", "311123", "311321", "331121", "312113", "312311", "332111",
    "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214",
    "112412", "122114", "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111",
    "111242", "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311", "113141",
    "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// Bar/space run widths in modules, starting with a bar.
pub fn code39_runs(text: &str) -> Result<Vec<u32>> {
    let mut runs = vec![];
    let full = format!("*{}*", text.to_uppercase());
    for (ci, ch) in full.chars().enumerate() {
        let Some((_, pat)) = CODE39.iter().find(|(c, _)| *c == ch) else { bail!("Code 39 can't encode '{ch}'") };
        if ci > 0 {
            runs.push(1); // inter-character gap (space)
        }
        for b in pat.bytes() {
            runs.push(if b == b'1' { 3 } else { 1 });
        }
    }
    Ok(runs)
}

pub fn code128_runs(text: &str) -> Result<Vec<u32>> {
    let mut vals = vec![104u32]; // Start B
    for ch in text.chars() {
        let c = ch as u32;
        if !(32..=127).contains(&c) {
            bail!("Code 128 (set B) can't encode '{ch}'");
        }
        vals.push(c - 32);
    }
    let sum: u32 = vals.iter().enumerate().map(|(i, v)| if i == 0 { *v } else { *v * i as u32 }).sum();
    vals.push(sum % 103);
    vals.push(106);
    Ok(vals.iter().flat_map(|v| CODE128[*v as usize].bytes().map(|b| (b - b'0') as u32)).collect())
}

/// Linear barcode as filled bar rectangles.
fn bars(runs: &[u32], x0: f64, y0: f64, module: f64, height: f64) -> BezPath {
    let mut x = x0;
    let mut b = BezPath::new();
    for (i, r) in runs.iter().enumerate() {
        let w = *r as f64 * module;
        if i % 2 == 0 {
            b.extend(geom::rect_path(Rect::new(x, y0, x + w, y0 + height)).elements().iter().copied());
        }
        x += w;
    }
    b
}

/// Generate a barcode at (x,y). `module` is the narrow bar / QR module size, mm.
pub fn generate(sym: Symbology, text: &str, x: f64, y: f64, module: f64, height: f64) -> Result<BezPath> {
    if text.is_empty() {
        bail!("Enter some text to encode");
    }
    Ok(match sym {
        Symbology::Code39 => bars(&code39_runs(text)?, x, y, module, height),
        Symbology::Code128 => bars(&code128_runs(text)?, x, y, module, height),
        Symbology::Qr => {
            let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).map_err(|_| anyhow::anyhow!("Text too long for a QR code"))?;
            let n = qr.size();
            let mut cells = vec![];
            for j in 0..n {
                // merge horizontal runs of dark modules into one rectangle
                let mut i = 0;
                while i < n {
                    if qr.get_module(i, j) {
                        let s = i;
                        while i < n && qr.get_module(i, j) {
                            i += 1;
                        }
                        cells.push(geom::rect_path(Rect::new(
                            x + s as f64 * module,
                            y + j as f64 * module,
                            x + i as f64 * module,
                            y + (j + 1) as f64 * module,
                        )));
                    } else {
                        i += 1;
                    }
                }
            }
            // join touching runs so a stencil cuts clean outlines
            geom::boolean(&cells, &[], geom::BoolOp::Union)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code128_table_is_consistent() {
        for (i, p) in CODE128.iter().enumerate() {
            let s: u32 = p.bytes().map(|b| (b - b'0') as u32).sum();
            assert_eq!(s, if i == 106 { 13 } else { 11 }, "pattern {i}");
        }
    }

    #[test]
    fn code39_table_has_three_wide() {
        for (c, p) in CODE39 {
            assert_eq!(p.bytes().filter(|b| *b == b'1').count(), 3, "{c}");
        }
        assert!(code39_runs("cut-123").is_ok());
        assert!(code39_runs("ä").is_err());
    }

    #[test]
    fn code128_checksum_example() {
        // "Wikipedia" example: checksum value for "PJJ123C" with Start B is 54
        let r = code128_runs("PJJ123C").unwrap();
        let modules: u32 = r.iter().sum();
        assert_eq!(modules, 11 * 9 + 13); // start + 7 chars + checksum + stop
    }

    #[test]
    fn qr_generates_paths() {
        let p = generate(Symbology::Qr, "https://example.com", 0.0, 0.0, 1.0, 0.0).unwrap();
        let bb = kurbo::Shape::bounding_box(&p);
        assert!(bb.width() >= 21.0 && bb.width() <= 30.0);
    }
}
