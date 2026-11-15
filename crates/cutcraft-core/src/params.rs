//! Parametric design, ported from BurnCraft: named values (`thickness = 3`)
//! and locked sizes and distances whose values can be formulas
//! (`thickness * 2 + 0.2`). Changing a value updates the design.
//!
//! Constraints act on top-level shapes (a group is one shape): a locked width
//! scales the shape about the chosen edge, and a locked distance moves the
//! second shape so the gap is right. Values are mm; a number may carry a unit
//! (`2in`, `5cm`, `12mm`).

use crate::doc::{Document, ShapeId};
use kurbo::{Affine, Rect};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
}

/// A point along one axis of a bounding box: left/top, centre, or right/bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    pub const ALL: [Anchor; 3] = [Anchor::Start, Anchor::Middle, Anchor::End];

    pub fn label(self, axis: Axis) -> &'static str {
        match (self, axis) {
            (Anchor::Start, Axis::X) => "left",
            (Anchor::Start, Axis::Y) => "top",
            (Anchor::Middle, Axis::X) => "centre",
            (Anchor::Middle, Axis::Y) => "middle",
            (Anchor::End, Axis::X) => "right",
            (Anchor::End, Axis::Y) => "bottom",
        }
    }

    pub fn of(self, r: Rect, axis: Axis) -> f64 {
        let (lo, hi) = match axis {
            Axis::X => (r.x0, r.x1),
            Axis::Y => (r.y0, r.y1),
        };
        match self {
            Anchor::Start => lo,
            Anchor::Middle => (lo + hi) / 2.0,
            Anchor::End => hi,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintKind {
    /// The width (X) or height (Y) of a shape; `anchor` is the edge that stays put.
    Size { shape: ShapeId, axis: Axis, anchor: Anchor },
    /// The distance along an axis from a point of one shape to a point of
    /// another; the second shape moves.
    Distance { axis: Axis, from: ShapeId, from_anchor: Anchor, to: ShapeId, to_anchor: Anchor },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub kind: ConstraintKind,
    /// A number or formula (mm), e.g. `40` or `thickness * 2`.
    pub value: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    /// A number or formula using earlier parameters.
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Parametric {
    pub params: Vec<Param>,
    pub constraints: Vec<Constraint>,
}

impl Parametric {
    pub fn is_empty(&self) -> bool {
        self.params.is_empty() && self.constraints.is_empty()
    }

    /// Every parameter's value, in order (each may use the ones before it).
    /// Errors name the parameter.
    pub fn values(&self) -> (HashMap<String, f64>, Vec<String>) {
        let mut vars = HashMap::new();
        let mut errors = Vec::new();
        for p in &self.params {
            match eval(&p.value, &vars) {
                Ok(v) => {
                    vars.insert(p.name.trim().to_string(), v);
                }
                Err(e) => errors.push(format!("{}: {e}", p.name)),
            }
        }
        (vars, errors)
    }

    /// A name not used by any parameter yet.
    pub fn new_name(&self) -> String {
        (1..).map(|i| format!("p{i}")).find(|n| !self.params.iter().any(|p| &p.name == n)).expect("some name is free")
    }
}

/// A top-level shape's bounds, wherever its page.
fn bounds(doc: &Document, id: ShapeId) -> Option<Rect> {
    doc.pages.iter().find_map(|p| p.shape(id)).and_then(|s| s.bbox())
}

impl Constraint {
    /// The shapes this constraint refers to.
    pub fn shapes(&self) -> Vec<ShapeId> {
        match self.kind {
            ConstraintKind::Size { shape, .. } => vec![shape],
            ConstraintKind::Distance { from, to, .. } => vec![from, to],
        }
    }

    /// The shape the solver changes.
    pub fn moves(&self) -> ShapeId {
        match self.kind {
            ConstraintKind::Size { shape, .. } => shape,
            ConstraintKind::Distance { to, .. } => to,
        }
    }

    /// The constrained quantity as the design is now.
    pub fn measure(&self, doc: &Document) -> Option<f64> {
        match self.kind {
            ConstraintKind::Size { shape, axis, .. } => {
                let r = bounds(doc, shape)?;
                Some(match axis {
                    Axis::X => r.width(),
                    Axis::Y => r.height(),
                })
            }
            ConstraintKind::Distance { axis, from, from_anchor, to, to_anchor } => {
                let (a, b) = (bounds(doc, from)?, bounds(doc, to)?);
                Some(to_anchor.of(b, axis) - from_anchor.of(a, axis))
            }
        }
    }

    pub fn describe(&self, doc: &Document) -> String {
        let name = |id: ShapeId| doc.pages.iter().find_map(|p| p.shape(id)).map_or_else(|| "(deleted)".to_string(), |s| s.display_name());
        match &self.kind {
            ConstraintKind::Size { shape, axis, anchor } => {
                format!("{} of {} (keeps its {})", if *axis == Axis::X { "Width" } else { "Height" }, name(*shape), anchor.label(*axis))
            }
            ConstraintKind::Distance { axis, from, from_anchor, to, to_anchor } => format!(
                "{} from {} {} to {} {}",
                if *axis == Axis::X { "Across" } else { "Down" },
                name(*from),
                from_anchor.label(*axis),
                name(*to),
                to_anchor.label(*axis)
            ),
        }
    }
}

/// Makes the design meet its constraints. Returns whether anything moved or
/// changed size, and messages about constraints that can't be met.
/// Constraints on deleted shapes are dropped.
pub fn solve(doc: &mut Document) -> (bool, Vec<String>) {
    let present: std::collections::HashSet<ShapeId> = doc.pages.iter().flat_map(|p| p.shapes.iter().map(|s| s.id)).collect();
    doc.params.constraints.retain(|c| c.shapes().iter().all(|id| present.contains(id)));
    if doc.params.constraints.is_empty() {
        return (false, Vec::new());
    }
    let (vars, mut errors) = doc.params.values();
    let targets: Vec<Option<f64>> = doc
        .params
        .constraints
        .iter()
        .map(|c| {
            if !c.enabled {
                return None;
            }
            match eval(&c.value, &vars) {
                Ok(v) => Some(v),
                Err(e) => {
                    errors.push(format!("{}: {e}", c.describe(doc)));
                    None
                }
            }
        })
        .collect();
    let mut changed = false;
    // constraints can depend on each other (a width moves an edge another
    // distance is measured from), so go round until nothing moves
    for _ in 0..20 {
        let mut moved = false;
        for (c, target) in doc.params.constraints.clone().iter().zip(&targets) {
            let (Some(v), Some(now)) = (*target, c.measure(doc)) else { continue };
            if (now - v).abs() < 1e-6 {
                continue;
            }
            let id = c.moves();
            let t = match c.kind {
                ConstraintKind::Size { axis, anchor, .. } => {
                    if v <= 0.0 || now <= 1e-9 {
                        continue;
                    }
                    let Some(r) = bounds(doc, id) else { continue };
                    let k = v / now;
                    let pivot = anchor.of(r, axis);
                    match axis {
                        Axis::X => Affine::translate((pivot, 0.0)) * Affine::scale_non_uniform(k, 1.0) * Affine::translate((-pivot, 0.0)),
                        Axis::Y => Affine::translate((0.0, pivot)) * Affine::scale_non_uniform(1.0, k) * Affine::translate((0.0, -pivot)),
                    }
                }
                ConstraintKind::Distance { axis, .. } => {
                    let d = v - now;
                    if axis == Axis::X { Affine::translate((d, 0.0)) } else { Affine::translate((0.0, d)) }
                }
            };
            let Some(page) = doc.pages.iter_mut().find(|p| p.shape(id).is_some()) else { continue };
            if !page.shape(id).is_some_and(|s| page.is_editable(s)) {
                let msg = format!("{}: the shape is locked", c.describe(&*doc));
                if !errors.contains(&msg) {
                    errors.push(msg);
                }
                continue;
            }
            if let Some(s) = page.shape_mut(id) {
                s.transform = t * s.transform;
            }
            moved = true;
            changed = true;
        }
        if !moved {
            return (changed, errors);
        }
    }
    errors.push("Some constraints fight each other and can't all be met".into());
    (changed, errors)
}

// --- formulas -----------------------------------------------------------------

/// Evaluates a formula: numbers (optionally with `mm`, `cm` or `in`),
/// parameter names, + − × ÷ (`*` `/`), brackets and unary minus.
pub fn eval(text: &str, vars: &HashMap<String, f64>) -> Result<f64, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, vars };
    let v = p.expr()?;
    p.space();
    if p.i < p.s.len() {
        return Err(format!("unexpected \"{}\"", &text[p.i..]));
    }
    if !v.is_finite() {
        return Err("the result isn't a number (dividing by zero?)".into());
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    vars: &'a HashMap<String, f64>,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.space();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while let Some(op @ (b'*' | b'/' | b'x')) = self.peek() {
            // "x" only multiplies between numbers, like "2 x 3"; otherwise it starts a name
            if op == b'x' && self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                break;
            }
            self.i += 1;
            let r = self.factor()?;
            v = if op == b'/' { v / r } else { v * r };
        }
        Ok(v)
    }

    /// A unit written straight after a number: `mm`, `cm` or `in`.
    fn unit(&mut self) -> f64 {
        for (u, k) in [("mm", 1.0), ("cm", 10.0), ("in", 25.4)] {
            let end = self.i + u.len();
            if self.s.get(self.i..end) == Some(u.as_bytes()) && !self.s.get(end).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                self.i = end;
                return k;
            }
        }
        1.0
    }

    fn factor(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing \")\"".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).expect("ASCII");
                let v: f64 = t.parse().map_err(|_| format!("\"{t}\" isn't a number"))?;
                Ok(v * self.unit())
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).expect("ASCII");
                self.vars.get(name).copied().ok_or_else(|| format!("no value called \"{name}\""))
            }
            Some(_) => Err(format!("unexpected \"{}\"", String::from_utf8_lossy(&self.s[self.i..]))),
            None => Err("a value is missing".into()),
        }
    }
}
