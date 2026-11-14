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
    /// (on cutcutgo): 3 mm thick, 3 mm out from everything on the page.
    SensorBox,
}

/// How much of the sensor box the Maker 3 scans: quicker, or more exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxScan {
    /// The top-left corner: where the sheet is, assuming it's straight and
    /// printed at 100% (4 scans).
    Corner,
    /// Three corners (top-left, top-right, bottom-left): also how it's turned
    /// and its printed size across and down, separately (12 scans). Two
    /// corners can't tell a turned sheet from a print stretched more one way
    /// than the other, so a saved "two corners" choice reads as this.
    #[serde(alias = "TwoCorners")]
    ThreeCorners,
    /// Every side twice: also any skew, and a check that the scans agree
    /// (16 scans).
    #[default]
    Full,
}

impl BoxScan {
    pub const ALL: [BoxScan; 3] = [BoxScan::Corner, BoxScan::ThreeCorners, BoxScan::Full];

    /// How many `#YE` scans it takes.
    pub fn scans(self) -> usize {
        match self {
            BoxScan::Corner => 4,
            BoxScan::ThreeCorners => 12,
            BoxScan::Full => 16,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BoxScan::Corner => "One corner",
            BoxScan::ThreeCorners => "Three corners",
            BoxScan::Full => "The whole box",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            BoxScan::Corner => "4 scans, under a minute. Finds where the sheet is; assumes it's straight and printed at 100%.",
            BoxScan::ThreeCorners => "12 scans, about 2 minutes. Also finds how it's turned, and its printed size across and down.",
            BoxScan::Full => "16 scans, about 3 minutes. Also finds any skew, and checks the scans agree. The most accurate.",
        }
    }
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
    /// Maker 3: how much of the box to scan.
    #[serde(default)]
    pub scan: BoxScan,
}

impl Default for RegMarks {
    fn default() -> Self {
        RegMarks { enabled: false, length: 20.0, thickness: 0.5, inset: 10.0, style: MarkStyle::Corners, scan: BoxScan::Full }
    }
}

impl RegMarks {
    /// Filled outlines of the marks for printing (page coordinates).
    pub fn print_geometry(&self, page_w: f64, page_h: f64) -> Vec<BezPath> {
        if !self.enabled {
            return vec![];
        }
        if self.style == MarkStyle::SensorBox {
            // drawn round the artwork: see print_marks
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
        if self.style == MarkStyle::SensorBox {
            let m = self.inset + SENSOR_BOX_THICKNESS + SENSOR_BOX_GAP;
            return Rect::new(m, m, page_w - m, page_h - m);
        }
        let m = self.inset + self.length.max(5.0) * 0.5 + 2.0;
        Rect::new(m, m, page_w - m, page_h - m)
    }

    /// Distance between the outer corners of the marks, used by the cutter's search.
    pub fn span(&self, page_w: f64, page_h: f64) -> (f64, f64) {
        (page_w - 2.0 * self.inset, page_h - 2.0 * self.inset)
    }
}

/// The sensor box's centre line (page mm), when that's the style in use: 3 mm
/// out from everything on the page, like Cricut's own, so it only takes the
/// paper the design needs. With nothing on the page, round the page.
pub fn sensor_box(page: &crate::doc::Page) -> Option<Rect> {
    let r = &page.regmarks;
    if !r.enabled || r.style != MarkStyle::SensorBox {
        return None;
    }
    let content = page.shapes.iter().filter(|s| page.is_visible(s)).filter_map(|s| s.bbox()).reduce(|a, b| a.union(b));
    let half = SENSOR_BOX_THICKNESS / 2.0;
    Some(match content {
        Some(c) => c.inflate(SENSOR_BOX_GAP + half, SENSOR_BOX_GAP + half),
        None => {
            let c = r.inset + half;
            Rect::new(c, c, page.width - c, page.height - c)
        }
    })
}

/// The marks to print on `page`, filled outlines in page mm.
pub fn print_marks(page: &crate::doc::Page) -> Vec<BezPath> {
    let Some(b) = sensor_box(page) else { return page.regmarks.print_geometry(page.width, page.height) };
    // four bars that don't overlap, so any fill rule prints them solid
    let (o, i) = (b.inflate(SENSOR_BOX_THICKNESS / 2.0, SENSOR_BOX_THICKNESS / 2.0), b.inflate(-SENSOR_BOX_THICKNESS / 2.0, -SENSOR_BOX_THICKNESS / 2.0));
    vec![
        geom::rect_path(Rect::new(o.x0, o.y0, o.x1, i.y0)),
        geom::rect_path(Rect::new(o.x0, i.y1, o.x1, o.y1)),
        geom::rect_path(Rect::new(o.x0, i.y0, i.x0, i.y1)),
        geom::rect_path(Rect::new(i.x1, i.y0, o.x1, i.y1)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sensor_box_hugs_the_artwork() {
        let mut d = crate::doc::Document::default();
        d.page_mut().regmarks = RegMarks { enabled: true, inset: 15.0, style: MarkStyle::SensorBox, ..Default::default() };
        // nothing on the page: round the page
        assert_eq!(sensor_box(d.page()), Some(Rect::new(16.5, 16.5, d.page().width - 16.5, d.page().height - 16.5)));
        d.add_path(geom::rect_path(Rect::new(40.0, 50.0, 90.0, 80.0)), crate::doc::Style { stroke: None, ..Default::default() }, 0);
        let b = sensor_box(d.page()).unwrap();
        assert!((b.x0 - 35.5).abs() < 1e-9 && (b.y0 - 45.5).abs() < 1e-9 && (b.x1 - 94.5).abs() < 1e-9 && (b.y1 - 84.5).abs() < 1e-9, "{b:?}");
        // printed as a 3 mm frame with a 3 mm gap to the artwork
        let marks = print_marks(d.page());
        assert_eq!(marks.len(), 4);
        let outer = marks.iter().map(|m| kurbo::Shape::bounding_box(m)).reduce(|a, b| a.union(b)).unwrap();
        assert_eq!(outer, Rect::new(34.0, 44.0, 96.0, 86.0));
        // other machines' marks don't change
        d.page_mut().regmarks.style = MarkStyle::Corners;
        assert_eq!(sensor_box(d.page()), None);
        assert_eq!(print_marks(d.page()).len(), 5);
    }
}
