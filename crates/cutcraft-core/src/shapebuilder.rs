//! Shape Builder: overlapping shapes split into faces - every separate area,
//! by which shapes cover it - so faces can be merged into one shape or
//! deleted. Also gives Divide its pieces. Curves stay curves (linesweeper's
//! sweep works on the Béziers themselves).

use crate::doc::{Document, Shape, ShapeId};
use crate::geom;
use kurbo::{BezPath, Point, Shape as _};
use linesweeper::topology::{Contours, Topology, WindingNumber};
use std::collections::BTreeSet;

/// One face: an area covered by exactly the same shapes all over.
#[derive(Clone, Debug)]
pub struct Face {
    /// Its outline (with any holes), in page coordinates.
    pub path: BezPath,
    /// The shapes covering it, as indices into the list given to [`faces`].
    pub covers: Vec<usize>,
}

/// Winding numbers of each shape at a point: (shape, winding), sorted, no zeros.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Cover(Vec<(u32, i32)>);

impl std::ops::Add for Cover {
    type Output = Cover;
    fn add(mut self, rhs: Cover) -> Cover {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Cover {
    fn add_assign(&mut self, rhs: Cover) {
        for (tag, w) in rhs.0 {
            match self.0.binary_search_by_key(&tag, |e| e.0) {
                Ok(i) => {
                    self.0[i].1 += w;
                    if self.0[i].1 == 0 {
                        self.0.remove(i);
                    }
                }
                Err(i) => self.0.insert(i, (tag, w)),
            }
        }
    }
}

impl WindingNumber for Cover {
    type Tag = u32;
    fn single(tag: u32, positive: bool) -> Cover {
        Cover(vec![(tag, if positive { 1 } else { -1 })])
    }
    fn of_tag(&self, tag: u32) -> Cover {
        Cover(self.0.iter().copied().filter(|e| e.0 == tag).collect())
    }
}

impl Cover {
    /// The shapes this point is inside (even-odd within each shape).
    fn inside(&self) -> Vec<usize> {
        self.0.iter().filter(|e| e.1 % 2 != 0).map(|e| e.0 as usize).collect()
    }
}

/// The closed subpaths of `path` (open lines can't bound a face).
fn closed(path: &BezPath) -> BezPath {
    geom::closed_subpaths(path)
}

/// Every face of the overlapping `shapes` (each a set of paths in page
/// coordinates, even-odd inside), largest first. Empty if they can't be
/// worked out.
pub fn faces(shapes: &[Vec<BezPath>]) -> Vec<Face> {
    let paths: Vec<BezPath> = shapes.iter().map(|ps| closed(&geom::merge_paths(ps))).collect();
    let bbox = paths.iter().filter(|p| !p.elements().is_empty()).map(|p| p.bounding_box()).reduce(|a, b| a.union(b));
    let Some(bbox) = bbox else { return vec![] };
    let m = [bbox.x0, bbox.y0, bbox.x1, bbox.y1].iter().fold(0.0f64, |a, v| a.max(v.abs()));
    if !m.is_finite() {
        return vec![];
    }
    let eps = (m * f64::EPSILON * 64.0).max(1e-6);
    let built = std::panic::catch_unwind(|| {
        let top = Topology::<Cover>::from_paths(paths.iter().enumerate().map(|(i, p)| (p, i as u32)), eps).ok()?;
        // every combination of shapes some area is inside
        let mut sets: BTreeSet<Vec<usize>> = BTreeSet::new();
        for seg in top.segment_indices() {
            let half = seg.first_half();
            sets.insert(top.winding_clockwise(half).inside());
            sets.insert(top.winding_counter_clockwise(half).inside());
        }
        let mut out = vec![];
        for set in sets.into_iter().filter(|s| !s.is_empty()) {
            for path in regions_of(&top.contours(|w| w.inside() == set)) {
                out.push(Face { path, covers: set.clone() });
            }
        }
        Some(out)
    });
    let mut out = built.ok().flatten().unwrap_or_default();
    out.retain(|f| f.path.area().abs() > 1e-9);
    out.sort_by(|a, b| b.path.area().abs().total_cmp(&a.path.area().abs()));
    out
}

/// Separate regions of a set of contours: each outline with its own holes
/// (an island inside a hole is a region of its own).
fn regions_of(c: &Contours) -> Vec<BezPath> {
    let all: Vec<_> = c.contours().collect();
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = all[i].parent {
            d += 1;
            i = p.0;
        }
        d
    };
    let mut out = vec![];
    for (i, outer) in all.iter().enumerate().filter(|(i, _)| depth(*i) % 2 == 0) {
        let mut p = outer.path.clone();
        for hole in all.iter().filter(|h| h.parent.is_some_and(|q| q.0 == i)) {
            p.extend(hole.path.elements().iter().copied());
        }
        out.push(p);
    }
    out
}

/// The separate regions of `path` (each outline with its holes), curves kept.
pub fn regions(path: &BezPath) -> Vec<BezPath> {
    let p = closed(path);
    if p.elements().is_empty() {
        return vec![];
    }
    match std::panic::catch_unwind(|| linesweeper::binary_op(&p, &BezPath::new(), linesweeper::FillRule::EvenOdd, linesweeper::BinaryOp::Union)) {
        Ok(Ok(c)) => regions_of(&c),
        _ => vec![],
    }
}

/// The face at `p`, if any (the smallest, where faces touch).
pub fn face_at(faces: &[Face], p: Point) -> Option<usize> {
    faces.iter().enumerate().rev().find(|(_, f)| f.path.bounding_box().contains(p) && geom::contains(std::slice::from_ref(&f.path), p)).map(|(i, _)| i)
}

/// Shape Builder on the selected shapes `sel`, whose faces are `faces` (from
/// [`faces`] on their world paths, bottom to top): the `picked` faces are
/// merged into one shape - or deleted, with `delete`. What's left of each
/// shape stays a shape of its own, in its own style. Returns the new shapes
/// (the merged one, or if deleting what's left).
pub fn build(doc: &mut Document, sel: &[ShapeId], faces: &[Face], picked: &[usize], delete: bool) -> Vec<ShapeId> {
    let ordered = crate::ops::z_ordered(doc, sel);
    let page = doc.page();
    let originals: Vec<Shape> = ordered.iter().filter_map(|id| page.shape(*id).cloned()).collect();
    let Some(at) = page.shapes.iter().position(|s| sel.contains(&s.id)) else { return vec![] };
    if originals.is_empty() || picked.is_empty() {
        return vec![];
    }
    // a face belongs to the topmost shape covering it
    let owner = |f: &Face| f.covers.iter().copied().max().unwrap_or(0).min(originals.len() - 1);
    let make = |doc: &mut Document, path: BezPath, from: &Shape| {
        let id = doc.alloc_id();
        let mut s = Shape::new_path(id, path, from.style.clone());
        s.layer = from.layer;
        s
    };
    let mut made = vec![];
    for (i, from) in originals.iter().enumerate() {
        let left: Vec<BezPath> = faces.iter().enumerate().filter(|(k, f)| !picked.contains(k) && owner(f) == i).map(|(_, f)| f.path.clone()).collect();
        if !left.is_empty() {
            let s = make(doc, geom::union_each(&left), from);
            made.push((s, false));
        }
    }
    if !delete {
        let parts: Vec<BezPath> = picked.iter().filter_map(|k| faces.get(*k)).map(|f| f.path.clone()).collect();
        let from = &originals[picked.first().and_then(|k| faces.get(*k)).map(owner).unwrap_or(0)];
        let s = make(doc, geom::union_each(&parts), from);
        made.push((s, true));
    }
    let page = doc.page_mut();
    page.shapes.retain(|s| !sel.contains(&s.id));
    let at = at.min(page.shapes.len());
    let mut out = vec![];
    for (k, (s, merged)) in made.into_iter().enumerate() {
        if merged || delete {
            out.push(s.id);
        }
        page.shapes.insert(at + k, s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::circle_path;

    #[test]
    fn three_circles_make_seven_faces() {
        // the Venn diagram: 3 single, 3 double and 1 triple overlap
        let c = |x, y| vec![circle_path(Point::new(x, y), 20.0)];
        let f = faces(&[c(0.0, 0.0), c(24.0, 0.0), c(12.0, 20.0)]);
        assert_eq!(f.len(), 7, "{:?}", f.iter().map(|f| &f.covers).collect::<Vec<_>>());
        assert_eq!(f.iter().filter(|f| f.covers.len() == 3).count(), 1);
        // together they're exactly the union
        let total: f64 = f.iter().map(|f| f.path.area().abs()).sum();
        let union = geom::union_each(&[circle_path(Point::new(0.0, 0.0), 20.0), circle_path(Point::new(24.0, 0.0), 20.0), circle_path(Point::new(12.0, 20.0), 20.0)]);
        assert!((total - union.area().abs()).abs() < 0.01, "{total} vs {}", union.area().abs());
        // the middle point is in the triple overlap
        let mid = face_at(&f, Point::new(12.0, 7.0)).unwrap();
        assert_eq!(f[mid].covers, vec![0, 1, 2]);
    }

    #[test]
    fn holes_and_islands_are_their_own_regions() {
        // a ring with a dot in the middle: two regions
        let p = geom::merge_paths(&[circle_path(Point::ORIGIN, 20.0), circle_path(Point::ORIGIN, 10.0), circle_path(Point::ORIGIN, 4.0)]);
        let r = regions(&p);
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|q| geom::subpaths(q).len() == 2), "the ring keeps its hole");
    }

    #[test]
    fn building_merges_picked_faces_and_keeps_the_rest() {
        let mut doc = Document::default();
        let a = doc.add_path(circle_path(Point::new(50.0, 50.0), 20.0), Default::default(), 0);
        let b = doc.add_path(circle_path(Point::new(75.0, 50.0), 20.0), Default::default(), 0);
        let sel = vec![a, b];
        let f = faces(&[doc.page().shape(a).unwrap().world_paths(), doc.page().shape(b).unwrap().world_paths()]);
        assert_eq!(f.len(), 3);
        // merge the left crescent with the lens: the whole left circle, and the right crescent left over
        let left = face_at(&f, Point::new(35.0, 50.0)).unwrap();
        let lens = face_at(&f, Point::new(62.5, 50.0)).unwrap();
        let made = build(&mut doc, &sel, &f, &[left, lens], false);
        assert_eq!(made.len(), 1);
        assert_eq!(doc.page().shapes.len(), 2);
        let merged = doc.page().shape(made[0]).unwrap().world_paths();
        let area: f64 = merged.iter().map(|p| p.area().abs()).sum();
        assert!((area - std::f64::consts::PI * 400.0).abs() < 1.0, "{area}");
        // deleting the lens instead leaves two crescents
        let mut doc2 = Document::default();
        let a = doc2.add_path(circle_path(Point::new(50.0, 50.0), 20.0), Default::default(), 0);
        let b = doc2.add_path(circle_path(Point::new(75.0, 50.0), 20.0), Default::default(), 0);
        build(&mut doc2, &[a, b], &f, &[lens], true);
        assert_eq!(doc2.page().shapes.len(), 2);
        assert!(doc2.page().shapes.iter().all(|s| !geom::contains(&s.world_paths(), Point::new(62.5, 50.0))));
    }
}
