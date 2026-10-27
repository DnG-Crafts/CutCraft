//! HP-GL import (`.plt`, `.hpgl`): the plotter language cutters and older
//! design programs write — including CutCraft's own HP-GL export. Ported from
//! BurnCraft.
//!
//! Understands PU/PD (pen up/down, with or without points), PA/PR
//! (absolute/relative), SP (pen select: each pen gets its own colour), IN, and
//! the arc and circle commands AA, AR and CI. Units are plotter units, 40 per
//! millimetre. HP-GL's Y points up; the result is flipped.

use crate::doc::{CutStyle, Rgba, Shape, Style};
use crate::geom::polyline_path;
use anyhow::{Result, bail};
use kurbo::Point;

/// Plotter units per millimetre.
const UNITS_PER_MM: f64 = 40.0;

/// Line colours for pens 1, 2, … when a file uses more than one.
const PEN_COLOURS: [Rgba; 8] = [
    Rgba(220, 30, 30, 255),
    Rgba(30, 90, 220, 255),
    Rgba(20, 150, 60, 255),
    Rgba(230, 130, 0, 255),
    Rgba(140, 40, 180, 255),
    Rgba(0, 160, 170, 255),
    Rgba(200, 40, 140, 255),
    Rgba(90, 90, 90, 255),
];

/// The plotter's state while reading, and the lines drawn so far (plotter units).
struct Plotter {
    at: (f64, f64),
    down: bool,
    absolute: bool,
    pen: usize,
    current: Vec<Point>,
    paths: Vec<(usize, Vec<Point>)>,
}

impl Plotter {
    /// Ends the line being drawn.
    fn lift(&mut self) {
        if self.current.len() >= 2 {
            self.paths.push((self.pen, std::mem::take(&mut self.current)));
        }
        self.current.clear();
    }

    /// Moves to a point (absolute or relative by the current mode), drawing if the pen is down.
    fn go(&mut self, px: f64, py: f64) {
        let to = if self.absolute { (px, py) } else { (self.at.0 + px, self.at.1 + py) };
        self.draw_to(to);
    }

    fn draw_to(&mut self, to: (f64, f64)) {
        if self.down {
            if self.current.is_empty() {
                self.current.push(Point::new(self.at.0, self.at.1));
            }
            self.current.push(Point::new(to.0, to.1));
        } else {
            self.lift();
        }
        self.at = to;
    }
}

pub fn import(data: &[u8]) -> Result<Vec<Shape>> {
    let text = String::from_utf8_lossy(data);
    let mut plot = Plotter { at: (0.0, 0.0), down: false, absolute: true, pen: 1, current: Vec::new(), paths: Vec::new() };
    let mut commands = 0usize;
    // commands are two letters followed by parameters, ended by ';' or the next command
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        if !(chars[i].is_ascii_alphabetic() && chars[i + 1].is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let cmd: String = chars[i..i + 2].iter().collect::<String>().to_ascii_uppercase();
        i += 2;
        let start = i;
        while i < chars.len() && !chars[i].is_ascii_alphabetic() && chars[i] != ';' {
            i += 1;
        }
        let params: Vec<f64> = chars[start..i].iter().collect::<String>().split(|c: char| c == ',' || c.is_whitespace()).filter_map(|s| s.trim().parse().ok()).collect();
        commands += 1;
        match cmd.as_str() {
            "IN" => {
                plot.lift();
                (plot.at, plot.down, plot.absolute) = ((0.0, 0.0), false, true);
            }
            "SP" => {
                plot.lift();
                plot.pen = params.first().map_or(1, |p| *p as usize);
            }
            "PA" | "PR" => {
                plot.absolute = cmd == "PA";
                for [px, py] in params.as_chunks::<2>().0 {
                    plot.go(*px, *py);
                }
            }
            "PU" | "PD" => {
                plot.down = cmd == "PD";
                if !plot.down {
                    plot.lift();
                }
                for [px, py] in params.as_chunks::<2>().0 {
                    plot.go(*px, *py);
                }
            }
            "CI" => {
                // a circle round the current point; the pen ends where it started
                if let Some(&r) = params.first() {
                    plot.lift();
                    let (x, y) = plot.at;
                    let ring: Vec<Point> = (0..=72)
                        .map(|k| {
                            let a = k as f64 / 72.0 * std::f64::consts::TAU;
                            Point::new(x + r * a.cos(), y + r * a.sin())
                        })
                        .collect();
                    plot.paths.push((plot.pen, ring));
                }
            }
            "AA" | "AR" => {
                // an arc about a centre (absolute or relative), sweeping `angle` degrees anticlockwise
                if let [cx, cy, angle, ..] = params[..] {
                    let (x, y) = plot.at;
                    let (cx, cy) = if cmd == "AA" { (cx, cy) } else { (x + cx, y + cy) };
                    let r = (x - cx).hypot(y - cy);
                    let a0 = (y - cy).atan2(x - cx);
                    let steps = ((angle.abs() / 5.0).ceil() as usize).max(2);
                    for k in 1..=steps {
                        let a = a0 + angle.to_radians() * k as f64 / steps as f64;
                        plot.draw_to((cx + r * a.cos(), cy + r * a.sin()));
                    }
                }
            }
            _ => {}
        }
        if i < chars.len() && chars[i] == ';' {
            i += 1;
        }
    }
    plot.lift();
    let paths = plot.paths;
    if paths.is_empty() {
        bail!(if commands == 0 { "No HP-GL commands found" } else { "Nothing is drawn with the pen down" });
    }
    let top = paths.iter().flat_map(|(_, p)| p.iter().map(|q| q.y)).fold(f64::MIN, f64::max);
    let mut pens: Vec<usize> = Vec::new();
    for (p, _) in &paths {
        if !pens.contains(p) {
            pens.push(*p);
        }
    }
    let mut out = vec![];
    for (p, pts) in paths {
        let mm: Vec<Point> = pts.iter().map(|q| Point::new(q.x / UNITS_PER_MM, (top - q.y) / UNITS_PER_MM)).collect();
        let closed = mm.len() > 2 && mm.first().zip(mm.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-6);
        let mm = if closed { mm[..mm.len() - 1].to_vec() } else { mm };
        // one colour per pen, when more than one pen is used
        let colour = if pens.len() > 1 { PEN_COLOURS[p.saturating_sub(1) % PEN_COLOURS.len()] } else { Rgba::RED };
        let style = Style { fill: None, stroke: Some(colour), stroke_width: 0.25, cut: CutStyle::Cut, no_print: false };
        out.push(Shape::new_path(0, polyline_path(&mm, closed), style));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    #[test]
    fn square_in_plotter_units() {
        // a 10 mm square (400 units), closed
        let r = import(b"IN;SP1;PU0,0;PD400,0,400,400,0,400,0,0;PU;").unwrap();
        assert_eq!(r.len(), 1);
        let p = &r[0].world_paths()[0];
        let b = p.bounding_box();
        assert!((b.width() - 10.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9);
        assert!(crate::geom::flatten(p, 0.01)[0].closed);
    }

    #[test]
    fn relative_moves_pens_and_circles() {
        let r = import(b"IN;SP1;PU0,0;PD;PR400,0,0,400;PU;SP2;PA1000,1000;CI200;").unwrap();
        assert_eq!(r.len(), 2);
        assert_ne!(r[0].style.stroke, r[1].style.stroke, "each pen its own colour");
        let c = r[1].world_paths()[0].bounding_box();
        assert!((c.width() - 10.0).abs() < 0.01, "a circle of radius 200 units = 5 mm: {c:?}");
        assert!(import(b"IN;PU100,100;").is_err());
        assert!(import(b"").is_err());
    }

    #[test]
    fn reads_cutcrafts_own_hpgl_export() {
        use crate::doc::Document;
        let mut d = Document::default();
        d.add_path(crate::geom::rect_path(kurbo::Rect::new(20.0, 30.0, 70.0, 50.0)), Style::default(), 0);
        let mut plan = crate::cutjob::JobPlan::default();
        crate::cutjob::sync_plan(&mut plan, d.page());
        let job = crate::cutjob::generate(d.page(), &plan);
        let plt = crate::gpgl::encode_hpgl(&job, d.page().height);
        let r = import(plt.as_bytes()).unwrap();
        let b = r[0].world_paths()[0].bounding_box();
        // within blade-offset compensation and overcut of the drawn size
        assert!((b.width() - 50.0).abs() < 1.0 && (b.height() - 20.0).abs() < 1.0, "{b:?}");
    }
}
