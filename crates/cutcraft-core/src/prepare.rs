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
