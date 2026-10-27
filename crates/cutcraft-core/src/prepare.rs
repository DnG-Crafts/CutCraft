//! The Prepare ("Make it") step, as in Cricut Design Space: the design split
//! onto mats — one or more per material group (simple / by line colour / by
//! layer) — with copies laid out, shapes movable or hidden per mat, and each
//! mat cut on its own.
//!
//! A mat holds items: a top-level shape (one copy of it) and where it sits on
//! that mat. Cutting a mat builds a temporary page with just those items in
//! place and runs the normal cut job for the mat's group, so mirroring,
//! perforation, cut order and the Brother export all work unchanged.

use crate::cutjob::{self, JobPlan};
use crate::doc::{CutStyle, Document, Page, Shape, ShapeId};
use crate::materials::Action;
use kurbo::{Affine, Rect, Vec2};

/// One copy of a top-level shape on a mat, moved by `offset` from where it
/// sits in the design.
#[derive(Clone, Debug, PartialEq)]
pub struct MatItem {
    pub shape: ShapeId,
    /// Which copy (0 = the original).
    pub copy: u32,
    pub offset: Vec2,
    /// Left out of this mat's cut.
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    /// The design page this mat comes from (its media size and cutting mat).
    pub page: usize,
    /// Index into the plan's groups: the material this mat is cut with.
    pub group: usize,
    pub items: Vec<MatItem>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prepared {
    pub mats: Vec<Mat>,
}

/// Gap kept between packed shapes and around the media edge, mm.
pub const SPACING: f64 = 5.0;

/// The top-level shapes with something cut by group `gi`.
pub fn group_shapes(page: &Page, plan: &JobPlan, gi: usize) -> Vec<ShapeId> {
    let Some(g) = plan.groups.get(gi) else { return vec![] };
    page.shapes
        .iter()
        .filter(|s| page.is_visible(s))
        .filter(|s| {
            s.leaves().iter().any(|leaf| !leaf.hidden && !leaf.is_image() && leaf.style.cut != CutStyle::NoCut && cutjob::leaf_in_group(plan, g, s.layer, leaf))
        })
        .map(|s| s.id)
        .collect()
}

/// The part of shape `s` that group `gi` cuts, as a box (page mm): a
/// grouped design split by colour or layer is packed by the pieces each mat
/// really cuts, not the whole group. Falls back to the whole shape.
pub fn group_bounds(s: &Shape, plan: &JobPlan, gi: usize) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    if let Some(g) = plan.groups.get(gi) {
        for leaf in s.leaves() {
            if leaf.hidden || leaf.is_image() || leaf.style.cut == CutStyle::NoCut || !cutjob::leaf_in_group(plan, g, s.layer, &leaf) {
                continue;
            }
            for p in leaf.cut_paths() {
                r = crate::geom::union_rect(r, kurbo::Shape::bounding_box(&p));
            }
        }
    }
    r.or_else(|| s.bbox())
}

/// Groups the cut plan over every page (line colours / layers of all pages).
pub fn sync_plan_all(plan: &mut JobPlan, doc: &Document) {
    let mut all = doc.page().clone();
    all.shapes = doc.pages.iter().flat_map(|p| p.shapes.iter().filter(|s| p.is_visible(s)).cloned()).collect();
    for p in &doc.pages {
        for l in &p.layers {
            if !all.layers.iter().any(|x| x.id == l.id) {
                all.layers.push(l.clone());
            }
        }
    }
    cutjob::sync_plan(plan, &all);
}

/// A top-level shape by id, on whichever page it is.
pub fn find_shape(doc: &Document, id: ShapeId) -> Option<&Shape> {
    doc.pages.iter().find_map(|p| p.shape(id))
}

/// Splits every page of the design onto mats, material by material. With
/// `pack` false and one copy, each page keeps its layout on one mat per
/// material (what you see is what's cut); otherwise each page's shapes
/// (× copies) are packed in rows onto as many mats as they need.
pub fn prepare(doc: &Document, plan: &JobPlan, pack: bool) -> Prepared {
    let mut mats = vec![];
    for gi in 0..plan.groups.len() {
        for pi in 0..doc.pages.len() {
            mats.extend(prepare_page(&doc.pages[pi], pi, plan, gi, pack));
        }
    }
    Prepared { mats }
}

fn prepare_page(page: &Page, pi: usize, plan: &JobPlan, gi: usize, pack: bool) -> Vec<Mat> {
    let copies = plan.copies.max(1) * page.copies.max(1);
    let area = page.rect().inset(-SPACING);
    let mut mats = vec![];
    {
        let g = &plan.groups[gi];
        if !g.enabled || g.action == Action::None {
            return mats;
        }
        let ids = group_shapes(page, plan, gi);
        if ids.is_empty() {
            return mats;
        }
        if !pack && copies == 1 {
            mats.push(Mat { page: pi, group: gi, items: ids.iter().map(|&shape| MatItem { shape, copy: 0, offset: Vec2::ZERO, hidden: false }).collect() });
            return mats;
        }
        let todo: Vec<(ShapeId, u32, Rect, bool)> = ids
            .iter()
            .filter_map(|&id| page.shape(id).and_then(|s| group_bounds(s, plan, gi)).map(|b| (id, b)))
            .flat_map(|(id, b)| (0..copies).map(move |c| (id, c, b, false)))
            .collect();
        mats.extend(pack_items(todo, area).into_iter().map(|items| Mat { page: pi, group: gi, items }));
    }
    mats
}

/// Packs pieces (shape, copy, its box in the design, hidden) in rows
/// ("shelves"), tallest first, filling one mat's `area` after another.
fn pack_items(mut todo: Vec<(ShapeId, u32, Rect, bool)>, area: Rect) -> Vec<Vec<MatItem>> {
    todo.sort_by(|a, b| b.2.height().total_cmp(&a.2.height()).then(b.2.width().total_cmp(&a.2.width())));
    let mut mats = vec![];
    let mut mat: Vec<MatItem> = vec![];
    let (mut x, mut y, mut row_h) = (area.x0, area.y0, 0.0f64);
    for (shape, copy, b, hidden) in todo {
        if x > area.x0 && x + b.width() > area.x1 {
            // next row
            x = area.x0;
            y += row_h + SPACING;
            row_h = 0.0;
        }
        if !mat.is_empty() && y + b.height() > area.y1 {
            // next mat
            mats.push(std::mem::take(&mut mat));
            (x, y, row_h) = (area.x0, area.y0, 0.0);
        }
        mat.push(MatItem { shape, copy, offset: Vec2::new(x - b.x0, y - b.y0), hidden });
        x += b.width() + SPACING;
        row_h = row_h.max(b.height());
    }
    if !mat.is_empty() {
        mats.push(mat);
    }
    mats
}

/// Packs just mat `mi` tightly. Pieces that don't fit go onto new mats of
/// the same material right after it; returns how many mats were added.
pub fn pack_mat(doc: &Document, plan: &JobPlan, prep: &mut Prepared, mi: usize) -> usize {
    let Some(m) = prep.mats.get(mi) else { return 0 };
    let Some(page) = doc.pages.get(m.page) else { return 0 };
    let area = page.rect().inset(-SPACING);
    let todo: Vec<(ShapeId, u32, Rect, bool)> = m
        .items
        .iter()
        .filter_map(|it| find_shape(doc, it.shape).and_then(|s| group_bounds(s, plan, m.group)).map(|b| (it.shape, it.copy, b, it.hidden)))
        .collect();
    let (page_i, group) = (m.page, m.group);
    let mut packed = pack_items(todo, area).into_iter();
    let Some(first) = packed.next() else { return 0 };
    prep.mats[mi].items = first;
    let extra: Vec<Mat> = packed.map(|items| Mat { page: page_i, group, items }).collect();
    let n = extra.len();
    for (k, m) in extra.into_iter().enumerate() {
        prep.mats.insert(mi + 1 + k, m);
    }
    n
}

/// Puts mat `mi`'s pieces back where they are in the design.
pub fn design_layout_mat(prep: &mut Prepared, mi: usize) {
    if let Some(m) = prep.mats.get_mut(mi) {
        for it in &mut m.items {
            it.offset = Vec2::ZERO;
        }
    }
}

/// Where an item is on mat `mat`: the part of its shape that mat cuts,
/// moved by the item's offset.
pub fn item_bounds(doc: &Document, plan: &JobPlan, mat: &Mat, item: &MatItem) -> Option<Rect> {
    find_shape(doc, item.shape).and_then(|s| group_bounds(s, plan, mat.group)).map(|b| b + item.offset)
}

/// A page holding only this mat's (visible) items, each where it sits on the
/// mat (with its page's media and cutting mat) - cut it with [`mat_plan`].
pub fn mat_page(doc: &Document, mat: &Mat) -> Page {
    let mut p = doc.pages.get(mat.page).unwrap_or(doc.page()).clone();
    p.shapes = mat
        .items
        .iter()
        .filter(|it| !it.hidden)
        .filter_map(|it| {
            find_shape(doc, it.shape).map(|s| {
                let mut s = s.clone();
                s.transform = Affine::translate(it.offset) * s.transform;
                s
            })
        })
        .collect();
    p
}

/// The cut plan for one mat: only its group, once (copies are on the mats).
pub fn mat_plan(plan: &JobPlan, mat: &Mat) -> JobPlan {
    let mut p = plan.clone();
    p.copies = 1;
    for (i, g) in p.groups.iter_mut().enumerate() {
        g.enabled = i == mat.group;
    }
    p
}

/// Moves an item to another mat (keeping its place on the media).
pub fn move_item(prep: &mut Prepared, from: usize, item: usize, to: usize) -> bool {
    if from == to || from >= prep.mats.len() || to >= prep.mats.len() || item >= prep.mats[from].items.len() {
        return false;
    }
    let it = prep.mats[from].items.remove(item);
    prep.mats[to].items.push(it);
    true
}

/// Moves several items of mat `from` (by index) onto mat `to`.
pub fn move_items(prep: &mut Prepared, from: usize, items: &[usize], to: usize) {
    let mut idx: Vec<usize> = items.to_vec();
    idx.sort_unstable();
    idx.dedup();
    // from the back, so the earlier indices stay right
    for i in idx.into_iter().rev() {
        move_item(prep, from, i, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutjob::{GroupMode, sync_plan};
    use crate::doc::{Document, Rgba, Style};
    use crate::geom;

    fn doc_with(rects: &[(Rect, Rgba)]) -> Document {
        let mut d = Document::default();
        for (r, c) in rects {
            d.add_path(geom::rect_path(*r), Style { stroke: Some(*c), ..Style::default() }, 0);
        }
        d
    }

    fn plan_for(page: &Page, mode: GroupMode) -> JobPlan {
        let mut plan = JobPlan { mode, ..JobPlan::default() };
        sync_plan(&mut plan, page);
        plan
    }

    #[test]
    fn one_mat_per_colour_keeping_the_design_layout() {
        let red = Rgba::RED;
        let blue = Rgba(0, 0, 255, 255);
        let d = doc_with(&[(Rect::new(10.0, 10.0, 50.0, 30.0), red), (Rect::new(60.0, 10.0, 90.0, 40.0), blue), (Rect::new(10.0, 60.0, 30.0, 80.0), red)]);
        let plan = plan_for(d.page(), GroupMode::ByColor);
        let prep = prepare(&d, &plan, false);
        assert_eq!(prep.mats.len(), 2);
        assert_eq!(prep.mats.iter().map(|m| m.items.len()).sum::<usize>(), 3);
        assert!(prep.mats.iter().flat_map(|m| &m.items).all(|it| it.offset == Vec2::ZERO), "kept where drawn");
        // each mat cuts only its colour
        for m in &prep.mats {
            let job = cutjob::generate(&mat_page(&d, m), &mat_plan(&plan, m));
            assert_eq!(job.passes.len(), 1);
            assert_eq!(job.passes[0].paths.len(), m.items.len());
        }
    }

    #[test]
    fn a_group_split_by_fill_packs_each_piece_by_its_own_size() {
        use crate::doc::Style;
        // one group: a big dark frame with two coloured panels inside it
        let mut d = Document::default();
        let fill = |c: Rgba| Style { fill: Some(c), ..Style::default() };
        let frame = d.add_path(geom::rect_path(Rect::new(100.0, 100.0, 250.0, 280.0)), fill(Rgba(57, 66, 64, 255)), 0);
        let blue = d.add_path(geom::rect_path(Rect::new(120.0, 120.0, 230.0, 180.0)), fill(Rgba(80, 108, 127, 255)), 0);
        let green = d.add_path(geom::rect_path(Rect::new(120.0, 210.0, 230.0, 250.0)), fill(Rgba(180, 204, 185, 255)), 0);
        crate::ops::group(&mut d, &[frame, blue, green]);
        let plan = plan_for(d.page(), GroupMode::ByFill);
        let prep = prepare(&d, &plan, true);
        assert_eq!(prep.mats.len(), 3);
        for m in &prep.mats {
            let b = item_bounds(&d, &plan, m, &m.items[0]).unwrap();
            // packed into the corner by the piece itself, not the whole group
            assert!((b.x0 - SPACING).abs() < 1e-6 && (b.y0 - SPACING).abs() < 1e-6, "{b:?}");
            assert!(b.width() <= 150.0 + 1e-6);
        }
        let sizes: Vec<(f64, f64)> = prep.mats.iter().map(|m| {
            let b = item_bounds(&d, &plan, m, &m.items[0]).unwrap();
            (b.width().round(), b.height().round())
        }).collect();
        assert!(sizes.contains(&(110.0, 60.0)) && sizes.contains(&(110.0, 40.0)), "{sizes:?}");
    }

    #[test]
    fn one_mat_can_be_packed_or_laid_out_on_its_own() {
        let d = doc_with(&[
            (Rect::new(100.0, 100.0, 160.0, 140.0), Rgba(200, 30, 30, 255)),
            (Rect::new(150.0, 200.0, 230.0, 260.0), Rgba(30, 30, 200, 255)),
        ]);
        let plan = plan_for(d.page(), GroupMode::ByColor);
        let mut prep = prepare(&d, &plan, false);
        assert_eq!(prep.mats.len(), 2);
        let before = prep.mats[0].clone();
        // pack only the second mat: its piece moves to the corner, the first stays as designed
        assert_eq!(pack_mat(&d, &plan, &mut prep, 1), 0);
        assert_eq!(prep.mats[0], before);
        let b = item_bounds(&d, &plan, &prep.mats[1], &prep.mats[1].items[0]).unwrap();
        assert!((b.x0 - SPACING).abs() < 1e-6 && (b.y0 - SPACING).abs() < 1e-6, "{b:?}");
        // and back to the design layout
        design_layout_mat(&mut prep, 1);
        let b = item_bounds(&d, &plan, &prep.mats[1], &prep.mats[1].items[0]).unwrap();
        assert!((b.x0 - 150.0).abs() < 1e-6 && (b.y0 - 200.0).abs() < 1e-6, "{b:?}");
    }

    #[test]
    fn every_page_gets_its_own_mats() {
        use crate::doc::Page;
        let mut d = doc_with(&[(Rect::new(10.0, 10.0, 50.0, 30.0), Rgba::RED)]);
        // a second page with a blue shape and a third left empty
        let mut p2 = Page::new("Page 2", 200.0, 300.0);
        p2.layers = d.page().layers.clone();
        d.pages.push(p2);
        d.pages.push(Page::new("Page 3", 304.8, 304.8));
        d.active = 1;
        d.add_path(geom::rect_path(Rect::new(20.0, 20.0, 60.0, 60.0)), Style { stroke: Some(Rgba(0, 0, 255, 255)), ..Style::default() }, 0);
        d.active = 0;
        let mut plan = JobPlan { mode: GroupMode::ByColor, ..JobPlan::default() };
        sync_plan_all(&mut plan, &d);
        assert_eq!(plan.groups.len(), 2, "colours from every page");
        let prep = prepare(&d, &plan, false);
        assert_eq!(prep.mats.len(), 2, "one per page with something on it");
        let pages: Vec<usize> = prep.mats.iter().map(|m| m.page).collect();
        assert!(pages.contains(&0) && pages.contains(&1));
        let m2 = prep.mats.iter().find(|m| m.page == 1).unwrap();
        let page = mat_page(&d, m2);
        assert_eq!((page.width, page.height), (200.0, 300.0), "the mat uses its own page's media");
        assert_eq!(page.shapes.len(), 1);
    }

    #[test]
    fn copies_pack_without_overlapping_and_overflow_onto_new_mats() {
        let d = doc_with(&[(Rect::new(100.0, 100.0, 200.0, 180.0), Rgba::RED)]);
        let mut plan = plan_for(d.page(), GroupMode::Simple);
        plan.copies = 7; // 100 x 80 pieces: 2 x 3 fit a 12 x 12 in mat
        let prep = prepare(&d, &plan, true);
        assert_eq!(prep.mats.len(), 2);
        assert_eq!((prep.mats[0].items.len(), prep.mats[1].items.len()), (6, 1));
        let area = d.page().rect();
        for m in &prep.mats {
            let boxes: Vec<Rect> = m.items.iter().map(|it| item_bounds(&d, &plan, m, it).unwrap()).collect();
            for (i, a) in boxes.iter().enumerate() {
                assert!(a.x0 >= area.x0 && a.y0 >= area.y0 && a.x1 <= area.x1 && a.y1 <= area.y1, "on the media: {a:?}");
                for b in &boxes[i + 1..] {
                    assert!(a.intersect(*b).area() <= 1e-9, "{a:?} overlaps {b:?}");
                }
            }
        }
        // a hidden item isn't cut, and items can move between mats
        let mut prep = prep;
        prep.mats[0].items[0].hidden = true;
        assert_eq!(mat_page(&d, &prep.mats[0]).shapes.len(), 5);
        assert!(move_item(&mut prep, 1, 0, 0));
        assert_eq!(prep.mats[0].items.len(), 7);
    }
}

#[cfg(test)]
mod demo {
    /// Writes a three-page design to the temp folder (for checking the Prepare screen).
    #[test]
    #[ignore = "diagnostic"]
    fn write_pages_demo() {
        use crate::doc::{Document, Page, Rgba, Style};
        use kurbo::Rect;
        let mut d = Document::default();
        for (i, (r, c)) in [(Rect::new(30.0, 30.0, 120.0, 90.0), Rgba::RED), (Rect::new(60.0, 60.0, 200.0, 150.0), Rgba(0, 0, 220, 255)), (Rect::new(100.0, 120.0, 180.0, 260.0), Rgba::RED)].into_iter().enumerate() {
            if i > 0 {
                let mut p = Page::new(&format!("Page {}", i + 1), 304.8, 304.8);
                p.layers = d.page().layers.clone();
                d.pages.push(p);
                d.active = i;
            }
            d.add_path(crate::geom::circle_path(r.center(), r.width().min(r.height()) / 2.0), Style { stroke: Some(c), ..Style::default() }, 0);
        }
        d.active = 0;
        let p = std::env::temp_dir().join("cutcraft-pages-demo.cutcraft");
        crate::io::save_native(&d, &p).unwrap();
        println!("wrote {}", p.display());
    }
}
