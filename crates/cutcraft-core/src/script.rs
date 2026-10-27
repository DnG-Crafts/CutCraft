//! Scripting: run a small [Rhai](https://rhai.rs) script that generates artwork.
//!
//! Scripts call geometry builders (`rect`, `circle`, `polygon`, `star`, `line`,
//! `poly`, …) which accumulate shapes; everything is added to the document when
//! the script finishes. Rhai's own control flow and math are available, so
//! parametric generators (grids, mandalas, spirographs) are a few lines.
//!
//! ```text
//! let n = 12;
//! for i in 0..n {
//!     let a = i * 2.0 * pi() / n;
//!     circle(page_w()/2.0 + cos(a)*40.0, page_h()/2.0 + sin(a)*40.0, 8.0);
//! }
//! ```

use crate::doc::{Document, Style};
use crate::geom;
use anyhow::{Result, anyhow};
use kurbo::{BezPath, Point, Rect};
use rhai::{Dynamic, Engine};
use std::cell::RefCell;
use std::rc::Rc;

/// Coerce a Rhai value (int or float) to `f64`.
fn f(d: &Dynamic) -> f64 {
    d.as_float().or_else(|_| d.as_int().map(|i| i as f64)).unwrap_or(0.0)
}

fn regular_polygon(cx: f64, cy: f64, r: f64, sides: i64) -> BezPath {
    let n = sides.clamp(3, 512) as usize;
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64 - std::f64::consts::FRAC_PI_2;
            Point::new(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    geom::polyline_path(&pts, true)
}

fn star(cx: f64, cy: f64, outer: f64, inner: f64, points: i64) -> BezPath {
    let n = points.clamp(2, 256) as usize;
    let pts: Vec<Point> = (0..n * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = std::f64::consts::PI * i as f64 / n as f64 - std::f64::consts::FRAC_PI_2;
            Point::new(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    geom::polyline_path(&pts, true)
}

/// Run a script against `doc`, adding generated shapes to `layer`.
/// Returns the number of shapes created.
pub fn run(doc: &mut Document, src: &str, layer: u32) -> Result<usize> {
    let sink: Rc<RefCell<Vec<BezPath>>> = Rc::new(RefCell::new(Vec::new()));
    let (pw, ph) = (doc.page().width, doc.page().height);

    let mut engine = Engine::new();
    // Safety limits so a runaway script can't hang or exhaust memory.
    engine.set_max_operations(5_000_000);
    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(128, 64);
    engine.set_max_string_size(100_000);
    engine.set_max_array_size(2_000_000);

    let push = |s: &Rc<RefCell<Vec<BezPath>>>, p: BezPath| {
        if !p.elements().is_empty() {
            s.borrow_mut().push(p);
        }
    };

    {
        let s = sink.clone();
        engine.register_fn("rect", move |x: Dynamic, y: Dynamic, w: Dynamic, h: Dynamic| {
            let (x, y, w, h) = (f(&x), f(&y), f(&w), f(&h));
            s.borrow_mut().push(geom::rect_path(Rect::new(x, y, x + w, y + h)));
        });
    }
    {
        let s = sink.clone();
        engine.register_fn("circle", move |cx: Dynamic, cy: Dynamic, r: Dynamic| {
            s.borrow_mut().push(geom::circle_path(Point::new(f(&cx), f(&cy)), f(&r).abs()));
        });
    }
    {
        let s = sink.clone();
        engine.register_fn("ellipse", move |cx: Dynamic, cy: Dynamic, rx: Dynamic, ry: Dynamic| {
            let (cx, cy, rx, ry) = (f(&cx), f(&cy), f(&rx).abs(), f(&ry).abs());
            let e = kurbo::Ellipse::new(Point::new(cx, cy), (rx, ry), 0.0);
            s.borrow_mut().push(kurbo::Shape::to_path(&e, 0.1));
        });
    }
    {
        let s = sink.clone();
        engine.register_fn("line", move |x1: Dynamic, y1: Dynamic, x2: Dynamic, y2: Dynamic| {
            let p = geom::polyline_path(&[Point::new(f(&x1), f(&y1)), Point::new(f(&x2), f(&y2))], false);
            s.borrow_mut().push(p);
        });
    }
    {
        let s = sink.clone();
        engine.register_fn("polygon", move |cx: Dynamic, cy: Dynamic, r: Dynamic, sides: Dynamic| {
            let sides = sides.as_int().unwrap_or(3);
            s.borrow_mut().push(regular_polygon(f(&cx), f(&cy), f(&r).abs(), sides));
        });
    }
    {
        let s = sink.clone();
        engine.register_fn("star", move |cx: Dynamic, cy: Dynamic, outer: Dynamic, inner: Dynamic, points: Dynamic| {
            let points = points.as_int().unwrap_or(5);
            s.borrow_mut().push(star(f(&cx), f(&cy), f(&outer).abs(), f(&inner).abs(), points));
        });
    }
    {
        // poly([x0,y0, x1,y1, …]) — closed polygon from a flat coordinate array.
        let s = sink.clone();
        let push = push;
        engine.register_fn("poly", move |arr: rhai::Array| {
            let nums: Vec<f64> = arr.iter().map(f).collect();
            let pts: Vec<Point> = nums.chunks_exact(2).map(|c| Point::new(c[0], c[1])).collect();
            if pts.len() >= 2 {
                push(&s, geom::polyline_path(&pts, true));
            }
        });
    }
    {
        // polyline([x0,y0, x1,y1, …]) — open path.
        let s = sink.clone();
        engine.register_fn("polyline", move |arr: rhai::Array| {
            let nums: Vec<f64> = arr.iter().map(f).collect();
            let pts: Vec<Point> = nums.chunks_exact(2).map(|c| Point::new(c[0], c[1])).collect();
            if pts.len() >= 2 {
                s.borrow_mut().push(geom::polyline_path(&pts, false));
            }
        });
    }
    engine.register_fn("page_w", move || pw);
    engine.register_fn("page_h", move || ph);
    engine.register_fn("pi", || std::f64::consts::PI);
    engine.register_fn("rad", |d: f64| d.to_radians());

    engine.run(src).map_err(|e| anyhow!("Script error: {e}"))?;

    let paths = sink.borrow();
    for p in paths.iter() {
        doc.add_path(p.clone(), Style::default(), layer);
    }
    Ok(paths.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_generates_a_grid() {
        let mut d = Document::default();
        let src = "for i in 0..5 { for j in 0..4 { rect(i*10.0, j*10.0, 8.0, 8.0); } }";
        let n = run(&mut d, src, 0).unwrap();
        assert_eq!(n, 20);
        assert_eq!(d.page().shapes.len(), 20);
    }

    #[test]
    fn script_primitives_and_math() {
        let mut d = Document::default();
        let src = "circle(page_w()/2.0, page_h()/2.0, 20.0); star(50.0, 50.0, 20.0, 8.0, 5); polygon(10.0, 10.0, 6.0, 6);";
        let n = run(&mut d, src, 0).unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn script_uses_builtin_trig() {
        let mut d = Document::default();
        // fails if sin/cos aren't available in the engine
        let src = "for i in 0..8 { circle(50.0 + cos(i*0.5)*20.0, 50.0 + sin(i*0.5)*20.0, 5.0); }";
        assert_eq!(run(&mut d, src, 0).unwrap(), 8);
    }

    /// Render a script's output to an HTML/SVG preview.
    /// `cargo test -p cutcraft-core preview_script -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_script() {
        let mut d = Document::default();
        let src = "let cx=page_w()/2.0; let cy=page_h()/2.0; let n=12; \
                   for i in 0..n { let a=i*2.0*pi()/n; circle(cx+cos(a)*35.0, cy+sin(a)*35.0, 12.0); } \
                   circle(cx, cy, 15.0);";
        run(&mut d, src, 0).unwrap();
        let paths: Vec<_> = d.page().shapes.iter().flat_map(|s| s.world_paths()).collect();
        let svg: String = paths.iter().map(|p| format!("<path d='{}' fill='none' stroke='#d33' stroke-width='0.4'/>", p.to_svg())).collect();
        let html = format!("<!doctype html><meta charset=utf8><body style='background:#fff'><svg viewBox='0 0 {} {}' width='500'>{svg}</svg></body>", d.page().width, d.page().height);
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/script_preview.html");
        std::fs::write(&out, html).unwrap();
        println!("wrote {}", out.display());
    }

    #[test]
    fn script_error_is_reported() {
        let mut d = Document::default();
        let err = run(&mut d, "this is not valid rhai @@@", 0).unwrap_err();
        assert!(err.to_string().contains("Script error"));
    }
}
