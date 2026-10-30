//! EPS (and old PostScript-based Illustrator .ai) import.
//!
//! Two readers in one:
//! - Illustrator EPS and .ai files draw with Illustrator's own short drawing
//!   commands (`m`, `l`, `c`, `f`, `S`, `k`, `*u`…, documented in Adobe's
//!   Illustrator file format specification). Their setup sections (procsets
//!   that define those commands in PostScript) are skipped and the commands
//!   are read directly.
//! - Everything else (Inkscape/cairo, CorelDRAW, other programs) goes through
//!   a small PostScript interpreter: the stack, dictionaries and procedures,
//!   loops, paths, transforms, gsave/grestore, colours, fill and stroke.
//!
//! Text, images and anything else that isn't drawn with paths are skipped
//! (with a note). A file can't hang CutCraft (there's a limit on how much
//! work it may do) and bad input is never a crash.

use crate::doc::{CutStyle, Rgba, Shape, Style};
use anyhow::{Result, bail};
use kurbo::{Affine, BezPath, Point, Shape as _, Vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Most operators a file may run.
const BUDGET: u64 = 30_000_000;
/// Deepest procedure nesting.
const MAX_DEPTH: usize = 200;

#[derive(Clone, Debug)]
enum Val {
    Num(f64),
    Bool(bool),
    /// An executable name.
    Name(Rc<str>),
    /// A literal name (`/name`).
    Lit(Rc<str>),
    Str(Rc<Vec<u8>>),
    Arr(Rc<RefCell<Vec<Val>>>),
    Proc(Rc<Vec<Val>>),
    Dict(Rc<RefCell<HashMap<String, Val>>>),
    Mark,
    Null,
    /// A picture's data, read from the file after its `image` operator.
    Pixels(Rc<Vec<u8>>),
}

enum Flow {
    Normal,
    Exit,
    Stop,
}

// ---------------------------------------------------------------- reading

/// The PostScript part of a DOS EPS file (one with a TIFF/WMF preview), or
/// the whole file.
fn postscript(bytes: &[u8]) -> &[u8] {
    if bytes.len() > 12 && bytes[..4] == [0xC5, 0xD0, 0xD3, 0xC6] {
        let start = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        if start < bytes.len() {
            return &bytes[start..(start.saturating_add(len)).min(bytes.len())];
        }
    }
    bytes
}

/// The file's text with the sections that aren't drawing taken out: data
/// blocks always, and for Illustrator files the setup code.
fn drawing_text(ps: &str, illustrator: bool) -> String {
    let mut out = String::with_capacity(ps.len());
    let mut skip_until: Option<&str> = None;
    for line in ps.split(['\n', '\r']) {
        if let Some(end) = skip_until {
            if line.starts_with(end) {
                skip_until = None;
            }
            continue;
        }
        let begins: &[(&str, &str)] = &[
            ("%%BeginData", "%%EndData"),
            ("%%BeginBinary", "%%EndBinary"),
            ("%AI9_PrivateDataBegin", "%AI9_PrivateDataEnd"),
            ("%AI5_BeginRaster", "%AI5_EndRaster"),
            ("%%BeginPreview", "%%EndPreview"),
        ];
        let ai_setup: &[(&str, &str)] = &[
            ("%%BeginProlog", "%%EndProlog"),
            ("%%BeginResource", "%%EndResource"),
            ("%%BeginProcSet", "%%EndProcSet"),
            ("%%BeginSetup", "%%EndSetup"),
            ("%%BeginPageSetup", "%%EndPageSetup"),
            ("%%BeginFont", "%%EndFont"),
            ("%AI5_BeginPalette", "%AI5_EndPalette"),
            ("%%BeginPattern", "%%EndPattern"),
            ("%AI3_BeginPattern", "%AI3_EndPattern"),
            ("%AI5_BeginGradient", "%AI5_EndGradient"),
        ];
        if let Some((_, end)) = begins.iter().chain(if illustrator { ai_setup.iter() } else { [].iter() }).find(|(b, _)| line.starts_with(b)) {
            skip_until = Some(end);
            continue;
        }
        if line.starts_with("%%Trailer") && illustrator {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The bounding box from the header comments (points), if given.
fn bounding_box(ps: &str) -> Option<kurbo::Rect> {
    let find = |key: &str| {
        ps.lines().find_map(|l| {
            let rest = l.strip_prefix(key)?;
            let v: Vec<f64> = rest.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            (v.len() == 4 && v[2] > v[0] && v[3] > v[1]).then(|| kurbo::Rect::new(v[0], v[1], v[2], v[3]))
        })
    };
    find("%%HiResBoundingBox:").or_else(|| find("%%BoundingBox:"))
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    /// Where the last picture's data ended (its filters are looked for after it).
    last_image: usize,
}

/// Largest picture data read (bytes, decoded).
const MAX_PIXELS: usize = 400_000_000;

fn ascii85(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 4 / 5);
    let mut group = [0u32; 5];
    let mut n = 0;
    for &c in s {
        match c {
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = (c - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u32, |a, d| a.wrapping_mul(85).wrapping_add(*d));
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u32, |a, d| a.wrapping_mul(85).wrapping_add(*d));
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    }
    out
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = vec![];
    flate2::read::ZlibDecoder::new(data).take(MAX_PIXELS as u64).read_to_end(&mut out).ok()?;
    Some(out)
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') || c.is_ascii_whitespace()
}

impl Lexer<'_> {
    /// Values up to the end (or the `}` closing a procedure).
    fn parse(&mut self, depth: usize) -> Vec<Val> {
        let mut out = vec![];
        while self.i < self.s.len() {
            let c = self.s[self.i];
            match c {
                c if c.is_ascii_whitespace() || c == 0 => self.i += 1,
                b'%' => {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                b'{' => {
                    self.i += 1;
                    if depth < MAX_DEPTH {
                        let body = self.parse(depth + 1);
                        out.push(Val::Proc(Rc::new(body)));
                    }
                }
                b'}' => {
                    self.i += 1;
                    if depth > 0 {
                        return out;
                    }
                }
                b'[' | b']' => {
                    self.i += 1;
                    out.push(Val::Name(if c == b'[' { "[".into() } else { "]".into() }));
                }
                b'(' => out.push(Val::Str(Rc::new(self.string()))),
                b'<' if self.s.get(self.i + 1) == Some(&b'<') => {
                    self.i += 2;
                    out.push(Val::Name("<<".into()));
                }
                b'>' if self.s.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    out.push(Val::Name(">>".into()));
                }
                b'<' => out.push(Val::Str(Rc::new(self.hex_or_a85()))),
                b')' | b'>' => self.i += 1,
                b'/' => {
                    self.i += 1;
                    if self.s.get(self.i) == Some(&b'/') {
                        self.i += 1;
                    }
                    let w = self.word();
                    out.push(Val::Lit(w.into()));
                }
                _ => {
                    let at = self.i;
                    let w = self.word();
                    if w.is_empty() {
                        self.i += 1;
                        continue;
                    }
                    if matches!(w.as_str(), "image" | "colorimage" | "imagemask") {
                        if let Some(px) = self.picture_data(at) {
                            out.push(Val::Pixels(Rc::new(px)));
                        }
                    }
                    out.push(match number(&w) {
                        Some(n) => Val::Num(n),
                        None => Val::Name(w.into()),
                    });
                }
            }
        }
        out
    }

    /// The data of a picture whose operator starts at `at`, when it reads
    /// it from the file (`currentfile`): ASCII85 or hex text, maybe
    /// compressed. Reads past it.
    fn picture_data(&mut self, at: usize) -> Option<Vec<u8>> {
        let from = self.last_image.max(at.saturating_sub(4000));
        let setup = String::from_utf8_lossy(&self.s[from..at]).into_owned();
        if !setup.contains("currentfile") {
            return None;
        }
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
        let start = self.i;
        let raw = if setup.contains("ASCII85Decode") {
            while self.i < self.s.len() && !(self.s[self.i] == b'~' && self.s.get(self.i + 1) == Some(&b'>')) {
                self.i += 1;
            }
            let data = ascii85(&self.s[start..self.i]);
            self.i = (self.i + 2).min(self.s.len());
            data
        } else if setup.contains("readhexstring") || setup.contains("ASCIIHexDecode") {
            while self.i < self.s.len() && (self.s[self.i].is_ascii_hexdigit() || self.s[self.i].is_ascii_whitespace()) {
                self.i += 1;
            }
            let hex: Vec<u8> = self.s[start..self.i].iter().copied().filter(|c| c.is_ascii_hexdigit()).collect();
            if self.s.get(self.i) == Some(&b'>') {
                self.i += 1;
            }
            hex.chunks(2).map(|p| u8::from_str_radix(&String::from_utf8_lossy(p), 16).unwrap_or(0)).collect()
        } else {
            return None;
        };
        self.last_image = self.i;
        if setup.contains("FlateDecode") { inflate(&raw) } else { Some(raw) }
    }

    fn word(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() && !is_delim(self.s[self.i]) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn string(&mut self) -> Vec<u8> {
        self.i += 1;
        let mut out = vec![];
        let mut depth = 1;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'\\' => {
                    if let Some(&e) = self.s.get(self.i) {
                        self.i += 1;
                        out.push(match e {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 8,
                            b'f' => 12,
                            b'0'..=b'7' => {
                                let mut v = (e - b'0') as u32;
                                for _ in 0..2 {
                                    match self.s.get(self.i) {
                                        Some(d @ b'0'..=b'7') => {
                                            v = v * 8 + (d - b'0') as u32;
                                            self.i += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                v as u8
                            }
                            b'\n' | b'\r' => continue,
                            e => e,
                        });
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                c => out.push(c),
            }
        }
        out
    }

    /// `<hex>` or `<~ascii85~>` (the bytes aren't needed, only skipped).
    fn hex_or_a85(&mut self) -> Vec<u8> {
        let a85 = self.s.get(self.i + 1) == Some(&b'~');
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() {
            if a85 && self.s[self.i] == b'~' && self.s.get(self.i + 1) == Some(&b'>') {
                self.i += 2;
                return vec![];
            }
            if !a85 && self.s[self.i] == b'>' {
                let hex: Vec<u8> = self.s[start..self.i].iter().copied().filter(|c| c.is_ascii_hexdigit()).collect();
                self.i += 1;
                return hex.chunks(2).map(|p| u8::from_str_radix(&String::from_utf8_lossy(p), 16).unwrap_or(0)).collect();
            }
            self.i += 1;
        }
        vec![]
    }
}

fn number(w: &str) -> Option<f64> {
    if let Some((radix, digits)) = w.split_once('#') {
        let r: u32 = radix.parse().ok()?;
        return (2..=36).contains(&r).then(|| i64::from_str_radix(digits, r).ok().map(|v| v as f64)).flatten();
    }
    let first = w.chars().next()?;
    if !(first.is_ascii_digit() || matches!(first, '-' | '+' | '.')) {
        return None;
    }
    w.parse::<f64>().ok().filter(|v| v.is_finite())
}

// ---------------------------------------------------------------- drawing

#[derive(Clone)]
struct GState {
    ctm: Affine,
    path: BezPath,
    /// Current point (device space).
    cur: Option<Point>,
    start: Option<Point>,
    fill: Rgba,
    stroke: Rgba,
    line_width: f64,
    /// Components of the current colour space (for `setcolor`).
    space_n: usize,
}

impl Default for GState {
    fn default() -> GState {
        GState {
            ctm: Affine::IDENTITY,
            path: BezPath::new(),
            cur: None,
            start: None,
            fill: Rgba(0, 0, 0, 255),
            stroke: Rgba(0, 0, 0, 255),
            line_width: 1.0,
            space_n: 1,
        }
    }
}

struct Interp {
    stack: Vec<Val>,
    dicts: Vec<Rc<RefCell<HashMap<String, Val>>>>,
    gs: GState,
    saved: Vec<GState>,
    /// Painted shapes so far: (path in points, style).
    shapes: Vec<(BezPath, Style)>,
    /// Pictures: RGBA pixels, size, where they go (pixel → points), and the
    /// number of shapes painted before each.
    pictures: Vec<(Vec<u8>, u32, u32, Affine, usize)>,
    /// Illustrator compound path being gathered (`*u` … `*U`).
    compound: Option<(BezPath, Option<Style>)>,
    ops: u64,
    depth: usize,
    text: usize,
    images: usize,
}

fn rgb(r: f64, g: f64, b: f64) -> Rgba {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba(c(r), c(g), c(b), 255)
}

fn cmyk(c: f64, m: f64, y: f64, k: f64) -> Rgba {
    rgb((1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k))
}

fn hsb(h: f64, s: f64, b: f64) -> Rgba {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (b * (1.0 - s), b * (1.0 - s * f), b * (1.0 - s * (1.0 - f)));
    let (r, g, bl) = match i as i32 {
        0 => (b, t, p),
        1 => (q, b, p),
        2 => (p, b, t),
        3 => (p, q, b),
        4 => (t, p, b),
        _ => (b, p, q),
    };
    rgb(r, g, bl)
}

impl Interp {
    fn new() -> Interp {
        Interp {
            stack: vec![],
            dicts: vec![Rc::default()],
            gs: GState::default(),
            saved: vec![],
            shapes: vec![],
            pictures: vec![],
            compound: None,
            ops: 0,
            depth: 0,
            text: 0,
            images: 0,
        }
    }

    fn num(&mut self) -> Option<f64> {
        match self.stack.pop()? {
            Val::Num(n) => Some(n),
            Val::Bool(b) => Some(b as u8 as f64),
            _ => None,
        }
    }

    fn nums<const N: usize>(&mut self) -> Option<[f64; N]> {
        if self.stack.len() < N {
            self.stack.clear();
            return None;
        }
        let mut out = [0.0; N];
        for i in (0..N).rev() {
            out[i] = self.num()?;
        }
        Some(out)
    }

    fn pop_n(&mut self, n: usize) {
        let keep = self.stack.len().saturating_sub(n);
        self.stack.truncate(keep);
    }

    fn lookup(&self, name: &str) -> Option<Val> {
        self.dicts.iter().rev().find_map(|d| d.borrow().get(name).cloned())
    }

    fn run(&mut self, body: &[Val]) -> Flow {
        if self.depth >= MAX_DEPTH {
            return Flow::Stop;
        }
        self.depth += 1;
        let mut flow = Flow::Normal;
        for v in body {
            self.ops += 1;
            if self.ops > BUDGET {
                flow = Flow::Stop;
                break;
            }
            let f = match v {
                Val::Name(n) => self.call(n),
                other => {
                    self.stack.push(other.clone());
                    Flow::Normal
                }
            };
            if !matches!(f, Flow::Normal) {
                flow = f;
                break;
            }
        }
        self.depth -= 1;
        flow
    }

    fn exec(&mut self, v: Val) -> Flow {
        match v {
            Val::Proc(b) => self.run(&b),
            Val::Name(n) => self.call(&n),
            other => {
                self.stack.push(other);
                Flow::Normal
            }
        }
    }

    fn call(&mut self, name: &str) -> Flow {
        if let Some(v) = self.lookup(name) {
            return self.exec(v);
        }
        self.builtin(name)
    }

    fn device(&self, x: f64, y: f64) -> Point {
        self.gs.ctm * Point::new(x, y)
    }

    fn move_to(&mut self, p: Point) {
        self.gs.path.move_to(p);
        self.gs.cur = Some(p);
        self.gs.start = Some(p);
    }

    fn line_to(&mut self, p: Point) {
        if self.gs.cur.is_none() {
            return self.move_to(p);
        }
        self.gs.path.line_to(p);
        self.gs.cur = Some(p);
    }

    fn curve_to(&mut self, a: Point, b: Point, c: Point) {
        if self.gs.cur.is_none() {
            self.move_to(a);
        }
        self.gs.path.curve_to(a, b, c);
        self.gs.cur = Some(c);
    }

    fn close(&mut self) {
        if self.gs.cur.is_some() {
            self.gs.path.close_path();
            self.gs.cur = self.gs.start;
        }
    }

    fn arc(&mut self, cx: f64, cy: f64, r: f64, a0: f64, a1: f64, ccw: bool) {
        let (a0, mut a1) = (a0.to_radians(), a1.to_radians());
        if ccw {
            while a1 < a0 {
                a1 += std::f64::consts::TAU;
            }
        } else {
            while a1 > a0 {
                a1 -= std::f64::consts::TAU;
            }
        }
        let arc = kurbo::Arc::new((cx, cy), (r, r), a0, a1 - a0, 0.0);
        let first = self.device(cx + r * a0.cos(), cy + r * a0.sin());
        if self.gs.cur.is_some() {
            self.line_to(first);
        } else {
            self.move_to(first);
        }
        let ctm = self.gs.ctm;
        for el in arc.append_iter(0.1) {
            if let kurbo::PathEl::CurveTo(a, b, c) = el {
                self.curve_to(ctm * a, ctm * b, ctm * c);
            }
        }
    }

    /// Paints the current path and starts a new one.
    fn paint(&mut self, fill: bool, stroke: bool) {
        let path = std::mem::take(&mut self.gs.path);
        self.gs.cur = None;
        if path.elements().len() < 2 {
            return;
        }
        let k = self.gs.ctm.determinant().abs().sqrt();
        let style = Style {
            fill: fill.then_some(self.gs.fill),
            stroke: stroke.then_some(self.gs.stroke),
            stroke_width: if stroke { (self.gs.line_width * k * 25.4 / 72.0).max(0.05) } else { 0.25 },
            cut: CutStyle::Cut,
            no_print: false,
        };
        match &mut self.compound {
            Some((p, st)) => {
                p.extend(path.elements().iter().copied());
                st.get_or_insert(style);
            }
            None => self.shapes.push((path, style)),
        }
    }

    fn set_colour(&mut self, c: Rgba) {
        self.gs.fill = c;
        self.gs.stroke = c;
    }

    fn matrix_of(v: &Val) -> Option<Affine> {
        let Val::Arr(a) = v else { return None };
        let a = a.borrow();
        let n: Vec<f64> = a.iter().filter_map(|v| if let Val::Num(n) = v { Some(*n) } else { None }).collect();
        (n.len() == 6).then(|| Affine::new([n[0], n[1], n[2], n[3], n[4], n[5]]))
    }

    fn matrix_val(m: Affine) -> Val {
        Val::Arr(Rc::new(RefCell::new(m.as_coeffs().iter().map(|v| Val::Num(*v)).collect())))
    }

    /// Illustrator's custom colour `c m y k (name) tint [type] x`: the CMYK
    /// numbers before the name, the tint after it.
    fn ai_custom(&mut self) -> Option<Rgba> {
        let at = self.stack.iter().rposition(|v| matches!(v, Val::Str(_)))?;
        let after: Vec<f64> = self.stack[at + 1..].iter().filter_map(|v| if let Val::Num(n) = v { Some(*n) } else { None }).collect();
        let before: Vec<f64> = self.stack[..at].iter().rev().take(4).filter_map(|v| if let Val::Num(n) = v { Some(*n) } else { None }).collect();
        self.stack.truncate(at.saturating_sub(4));
        let tint = after.first().copied().unwrap_or(1.0);
        let [k, y, m, c] = before[..] else { return None };
        // tint 0 is the full colour in Illustrator's convention
        let t = 1.0 - tint;
        Some(cmyk(c * t, m * t, y * t, k * t))
    }

    fn builtin(&mut self, name: &str) -> Flow {
        macro_rules! args {
            ($n:literal) => {
                match self.nums::<$n>() {
                    Some(v) => v,
                    None => return Flow::Normal,
                }
            };
        }
        match name {
            // --- numbers
            "add" | "sub" | "mul" | "div" | "idiv" | "mod" | "atan" | "exp" => {
                let [a, b] = args!(2);
                let r = match name {
                    "add" => a + b,
                    "sub" => a - b,
                    "mul" => a * b,
                    "div" => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    "idiv" => {
                        if b == 0.0 {
                            0.0
                        } else {
                            (a / b).trunc()
                        }
                    }
                    "mod" => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a % b
                        }
                    }
                    "atan" => a.atan2(b).to_degrees().rem_euclid(360.0),
                    _ => a.powf(b),
                };
                self.stack.push(Val::Num(if r.is_finite() { r } else { 0.0 }));
            }
            "neg" | "abs" | "sqrt" | "sin" | "cos" | "ln" | "log" | "floor" | "ceiling" | "round" | "truncate" | "cvi" | "cvr" => {
                let [a] = args!(1);
                let r = match name {
                    "neg" => -a,
                    "abs" => a.abs(),
                    "sqrt" => a.max(0.0).sqrt(),
                    "sin" => a.to_radians().sin(),
                    "cos" => a.to_radians().cos(),
                    "ln" => a.ln(),
                    "log" => a.log10(),
                    "floor" => a.floor(),
                    "ceiling" => a.ceil(),
                    "round" => a.round(),
                    _ => a.trunc(),
                };
                self.stack.push(Val::Num(if r.is_finite() { r } else { 0.0 }));
            }
            // --- comparisons and logic
            "eq" | "ne" => {
                if self.stack.len() < 2 {
                    return Flow::Normal;
                }
                let (b, a) = (self.stack.pop().unwrap_or(Val::Null), self.stack.pop().unwrap_or(Val::Null));
                let same = match (&a, &b) {
                    (Val::Num(x), Val::Num(y)) => x == y,
                    (Val::Bool(x), Val::Bool(y)) => x == y,
                    (Val::Lit(x) | Val::Name(x), Val::Lit(y) | Val::Name(y)) => x == y,
                    (Val::Str(x), Val::Str(y)) => x == y,
                    (Val::Lit(x) | Val::Name(x), Val::Str(y)) | (Val::Str(y), Val::Lit(x) | Val::Name(x)) => x.as_bytes() == y.as_slice(),
                    (Val::Null, Val::Null) => true,
                    _ => false,
                };
                self.stack.push(Val::Bool(same == (name == "eq")));
            }
            "gt" | "ge" | "lt" | "le" => {
                let [a, b] = args!(2);
                self.stack.push(Val::Bool(match name {
                    "gt" => a > b,
                    "ge" => a >= b,
                    "lt" => a < b,
                    _ => a <= b,
                }));
            }
            "and" | "or" | "xor" => {
                let (b, a) = (self.stack.pop(), self.stack.pop());
                self.stack.push(match (a, b) {
                    (Some(Val::Bool(a)), Some(Val::Bool(b))) => Val::Bool(match name {
                        "and" => a && b,
                        "or" => a || b,
                        _ => a != b,
                    }),
                    (Some(Val::Num(a)), Some(Val::Num(b))) => {
                        let (a, b) = (a as i64, b as i64);
                        Val::Num(match name {
                            "and" => a & b,
                            "or" => a | b,
                            _ => a ^ b,
                        } as f64)
                    }
                    _ => Val::Bool(false),
                });
            }
            "not" => match self.stack.pop() {
                Some(Val::Bool(b)) => self.stack.push(Val::Bool(!b)),
                Some(Val::Num(n)) => self.stack.push(Val::Num(!(n as i64) as f64)),
                _ => self.stack.push(Val::Bool(false)),
            },
            "true" => self.stack.push(Val::Bool(true)),
            "false" => self.stack.push(Val::Bool(false)),
            "null" => self.stack.push(Val::Null),
            // --- the stack
            "pop" => {
                self.stack.pop();
            }
            "exch" => {
                let n = self.stack.len();
                if n >= 2 {
                    self.stack.swap(n - 1, n - 2);
                }
            }
            "dup" => {
                if let Some(v) = self.stack.last().cloned() {
                    self.stack.push(v);
                }
            }
            "copy" => match self.stack.pop() {
                Some(Val::Num(n)) => {
                    let n = (n.max(0.0) as usize).min(self.stack.len());
                    let tail: Vec<Val> = self.stack[self.stack.len() - n..].to_vec();
                    self.stack.extend(tail);
                }
                Some(other) => {
                    // copying into an array/dict/string: keep the target
                    self.stack.pop();
                    self.stack.push(other);
                }
                None => {}
            },
            "index" => {
                let [n] = args!(1);
                let n = n.max(0.0) as usize;
                if n < self.stack.len() {
                    let v = self.stack[self.stack.len() - 1 - n].clone();
                    self.stack.push(v);
                }
            }
            "roll" => {
                let [n, j] = args!(2);
                let n = (n.max(0.0) as usize).min(self.stack.len());
                if n > 0 {
                    let len = self.stack.len();
                    let part = &mut self.stack[len - n..];
                    let j = (j as i64).rem_euclid(n as i64) as usize;
                    part.rotate_right(j);
                }
            }
            "clear" => self.stack.clear(),
            "count" => self.stack.push(Val::Num(self.stack.len() as f64)),
            "mark" | "[" | "<<" => self.stack.push(Val::Mark),
            "cleartomark" => {
                let at = self.stack.iter().rposition(|v| matches!(v, Val::Mark)).unwrap_or(0);
                self.stack.truncate(at);
            }
            "counttomark" => {
                let at = self.stack.iter().rposition(|v| matches!(v, Val::Mark)).map(|i| i + 1).unwrap_or(0);
                self.stack.push(Val::Num((self.stack.len() - at) as f64));
            }
            "]" => {
                let at = self.stack.iter().rposition(|v| matches!(v, Val::Mark));
                let items = match at {
                    Some(i) => {
                        let items = self.stack.split_off(i + 1);
                        self.stack.pop();
                        items
                    }
                    None => std::mem::take(&mut self.stack),
                };
                self.stack.push(Val::Arr(Rc::new(RefCell::new(items))));
            }
            ">>" => {
                let at = self.stack.iter().rposition(|v| matches!(v, Val::Mark));
                let items = match at {
                    Some(i) => {
                        let items = self.stack.split_off(i + 1);
                        self.stack.pop();
                        items
                    }
                    None => vec![],
                };
                let mut d = HashMap::new();
                for pair in items.chunks(2) {
                    if let [Val::Lit(k) | Val::Name(k), v] = pair {
                        d.insert(k.to_string(), v.clone());
                    }
                }
                self.stack.push(Val::Dict(Rc::new(RefCell::new(d))));
            }
            // --- dictionaries
            "dict" => {
                self.stack.pop();
                self.stack.push(Val::Dict(Rc::default()));
            }
            "begin" => match self.stack.pop() {
                Some(Val::Dict(d)) => self.dicts.push(d),
                _ => self.dicts.push(Rc::default()),
            },
            "end" => {
                if self.dicts.len() > 1 {
                    self.dicts.pop();
                }
            }
            "def" | "store" => {
                let (v, k) = (self.stack.pop(), self.stack.pop());
                if let (Some(v), Some(Val::Lit(k) | Val::Name(k))) = (v, k) {
                    let target = if name == "store" {
                        self.dicts.iter().rev().find(|d| d.borrow().contains_key(&*k)).cloned()
                    } else {
                        None
                    };
                    let d = target.unwrap_or_else(|| self.dicts.last().cloned().unwrap_or_default());
                    d.borrow_mut().insert(k.to_string(), v);
                }
            }
            "load" => {
                if let Some(Val::Lit(k) | Val::Name(k)) = self.stack.pop() {
                    let v = self.lookup(&k).unwrap_or(Val::Name(k));
                    self.stack.push(v);
                }
            }
            "where" => {
                if let Some(Val::Lit(k) | Val::Name(k)) = self.stack.pop() {
                    match self.dicts.iter().rev().find(|d| d.borrow().contains_key(&*k)).cloned() {
                        Some(d) => {
                            self.stack.push(Val::Dict(d));
                            self.stack.push(Val::Bool(true));
                        }
                        None => self.stack.push(Val::Bool(false)),
                    }
                }
            }
            "known" => {
                let (k, d) = (self.stack.pop(), self.stack.pop());
                let found = match (d, k) {
                    (Some(Val::Dict(d)), Some(Val::Lit(k) | Val::Name(k))) => d.borrow().contains_key(&*k),
                    _ => false,
                };
                self.stack.push(Val::Bool(found));
            }
            "get" => {
                let (k, c) = (self.stack.pop(), self.stack.pop());
                let v = match (c, k) {
                    (Some(Val::Dict(d)), Some(Val::Lit(k) | Val::Name(k))) => d.borrow().get(&*k).cloned(),
                    (Some(Val::Arr(a)), Some(Val::Num(i))) => a.borrow().get(i as usize).cloned(),
                    (Some(Val::Proc(p)), Some(Val::Num(i))) => p.get(i as usize).cloned(),
                    (Some(Val::Str(s)), Some(Val::Num(i))) => s.get(i as usize).map(|b| Val::Num(*b as f64)),
                    _ => None,
                };
                self.stack.push(v.unwrap_or(Val::Null));
            }
            "put" => {
                let (v, k, c) = (self.stack.pop(), self.stack.pop(), self.stack.pop());
                match (c, k, v) {
                    (Some(Val::Dict(d)), Some(Val::Lit(k) | Val::Name(k)), Some(v)) => {
                        d.borrow_mut().insert(k.to_string(), v);
                    }
                    (Some(Val::Arr(a)), Some(Val::Num(i)), Some(v)) => {
                        if let Some(slot) = a.borrow_mut().get_mut(i as usize) {
                            *slot = v;
                        }
                    }
                    _ => {}
                }
            }
            "currentdict" | "userdict" | "globaldict" | "statusdict" | "errordict" => {
                let d = if name == "currentdict" { self.dicts.last().cloned() } else { self.dicts.first().cloned() };
                self.stack.push(Val::Dict(d.unwrap_or_default()));
            }
            "systemdict" => self.stack.push(Val::Dict(Rc::default())),
            "bind" | "readonly" | "executeonly" | "noaccess" | "cvlit" => {}
            "cvx" => {
                if let Some(v) = self.stack.pop() {
                    self.stack.push(match v {
                        Val::Arr(a) => Val::Proc(Rc::new(a.borrow().clone())),
                        Val::Lit(n) => Val::Name(n),
                        other => other,
                    });
                }
            }
            "array" => {
                let [n] = args!(1);
                self.stack.push(Val::Arr(Rc::new(RefCell::new(vec![Val::Null; (n.max(0.0) as usize).min(100_000)]))));
            }
            "string" => {
                let [n] = args!(1);
                self.stack.push(Val::Str(Rc::new(vec![0; (n.max(0.0) as usize).min(1_000_000)])));
            }
            "length" => {
                let n = match self.stack.pop() {
                    Some(Val::Arr(a)) => a.borrow().len(),
                    Some(Val::Proc(p)) => p.len(),
                    Some(Val::Str(s)) => s.len(),
                    Some(Val::Dict(d)) => d.borrow().len(),
                    Some(Val::Lit(n) | Val::Name(n)) => n.len(),
                    _ => 0,
                };
                self.stack.push(Val::Num(n as f64));
            }
            "aload" => {
                if let Some(v) = self.stack.pop() {
                    let items = match &v {
                        Val::Arr(a) => a.borrow().clone(),
                        Val::Proc(p) => p.to_vec(),
                        _ => vec![],
                    };
                    self.stack.extend(items);
                    self.stack.push(v);
                }
            }
            "astore" => {
                if let Some(Val::Arr(a)) = self.stack.pop() {
                    let n = a.borrow().len().min(self.stack.len());
                    let items = self.stack.split_off(self.stack.len() - n);
                    *a.borrow_mut() = items;
                    self.stack.push(Val::Arr(a));
                }
            }
            "type" => {
                let t = match self.stack.pop() {
                    Some(Val::Num(_)) => "realtype",
                    Some(Val::Bool(_)) => "booleantype",
                    Some(Val::Arr(_) | Val::Proc(_)) => "arraytype",
                    Some(Val::Dict(_)) => "dicttype",
                    Some(Val::Str(_)) => "stringtype",
                    Some(Val::Lit(_) | Val::Name(_)) => "nametype",
                    Some(Val::Mark) => "marktype",
                    _ => "nulltype",
                };
                self.stack.push(Val::Name(t.into()));
            }
            "cvs" | "cvn" => {
                let v = self.stack.pop();
                if name == "cvs" {
                    self.stack.pop();
                }
                self.stack.push(match v {
                    Some(Val::Str(s)) if name == "cvn" => Val::Lit(String::from_utf8_lossy(&s).as_ref().into()),
                    Some(v) => v,
                    None => Val::Null,
                });
            }
            // --- control
            "exec" => {
                if let Some(v) = self.stack.pop() {
                    return self.exec(v);
                }
            }
            "if" => {
                let (p, c) = (self.stack.pop(), self.stack.pop());
                if let (Some(p), Some(Val::Bool(true))) = (p, c) {
                    return self.exec(p);
                }
            }
            "ifelse" => {
                let (b, a, c) = (self.stack.pop(), self.stack.pop(), self.stack.pop());
                if let (Some(a), Some(b), Some(Val::Bool(c))) = (a, b, c) {
                    return self.exec(if c { a } else { b });
                }
            }
            "repeat" => {
                let p = self.stack.pop();
                let [n] = args!(1);
                if let Some(p) = p {
                    for _ in 0..(n.max(0.0) as u64) {
                        match self.exec(p.clone()) {
                            Flow::Exit => break,
                            Flow::Stop => return Flow::Stop,
                            Flow::Normal => {}
                        }
                        if self.ops > BUDGET {
                            return Flow::Stop;
                        }
                    }
                }
            }
            "for" => {
                let p = self.stack.pop();
                let [start, step, end] = args!(3);
                if let Some(p) = p {
                    if step == 0.0 {
                        return Flow::Normal;
                    }
                    let mut i = start;
                    while (step > 0.0 && i <= end) || (step < 0.0 && i >= end) {
                        self.stack.push(Val::Num(i));
                        match self.exec(p.clone()) {
                            Flow::Exit => break,
                            Flow::Stop => return Flow::Stop,
                            Flow::Normal => {}
                        }
                        if self.ops > BUDGET {
                            return Flow::Stop;
                        }
                        i += step;
                    }
                }
            }
            "loop" => {
                if let Some(p) = self.stack.pop() {
                    loop {
                        self.ops += 1;
                        match self.exec(p.clone()) {
                            Flow::Exit => break,
                            Flow::Stop => return Flow::Stop,
                            Flow::Normal => {}
                        }
                        if self.ops > BUDGET {
                            return Flow::Stop;
                        }
                    }
                }
            }
            "forall" => {
                let (p, c) = (self.stack.pop(), self.stack.pop());
                let (Some(p), Some(c)) = (p, c) else { return Flow::Normal };
                let items: Vec<Vec<Val>> = match c {
                    Val::Arr(a) => a.borrow().iter().map(|v| vec![v.clone()]).collect(),
                    Val::Proc(b) => b.iter().map(|v| vec![v.clone()]).collect(),
                    Val::Str(s) => s.iter().map(|b| vec![Val::Num(*b as f64)]).collect(),
                    Val::Dict(d) => d.borrow().iter().map(|(k, v)| vec![Val::Lit(k.as_str().into()), v.clone()]).collect(),
                    _ => vec![],
                };
                for it in items {
                    self.stack.extend(it);
                    match self.exec(p.clone()) {
                        Flow::Exit => break,
                        Flow::Stop => return Flow::Stop,
                        Flow::Normal => {}
                    }
                }
            }
            "exit" => return Flow::Exit,
            "stop" => return Flow::Stop,
            "stopped" => {
                if let Some(p) = self.stack.pop() {
                    let stopped = matches!(self.exec(p), Flow::Stop) && self.ops <= BUDGET;
                    self.stack.push(Val::Bool(stopped));
                }
            }
            "quit" => return Flow::Stop,
            // --- graphics state
            "gsave" | "save" => {
                if self.saved.len() < 1000 {
                    self.saved.push(self.gs.clone());
                }
                if name == "save" {
                    self.stack.push(Val::Null);
                }
            }
            "grestore" | "restore" => {
                if name == "restore" {
                    self.stack.pop();
                }
                if let Some(g) = self.saved.pop() {
                    self.gs = g;
                }
            }
            "grestoreall" => {
                if let Some(g) = self.saved.first().cloned() {
                    self.gs = g;
                }
                self.saved.clear();
            }
            "translate" | "scale" => {
                if matches!(self.stack.last(), Some(Val::Arr(_))) {
                    self.stack.pop();
                }
                let [x, y] = args!(2);
                let t = if name == "translate" { Affine::translate((x, y)) } else { Affine::scale_non_uniform(x, y) };
                self.gs.ctm *= t;
            }
            "rotate" => {
                if matches!(self.stack.last(), Some(Val::Arr(_))) {
                    self.stack.pop();
                }
                let [a] = args!(1);
                self.gs.ctm *= Affine::rotate(a.to_radians());
            }
            "concat" => {
                if let Some(m) = self.stack.pop().as_ref().and_then(Self::matrix_of) {
                    self.gs.ctm *= m;
                }
            }
            "setmatrix" => {
                if let Some(m) = self.stack.pop().as_ref().and_then(Self::matrix_of) {
                    self.gs.ctm = m;
                }
            }
            "initmatrix" => self.gs.ctm = Affine::IDENTITY,
            "matrix" => self.stack.push(Self::matrix_val(Affine::IDENTITY)),
            "currentmatrix" | "defaultmatrix" => {
                self.stack.pop();
                let m = if name == "currentmatrix" { self.gs.ctm } else { Affine::IDENTITY };
                self.stack.push(Self::matrix_val(m));
            }
            "setlinewidth" => {
                let [w] = args!(1);
                self.gs.line_width = w.abs();
            }
            "currentlinewidth" => self.stack.push(Val::Num(self.gs.line_width)),
            "setlinecap" | "setlinejoin" | "setmiterlimit" | "setflat" | "setoverprint" | "setstrokeadjust" | "setsmoothness" | "setcolorrendering" | "setrenderingintent" => {
                self.stack.pop();
            }
            "setdash" => self.pop_n(2),
            "setrgbcolor" => {
                let [r, g, b] = args!(3);
                self.set_colour(rgb(r, g, b));
            }
            "setgray" => {
                let [g] = args!(1);
                self.set_colour(rgb(g, g, g));
            }
            "setcmykcolor" => {
                let [c, m, y, k] = args!(4);
                self.set_colour(cmyk(c, m, y, k));
            }
            "sethsbcolor" => {
                let [h, s, b] = args!(3);
                self.set_colour(hsb(h, s, b));
            }
            "setcolorspace" => {
                let n = match self.stack.pop() {
                    Some(Val::Lit(s) | Val::Name(s)) => match &*s {
                        "DeviceRGB" | "CalRGB" => 3,
                        "DeviceCMYK" => 4,
                        _ => 1,
                    },
                    Some(Val::Arr(a)) => match a.borrow().first() {
                        Some(Val::Lit(s) | Val::Name(s)) if &**s == "DeviceRGB" || &**s == "CalRGB" || &**s == "Lab" => 3,
                        Some(Val::Lit(s) | Val::Name(s)) if &**s == "DeviceCMYK" => 4,
                        Some(Val::Lit(s) | Val::Name(s)) if &**s == "ICCBased" => 3,
                        _ => 1,
                    },
                    _ => 1,
                };
                self.gs.space_n = n;
            }
            "setcolor" => match self.gs.space_n {
                3 => {
                    let [r, g, b] = args!(3);
                    self.set_colour(rgb(r, g, b));
                }
                4 => {
                    let [c, m, y, k] = args!(4);
                    self.set_colour(cmyk(c, m, y, k));
                }
                _ => {
                    let [g] = args!(1);
                    self.set_colour(rgb(g, g, g));
                }
            },
            // --- paths
            "newpath" => {
                self.gs.path = BezPath::new();
                self.gs.cur = None;
            }
            "moveto" => {
                let [x, y] = args!(2);
                let p = self.device(x, y);
                self.move_to(p);
            }
            "lineto" => {
                let [x, y] = args!(2);
                let p = self.device(x, y);
                self.line_to(p);
            }
            "rmoveto" | "rlineto" => {
                let [dx, dy] = args!(2);
                let Some(c) = self.gs.cur else { return Flow::Normal };
                let p = c + (self.gs.ctm * Point::new(dx, dy) - self.gs.ctm * Point::ORIGIN);
                if name == "rmoveto" {
                    self.move_to(p);
                } else {
                    self.line_to(p);
                }
            }
            "curveto" => {
                let [a, b, c, d, e, f] = args!(6);
                let (p1, p2, p3) = (self.device(a, b), self.device(c, d), self.device(e, f));
                self.curve_to(p1, p2, p3);
            }
            "rcurveto" => {
                let [a, b, c, d, e, f] = args!(6);
                let Some(cur) = self.gs.cur else { return Flow::Normal };
                let lin = |v: Vec2| self.gs.ctm * v.to_point() - self.gs.ctm * Point::ORIGIN;
                let (p1, p2, p3) = (cur + lin(Vec2::new(a, b)), cur + lin(Vec2::new(c, d)), cur + lin(Vec2::new(e, f)));
                self.curve_to(p1, p2, p3);
            }
            "closepath" => self.close(),
            "arc" | "arcn" => {
                let [x, y, r, a0, a1] = args!(5);
                self.arc(x, y, r, a0, a1, name == "arc");
            }
            "currentpoint" => {
                let p = self.gs.ctm.inverse() * self.gs.cur.unwrap_or_default();
                self.stack.push(Val::Num(p.x));
                self.stack.push(Val::Num(p.y));
            }
            "rectfill" | "rectstroke" | "rectclip" | "re" => {
                if matches!(self.stack.last(), Some(Val::Arr(_))) {
                    self.stack.pop();
                    return Flow::Normal;
                }
                let [x, y, w, h] = args!(4);
                let saved = std::mem::take(&mut self.gs.path);
                let corners = [self.device(x, y), self.device(x + w, y), self.device(x + w, y + h), self.device(x, y + h)];
                self.move_to(corners[0]);
                for c in &corners[1..] {
                    self.line_to(*c);
                }
                self.close();
                match name {
                    "rectfill" => self.paint(true, false),
                    "rectstroke" => self.paint(false, true),
                    "re" => {
                        // PDF-style: the rectangle joins the path
                        let r = std::mem::replace(&mut self.gs.path, saved);
                        self.gs.path.extend(r.elements().iter().copied());
                    }
                    _ => self.gs.path = saved,
                }
            }
            "fill" | "eofill" => self.paint(true, false),
            "stroke" => self.paint(false, true),
            "clip" | "eoclip" => {}
            "shfill" => {
                self.stack.pop();
            }
            // --- text and pictures: skipped
            "show" | "ashow" | "widthshow" | "awidthshow" | "xshow" | "yshow" | "xyshow" | "kshow" | "glyphshow" | "charpath" => {
                self.text += 1;
                let n = match name {
                    "ashow" | "xshow" | "yshow" | "xyshow" | "kshow" => 3,
                    "widthshow" => 4,
                    "awidthshow" => 6,
                    "charpath" => 2,
                    _ => 1,
                };
                self.pop_n(n.min(self.stack.len()));
            }
            "findfont" | "scalefont" | "makefont" | "definefont" | "findresource" => {
                let n = match name {
                    "findfont" => 1,
                    _ => 2,
                };
                self.pop_n(n);
                self.stack.push(Val::Dict(Rc::default()));
            }
            "setfont" | "undefinefont" => {
                self.stack.pop();
            }
            "selectfont" => self.pop_n(2),
            "stringwidth" => {
                self.stack.pop();
                self.stack.push(Val::Num(0.0));
                self.stack.push(Val::Num(0.0));
            }
            "image" | "colorimage" | "imagemask" => {
                if !self.picture(name) {
                    self.images += 1;
                }
            }
            "currentfile" => self.stack.push(Val::Null),
            "readhexstring" | "readstring" | "readline" => {
                self.pop_n(2);
                self.stack.push(Val::Str(Rc::default()));
                self.stack.push(Val::Bool(false));
            }
            "filter" => {
                // source [params] /Name filter: the result stands for the source
                // (the data itself is read with the picture)
                self.stack.pop();
                if matches!(self.stack.last(), Some(Val::Dict(_))) {
                    self.stack.pop();
                }
            }
            "showpage" | "copypage" | "erasepage" | "flushfile" | "closefile" | "vmstatus" | "setpagedevice" | "setglobal" | "pdfmark" => {
                if matches!(name, "setpagedevice" | "setglobal" | "flushfile" | "closefile") {
                    self.stack.pop();
                }
            }
            "bytesavailable" | "currentglobal" | "languagelevel" => self.stack.push(Val::Num(if name == "languagelevel" { 2.0 } else { 0.0 })),
            // --- Illustrator's drawing commands
            _ => return self.illustrator(name),
        }
        Flow::Normal
    }

    /// Illustrator's short drawing commands (also used when a file defines
    /// none of its own for these names).
    fn illustrator(&mut self, name: &str) -> Flow {
        macro_rules! args {
            ($n:literal) => {
                match self.nums::<$n>() {
                    Some(v) => v,
                    None => return Flow::Normal,
                }
            };
        }
        match name {
            "m" => {
                let [x, y] = args!(2);
                let p = self.device(x, y);
                self.move_to(p);
            }
            "l" | "L" => {
                let [x, y] = args!(2);
                let p = self.device(x, y);
                self.line_to(p);
            }
            "c" | "C" => {
                let [a, b, c, d, e, f] = args!(6);
                let (p1, p2, p3) = (self.device(a, b), self.device(c, d), self.device(e, f));
                self.curve_to(p1, p2, p3);
            }
            // first control point at the current point
            "v" | "V" => {
                let [c, d, e, f] = args!(4);
                let Some(cur) = self.gs.cur else { return Flow::Normal };
                let (p2, p3) = (self.device(c, d), self.device(e, f));
                self.curve_to(cur, p2, p3);
            }
            // second control point at the end point
            "y" | "Y" => {
                let [a, b, e, f] = args!(4);
                let (p1, p3) = (self.device(a, b), self.device(e, f));
                self.curve_to(p1, p3, p3);
            }
            "h" | "H" => self.close(),
            "f" | "s" | "b" | "n" => {
                self.close();
                match name {
                    "f" => self.paint(true, false),
                    "s" => self.paint(false, true),
                    "b" => self.paint(true, true),
                    _ => self.discard(),
                }
            }
            "F" => self.paint(true, false),
            "S" => self.paint(false, true),
            "B" => self.paint(true, true),
            "N" => self.discard(),
            "*u" => {
                if self.compound.is_none() {
                    self.compound = Some((BezPath::new(), None));
                }
            }
            "*U" => {
                if let Some((p, st)) = self.compound.take() {
                    if let Some(st) = st.filter(|_| !p.elements().is_empty()) {
                        self.shapes.push((p, st));
                    }
                }
            }
            "k" | "K" => {
                let [c, m, y, k] = args!(4);
                let col = cmyk(c, m, y, k);
                if name == "k" { self.gs.fill = col } else { self.gs.stroke = col }
            }
            "g" | "G" => {
                let [g] = args!(1);
                let col = rgb(g, g, g);
                if name == "g" { self.gs.fill = col } else { self.gs.stroke = col }
            }
            "Xa" | "XA" => {
                let [r, g, b] = args!(3);
                let col = rgb(r, g, b);
                if name == "Xa" { self.gs.fill = col } else { self.gs.stroke = col }
            }
            "x" | "X" => {
                if let Some(col) = self.ai_custom() {
                    if name == "x" { self.gs.fill = col } else { self.gs.stroke = col }
                }
            }
            "w" => {
                let [w] = args!(1);
                self.gs.line_width = w.abs();
            }
            "j" | "J" | "M" | "i" | "A" | "O" | "R" | "D" | "XR" | "Ap" | "Ar" => {
                self.stack.pop();
            }
            "d" => self.pop_n(2),
            "q" | "Q" | "u" | "U" | "W" | "Lb" | "LB" | "Ln" | "Xs" | "XS" => {
                if name == "Lb" {
                    self.stack.clear();
                }
                if name == "Ln" {
                    self.stack.pop();
                }
            }
            "To" | "TO" | "Tp" | "TP" | "Tx" | "Tj" | "TX" | "Tk" | "Tf" => {
                if matches!(name, "Tx" | "Tj" | "TX") {
                    self.text += 1;
                }
                self.stack.clear();
            }
            // anything else: unknown, ignored (its operands are left behind,
            // which the drawing commands ignore)
            _ => {
                if self.stack.len() > 10_000 {
                    self.stack.clear();
                }
            }
        }
        Flow::Normal
    }

    /// Draws a picture from the image operator `op`'s operands (and the
    /// data read with it); false if it can't.
    fn picture(&mut self, op: &str) -> bool {
        let data = match self.stack.last() {
            Some(Val::Pixels(p)) => {
                let p = p.clone();
                self.stack.pop();
                Some(p)
            }
            _ => None,
        };
        let num = |v: Option<&Val>| if let Some(Val::Num(n)) = v { Some(*n) } else { None };
        // (width, height, bits, image matrix, components, decode inverted)
        let spec = match self.stack.last().cloned() {
            Some(Val::Dict(d)) => {
                self.stack.pop();
                let d = d.borrow();
                let w = num(d.get("Width"));
                let h = num(d.get("Height"));
                let bits = num(d.get("BitsPerComponent")).unwrap_or(if op == "imagemask" { 1.0 } else { 8.0 });
                let m = d.get("ImageMatrix").and_then(Self::matrix_of);
                let inverted = match d.get("Decode") {
                    Some(Val::Arr(a)) => matches!(a.borrow().first(), Some(Val::Num(v)) if *v > 0.5),
                    _ => false,
                };
                let comps = if op == "imagemask" { 1 } else { self.gs.space_n };
                w.zip(h).zip(m).map(|((w, h), m)| (w, h, bits, m, comps, inverted))
            }
            _ => {
                // Level 1: w h bits matrix proc image / w h invert matrix proc imagemask /
                // w h bits matrix proc… multi ncomp colorimage
                let mut comps = 1;
                if op == "colorimage" {
                    let n = self.num().unwrap_or(1.0) as usize;
                    let multi = matches!(self.stack.pop(), Some(Val::Bool(true)));
                    comps = n.clamp(1, 4);
                    self.pop_n(if multi { comps } else { 1 });
                } else {
                    self.stack.pop();
                }
                let m = self.stack.pop().as_ref().and_then(Self::matrix_of);
                let third = self.stack.pop();
                let (h, w) = (self.num(), self.num());
                let (bits, inverted) = match (op, third) {
                    ("imagemask", Some(Val::Bool(b))) => (1.0, b),
                    (_, Some(Val::Num(b))) => (b, false),
                    _ => (8.0, false),
                };
                w.zip(h).zip(m).map(|((w, h), m)| (w, h, bits, m, comps, inverted))
            }
        };
        let (Some(data), Some((w, h, bits, m, comps, inverted))) = (data, spec) else { return false };
        let (w, h, bits) = (w as usize, h as usize, bits as usize);
        if w == 0 || h == 0 || w.saturating_mul(h) > MAX_PIXELS / 4 || !matches!(bits, 1 | 8) {
            return false;
        }
        let row = (w * comps * bits).div_ceil(8);
        if data.len() < row * h {
            return false;
        }
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let r = &data[y * row..(y + 1) * row];
            for x in 0..w {
                let sample = |c: usize| -> f64 {
                    let v = if bits == 8 {
                        r[x * comps + c] as f64 / 255.0
                    } else {
                        let bit = x * comps + c;
                        ((r[bit / 8] >> (7 - bit % 8)) & 1) as f64
                    };
                    if inverted { 1.0 - v } else { v }
                };
                let px = if op == "imagemask" {
                    // a 1 sample paints (or 0, when inverted) in the fill colour
                    let f = self.gs.fill;
                    if sample(0) < 0.5 { [f.0, f.1, f.2, 255] } else { [0, 0, 0, 0] }
                } else {
                    let c = match comps {
                        3 => rgb(sample(0), sample(1), sample(2)),
                        4 => cmyk(sample(0), sample(1), sample(2), sample(3)),
                        _ => rgb(sample(0), sample(0), sample(0)),
                    };
                    [c.0, c.1, c.2, 255]
                };
                rgba.extend_from_slice(&px);
            }
        }
        // image pixels → user space (the unit square) → device
        let place = self.gs.ctm * m.inverse();
        self.pictures.push((rgba, w as u32, h as u32, place, self.shapes.len()));
        true
    }

    fn discard(&mut self) {
        self.gs.path = BezPath::new();
        self.gs.cur = None;
    }
}

/// Reads an EPS file (or an old PostScript-based Illustrator .ai) as shapes
/// in millimetres, with notes about what was left out.
pub fn import(bytes: &[u8]) -> Result<(Vec<Shape>, Vec<String>)> {
    let ps: String = postscript(bytes).iter().map(|&b| b as char).collect();
    if !ps.starts_with("%!") && !ps.contains("%!PS") {
        bail!("this isn't a PostScript/EPS file");
    }
    let illustrator = ps.contains("%%Creator: Adobe Illustrator") || ps.contains("%AI5_") || ps.contains("%AI3_") || ps.contains("%%AI8_CreatorVersion");
    let bbox = bounding_box(&ps);
    let text = drawing_text(&ps, illustrator);
    let body = Lexer { s: text.as_bytes(), i: 0, last_image: 0 }.parse(0);
    let mut it = Interp::new();
    // a run that stops (error, limit) still keeps what it drew
    let _ = it.run(&body);

    let mut notes = vec![];
    if it.ops > BUDGET {
        notes.push("the file was too complicated to read completely".into());
    }
    if it.text > 0 {
        notes.push("text skipped (convert it to outlines first)".into());
    }
    if it.images > 0 {
        notes.push(format!("{} image(s) skipped", it.images));
    }
    let pictures = std::mem::take(&mut it.pictures);
    if it.shapes.is_empty() && pictures.is_empty() {
        bail!("no shapes found in the file");
    }
    // points to mm, y downwards, the top left of the drawing at the page origin
    let picture_box = |(_, w, h, m, _): &(Vec<u8>, u32, u32, Affine, usize)| {
        let corners = [(0.0, 0.0), (*w as f64, 0.0), (*w as f64, *h as f64), (0.0, *h as f64)].map(|c| *m * Point::from(c));
        corners.iter().skip(1).fold(kurbo::Rect::from_points(corners[0], corners[0]), |r, p| r.union_pt(*p))
    };
    let area = bbox.or_else(|| it.shapes.iter().map(|(p, _)| p.bounding_box()).chain(pictures.iter().map(picture_box)).reduce(|a, b| a.union(b)));
    let area = area.unwrap_or_default();
    let k = 25.4 / 72.0;
    let to_mm = Affine::new([k, 0.0, 0.0, -k, -area.x0 * k, area.y1 * k]);
    let mut out = vec![];
    let mut pics = pictures.into_iter().peekable();
    for (i, (p, st)) in it.shapes.into_iter().enumerate() {
        while let Some(pic) = pics.next_if(|pic| pic.4 <= i) {
            out.extend(picture_shape(pic, to_mm));
        }
        if p.elements().len() > 1 {
            out.push(Shape::new_path(0, to_mm * p, st));
        }
    }
    for pic in pics {
        out.extend(picture_shape(pic, to_mm));
    }
    Ok((out, notes))
}

/// A picture as an image shape, placed as the file places it.
fn picture_shape((rgba, w, h, place, _): (Vec<u8>, u32, u32, Affine, usize), to_mm: Affine) -> Option<Shape> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    // grey pictures store smaller as grey
    let grey = img.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2] && p.0[3] == 255);
    let mut png = vec![];
    let dynimg = if grey { image::DynamicImage::ImageLuma8(image::DynamicImage::ImageRgba8(img).to_luma8()) } else { image::DynamicImage::ImageRgba8(img) };
    dynimg.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    // pixel → mm, and the picture's size in mm along its edges
    let m = to_mm * place;
    let along = |v: Vec2| (m * v.to_point() - m * Point::ORIGIN).hypot();
    let (wmm, hmm) = (along(Vec2::new(w as f64, 0.0)), along(Vec2::new(0.0, h as f64)));
    if !(wmm > 0.0 && hmm > 0.0) {
        return None;
    }
    let mut s = crate::io::import_image(&png).ok()?;
    if let crate::doc::ShapeKind::Image(d) = &mut s.kind {
        d.width_mm = wmm;
        d.height_mm = hmm;
    }
    s.transform = m * Affine::scale_non_uniform(w as f64 / wmm, h as f64 / hmm);
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filled area (even-odd, as CutCraft fills).
    fn area(s: &Shape) -> f64 {
        use geo::Area;
        crate::geom::to_multipolygon(&s.world_paths()).unsigned_area()
    }

    #[test]
    fn plain_postscript_with_procedures_and_loops() {
        // cairo-style short names, a loop, gsave/grestore and a transform
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n\
            /m { moveto } bind def /l { lineto } bind def /h { closepath } bind def\n\
            %%EndProlog\n\
            1 0 0 setrgbcolor\n\
            0 1 2 { gsave 60 mul 10 add 10 translate 0 0 m 40 0 l 40 40 l 0 40 l h fill grestore } for\n\
            0 0 1 setrgbcolor 2 setlinewidth newpath 100 70 20 0 360 arc closepath stroke\n\
            showpage\n%%EOF\n";
        let (shapes, notes) = import(eps).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(shapes.len(), 4);
        // three 40 pt squares, red, filled
        for s in &shapes[..3] {
            assert_eq!(s.style.fill, Some(Rgba(255, 0, 0, 255)));
            let side = 40.0 * 25.4 / 72.0;
            assert!((area(s) - side * side).abs() < 0.01, "{}", area(s));
        }
        // the third square is 120 pt right of the first
        let x = |s: &Shape| s.bbox().unwrap().x0;
        assert!((x(&shapes[2]) - x(&shapes[0]) - 120.0 * 25.4 / 72.0).abs() < 0.01);
        // y flipped: the squares' bottom edge (y = 10 pt) is 90 pt down from the top
        assert!((shapes[0].bbox().unwrap().y1 - 90.0 * 25.4 / 72.0).abs() < 0.01);
        // the circle: stroked blue, 2 pt wide, made of curves
        let c = &shapes[3];
        assert_eq!((c.style.stroke, c.style.fill), (Some(Rgba(0, 0, 255, 255)), None));
        assert!((c.style.stroke_width - 2.0 * 25.4 / 72.0).abs() < 1e-9);
        let r = 20.0 * 25.4 / 72.0;
        assert!((area(c) / (std::f64::consts::PI * r * r) - 1.0).abs() < 0.01, "{}", area(c));
    }

    #[test]
    fn illustrator_drawing_commands() {
        // the prolog would define m/l/c… in PostScript; it's skipped and the
        // commands are read directly. A compound path (a square with a square hole).
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%Creator: Adobe Illustrator(R) 24.0\n%%BoundingBox: 0 0 100 100\n\
            %%BeginProlog\n/m { this would break } def\n%%EndProlog\n\
            %%BeginSetup\nAdobe_AGM_Core/setup get exec\n%%EndSetup\n\
            0 0.5 1 0 k\n*u\n10 10 m\n90 10 L\n90 90 L\n10 90 L\n10 10 L\nf\n\
            30 30 m\n70 30 L\n70 70 L\n30 70 L\n30 30 L\nf\n*U\n\
            0 g 1 w\n0 0 0 1 K\n10 50 m\n30 80 50 80 50 50 c\n50 20 v\nS\n\
            %%Trailer\n%%EOF\n";
        let (shapes, _) = import(eps).unwrap();
        assert_eq!(shapes.len(), 2);
        let k = 25.4 / 72.0;
        // the compound: 80x80 less 40x40
        assert!((area(&shapes[0]) - (6400.0 - 1600.0) * k * k).abs() < 0.5, "{}", area(&shapes[0]));
        assert_eq!(shapes[0].style.fill, Some(cmyk(0.0, 0.5, 1.0, 0.0)));
        // the curve: stroked black
        assert_eq!(shapes[1].style.stroke, Some(Rgba(0, 0, 0, 255)));
        assert!(shapes[1].style.fill.is_none());
    }

    #[test]
    fn pictures_come_in_where_the_file_puts_them() {
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n\
            gsave 50 25 translate 100 50 scale\n\
            /picstr 2 string def\n\
            2 2 8 [2 0 0 -2 0 2] { currentfile picstr readhexstring pop } image\n\
            00ff\nff00\n\
            grestore\n\
            0 0 moveto 10 0 lineto 10 10 lineto closepath fill\n%%EOF\n";
        let (shapes, notes) = import(eps).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(shapes.len(), 2, "the picture, then the square");
        let crate::doc::ShapeKind::Image(img) = &shapes[0].kind else { panic!("a picture first") };
        assert_eq!((img.px_w, img.px_h), (2, 2));
        let k = 25.4 / 72.0;
        assert!((img.width_mm - 100.0 * k).abs() < 1e-6 && (img.height_mm - 50.0 * k).abs() < 1e-6);
        // its top-left corner: 50 pt in, and 100 - 75 = 25 pt down
        let tl = shapes[0].transform * Point::ORIGIN;
        assert!((tl.x - 50.0 * k).abs() < 1e-6 && (tl.y - 25.0 * k).abs() < 1e-6, "{tl:?}");
        // the top-left pixel is black (row 0 of the data is the top row)
        let px = image::load_from_memory(&img.png).unwrap().to_luma8();
        assert_eq!((px.get_pixel(0, 0).0[0], px.get_pixel(1, 0).0[0]), (0, 255));
    }

    #[test]
    fn ascii85_decodes() {
        assert_eq!(ascii85(b"87cURD]i,\"Ebo80"), b"Hello World!");
        assert_eq!(ascii85(b"z"), [0, 0, 0, 0]);
    }

    /// Imports the files listed (separated by `;`) in CUTCRAFT_EPS and says
    /// what came in (run by hand, in release).
    #[test]
    #[ignore = "needs files"]
    fn import_files_from_env() {
        let Ok(list) = std::env::var("CUTCRAFT_EPS") else { return };
        for f in list.split(';').filter(|f| !f.is_empty()) {
            let t = std::time::Instant::now();
            match std::fs::read(f).map_err(anyhow::Error::from).and_then(|b| import(&b)) {
                Ok((shapes, notes)) => {
                    let bb = shapes.iter().filter_map(|s| s.bbox()).reduce(|a, b| a.union(b)).unwrap_or_default();
                    let filled = shapes.iter().filter(|s| s.style.fill.is_some()).count();
                    println!("{f}: {} shapes ({filled} filled), {:.1} x {:.1} mm, notes {notes:?}, {:?}", shapes.len(), bb.width(), bb.height(), t.elapsed());
                }
                Err(e) => println!("{f}: error {e}"),
            }
        }
    }

    #[test]
    fn junk_and_endless_loops_dont_hang_or_crash() {
        let eps = b"%!PS\n{ 1 2 add pop } loop\n0 0 moveto 10 0 lineto 10 10 lineto closepath fill\n";
        // the loop runs into the limit; the file still ends cleanly
        let r = import(eps);
        assert!(r.is_err() || r.unwrap().1.iter().any(|n| n.contains("complicated")));
        for junk in [&b"%!PS\n) ] } >> exch pop roll 1 2 3 4 5 6 curveto fill"[..], b"%!PS\n<< /a [ 1 2 { 3 } ] >> begin end 9 9 9 arc stroke", b"%!PS\n(\\" ] {
            let _ = import(junk);
        }
        assert!(import(b"not postscript at all").is_err());
    }
}
