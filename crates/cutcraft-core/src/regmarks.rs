//! Registration marks for Print & Cut.
//!
//! Layout follows the common "Type 1" arrangement used by desktop cutters: a filled
//! square at the top-left and L-shaped corner marks at the top-right and
//! bottom-left. The cutter's optical sensor locates these after printing.

use crate::geom;
use kurbo::{BezPath, Rect};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegMarks {
    pub enabled: bool,
    /// Size of the square and L arm length, mm.
    pub length: f64,
    /// Line thickness, mm.
    pub thickness: f64,
    /// Inset from the page edge, mm.
    pub inset: f64,
}

impl Default for RegMarks {
    fn default() -> Self {
        RegMarks { enabled: false, length: 20.0, thickness: 0.5, inset: 10.0 }
    }
}

impl RegMarks {
    /// Filled outlines of the marks for printing (page coordinates).
    pub fn print_geometry(&self, page_w: f64, page_h: f64) -> Vec<BezPath> {
        if !self.enabled {
            return vec![];
        }
        let (i, l, t) = (self.inset, self.length, self.thickness);
        let sq = 5.0_f64.min(l);
        let mut v = vec![geom::rect_path(Rect::new(i, i, i + sq, i + sq))];
        // top-right L
        let x1 = page_w - i;
        v.push(geom::rect_path(Rect::new(x1 - l, i, x1, i + t)));
        v.push(geom::rect_path(Rect::new(x1 - t, i, x1, i + l)));
        // bottom-left L
        let y1 = page_h - i;
        v.push(geom::rect_path(Rect::new(i, y1 - t, i + l, y1)));
        v.push(geom::rect_path(Rect::new(i, y1 - l, i + t, y1)));
        v
    }

    /// Area that may contain artwork (inside the marks, with a safety margin).
    pub fn safe_area(&self, page_w: f64, page_h: f64) -> Rect {
        let m = self.inset + self.length.max(5.0) * 0.5 + 2.0;
        Rect::new(m, m, page_w - m, page_h - m)
    }

    /// Distance between the outer corners of the marks, used by the cutter's search.
    pub fn span(&self, page_w: f64, page_h: f64) -> (f64, f64) {
        (page_w - 2.0 * self.inset, page_h - 2.0 * self.inset)
    }
}
