//! GPGL encoder (Graphtec / Silhouette desktop cutters) and HP-GL export.
//!
//! The command set below follows the publicly documented behaviour used by
//! open-source drivers (e.g. inkscape-silhouette). It was NOT taken from
//! Silhouette Studio, whose command table is encrypted. Verify against a USB
//! capture of your own machine before trusting unattended cuts.
//!
//! - Commands are ASCII terminated by ETX (0x03).
//! - Units: 20 plotter units per mm. Coordinates are sent as `y,x`.

use crate::cutjob::CutJob;
use crate::materials::Tool;
use crate::regmarks::RegMarks;

pub const ETX: u8 = 0x03;
pub const ESC: u8 = 0x1b;
pub const UNITS_PER_MM: f64 = 20.0;

/// Which wire language to speak to a machine.
///
/// Silhouette / Graphtec desktop cutters speak GPGL; most vinyl cutters
/// (Roland, GCC, USCutter, …) speak HP-GL. CutCraft lets you switch per job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Protocol {
    /// Graphtec Plotter Graphics Language (Silhouette Cameo/Portrait/Curio, Graphtec).
    #[default]
    Gpgl,
    /// Hewlett-Packard Graphics Language (`.plt`): GCC, Roland, USCutter, most vinyl cutters.
    Hpgl,
    /// Digital Microprocessor Plotter Language: Summa, Houston Instrument, Tenneth.
    Dmpl,
    /// Brother ScanNCut: no connection — the job is saved as an `.fcm` file
    /// for CanvasWorkspace (see `fcm.rs`).
    Fcm,
}

impl Protocol {
    pub const ALL: [(Protocol, &'static str); 4] = [
        (Protocol::Gpgl, "GPGL (Silhouette / Graphtec)"),
        (Protocol::Hpgl, "HP-GL (GCC / Roland / vinyl cutters)"),
        (Protocol::Dmpl, "DMPL (Summa / Houston)"),
        (Protocol::Fcm, "Brother ScanNCut (.fcm file)"),
    ];
}

/// Variant of the GPGL command set to emit.
///
/// `Community` is the long-standing open-source form (inkscape-silhouette /
/// robocut) that CutCraft has shipped and tested. `Graphtec` mirrors the
/// command forms recorded from the Graphtec GPGL engine in the user's own
/// reverse-engineering notes — the force command carries its second parameter
/// (`FX<f>,0`) and registration-mark search uses the `MARK`/`DETECT`/`EYE_AT`
/// trio instead of the `TB` sequence. The exact numeric arguments of the
/// Graphtec reg-mark search are not fully documented, so verify with a test
/// scan before trusting an unattended Print & Cut job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GpglDialect {
    #[default]
    Community,
    Graphtec,
}

impl GpglDialect {
    pub const ALL: [(GpglDialect, &'static str); 2] =
        [(GpglDialect::Community, "Community (default)"), (GpglDialect::Graphtec, "Graphtec (experimental)")];
}

/// Device-level control sequences (not part of a job file).
pub const INIT: [u8; 2] = [ESC, 0x04];
pub const STATUS: [u8; 2] = [ESC, 0x05];

pub fn cmd(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(ETX);
    v
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ready,
    Moving,
    NoMedia,
    Paused,
    Unknown(String),
}

/// Query appended to the job during registration-mark search. The device
/// answers an ASCII `…=<code>;` reply (see [`parse_mark_status`]).
pub const MARK_STATUS: &str = "mark_status;";

/// Result code of a registration-mark search reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkStatus {
    /// Mark found (`=4`).
    Found,
    /// Search failed (`=5`).
    Failed,
    /// Still searching / unrecognised code — keep polling.
    Pending(i32),
}

/// Parse a `mark_status;` reply of the documented form `…=<code>;`.
/// Returns `None` until a complete `=<digits>;` field has arrived.
pub fn parse_mark_status(resp: &[u8]) -> Option<MarkStatus> {
    let s = String::from_utf8_lossy(resp);
    let after = s.rsplit('=').next()?;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    if digits.is_empty() || !after[digits.len()..].starts_with(';') {
        return None;
    }
    let code: i32 = digits.parse().ok()?;
    Some(match code {
        4 => MarkStatus::Found,
        5 => MarkStatus::Failed,
        other => MarkStatus::Pending(other),
    })
}

pub fn parse_status(resp: &[u8]) -> Status {
    let s = String::from_utf8_lossy(resp).trim_matches(|c: char| c == '\u{3}' || c.is_whitespace()).to_string();
    match s.as_str() {
        "0" => Status::Ready,
        "1" => Status::Moving,
        "2" => Status::NoMedia,
        "3" => Status::Paused,
        _ => Status::Unknown(s),
    }
}

#[derive(Clone, Debug)]
pub struct JobOptions {
    /// Page offset applied before sending (mat origin), mm.
    pub origin_mm: (f64, f64),
    /// Media size for the cut window, mm.
    pub media_mm: (f64, f64),
    /// Track enhancing (rollers pass media back and forth before cutting).
    pub track_enhancing: bool,
    /// Registration marks to search for before cutting.
    pub regmarks: Option<RegMarks>,
    /// Feed media out after the job.
    pub feed_after: bool,
    /// Which GPGL command variant to emit.
    pub dialect: GpglDialect,
}

impl Default for JobOptions {
    fn default() -> Self {
        JobOptions {
            origin_mm: (0.0, 0.0),
            media_mm: (304.8, 304.8),
            track_enhancing: false,
            regmarks: None,
            feed_after: false,
            dialect: GpglDialect::default(),
        }
    }
}

fn u(mm: f64) -> i64 {
    (mm * UNITS_PER_MM).round() as i64
}

/// Mark-detection resolution: the Silhouette optical search works in 1016 units
/// per inch (= 40 units/mm), distinct from the 20 units/mm used for cutting.
pub const MARK_UNITS_PER_MM: f64 = 1016.0 / 25.4;

fn um(mm: f64) -> i64 {
    (mm * MARK_UNITS_PER_MM).round() as i64
}

fn push_cmd(out: &mut Vec<u8>, s: &str) {
    out.extend(cmd(s));
}

/// Page/orientation setup and (Graphtec) start-cut TB block.
fn push_setup(out: &mut Vec<u8>, opt: &JobOptions) {
    let (mw, mh) = opt.media_mm;
    push_cmd(out, "FN0"); // portrait orientation
    push_cmd(out, "TB50,0");
    push_cmd(out, if opt.track_enhancing { "FY0" } else { "FY1" });
    push_cmd(out, &format!("\\{},{}", 0, 0));
    push_cmd(out, &format!("Z{},{}", u(mh), u(mw)));
    if opt.dialect == GpglDialect::Graphtec {
        // Documented Silhouette/Graphtec start-cut block (the TB-command variant).
        // TB51/TB53 carry settings-dependent parameters whose meaning is not
        // documented, so they are omitted rather than guessed. TB24 is the cut
        // area (height,width in plotter units, matching the Z convention).
        push_cmd(out, "TB99");
        push_cmd(out, "TB57,1,1");
        push_cmd(out, "TB59,0,0");
        push_cmd(out, "TB52,2");
        push_cmd(out, "TB54,0,0");
        push_cmd(out, "TB55,1");
        push_cmd(out, &format!("TB24,{},{}", u(mh), u(mw)));
        push_cmd(out, "TB99");
    }
}

/// The non-interactive registration-mark search commands, emitted inline.
fn push_regmarks_inline(out: &mut Vec<u8>, opt: &JobOptions) {
    let (mw, mh) = opt.media_mm;
    let Some(r) = &opt.regmarks else { return };
    if !r.enabled {
        return;
    }
    let (sw, sh) = r.span(mw, mh);
    match opt.dialect {
        GpglDialect::Community => {
            // automatic registration mark search (community-documented sequence)
            push_cmd(out, "TB99");
            push_cmd(out, "TB55,1");
            push_cmd(out, &format!("TB123,{},{}", u(sh), u(sw)));
        }
        GpglDialect::Graphtec => {
            // MARK / DETECT / EYE_AT search as recorded from the Graphtec GPGL
            // engine. Marks are the top-left square (B), top-right (C) and
            // bottom-left (D). Argument meaning is not fully documented — confirm
            // by a scan. For a live poll-based search use [`encode_staged`].
            let ml = u(r.length);
            let (iy, ix) = (u(r.inset), u(r.inset));
            push_cmd(out, &format!("MARKB{},{}", iy, ix));
            push_cmd(out, &format!("MARKC{},{}", iy, u(mw - r.inset)));
            push_cmd(out, &format!("MARKD{},{}", u(mh - r.inset), ix));
            push_cmd(out, &format!("DETECT {},{},{},{}", ml, ml, u(sh), u(sw)));
            push_cmd(out, &format!("EYE_AT {},{}", iy, ix));
        }
    }
}

/// The cut passes (speed/force/tool + M/D geometry).
fn push_body(out: &mut Vec<u8>, job: &CutJob, opt: &JobOptions) {
    for pass in &job.passes {
        let s = &pass.settings;
        push_cmd(out, &format!("J{}", pass.slot.clamp(1, 2)));
        push_cmd(out, &format!("!{}", s.speed.clamp(1, 10)));
        // Force. The Graphtec engine carries a second (carriage/tool) parameter.
        match opt.dialect {
            GpglDialect::Community => push_cmd(out, &format!("FX{}", s.force.clamp(1, 33))),
            GpglDialect::Graphtec => push_cmd(out, &format!("FX{},0", s.force.clamp(1, 33))),
        }
        if matches!(s.tool, Tool::AutoBlade) {
            // AutoBlade depth (firmware-dependent; ignored by machines without it)
            push_cmd(out, &format!("TF{},{}", s.depth.min(10), pass.slot.clamp(1, 2)));
        }
        // tool offset in 0.01 mm: blades get their tip offset, pens 0
        let off = (s.tool.blade_offset() * 100.0).round() as i64;
        push_cmd(out, &format!("FC{},1,{}", off, pass.slot.clamp(1, 2)));
        for pl in &pass.paths {
            if pl.pts.is_empty() {
                continue;
            }
            let mut pts = pl.pts.clone();
            if pl.closed {
                pts.push(pts[0]);
            }
            let tr = |p: &kurbo::Point| (u(p.y + opt.origin_mm.1), u(p.x + opt.origin_mm.0));
            let (y0, x0) = tr(&pts[0]);
            push_cmd(out, &format!("M{},{}", y0, x0));
            // batch draw coordinates into multi-point D commands
            for chunk in pts[1..].chunks(64) {
                let coords: Vec<String> =
                    chunk.iter().map(|p| { let (y, x) = tr(p); format!("{},{}", y, x) }).collect();
                push_cmd(out, &format!("D{}", coords.join(",")));
            }
        }
    }
}

/// Finish: raise tool, home, optional media feed-out.
fn push_finish(out: &mut Vec<u8>, opt: &JobOptions) {
    push_cmd(out, "&1,1,1");
    push_cmd(out, "TB50,0");
    push_cmd(out, "FO0");
    if opt.feed_after {
        push_cmd(out, &format!("M{},0", u(opt.media_mm.1)));
    }
    push_cmd(out, "H");
}

/// Encode a complete job as a GPGL byte stream (without the ESC init/status handshake).
pub fn encode(job: &CutJob, opt: &JobOptions) -> Vec<u8> {
    let mut out = Vec::new();
    push_setup(&mut out, opt);
    push_regmarks_inline(&mut out, opt);
    push_body(&mut out, job, opt);
    push_finish(&mut out, opt);
    out
}

/// One interactive registration-mark search step.
#[derive(Clone, Debug)]
pub struct MarkStep {
    /// Human-readable label for progress reporting.
    pub label: String,
    /// `DETECT x,y,w,h` command (ETX-terminated) that points the optical head
    /// at this mark's expected position.
    pub detect: Vec<u8>,
}

/// A job split into phases for an interactive Print & Cut: the `setup` bytes are
/// sent first, then each `MarkStep` is run as a DETECT + `mark_status;` poll
/// handshake, then the `body` (the actual cut) is streamed.
#[derive(Clone, Debug)]
pub struct Staged {
    pub setup: Vec<u8>,
    pub marks: Vec<MarkStep>,
    pub body: Vec<u8>,
}

/// Encode a job for the interactive registration-mark workflow. `marks` is
/// populated only when reg-marks are enabled; the body always encodes the full
/// cut so the caller can stream it after the search succeeds.
pub fn encode_staged(job: &CutJob, opt: &JobOptions) -> Staged {
    let mut setup = Vec::new();
    push_setup(&mut setup, opt);

    let mut marks = Vec::new();
    if let Some(r) = &opt.regmarks {
        if r.enabled {
            // Three marks, in the order the documented search visits them:
            // top-left origin, top-right, bottom-left. The search window (w,h) is
            // the mark size in mark units (1016 units/in); x,y is the expected
            // mark position in the same resolution.
            let (mw, mh) = opt.media_mm;
            let w = um(r.length);
            let positions = [
                ("top-left", r.inset, r.inset),
                ("top-right", mw - r.inset, r.inset),
                ("bottom-left", r.inset, mh - r.inset),
            ];
            for (label, x, y) in positions {
                // The documented optical handshake is ';'-terminated ASCII (the
                // reply is `…=4;`), not ETX-terminated like the cut stream.
                let detect = format!("DETECT {},{},{},{};", um(x), um(y), w, w).into_bytes();
                marks.push(MarkStep { label: label.into(), detect });
            }
        }
    }

    let mut body = Vec::new();
    push_body(&mut body, job, opt);
    push_finish(&mut body, opt);

    Staged { setup, marks, body }
}

/// Encode as HP-GL (.plt), 40 units/mm, origin bottom-left.
pub fn encode_hpgl(job: &CutJob, page_h: f64) -> String {
    let mut s = String::from("IN;");
    let t = |p: &kurbo::Point| ((p.x * 40.0).round() as i64, ((page_h - p.y) * 40.0).round() as i64);
    for pass in &job.passes {
        s += &format!("SP{};VS{};FS{};", pass.slot, pass.settings.speed, pass.settings.force);
        for pl in &pass.paths {
            if pl.pts.is_empty() {
                continue;
            }
            let (x, y) = t(&pl.pts[0]);
            s += &format!("PU{},{};", x, y);
            let mut pts: Vec<_> = pl.pts[1..].to_vec();
            if pl.closed {
                pts.push(pl.pts[0]);
            }
            if !pts.is_empty() {
                let c: Vec<String> = pts.iter().map(|p| {
                    let (x, y) = t(p);
                    format!("{},{}", x, y)
                }).collect();
                s += &format!("PD{};", c.join(","));
            }
        }
    }
    s += "PU;SP0;IN;";
    s
}

/// Encode as DMPL (Summa / Houston Instrument / Tenneth). 0.001 inch per step
/// (1000 units/inch), origin bottom-left. Move = ` U x,y;`, cut = `D x,y;`,
/// following the documented DMPL start/move/draw forms.
pub fn encode_dmpl(job: &CutJob, page_h: f64) -> String {
    let steps = 1000.0 / 25.4; // 0.001 inch per step
    let t = |p: &kurbo::Point| ((p.x * steps).round() as i64, ((page_h - p.y) * steps).round() as i64);
    // Start block: ECN (enable) + default units; per pass adds speed/force.
    let mut s = String::from(";:H A L0 ECN U ");
    for pass in &job.passes {
        // US = up-speed, VS = velocity, !FS = force (all emitted verbatim).
        s += &format!("P1;US{};VS{};!FS{};", pass.settings.speed, pass.settings.speed, pass.settings.force);
        for pl in &pass.paths {
            if pl.pts.is_empty() {
                continue;
            }
            let (x, y) = t(&pl.pts[0]);
            s += &format!("U{},{};", x, y); // pen-up move to start
            let mut pts: Vec<_> = pl.pts[1..].to_vec();
            if pl.closed {
                pts.push(pl.pts[0]);
            }
            for p in &pts {
                let (x, y) = t(p);
                s += &format!("D{},{};", x, y); // pen-down draw
            }
        }
    }
    s += "U0,0;"; // raise + home
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutjob::{CutJob, JobPass};
    use crate::geom::Polyline;
    use crate::materials::{Action, CutSettings};
    use kurbo::Point;

    #[test]
    fn encodes_moves_in_plotter_units() {
        let job = CutJob {
            passes: vec![JobPass {
                label: "t".into(),
                action: Action::Cut,
                settings: CutSettings::default(),
                slot: 1,
                paths: vec![Polyline { pts: vec![Point::new(1.0, 2.0), Point::new(3.0, 2.0)], closed: false }],
            }],
        };
        let b = encode(&job, &JobOptions::default());
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains("M40,20\u{3}"));
        assert!(s.contains("D40,60\u{3}"));
        assert_eq!(parse_status(b"0\x03"), Status::Ready);
    }

    #[test]
    fn graphtec_dialect_uses_documented_forms() {
        let job = CutJob {
            passes: vec![JobPass {
                label: "t".into(),
                action: Action::Cut,
                settings: CutSettings::default(),
                slot: 1,
                paths: vec![Polyline { pts: vec![Point::new(1.0, 2.0)], closed: false }],
            }],
        };
        let opt = JobOptions { dialect: GpglDialect::Graphtec, ..JobOptions::default() };
        let b = encode(&job, &opt);
        let s = String::from_utf8_lossy(&b);
        // force carries the second (carriage) parameter in the Graphtec form
        assert!(s.contains(&format!("FX{},0\u{3}", CutSettings::default().force)));
        // documented TB start-cut block is present
        assert!(s.contains("TB57,1,1\u{3}"));
        assert!(s.contains("TB24,"));
    }

    #[test]
    fn staged_job_splits_marks_from_body() {
        let job = CutJob {
            passes: vec![JobPass {
                label: "t".into(),
                action: Action::Cut,
                settings: CutSettings::default(),
                slot: 1,
                paths: vec![Polyline { pts: vec![Point::new(1.0, 2.0), Point::new(3.0, 4.0)], closed: false }],
            }],
        };
        let mut opt = JobOptions { dialect: GpglDialect::Graphtec, ..JobOptions::default() };
        opt.regmarks = Some(crate::regmarks::RegMarks { enabled: true, ..Default::default() });
        let st = encode_staged(&job, &opt);
        assert_eq!(st.marks.len(), 3);
        // marks are ';'-terminated DETECT commands, not ETX, and not in the body
        assert!(st.marks[0].detect.starts_with(b"DETECT "));
        assert!(st.marks[0].detect.ends_with(b";"));
        assert!(!st.marks[0].detect.contains(&ETX));
        let body = String::from_utf8_lossy(&st.body);
        assert!(body.contains("M40,20") && body.contains('D') && body.contains("H\u{3}"));
        assert!(!body.contains("DETECT"));
    }

    #[test]
    fn dmpl_encodes_move_and_draw() {
        let job = CutJob {
            passes: vec![JobPass {
                label: "t".into(),
                action: Action::Cut,
                settings: CutSettings::default(),
                slot: 1,
                paths: vec![Polyline { pts: vec![Point::new(0.0, 0.0), Point::new(25.4, 0.0)], closed: false }],
            }],
        };
        let s = encode_dmpl(&job, 100.0);
        assert!(s.starts_with(";:H A L0 ECN U "));
        assert!(s.contains("U0,")); // pen-up move to start
        assert!(s.contains("D1000,")); // 25.4 mm = 1000 steps
        assert!(s.ends_with("U0,0;"));
    }

    #[test]
    fn mark_status_parsing() {
        assert_eq!(parse_mark_status(b"RM=4;"), Some(MarkStatus::Found));
        assert_eq!(parse_mark_status(b"=5;"), Some(MarkStatus::Failed));
        assert_eq!(parse_mark_status(b"=1;"), Some(MarkStatus::Pending(1)));
        assert_eq!(parse_mark_status(b"=4"), None); // incomplete (no ';')
    }
}
