//! Toolbar and panel icons, drawn with simple vector strokes so they look the
//! same at any size and in both themes. Ported from BurnCraft (the same
//! author's laser app), plus CutCraft's own tools and panel categories.

use egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui};
use std::f32::consts::{PI, TAU};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    New,
    Open,
    Save,
    Import,
    Export,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    ZoomFit,
    ZoomIn,
    ZoomOut,
    ZoomSelection,
    Camera,
    Settings,
    Group,
    Ungroup,
    MirrorH,
    MirrorV,
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignCenters,
    AlignBottom,
    DistributeH,
    DistributeV,
    Preview,
    Select,
    Nodes,
    Line,
    Rect,
    Ellipse,
    Polygon,
    Star,
    Text,
    Measure,
    Offset,
    Weld,
    Subtract,
    Intersect,
    Exclude,
    Unite,
    Flatten,
    GridArray,
    CircularArray,
    Trace,
    Barcode,
    Corner,
    Trim,
    ShapeBuilder,
    Knife,
    Dimension,
    Eraser,
    Lock,
    Unlocked,
    // --- CutCraft
    Pan,
    Polyline,
    Curve,
    Freehand,
    RoundRect,
    ImageEraser,
    Page,
    Layers,
    History,
    Params,
    Transform,
    Image,
    Style,
    Effects,
    Art,
    More,
    Make,
    Fonts,
    Mail,
    PrintCut,
    Script,
    Dieline,
    PathTools,
    Warp,
    Colours,
}

/// A square icon button. `selected` draws it pressed.
pub fn button(ui: &mut Ui, icon: Icon, size: f32, selected: bool, enabled: bool, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), if enabled { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    let bg = if selected {
        visuals.selection.bg_fill.gamma_multiply(0.6)
    } else if enabled && response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    let ink = if enabled { visuals.strong_text_color() } else { visuals.weak_text_color().gamma_multiply(0.5) };
    paint(ui.painter(), rect.shrink(size * 0.16), icon, ink, visuals.selection.stroke.color);
    response.on_hover_text(tooltip)
}

fn pts(r: Rect, list: &[(f32, f32)]) -> Vec<Pos2> {
    list.iter().map(|&(x, y)| r.lerp_inside(egui::vec2(x, y))).collect()
}

/// Draws `icon` inside `r` (coordinates below are 0..1 within the box).
pub fn paint(p: &egui::Painter, r: Rect, icon: Icon, ink: Color32, accent: Color32) {
    let w = (r.width() / 14.0).max(1.2);
    let s = Stroke::new(w, ink);
    let a = Stroke::new(w, accent);
    let at = |x: f32, y: f32| r.lerp_inside(egui::vec2(x, y));
    let line = |list: &[(f32, f32)], st: Stroke| {
        p.add(egui::Shape::line(pts(r, list), st));
    };
    let closed = |list: &[(f32, f32)], st: Stroke| {
        p.add(egui::Shape::closed_line(pts(r, list), st));
    };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32, st: Stroke| {
        p.rect_stroke(Rect::from_min_max(at(x0, y0), at(x1, y1)), 1.0, st, StrokeKind::Middle);
    };
    let fill_rect = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color32| {
        p.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), 1.0, c);
    };
    let arc = |cx: f32, cy: f32, rad: f32, a0: f32, a1: f32, st: Stroke| {
        let c = at(cx, cy);
        let rr = rad * r.width();
        let n = 16;
        let v: Vec<Pos2> = (0..=n).map(|i| {
            let t = a0 + (a1 - a0) * i as f32 / n as f32;
            c + egui::vec2(t.cos(), t.sin()) * rr
        }).collect();
        p.add(egui::Shape::line(v, st));
    };
    let arrow_head = |tip: (f32, f32), dir: f32, st: Stroke| {
        let t = at(tip.0, tip.1);
        let l = r.width() * 0.22;
        for d in [dir + 2.6, dir - 2.6] {
            p.line_segment([t, t + egui::vec2(d.cos(), d.sin()) * l], st);
        }
    };
    let magnifier = |st: Stroke| {
        p.circle_stroke(at(0.42, 0.42), r.width() * 0.3, st);
        p.line_segment([at(0.64, 0.64), at(0.95, 0.95)], Stroke::new(st.width * 1.6, st.color));
    };
    match icon {
        Icon::New => {
            closed(&[(0.2, 0.05), (0.62, 0.05), (0.85, 0.28), (0.85, 0.95), (0.2, 0.95)], s);
            line(&[(0.62, 0.05), (0.62, 0.28), (0.85, 0.28)], s);
            line(&[(0.52, 0.5), (0.52, 0.8)], a);
            line(&[(0.37, 0.65), (0.67, 0.65)], a);
        }
        Icon::Open => {
            closed(&[(0.05, 0.2), (0.38, 0.2), (0.48, 0.32), (0.85, 0.32), (0.85, 0.85), (0.05, 0.85)], s);
            closed(&[(0.18, 0.48), (1.0, 0.48), (0.85, 0.85), (0.05, 0.85)], a);
        }
        Icon::Save => {
            closed(&[(0.08, 0.08), (0.78, 0.08), (0.92, 0.22), (0.92, 0.92), (0.08, 0.92)], s);
            rect(0.26, 0.08, 0.7, 0.36, s);
            fill_rect(0.25, 0.55, 0.75, 0.92, accent.gamma_multiply(0.5));
        }
        Icon::Import => {
            line(&[(0.1, 0.6), (0.1, 0.92), (0.9, 0.92), (0.9, 0.6)], s);
            line(&[(0.5, 0.05), (0.5, 0.7)], a);
            arrow_head((0.5, 0.72), PI / 2.0, a);
        }
        Icon::Export => {
            line(&[(0.1, 0.6), (0.1, 0.92), (0.9, 0.92), (0.9, 0.6)], s);
            line(&[(0.5, 0.72), (0.5, 0.08)], a);
            arrow_head((0.5, 0.06), -PI / 2.0, a);
        }
        Icon::Undo => {
            arc(0.52, 0.55, 0.32, -PI * 0.9, PI * 0.6, s);
            arrow_head((0.21, 0.5), -PI * 0.45, s);
        }
        Icon::Redo => {
            arc(0.48, 0.55, 0.32, -PI * 0.1, -PI * 1.6, s);
            arrow_head((0.79, 0.5), -PI * 0.55, s);
        }
        Icon::Cut => {
            p.circle_stroke(at(0.25, 0.78), r.width() * 0.15, s);
            p.circle_stroke(at(0.75, 0.78), r.width() * 0.15, s);
            line(&[(0.33, 0.66), (0.8, 0.05)], s);
            line(&[(0.67, 0.66), (0.2, 0.05)], s);
        }
        Icon::Copy => {
            rect(0.05, 0.05, 0.62, 0.65, s);
            fill_rect(0.38, 0.35, 0.95, 0.95, Color32::TRANSPARENT);
            rect(0.38, 0.35, 0.95, 0.95, a);
        }
        Icon::Paste => {
            rect(0.12, 0.15, 0.88, 0.95, s);
            rect(0.32, 0.05, 0.68, 0.25, s);
            line(&[(0.3, 0.5), (0.7, 0.5)], a);
            line(&[(0.3, 0.7), (0.7, 0.7)], a);
        }
        Icon::Delete => {
            line(&[(0.1, 0.2), (0.9, 0.2)], s);
            line(&[(0.38, 0.2), (0.38, 0.06), (0.62, 0.06), (0.62, 0.2)], s);
            line(&[(0.2, 0.2), (0.27, 0.95), (0.73, 0.95), (0.8, 0.2)], s);
            line(&[(0.43, 0.35), (0.43, 0.82)], s);
            line(&[(0.57, 0.35), (0.57, 0.82)], s);
        }
        Icon::ZoomFit => {
            for (x, y, dx, dy) in [(0.05, 0.05, 1.0, 1.0), (0.95, 0.05, -1.0, 1.0), (0.05, 0.95, 1.0, -1.0), (0.95, 0.95, -1.0, -1.0)] {
                line(&[(x + dx * 0.3, y), (x, y), (x, y + dy * 0.3)], s);
            }
            rect(0.32, 0.32, 0.68, 0.68, a);
        }
        Icon::ZoomIn | Icon::ZoomOut | Icon::ZoomSelection => {
            magnifier(s);
            match icon {
                Icon::ZoomIn => {
                    line(&[(0.27, 0.42), (0.57, 0.42)], a);
                    line(&[(0.42, 0.27), (0.42, 0.57)], a);
                }
                Icon::ZoomOut => line(&[(0.27, 0.42), (0.57, 0.42)], a),
                _ => rect(0.3, 0.3, 0.54, 0.54, a),
            }
        }
        Icon::Camera => {
            rect(0.05, 0.28, 0.95, 0.88, s);
            line(&[(0.3, 0.28), (0.38, 0.12), (0.62, 0.12), (0.7, 0.28)], s);
            p.circle_stroke(at(0.5, 0.58), r.width() * 0.18, a);
        }
        Icon::Settings => {
            let c = at(0.5, 0.5);
            for i in 0..8 {
                let t = i as f32 * TAU / 8.0;
                p.line_segment([c + egui::vec2(t.cos(), t.sin()) * r.width() * 0.3, c + egui::vec2(t.cos(), t.sin()) * r.width() * 0.47], Stroke::new(w * 1.8, ink));
            }
            p.circle_stroke(c, r.width() * 0.3, s);
            p.circle_stroke(c, r.width() * 0.12, a);
        }
        Icon::Group | Icon::Ungroup => {
            rect(0.08, 0.08, 0.55, 0.55, s);
            rect(0.45, 0.45, 0.92, 0.92, s);
            if icon == Icon::Group {
                rect(0.02, 0.02, 0.98, 0.98, Stroke::new(w * 0.8, accent));
            } else {
                line(&[(0.62, 0.15), (0.88, 0.15)], a);
                line(&[(0.15, 0.62), (0.15, 0.88)], a);
            }
        }
        Icon::MirrorH => {
            closed(&[(0.42, 0.15), (0.42, 0.85), (0.05, 0.85)], s);
            closed(&[(0.58, 0.15), (0.58, 0.85), (0.95, 0.85)], a);
            line(&[(0.5, 0.0), (0.5, 1.0)], Stroke::new(w * 0.7, ink));
        }
        Icon::MirrorV => {
            closed(&[(0.15, 0.42), (0.85, 0.42), (0.85, 0.05)], s);
            closed(&[(0.15, 0.58), (0.85, 0.58), (0.85, 0.95)], a);
            line(&[(0.0, 0.5), (1.0, 0.5)], Stroke::new(w * 0.7, ink));
        }
        Icon::AlignLeft | Icon::AlignHCenter | Icon::AlignRight => {
            let (x0a, x0b) = match icon {
                Icon::AlignLeft => (0.12, 0.12),
                Icon::AlignHCenter => (0.2, 0.32),
                _ => (0.28, 0.52),
            };
            fill_rect(x0a, 0.15, x0a + 0.6, 0.38, ink.gamma_multiply(0.7));
            fill_rect(x0b, 0.6, x0b + 0.36, 0.83, ink.gamma_multiply(0.7));
            let gx = match icon {
                Icon::AlignLeft => 0.08,
                Icon::AlignHCenter => 0.5,
                _ => 0.92,
            };
            line(&[(gx, 0.0), (gx, 1.0)], a);
        }
        Icon::AlignTop | Icon::AlignVCenter | Icon::AlignBottom => {
            let (y0a, y0b) = match icon {
                Icon::AlignTop => (0.12, 0.12),
                Icon::AlignVCenter => (0.2, 0.32),
                _ => (0.28, 0.52),
            };
            fill_rect(0.15, y0a, 0.38, y0a + 0.6, ink.gamma_multiply(0.7));
            fill_rect(0.6, y0b, 0.83, y0b + 0.36, ink.gamma_multiply(0.7));
            let gy = match icon {
                Icon::AlignTop => 0.08,
                Icon::AlignVCenter => 0.5,
                _ => 0.92,
            };
            line(&[(0.0, gy), (1.0, gy)], a);
        }
        Icon::AlignCenters => {
            // A big and a small box sharing one centre, on crossed centre lines.
            fill_rect(0.12, 0.2, 0.88, 0.8, ink.gamma_multiply(0.35));
            fill_rect(0.32, 0.36, 0.68, 0.64, ink.gamma_multiply(0.8));
            line(&[(0.5, 0.0), (0.5, 1.0)], a);
            line(&[(0.0, 0.5), (1.0, 0.5)], a);
        }
        Icon::DistributeH => {
            for x in [0.05, 0.4, 0.75] {
                fill_rect(x, 0.25, x + 0.2, 0.75, ink.gamma_multiply(0.7));
            }
            line(&[(0.0, 0.92), (1.0, 0.92)], a);
        }
        Icon::DistributeV => {
            for y in [0.05, 0.4, 0.75] {
                fill_rect(0.25, y, 0.75, y + 0.2, ink.gamma_multiply(0.7));
            }
            line(&[(0.92, 0.0), (0.92, 1.0)], a);
        }
        Icon::Preview => {
            closed(&[(0.05, 0.5), (0.3, 0.22), (0.7, 0.22), (0.95, 0.5), (0.7, 0.78), (0.3, 0.78)], s);
            p.circle_filled(at(0.5, 0.5), r.width() * 0.14, accent);
        }
        Icon::Select => {
            closed(&[(0.2, 0.05), (0.2, 0.8), (0.4, 0.62), (0.55, 0.95), (0.68, 0.88), (0.53, 0.56), (0.78, 0.56)], s);
        }
        Icon::Nodes => {
            line(&[(0.1, 0.85), (0.35, 0.3), (0.9, 0.15)], s);
            for (x, y) in [(0.1, 0.85), (0.35, 0.3), (0.9, 0.15)] {
                p.rect_filled(Rect::from_center_size(at(x, y), egui::vec2(w * 3.2, w * 3.2)), 0.0, accent);
            }
        }
        Icon::Line => {
            line(&[(0.1, 0.9), (0.3, 0.7), (0.85, 0.15), (0.95, 0.05)], s);
            line(&[(0.1, 0.9), (0.05, 0.95)], a);
            closed(&[(0.25, 0.75), (0.75, 0.25), (0.85, 0.35), (0.35, 0.85)], s);
        }
        Icon::Rect => rect(0.08, 0.2, 0.92, 0.8, s),
        Icon::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(at(0.5, 0.5), egui::vec2(r.width() * 0.45, r.height() * 0.33), s));
        }
        Icon::Polygon | Icon::Star => {
            let n = if icon == Icon::Polygon { 6 } else { 10 };
            let c = (0.5, 0.52);
            let v: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    let t = i as f32 * TAU / n as f32 - PI / 2.0;
                    let rad = if icon == Icon::Star && i % 2 == 1 { 0.2 } else { 0.46 };
                    (c.0 + rad * t.cos(), c.1 + rad * t.sin())
                })
                .collect();
            closed(&v, s);
        }
        Icon::Text => {
            line(&[(0.1, 0.95), (0.5, 0.05), (0.9, 0.95)], Stroke::new(w * 1.4, ink));
            line(&[(0.27, 0.6), (0.73, 0.6)], Stroke::new(w * 1.4, ink));
        }
        Icon::Measure => {
            closed(&[(0.05, 0.7), (0.7, 0.05), (0.95, 0.3), (0.3, 0.95)], s);
            for i in 1..5 {
                let t = i as f32 / 5.0;
                let (x, y) = (0.05 + 0.65 * t, 0.7 - 0.65 * t);
                line(&[(x, y), (x + 0.1, y + 0.1)], a);
            }
        }
        Icon::Offset => {
            rect(0.3, 0.3, 0.7, 0.7, s);
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.08), at(0.92, 0.92)), r.width() * 0.2, a, StrokeKind::Middle);
        }
        Icon::Exclude => {
            // the two outer parts kept, the overlap gone
            let (c1, c2, rad) = (at(0.37, 0.5), at(0.63, 0.5), r.width() * 0.3);
            p.circle_stroke(c1, rad, s);
            p.circle_stroke(c2, rad, s);
            p.circle_filled(at(0.2, 0.5), rad * 0.38, accent);
            p.circle_filled(at(0.8, 0.5), rad * 0.38, accent);
        }
        Icon::Unite => {
            // two outlines with one welded outline round both
            let (c1, c2, rad) = (at(0.37, 0.5), at(0.63, 0.5), r.width() * 0.3);
            p.circle_stroke(c1, rad, Stroke::new(w * 0.6, ink.gamma_multiply(0.6)));
            p.circle_stroke(c2, rad, Stroke::new(w * 0.6, ink.gamma_multiply(0.6)));
            arc(0.37, 0.5, 0.3, std::f32::consts::FRAC_PI_2 * 0.6, TAU - std::f32::consts::FRAC_PI_2 * 0.6, a);
            arc(0.63, 0.5, 0.3, -std::f32::consts::FRAC_PI_2 * 1.4, std::f32::consts::FRAC_PI_2 * 1.4, a);
        }
        Icon::Flatten => {
            // stacked sheets pressed into one printed sheet
            rect(0.18, 0.08, 0.82, 0.38, s);
            rect(0.12, 0.22, 0.76, 0.52, s);
            line(&[(0.5, 0.58), (0.5, 0.72)], a);
            arrow_head((0.5, 0.74), PI / 2.0, a);
            fill_rect(0.1, 0.78, 0.9, 0.95, accent.gamma_multiply(0.7));
        }
        Icon::Colours => {
            // three overlapping swatches
            for (x, y, k) in [(0.33, 0.35, 0.9), (0.67, 0.35, 0.6), (0.5, 0.65, 0.35)] {
                p.circle_filled(at(x, y), r.width() * 0.24, accent.gamma_multiply(k));
            }
            p.circle_stroke(at(0.5, 0.5), r.width() * 0.47, s);
        }
        Icon::Weld | Icon::Subtract | Icon::Intersect => {
            let (c1, c2, rad) = (at(0.37, 0.5), at(0.63, 0.5), r.width() * 0.3);
            match icon {
                Icon::Weld => {
                    p.circle_filled(c1, rad, accent.gamma_multiply(0.45));
                    p.circle_filled(c2, rad, accent.gamma_multiply(0.45));
                }
                Icon::Subtract => {
                    p.circle_filled(c1, rad, accent.gamma_multiply(0.45));
                }
                _ => {}
            }
            p.circle_stroke(c1, rad, s);
            p.circle_stroke(c2, rad, s);
            if icon == Icon::Intersect {
                p.circle_filled(at(0.5, 0.5), rad * 0.45, accent);
            }
        }
        Icon::GridArray => {
            for y in [0.08, 0.4, 0.72] {
                for x in [0.08, 0.4, 0.72] {
                    rect(x, y, x + 0.2, y + 0.2, s);
                }
            }
        }
        Icon::CircularArray => {
            for i in 0..8 {
                let t = i as f32 * TAU / 8.0;
                p.circle_stroke(at(0.5 + 0.36 * t.cos(), 0.5 + 0.36 * t.sin()), r.width() * 0.08, if i == 0 { a } else { s });
            }
        }
        Icon::Trace => {
            fill_rect(0.05, 0.05, 0.55, 0.55, ink.gamma_multiply(0.4));
            closed(&[(0.45, 0.45), (0.95, 0.45), (0.95, 0.95), (0.45, 0.95)], a);
            line(&[(0.05, 0.95), (0.95, 0.05)], Stroke::new(w * 0.7, ink));
        }
        Icon::Barcode => {
            for (x, wd) in [(0.05, 0.08), (0.18, 0.04), (0.27, 0.1), (0.42, 0.04), (0.5, 0.06), (0.62, 0.1), (0.77, 0.04), (0.85, 0.08)] {
                fill_rect(x, 0.12, x + wd, 0.88, ink);
            }
        }
        Icon::Corner => {
            line(&[(0.1, 0.95), (0.1, 0.5)], s);
            arc(0.55, 0.5, 0.45, PI, PI * 1.5, a);
            line(&[(0.55, 0.05), (0.95, 0.05)], s);
        }
        Icon::Dimension => {
            // A measured span: end ticks, the dimension line with arrows, and a label bar.
            line(&[(0.1, 0.35), (0.1, 0.85)], s);
            line(&[(0.9, 0.35), (0.9, 0.85)], s);
            line(&[(0.1, 0.6), (0.9, 0.6)], a);
            line(&[(0.22, 0.5), (0.1, 0.6), (0.22, 0.7)], a);
            line(&[(0.78, 0.5), (0.9, 0.6), (0.78, 0.7)], a);
            line(&[(0.35, 0.25), (0.65, 0.25)], s);
        }
        Icon::Trim => {
            // Two crossing lines; the piece between them (accent) is what gets removed.
            line(&[(0.3, 0.1), (0.3, 0.9)], s);
            line(&[(0.7, 0.1), (0.7, 0.9)], s);
            line(&[(0.05, 0.5), (0.3, 0.5)], s);
            line(&[(0.7, 0.5), (0.95, 0.5)], s);
            line(&[(0.4, 0.4), (0.6, 0.6)], a);
            line(&[(0.4, 0.6), (0.6, 0.4)], a);
        }
        Icon::ShapeBuilder => {
            // Two overlapping circles; the arrow (accent) drags across them to merge.
            p.circle_stroke(at(0.36, 0.52), r.width() * 0.27, s);
            p.circle_stroke(at(0.64, 0.52), r.width() * 0.27, s);
            line(&[(0.18, 0.52), (0.82, 0.52)], a);
            line(&[(0.7, 0.42), (0.82, 0.52), (0.7, 0.62)], a);
        }
        Icon::Knife => {
            // A blade (accent) on a handle, cutting a shape.
            rect(0.1, 0.55, 0.6, 0.92, s);
            line(&[(0.15, 0.95), (0.45, 0.65)], s);
            line(&[(0.45, 0.65), (0.92, 0.12), (0.62, 0.62)], a);
        }
        Icon::Eraser => {
            // A tilted eraser block; its rubber end (accent) wiping a line.
            line(&[(0.32, 0.43), (0.6, 0.15), (0.92, 0.47), (0.64, 0.75), (0.32, 0.43)], s);
            line(&[(0.32, 0.43), (0.12, 0.63), (0.44, 0.95), (0.64, 0.75)], a);
            line(&[(0.44, 0.95), (0.95, 0.95)], s);
        }
        Icon::Lock | Icon::Unlocked => {
            rect(0.18, 0.45, 0.82, 0.92, s);
            if icon == Icon::Lock {
                arc(0.5, 0.42, 0.22, PI, TAU, s);
            } else {
                arc(0.72, 0.38, 0.2, PI, TAU, s);
            }
            p.circle_filled(at(0.5, 0.68), r.width() * 0.06, accent);
        }
        Icon::Pan => {
            // an open hand: palm and four fingers, thumb out
            p.rect_stroke(Rect::from_min_max(at(0.28, 0.45), at(0.78, 0.92)), r.width() * 0.12, s, StrokeKind::Middle);
            for (x, top) in [(0.34, 0.15), (0.46, 0.08), (0.58, 0.1), (0.7, 0.2)] {
                line(&[(x, 0.5), (x, top)], s);
            }
            line(&[(0.3, 0.7), (0.12, 0.5)], s);
        }
        Icon::Polyline => {
            let v = [(0.08, 0.85), (0.3, 0.25), (0.6, 0.7), (0.92, 0.15)];
            line(&v, s);
            for (x, y) in v {
                p.rect_filled(Rect::from_center_size(at(x, y), egui::vec2(w * 2.6, w * 2.6)), 0.0, accent);
            }
        }
        Icon::Curve => {
            let v: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let t = i as f32 / 20.0;
                    (0.08 + 0.84 * t, 0.5 - 0.36 * (t * TAU).sin())
                })
                .collect();
            line(&v, s);
            p.circle_filled(at(0.08, 0.5), w * 1.6, accent);
            p.circle_filled(at(0.92, 0.5), w * 1.6, accent);
        }
        Icon::Freehand => {
            // a pencil drawing a wavy line
            let v: Vec<(f32, f32)> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    (0.05 + 0.5 * t, 0.85 - 0.08 * (t * 9.0).sin())
                })
                .collect();
            line(&v, a);
            closed(&[(0.55, 0.78), (0.9, 0.2), (0.98, 0.28), (0.63, 0.86)], s);
        }
        Icon::RoundRect => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.2), at(0.92, 0.8)), r.width() * 0.16, s, StrokeKind::Middle);
        }
        Icon::ImageEraser => {
            // a picture with an eraser rubbing a corner out
            rect(0.05, 0.1, 0.75, 0.7, s);
            line(&[(0.1, 0.62), (0.3, 0.38), (0.45, 0.52), (0.55, 0.42), (0.7, 0.6)], s);
            closed(&[(0.5, 0.75), (0.75, 0.5), (0.95, 0.7), (0.7, 0.95)], a);
        }
        Icon::Page => {
            closed(&[(0.18, 0.05), (0.62, 0.05), (0.85, 0.28), (0.85, 0.95), (0.18, 0.95)], s);
            line(&[(0.62, 0.05), (0.62, 0.28), (0.85, 0.28)], s);
            line(&[(0.32, 0.55), (0.7, 0.55)], a);
            line(&[(0.32, 0.72), (0.7, 0.72)], a);
        }
        Icon::Layers => {
            closed(&[(0.5, 0.12), (0.95, 0.35), (0.5, 0.58), (0.05, 0.35)], a);
            line(&[(0.05, 0.52), (0.5, 0.75), (0.95, 0.52)], s);
            line(&[(0.05, 0.68), (0.5, 0.91), (0.95, 0.68)], s);
        }
        Icon::History => {
            p.circle_stroke(at(0.5, 0.52), r.width() * 0.42, s);
            line(&[(0.5, 0.22), (0.5, 0.52), (0.72, 0.64)], a);
        }
        Icon::Params => {
            for (y, k) in [(0.22, 0.3), (0.5, 0.68), (0.78, 0.45)] {
                line(&[(0.08, y), (0.92, y)], s);
                p.circle_filled(at(k, y), r.width() * 0.09, accent);
            }
        }
        Icon::Transform => {
            line(&[(0.5, 0.08), (0.5, 0.92)], s);
            line(&[(0.08, 0.5), (0.92, 0.5)], s);
            arrow_head((0.5, 0.06), -PI / 2.0, a);
            arrow_head((0.5, 0.94), PI / 2.0, a);
            arrow_head((0.06, 0.5), PI, a);
            arrow_head((0.94, 0.5), 0.0, a);
        }
        Icon::Image => {
            rect(0.05, 0.12, 0.95, 0.88, s);
            line(&[(0.1, 0.8), (0.38, 0.45), (0.58, 0.68), (0.7, 0.55), (0.9, 0.8)], s);
            p.circle_filled(at(0.72, 0.32), r.width() * 0.08, accent);
        }
        Icon::Style => {
            // a filled swatch over an outline swatch
            rect(0.05, 0.05, 0.62, 0.62, s);
            fill_rect(0.38, 0.38, 0.95, 0.95, accent.gamma_multiply(0.7));
        }
        Icon::Effects => {
            // a four-point sparkle and a small one
            closed(&[(0.42, 0.08), (0.52, 0.42), (0.85, 0.52), (0.52, 0.62), (0.42, 0.95), (0.32, 0.62), (0.02, 0.52), (0.32, 0.42)], s);
            closed(&[(0.8, 0.06), (0.84, 0.18), (0.96, 0.22), (0.84, 0.26), (0.8, 0.38), (0.76, 0.26), (0.64, 0.22), (0.76, 0.18)], a);
        }
        Icon::Art => {
            p.circle_stroke(at(0.3, 0.3), r.width() * 0.22, s);
            closed(&[(0.72, 0.08), (0.95, 0.5), (0.5, 0.5)], a);
            rect(0.12, 0.6, 0.48, 0.95, s);
            closed(&[(0.55, 0.95), (0.95, 0.95), (0.95, 0.6), (0.55, 0.6)], a);
        }
        Icon::More => {
            for x in [0.2, 0.5, 0.8] {
                p.circle_filled(at(x, 0.5), r.width() * 0.08, ink);
            }
        }
        Icon::Make => {
            // a blade cutting along a dashed line
            for i in 0..4 {
                let x = 0.05 + i as f32 * 0.17;
                line(&[(x, 0.85), (x + 0.1, 0.85)], a);
            }
            closed(&[(0.55, 0.85), (0.95, 0.15), (0.98, 0.35), (0.68, 0.85)], s);
            line(&[(0.75, 0.5), (0.9, 0.58)], s);
        }
        Icon::Fonts => {
            // "Aa"
            line(&[(0.02, 0.9), (0.28, 0.12), (0.54, 0.9)], Stroke::new(w * 1.3, ink));
            line(&[(0.12, 0.62), (0.44, 0.62)], Stroke::new(w * 1.3, ink));
            p.circle_stroke(at(0.76, 0.7), r.width() * 0.17, a);
            line(&[(0.93, 0.5), (0.93, 0.9)], a);
        }
        Icon::Mail => {
            rect(0.05, 0.2, 0.95, 0.8, s);
            line(&[(0.05, 0.2), (0.5, 0.55), (0.95, 0.2)], a);
        }
        Icon::PrintCut => {
            // a printed page with registration marks in its corners
            rect(0.15, 0.08, 0.85, 0.92, s);
            for (x, y, dx, dy) in [(0.25, 0.18, 1.0, 1.0), (0.75, 0.18, -1.0, 1.0), (0.25, 0.82, 1.0, -1.0)] {
                line(&[(x + dx * 0.14, y), (x, y), (x, y + dy * 0.14)], a);
            }
            p.circle_stroke(at(0.55, 0.55), r.width() * 0.14, s);
        }
        Icon::Script => {
            line(&[(0.3, 0.2), (0.06, 0.5), (0.3, 0.8)], s);
            line(&[(0.7, 0.2), (0.94, 0.5), (0.7, 0.8)], s);
            line(&[(0.58, 0.12), (0.42, 0.88)], a);
        }
        Icon::Dieline => {
            // a box net: centre panel with four flaps
            rect(0.32, 0.32, 0.68, 0.68, s);
            closed(&[(0.32, 0.32), (0.4, 0.06), (0.6, 0.06), (0.68, 0.32)], s);
            closed(&[(0.32, 0.68), (0.4, 0.94), (0.6, 0.94), (0.68, 0.68)], s);
            closed(&[(0.32, 0.32), (0.06, 0.4), (0.06, 0.6), (0.32, 0.68)], a);
            closed(&[(0.68, 0.32), (0.94, 0.4), (0.94, 0.6), (0.68, 0.68)], a);
        }
        Icon::PathTools => {
            // a pen nib
            closed(&[(0.5, 0.95), (0.18, 0.45), (0.38, 0.08), (0.62, 0.08), (0.82, 0.45)], s);
            line(&[(0.5, 0.95), (0.5, 0.5)], s);
            p.circle_filled(at(0.5, 0.45), r.width() * 0.07, accent);
        }
        Icon::Warp => {
            for i in 0..3 {
                let y = 0.2 + i as f32 * 0.3;
                let v: Vec<(f32, f32)> = (0..=12)
                    .map(|k| {
                        let t = k as f32 / 12.0;
                        (0.05 + 0.9 * t, y + 0.1 * (t * PI).sin())
                    })
                    .collect();
                line(&v, if i == 1 { a } else { s });
            }
        }
    }
}
