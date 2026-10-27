//! Non-destructive, re-editable effect stack carried by a [`crate::doc::Shape`].
//!
//! Each shape may hold an ordered list of [`Effect`]s. They are applied, in
//! order, to the shape's base geometry every time its paths are evaluated, so
//! the original artwork is never modified — parameters can be tweaked, effects
//! reordered or removed, and the whole stack "released to paths" when final.

use crate::fx;
use crate::geom::{self, CornerStyle};
use kurbo::BezPath;
use serde::{Deserialize, Serialize};

/// One non-destructive, path→path operation in a shape's stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Inflate (+) or deflate (−) by `dist` mm.
    Offset { dist: f64, round: bool },
    /// Round sharp corners to `radius` mm.
    RoundCorners { radius: f64 },
    /// Envelope distortion (arch, bulge, wave, …); `amount` and `k2` are its controls.
    Warp { kind: fx::Warp, amount: f64, k2: f64 },
    /// Drop points within `tol` mm (reduce node count).
    Simplify { tol: f64 },
    /// Smooth into flowing curves (`strength` in mm).
    Smooth { strength: f64 },
    /// Unite: weld everything into one outline, live (the parts of a group
    /// stay editable underneath — ungroup to get them back).
    Unite,
    /// Contour: leave out these sub-paths (by index, in the shape's own
    /// outline order) — hides inner cut lines without breaking the shape up.
    HideContours { hidden: Vec<usize> },
}

/// The separate outlines (sub-paths) of `paths`, in order.
pub fn subpaths(paths: &[BezPath]) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = vec![];
    for p in paths {
        let mut cur = BezPath::new();
        for el in p.elements() {
            if matches!(el, kurbo::PathEl::MoveTo(_)) && !cur.elements().is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(*el);
        }
        if !cur.elements().is_empty() {
            out.push(cur);
        }
    }
    out
}

impl Effect {
    /// Short human-readable label for the UI.
    pub fn label(&self) -> String {
        match self {
            Effect::Offset { dist, .. } => format!("Offset {dist:+.1} mm"),
            Effect::RoundCorners { radius } => format!("Round corners {radius:.1} mm"),
            Effect::Warp { kind, amount, .. } => format!("{} {amount:+.2}", kind.name()),
            Effect::Simplify { tol } => format!("Simplify {tol:.2} mm"),
            Effect::Smooth { strength } => format!("Smooth {strength:.2} mm"),
            Effect::Unite => "Unite".into(),
            Effect::HideContours { hidden } => format!("Contour: {} hidden", hidden.len()),
        }
    }

    /// Apply this effect to a set of local-space paths. On a degenerate result
    /// the input is passed through unchanged, so a bad parameter never makes the
    /// shape vanish mid-edit.
    pub fn apply(&self, paths: Vec<BezPath>) -> Vec<BezPath> {
        if paths.is_empty() {
            return paths;
        }
        let keep = |p: BezPath, input: Vec<BezPath>| if p.elements().is_empty() { input } else { vec![p] };
        match self {
            Effect::Offset { dist, round } => {
                let c = if *round { CornerStyle::Round } else { CornerStyle::Sharp };
                let p = geom::offset(&paths, *dist, c);
                keep(p, paths)
            }
            Effect::RoundCorners { radius } => {
                let p = fx::round_corners(&paths, *radius);
                keep(p, paths)
            }
            Effect::Warp { kind, amount, k2 } => {
                let p = kind.apply(&paths, *amount, *k2);
                keep(p, paths)
            }
            Effect::Simplify { tol } => paths.iter().map(|p| geom::simplify_path(p, tol.max(0.001), false)).collect(),
            Effect::Smooth { strength } => paths.iter().map(|p| geom::simplify_path(p, strength.max(0.001), true)).collect(),
            Effect::Unite => {
                let p = geom::union_each(&paths);
                keep(p, paths)
            }
            Effect::HideContours { hidden } => subpaths(&paths).into_iter().enumerate().filter(|(i, _)| !hidden.contains(i)).map(|(_, p)| p).collect(),
        }
    }
}

/// Apply a whole stack, in order, to base geometry.
pub fn apply_all(effects: &[Effect], base: Vec<BezPath>) -> Vec<BezPath> {
    let mut p = base;
    for e in effects {
        p = e.apply(p);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_then_round_stacks() {
        let sq = vec![geom::rect_path(kurbo::Rect::new(0.0, 0.0, 20.0, 20.0))];
        let stack = vec![Effect::Offset { dist: 3.0, round: true }, Effect::RoundCorners { radius: 2.0 }];
        let out = apply_all(&stack, sq.clone());
        assert!(!out.is_empty());
        // outward offset grows the bounding box
        use kurbo::Shape as _;
        let before = sq[0].bounding_box();
        let after = geom::merge_paths(&out).bounding_box();
        assert!(after.width() > before.width());
    }

    #[test]
    fn shape_applies_and_roundtrips_stack() {
        use crate::doc::Shape;
        use kurbo::Shape as _;
        let mut s = Shape::new_path(1, geom::rect_path(kurbo::Rect::new(0.0, 0.0, 20.0, 20.0)), Default::default());
        let base = geom::merge_paths(&s.world_paths()).bounding_box().width();
        s.effects.push(Effect::Offset { dist: 3.0, round: true });
        let grown = geom::merge_paths(&s.world_paths()).bounding_box().width();
        assert!(grown > base, "effect stack should grow the shape ({grown} > {base})");
        // the stack survives serialisation
        let json = serde_json::to_string(&s).unwrap();
        let back: Shape = serde_json::from_str(&json).unwrap();
        assert_eq!(back.effects, s.effects);
    }

    #[test]
    fn bad_param_passes_through() {
        let sq = vec![geom::rect_path(kurbo::Rect::new(0.0, 0.0, 20.0, 20.0))];
        // a huge inward offset would erase the shape; apply() keeps the input instead
        let out = Effect::Offset { dist: -100.0, round: false }.apply(sq.clone());
        assert!(!out.is_empty());
    }
}
