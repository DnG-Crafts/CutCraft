//! Turn a page into an ordered, per-tool cut job.

use crate::doc::{CutStyle, Page, Rgba, ShapeKind};
use crate::geom::{self, Polyline};
use crate::materials::{Action, CutSettings};
use kurbo::{Point, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupMode {
    /// Everything with a cut style uses one setting.
    Simple,
    /// One setting per line colour.
    ByColor,
    /// One setting per layer.
    ByLayer,
    /// One setting per fill colour.
    ByFill,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobGroupSpec {
    /// Colour hex or layer id as string ("" in simple mode).
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub action: Action,
    pub settings: CutSettings,
    pub material: String,
    /// Carriage 1 or 2.
    pub slot: u8,
    /// Cut mirror-image (iron-on / heat-transfer vinyl is cut from the back).
    #[serde(default)]
    pub mirror: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobPlan {
    pub mode: GroupMode,
    pub groups: Vec<JobGroupSpec>,
    pub copies: u32,
    /// Only cut shapes within the page / media.
    pub clip_to_page: bool,
}

impl Default for JobPlan {
    fn default() -> Self {
        JobPlan {
            mode: GroupMode::Simple,
            groups: vec![default_group("", "All cut lines")],
            copies: 1,
            clip_to_page: true,
        }
    }
}

pub fn default_group(key: &str, label: &str) -> JobGroupSpec {
    let mat = crate::materials::builtin()[1].clone();
    JobGroupSpec {
        key: key.into(),
        label: label.into(),
        enabled: true,
        action: Action::Cut,
        settings: mat.settings,
        material: mat.name,
        slot: 1,
        mirror: false,
    }
}

/// The key of a fill colour group ("none" for unfilled shapes).
pub fn fill_key(c: Option<Rgba>) -> String {
    stroke_key(c)
}

fn stroke_key(c: Option<Rgba>) -> String {
    c.map(|c| c.hex()).unwrap_or_else(|| "none".into())
}

/// Recompute the list of groups for the current page, preserving existing settings.
pub fn sync_plan(plan: &mut JobPlan, page: &Page) {
    let mut keys: Vec<(String, String)> = vec![];
    match plan.mode {
        GroupMode::Simple => keys.push((String::new(), "All cut lines".into())),
        GroupMode::ByColor => {
            for s in &page.shapes {
                for l in s.leaves() {
                    if l.style.cut == CutStyle::NoCut || l.is_image() {
                        continue;
                    }
                    let k = stroke_key(l.style.stroke);
                    if !keys.iter().any(|x| x.0 == k) {
                        let label = if l.style.stroke.is_some_and(|c| c.is_transparent()) { "Transparent line".into() } else { format!("Line colour {k}") };
                        keys.push((k.clone(), label));
                    }
                }
            }
        }
        GroupMode::ByLayer => {
            for l in &page.layers {
                keys.push((l.id.to_string(), l.name.clone()));
            }
        }
        GroupMode::ByFill => {
            for s in &page.shapes {
                for l in s.leaves() {
                    if l.style.cut == CutStyle::NoCut || l.is_image() {
                        continue;
                    }
                    let k = fill_key(l.style.fill);
                    if !keys.iter().any(|x| x.0 == k) {
                        let label = match l.style.fill {
                            Some(c) if c.is_transparent() => "Transparent fill".into(),
                            Some(_) => format!("Fill colour {k}"),
                            None => "No fill".into(),
                        };
                        keys.push((k, label));
                    }
                }
            }
        }
    }
    let old = std::mem::take(&mut plan.groups);
    plan.groups = keys
        .into_iter()
        .map(|(k, label)| {
            old.iter()
                .find(|g| g.key == k)
                .cloned()
                .map(|mut g| {
                    g.label = label.clone();
                    g
                })
                .unwrap_or_else(|| default_group(&k, &label))
        })
        .collect();
}

#[derive(Clone, Debug)]
pub struct JobPass {
    pub label: String,
    pub action: Action,
    pub settings: CutSettings,
    pub slot: u8,
    /// Ordered toolpaths in page coordinates (mm).
    pub paths: Vec<Polyline>,
}

#[derive(Clone, Debug, Default)]
pub struct CutJob {
    pub passes: Vec<JobPass>,
}

#[derive(Clone, Debug, Default)]
pub struct JobStats {
    pub cut_mm: f64,
    pub travel_mm: f64,
    pub lifts: usize,
    pub seconds: f64,
}

impl CutJob {
    pub fn stats(&self) -> JobStats {
        let mut st = JobStats::default();
        let mut pos = Point::ZERO;
        for p in &self.passes {
            let v = p.settings.speed.max(1) as f64 * 10.0; // mm/s
            for pl in &p.paths {
                if let Some(f) = pl.pts.first() {
                    let t = pos.distance(*f);
                    st.travel_mm += t;
                    st.seconds += t / (v * 2.0) + 0.15;
                    st.lifts += 1;
                }
                let l = pl.length();
                st.cut_mm += l;
                st.seconds += l / v;
                pos = if pl.closed { pl.pts[0] } else { *pl.pts.last().unwrap_or(&pos) };
            }
        }
        st
    }

    pub fn is_empty(&self) -> bool {
        self.passes.iter().all(|p| p.paths.is_empty())
    }

    /// Bounding box of all toolpaths (page mm).
    pub fn bbox(&self) -> Option<kurbo::Rect> {
        let mut r: Option<kurbo::Rect> = None;
        for p in &self.passes {
            for pl in &p.paths {
                for pt in &pl.pts {
                    r = Some(match r {
                        None => kurbo::Rect::new(pt.x, pt.y, pt.x, pt.y),
                        Some(b) => b.union_pt(*pt),
                    });
                }
            }
        }
        r
    }

    /// A material + labour cost estimate for this job.
    pub fn estimate(&self, p: &CostParams) -> CostEstimate {
        let st = self.stats();
        let bb = self.bbox();
        let used_area_mm2 = bb.map(|b| b.width() * b.height()).unwrap_or(0.0);
        let waste = p.waste_factor.max(1.0);
        let material_cost = used_area_mm2 / 1_000_000.0 * waste * p.material_price_per_m2;
        let labor_cost = st.seconds / 3600.0 * p.labor_per_hour;
        let total = material_cost + labor_cost;
        let pieces = p.pieces.max(1);
        CostEstimate {
            cut_mm: st.cut_mm,
            seconds: st.seconds,
            used_area_mm2,
            material_cost,
            labor_cost,
            total,
            per_piece: total / pieces as f64,
        }
    }
}

/// Inputs for a job cost estimate.
#[derive(Clone, Copy, Debug)]
pub struct CostParams {
    /// Material price per square metre.
    pub material_price_per_m2: f64,
    /// Labour / machine-time rate per hour.
    pub labor_per_hour: f64,
    /// Number of finished pieces this job makes (for a per-piece figure).
    pub pieces: usize,
    /// Waste multiplier for off-cuts and margins (1.0 = none, 1.15 = +15%).
    pub waste_factor: f64,
}

impl Default for CostParams {
    fn default() -> Self {
        CostParams { material_price_per_m2: 20.0, labor_per_hour: 0.0, pieces: 1, waste_factor: 1.15 }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CostEstimate {
    pub cut_mm: f64,
    pub seconds: f64,
    pub used_area_mm2: f64,
    pub material_cost: f64,
    pub labor_cost: f64,
    pub total: f64,
    pub per_piece: f64,
}

/// Splits a job wherever a carriage needs a different tool than the one it
/// holds, so the cutter can pause for the swap (a pen, then the blade, …).
/// Each part comes with the tool and carriage to load before it — None for
/// the first part (whatever is loaded is used). `slots` is how many tool
/// carriages the machine has; with one, every pass uses it.
pub fn split_at_tool_changes(job: &CutJob, slots: u8) -> Vec<(CutJob, Option<(crate::materials::Tool, u8)>)> {
    let mut out: Vec<(CutJob, Option<(crate::materials::Tool, u8)>)> = vec![];
    let mut loaded: std::collections::HashMap<u8, crate::materials::Tool> = Default::default();
    for pass in job.passes.iter().filter(|p| !p.paths.is_empty()) {
        let slot = if slots > 1 { pass.slot } else { 1 };
        let tool = pass.settings.tool;
        let change = match loaded.get(&slot) {
            Some(t) => *t != tool,
            None => false,
        };
        loaded.insert(slot, tool);
        if change || out.is_empty() {
            out.push((CutJob::default(), if out.is_empty() { None } else { Some((tool, slot)) }));
        }
        out.last_mut().unwrap().0.passes.push(pass.clone());
    }
    out
}

/// Whether `leaf` (of a top-level shape on `layer`) is cut by group `g`.
pub fn leaf_in_group(plan: &JobPlan, g: &JobGroupSpec, layer: u32, leaf: &crate::doc::Shape) -> bool {
    match plan.mode {
        GroupMode::Simple => true,
        GroupMode::ByColor => stroke_key(leaf.style.stroke) == g.key,
        GroupMode::ByLayer => layer.to_string() == g.key,
        GroupMode::ByFill => fill_key(leaf.style.fill) == g.key,
    }
}

/// Generate a job for one page.
pub fn generate(page: &Page, plan: &JobPlan) -> CutJob {
    let mut job = CutJob::default();
    let page_rect = geom::rect_path(page.rect());
    for g in &plan.groups {
        if !g.enabled || g.action == Action::None {
            continue;
        }
        let mut pls: Vec<Polyline> = vec![];
        for s in &page.shapes {
            if !page.is_visible(s) {
                continue;
            }
            for leaf in s.leaves() {
                if leaf.hidden || matches!(leaf.kind, ShapeKind::Image(_)) || leaf.style.cut == CutStyle::NoCut {
                    continue;
                }
                if !leaf_in_group(plan, g, s.layer, &leaf) {
                    continue;
                }
                let paths = leaf.cut_paths();
                let mut flat = geom::flatten_all(&paths, 0.05);
                if plan.clip_to_page {
                    let clipped = clip_polylines(&flat, &page_rect);
                    flat = clipped;
                }
                if g.mirror {
                    // flip across the media, as seen from the back
                    for pl in &mut flat {
                        for p in &mut pl.pts {
                            p.x = page.width - p.x;
                        }
                    }
                }
                match (leaf.style.cut, g.action) {
                    (CutStyle::Perforate { on, off }, _) => flat = geom::dash(&flat, on, off),
                    (_, Action::Perforate) => flat = geom::dash(&flat, 3.0, 1.0),
                    _ => {}
                }
                pls.extend(flat);
            }
        }
        if pls.is_empty() {
            continue;
        }
        if g.action == Action::Stipple {
            // every small closed shape → one plunge at its centre
            pls = pls
                .into_iter()
                .filter(|p| p.closed)
                .map(|p| {
                    let c = p.bbox().center();
                    Polyline { pts: vec![c, c], closed: false }
                })
                .collect();
        }
        let mut ordered = sort_inside_first(pls);
        if g.settings.overcut > 0.0 {
            ordered = ordered.into_iter().map(|p| overcut(p, g.settings.overcut)).collect();
        }
        if g.settings.blade_compensation && g.settings.tool.is_blade() {
            let o = g.settings.tool.blade_offset();
            ordered = ordered.into_iter().map(|p| blade_compensate(&p, o)).collect();
        }
        let mut all = vec![];
        for _ in 0..plan.copies.max(1) {
            for _ in 0..g.settings.passes.max(1) {
                all.extend(ordered.iter().cloned());
            }
        }
        job.passes.push(JobPass { label: g.label.clone(), action: g.action, settings: g.settings.clone(), slot: g.slot, paths: all });
    }
    job
}

fn clip_polylines(pls: &[Polyline], region: &kurbo::BezPath) -> Vec<Polyline> {
    let bb = kurbo::Shape::bounding_box(region);
    let (inside, outside): (Vec<&Polyline>, Vec<&Polyline>) =
        pls.iter().partition(|p| p.pts.iter().all(|q| bb.contains(*q)));
    let mut out: Vec<Polyline> = inside.into_iter().cloned().collect();
    if !outside.is_empty() {
        let open: Vec<kurbo::BezPath> = outside
            .iter()
            .map(|p| {
                let mut pts = p.pts.clone();
                if p.closed {
                    pts.push(pts[0]);
                }
                geom::polyline_path(&pts, false)
            })
            .collect();
        let c = geom::clip_lines(&open, &[region.clone()], true);
        out.extend(geom::flatten(&c, 0.05));
    }
    out
}

/// Greedy nearest-neighbour ordering that never cuts a contour before the
/// contours inside it (so parts don't shift before their holes are cut).
pub fn sort_inside_first(pls: Vec<Polyline>) -> Vec<Polyline> {
    let n = pls.len();
    let bbs: Vec<kurbo::Rect> = pls.iter().map(|p| p.bbox()).collect();
    // inner[j] = list of contours inside j
    let mut blockers = vec![0usize; n];
    let mut inside_of: Vec<Vec<usize>> = vec![vec![]; n];
    for i in 0..n {
        for j in 0..n {
            if i == j || !pls[j].closed {
                continue;
            }
            let (a, b) = (bbs[i], bbs[j]);
            let strictly = a.x0 >= b.x0 && a.y0 >= b.y0 && a.x1 <= b.x1 && a.y1 <= b.y1 && a.area() < b.area();
            if strictly && point_in(&pls[j].pts, pls[i].pts[0]) {
                blockers[j] += 1;
                inside_of[i].push(j);
            }
        }
    }
    let mut done = vec![false; n];
    let mut out = Vec::with_capacity(n);
    let mut pos = Point::ZERO;
    for _ in 0..n {
        let mut best: Option<(f64, usize, bool)> = None;
        for i in 0..n {
            if done[i] || blockers[i] > 0 {
                continue;
            }
            let d0 = pos.distance(pls[i].pts[0]);
            let rev = !pls[i].closed && pos.distance(*pls[i].pts.last().unwrap()) < d0;
            let d = if rev { pos.distance(*pls[i].pts.last().unwrap()) } else { d0 };
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i, rev));
            }
        }
        let Some((_, i, rev)) = best else { break };
        done[i] = true;
        for &j in &inside_of[i] {
            blockers[j] -= 1;
        }
        let mut p = pls[i].clone();
        if rev {
            p.pts.reverse();
        }
        pos = if p.closed { p.pts[0] } else { *p.pts.last().unwrap() };
        out.push(p);
    }
    out
}

fn point_in(poly: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// Close a contour and continue past the start point by `len` mm.
pub fn overcut(mut p: Polyline, len: f64) -> Polyline {
    if !p.closed || p.pts.len() < 2 {
        return p;
    }
    let mut pts = p.pts.clone();
    pts.push(p.pts[0]);
    let mut rem = len;
    for w in p.pts.windows(2) {
        let d = w[0].distance(w[1]);
        if d >= rem {
            pts.push(w[0].lerp(w[1], rem / d));
            break;
        }
        pts.push(w[1]);
        rem -= d;
    }
    p.pts = pts;
    p.closed = false;
    p
}

/// Swivel (drag) knife compensation: move the holder ahead of the tip by `o`
/// and sweep arcs around corners.
pub fn blade_compensate(p: &Polyline, o: f64) -> Polyline {
    let mut pts = p.pts.clone();
    if p.closed && !pts.is_empty() {
        pts.push(pts[0]);
        if pts.len() > 2 {
            pts.push(pts[1]);
        }
    }
    if pts.len() < 2 || o <= 0.0 {
        return p.clone();
    }
    let dir = |a: Point, b: Point| -> Vec2 {
        let d = b - a;
        let l = d.hypot();
        if l < 1e-9 { Vec2::ZERO } else { d / l }
    };
    let mut out = vec![];
    let mut prev_d = dir(pts[0], pts[1]);
    out.push(pts[0] + prev_d * o);
    for i in 1..pts.len() {
        let d = if i + 1 < pts.len() { dir(pts[i], pts[i + 1]) } else { prev_d };
        out.push(pts[i] + prev_d * o);
        let a0 = prev_d.y.atan2(prev_d.x);
        let mut da = d.y.atan2(d.x) - a0;
        while da > std::f64::consts::PI {
            da -= std::f64::consts::TAU;
        }
        while da < -std::f64::consts::PI {
            da += std::f64::consts::TAU;
        }
        if da.abs() > 0.2 {
            let steps = (da.abs() / 0.2).ceil() as usize;
            for s in 1..=steps {
                let a = a0 + da * s as f64 / steps as f64;
                out.push(pts[i] + Vec2::new(a.cos(), a.sin()) * o);
            }
        }
        if d != Vec2::ZERO {
            prev_d = d;
        }
    }
    Polyline { pts: out, closed: false }
}

#[cfg(test)]
mod tests {
    #[test]
    fn groups_by_fill_colour() {
        use crate::doc::{Rgba, Style};
        let mut d = crate::doc::Document::default();
        let fill = |c: Option<Rgba>| Style { fill: c, ..Style::default() };
        let rect = |x: f64| crate::geom::rect_path(kurbo::Rect::new(x, 10.0, x + 10.0, 20.0));
        d.add_path(rect(0.0), fill(Some(Rgba::RED)), 0);
        d.add_path(rect(20.0), fill(Some(Rgba::RED)), 0);
        d.add_path(rect(40.0), fill(Some(Rgba(30, 30, 220, 255))), 0);
        let mut plan = JobPlan { mode: GroupMode::ByFill, ..JobPlan::default() };
        sync_plan(&mut plan, d.page());
        assert_eq!(plan.groups.len(), 2, "{:?}", plan.groups.iter().map(|g| &g.label).collect::<Vec<_>>());
        let job = generate(d.page(), &plan);
        let counts: Vec<usize> = job.passes.iter().map(|p| p.paths.len()).collect();
        assert_eq!(counts, [2, 1]);
    }

    #[test]
    fn tool_changes_split_the_job() {
        use crate::materials::Tool;
        let pass = |tool: Tool, slot: u8| JobPass { label: String::new(), action: Action::Cut, settings: CutSettings { tool, ..crate::materials::builtin()[0].settings.clone() }, slot, paths: vec![Polyline { pts: vec![kurbo::Point::ZERO, kurbo::Point::new(1.0, 0.0)], closed: false }] };
        let job = CutJob { passes: vec![pass(Tool::SketchPen, 1), pass(Tool::AutoBlade, 1), pass(Tool::AutoBlade, 1)] };
        let parts = split_at_tool_changes(&job, 1);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].1, Some((Tool::AutoBlade, 1)));
        assert_eq!(parts[1].0.passes.len(), 2);
        // a pen in carriage 1 and a blade in carriage 2 need no swap
        let job = CutJob { passes: vec![pass(Tool::SketchPen, 1), pass(Tool::AutoBlade, 2)] };
        assert_eq!(split_at_tool_changes(&job, 2).len(), 1);
        assert_eq!(split_at_tool_changes(&job, 1).len(), 2, "one carriage: swap");
    }

    #[test]
    fn mirrored_groups_cut_flipped_across_the_media() {
        let mut d = crate::doc::Document::default();
        d.add_path(crate::geom::rect_path(kurbo::Rect::new(10.0, 10.0, 30.0, 20.0)), Default::default(), 0);
        let mut plan = JobPlan::default();
        sync_plan(&mut plan, d.page());
        let xs = |plan: &JobPlan| {
            let job = generate(d.page(), plan);
            let pts: Vec<f64> = job.passes[0].paths.iter().flat_map(|p| p.pts.iter().map(|q| q.x)).collect();
            (pts.iter().copied().fold(f64::MAX, f64::min), pts.iter().copied().fold(f64::MIN, f64::max))
        };
        let (a0, a1) = xs(&plan);
        plan.groups[0].mirror = true;
        let (b0, b1) = xs(&plan);
        let w = d.page().width;
        assert!((b0 - (w - a1)).abs() < 1e-6 && (b1 - (w - a0)).abs() < 1e-6, "{a0} {a1} -> {b0} {b1}");
    }

    use super::*;
    use crate::doc::{Document, Style};
    use kurbo::Rect;

    #[test]
    fn hole_is_cut_before_outline() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(10.0, 10.0, 100.0, 100.0)), Style::default(), 0);
        d.add_path(geom::rect_path(Rect::new(40.0, 40.0, 60.0, 60.0)), Style::default(), 0);
        let job = generate(d.page(), &JobPlan::default());
        assert_eq!(job.passes.len(), 1);
        let p = &job.passes[0].paths;
        assert_eq!(p.len(), 2);
        assert!(p[0].bbox().width() < 30.0, "inner contour first");
        assert!(job.stats().cut_mm > 400.0);
    }

    #[test]
    fn cost_estimate_scales_with_area_and_time() {
        let mut d = Document::default();
        d.add_path(geom::rect_path(Rect::new(0.0, 0.0, 100.0, 100.0)), Style::default(), 0); // 0.01 m²
        let job = generate(d.page(), &JobPlan::default());
        let e = job.estimate(&CostParams { material_price_per_m2: 20.0, labor_per_hour: 60.0, pieces: 2, waste_factor: 1.0 });
        // 0.01 m² at $20/m² = $0.20 material
        assert!((e.material_cost - 0.20).abs() < 1e-3, "material {}", e.material_cost);
        assert!(e.labor_cost > 0.0 && e.total > e.material_cost);
        assert!((e.per_piece - e.total / 2.0).abs() < 1e-9);
    }

    #[test]
    fn overcut_extends() {
        let p = Polyline { pts: vec![Point::new(0., 0.), Point::new(10., 0.), Point::new(10., 10.)], closed: true };
        let o = overcut(p, 2.0);
        assert!(!o.closed);
        assert_eq!(*o.pts.last().unwrap(), Point::new(2.0, 0.0));
    }
}
