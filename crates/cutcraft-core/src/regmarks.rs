//! Registration marks for Print & Cut.
//!
//! Layout follows the common "Type 1" arrangement used by desktop cutters: a filled
//! square at the top-left and L-shaped corner marks at the top-right and
//! bottom-left. The cutter's optical sensor locates these after printing.

use crate::geom;
use kurbo::{BezPath, Rect};
use serde::{Deserialize, Serialize};

/// What gets printed for the cutter to find.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkStyle {
    /// A filled square and two L shapes in the corners (Silhouette and others).
    #[default]
    Corners,
    /// A black box round the artwork, for the Cricut Maker 3's light sensor
    /// (on cutcutgo): `inset` from the page edges, 3 mm thick.
    SensorBox,
}

/// How thick the sensor box is printed: 3 mm (2 mm is the least that works).
pub const SENSOR_BOX_THICKNESS: f64 = 3.0;
/// The gap kept between the sensor box and any artwork.
pub const SENSOR_BOX_GAP: f64 = 3.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegMarks {
    pub enabled: bool,
    /// Size of the square and L arm length, mm.
    pub length: f64,
    /// Line thickness, mm.
    pub thickness: f64,
    /// Inset from the page edge, mm.
    pub inset: f64,
    #[serde(default)]
    pub style: MarkStyle,
}

impl Default for RegMarks {
    fn default() -> Self {
        RegMarks { enabled: false, length: 20.0, thickness: 0.5, inset: 10.0, style: MarkStyle::Corners }
    }
}

impl RegMarks {
    /// Filled outlines of the marks for printing (page coordinates).
    pub fn print_geometry(&self, page_w: f64, page_h: f64) -> Vec<BezPath> {
        if !self.enabled {
            return vec![];
        }
        if self.style == MarkStyle::SensorBox {
            // four bars that don't overlap, so any fill rule prints them solid
            let (i, t) = (self.inset, SENSOR_BOX_THICKNESS);
            let (x1, y1) = (page_w - i, page_h - i);
            return vec![
                geom::rect_path(Rect::new(i, i, x1, i + t)),
                geom::rect_path(Rect::new(i, y1 - t, x1, y1)),
                geom::rect_path(Rect::new(i, i + t, i + t, y1 - t)),
                geom::rect_path(Rect::new(x1 - t, i + t, x1, y1 - t)),
            ];
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
        if self.style == MarkStyle::SensorBox {
            let m = self.inset + SENSOR_BOX_THICKNESS + SENSOR_BOX_GAP;
            return Rect::new(m, m, page_w - m, page_h - m);
        }
        let m = self.inset + self.length.max(5.0) * 0.5 + 2.0;
        Rect::new(m, m, page_w - m, page_h - m)
    }

    /// The sensor box's centre line (page mm), when that's the style in use.
    pub fn sensor_box(&self, page_w: f64, page_h: f64) -> Option<Rect> {
        (self.enabled && self.style == MarkStyle::SensorBox).then(|| {
            let c = self.inset + SENSOR_BOX_THICKNESS / 2.0;
            Rect::new(c, c, page_w - c, page_h - c)
        })
    }

    /// Distance between the outer corners of the marks, used by the cutter's search.
    pub fn span(&self, page_w: f64, page_h: f64) -> (f64, f64) {
        (page_w - 2.0 * self.inset, page_h - 2.0 * self.inset)
    }
}
