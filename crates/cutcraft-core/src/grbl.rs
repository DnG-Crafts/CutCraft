//! GRBL: G-code over a serial port, for hobby CNC drag knives, pen plotters
//! and diode lasers (GRBL 1.1, grblHAL, FluidNC and other GRBL-speaking
//! controllers).
//!
//! [`encode`] turns a cut job into G-code lines. [`GrblLink`] keeps a serial
//! connection open on its own thread: it polls the machine's status, streams a
//! job using GRBL's character-counting flow control (never more than the
//! controller's 128-byte receive buffer is waiting), sends jog and console
//! commands, and stops safely (feed hold, then reset).
//!
//! The Cricut Maker 3 running cutcutgo speaks GRBL with differences that
//! matter, so it has its own [`Dialect`] and encoder ([`encode_maker3`]),
//! and Print & Cut by scanning a printed box with its light sensor
//! ([`GrblLink::print_cut`]).

use crate::cutjob::CutJob;
use crate::device::Progress;
use kurbo::Point;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ G-code

/// How the machine puts its tool down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Head {
    /// A Z axis moves the knife or pen up and down.
    #[default]
    ZAxis,
    /// A hobby servo lifts the pen, driven by the spindle output (M3 S…).
    Servo,
    /// A laser, switched on for each line (M4 S…, dynamic power).
    Laser,
}

impl Head {
    pub const ALL: [(Head, &'static str); 3] = [(Head::ZAxis, "Z axis (knife / pen)"), (Head::Servo, "Servo pen lift"), (Head::Laser, "Laser")];
}

/// Settings for a GRBL machine, kept in the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrblOptions {
    /// Serial port (e.g. `COM3`, `/dev/ttyUSB0`) and speed.
    pub port: String,
    pub baud: u32,
    /// Connect by itself with these settings: shortly after CutCraft starts,
    /// and whenever the machine's port appears (switched on later).
    pub auto_connect: bool,
    pub head: Head,
    /// Z heights, mm: travelling and cutting.
    pub z_up: f64,
    pub z_down: f64,
    /// How fast Z goes down, mm/min.
    pub plunge_feed: f64,
    /// Servo positions (S values) and how long to wait for the servo, ms.
    pub servo_up: f64,
    pub servo_down: f64,
    pub servo_wait_ms: u32,
    /// The controller's S value for full power ($30, usually 1000).
    pub s_max: f64,
    /// Each group's Speed sets the feed (Speed × `feed_per_speed` mm/min);
    /// off: always `feed`.
    pub feed_from_speed: bool,
    pub feed_per_speed: f64,
    pub feed: f64,
    /// Origin at the front-left of the bed with Y going away from you (most
    /// CNC machines); off: top-left with Y coming towards you, like the page.
    pub origin_bottom_left: bool,
    /// Go back to X0 Y0 at the end of the job.
    pub return_to_origin: bool,
}

impl Default for GrblOptions {
    fn default() -> Self {
        GrblOptions {
            port: String::new(),
            baud: 115200,
            auto_connect: false,
            head: Head::ZAxis,
            z_up: 3.0,
            z_down: -1.0,
            plunge_feed: 300.0,
            servo_up: 50.0,
            servo_down: 30.0,
            servo_wait_ms: 150,
            s_max: 1000.0,
            feed_from_speed: true,
            feed_per_speed: 300.0,
            feed: 1500.0,
            origin_bottom_left: true,
            return_to_origin: true,
        }
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The job as G-code lines (no line endings). `max_force` is the machine's
/// Force range: in laser mode a group's Force is its power, as a share of it.
pub fn encode(job: &CutJob, page_height: f64, max_force: u8, o: &GrblOptions) -> Vec<String> {
    let mut g = vec!["; CutCraft G-code for GRBL".to_string(), "G21 G90 G17 G94".into()];
    let y = |v: f64| if o.origin_bottom_left { page_height - v } else { v };
    let up = |g: &mut Vec<String>| match o.head {
        Head::ZAxis => g.push(format!("G0 Z{}", num(o.z_up))),
        Head::Servo => {
            g.push(format!("M3 S{}", num(o.servo_up)));
            g.push(format!("G4 P{}", num(o.servo_wait_ms as f64 / 1000.0)));
        }
        Head::Laser => g.push("M5".into()),
    };
    match o.head {
        Head::Laser => g.push("M5".into()),
        _ => up(&mut g),
    }
    for pass in &job.passes {
        let s = &pass.settings;
        let feed = if o.feed_from_speed { s.speed.max(1) as f64 * o.feed_per_speed } else { o.feed };
        let power = (s.force as f64 / max_force.max(1) as f64).clamp(0.0, 1.0) * o.s_max;
        g.push(format!("; {}", pass.label));
        for pl in &pass.paths {
            let Some(first) = pl.pts.first() else { continue };
            g.push(format!("G0 X{} Y{}", num(first.x), num(y(first.y))));
            match o.head {
                Head::ZAxis => g.push(format!("G1 Z{} F{}", num(o.z_down), num(o.plunge_feed))),
                Head::Servo => {
                    g.push(format!("M3 S{}", num(o.servo_down)));
                    g.push(format!("G4 P{}", num(o.servo_wait_ms as f64 / 1000.0)));
                }
                Head::Laser => g.push(format!("M4 S{}", num(power))),
            }
            let mut feed_set = false;
            let mut pts: Vec<_> = pl.pts[1..].to_vec();
            if pl.closed && pl.pts.len() > 2 && pl.pts.last() != Some(first) {
                pts.push(*first);
            }
            for p in pts {
                if feed_set {
                    g.push(format!("G1 X{} Y{}", num(p.x), num(y(p.y))));
                } else {
                    g.push(format!("G1 X{} Y{} F{}", num(p.x), num(y(p.y)), num(feed)));
                    feed_set = true;
                }
            }
            up(&mut g);
        }
    }
    if o.head == Head::Servo {
        g.push("M5".into());
    }
    if o.return_to_origin {
        g.push("G0 X0 Y0".into());
    }
    g
}

/// Trace the job's outer box with the tool up (laser off), so you can check
/// where it will land on the material. None for an empty job.
pub fn frame(job: &CutJob, page_height: f64, o: &GrblOptions) -> Option<Vec<String>> {
    let pts = job.passes.iter().flat_map(|p| &p.paths).flat_map(|pl| &pl.pts);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in pts {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    if x0 > x1 {
        return None;
    }
    let y = |v: f64| if o.origin_bottom_left { page_height - v } else { v };
    let mut g = vec!["G21 G90".to_string(), tool_up(o)];
    for (px, py) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)] {
        g.push(format!("G0 X{} Y{}", num(px), num(y(py))));
    }
    Some(g)
}

/// The tool-up command on its own, for the Tool up button.
pub fn tool_up(o: &GrblOptions) -> String {
    match o.head {
        Head::ZAxis => format!("G0 Z{}", num(o.z_up)),
        Head::Servo => format!("M3 S{}", num(o.servo_up)),
        Head::Laser => "M5".into(),
    }
}

// ------------------------------------------------------------------ Cricut Maker 3

/// The machine id of the Cricut Maker 3 running cutcutgo.
pub const MAKER3_ID: &str = "maker3_cutcutgo";
/// Its USB serial port's vendor and product id.
pub const MAKER3_USB: (u16, u16) = (0x04D8, 0x000A);

/// The machine id of the Cricut Explore 3 running cutcutgo.
pub const EXPLORE3_ID: &str = "explore3_cutcutgo";

/// Whether `machine` is a Cricut on the cutcutgo firmware (a Maker 3 or an
/// Explore 3): same protocol, same rules, a few differences in
/// [`CricutModel`].
pub fn is_maker3(machine: &str) -> bool {
    cricut_model(machine).is_some()
}

/// Which Cricut on cutcutgo `machine` is.
pub fn cricut_model(machine: &str) -> Option<CricutModel> {
    match machine {
        MAKER3_ID => Some(CricutModel::Maker3),
        EXPLORE3_ID => Some(CricutModel::Explore3),
        _ => None,
    }
}

/// Park the carriage so the operator can change the pen or blade, on the
/// Explore 3 (its clamps are steppers, and it has no steering to turn off).
pub const EXPLORE3_LOAD_TOOL: [&str; 7] = ["G90 G21", "T1", "G0 Z0", "T0", "G0 Z0", "G0 X150", "G4 P0"];

/// The Cricuts on cutcutgo, and how they differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CricutModel {
    #[default]
    Maker3,
    /// Two clamps (pen, fine-point blade) on small steppers: no drive
    /// housing and no tool scan, no homing at power-on, clamps down to
    /// Z-8.5. Print & Cut works as on the Maker 3, with its own sensor.
    Explore3,
}

impl CricutModel {
    pub fn label(self) -> &'static str {
        match self {
            CricutModel::Maker3 => "Cricut Maker 3",
            CricutModel::Explore3 => "Cricut Explore 3",
        }
    }

    /// How wide the mat's work area is: X0 to X301 on both (the Explore's
    /// 12 x 12 mat centre measured at X150.5).
    pub fn work_x(self) -> f64 {
        MAKER3_WORK_X
    }

    /// How far each tool's carriage can go in X.
    pub fn reach(self, tool: Option<u8>) -> (f64, f64) {
        match self {
            CricutModel::Maker3 => maker3_reach(tool),
            CricutModel::Explore3 => explore3_reach(tool),
        }
    }

    /// Where the light sensor is from the pen: it's over pen point (x, y)
    /// when the machine is at (x + dx, y + dy).
    pub fn sensor_offset(self) -> (f64, f64) {
        match self {
            CricutModel::Maker3 => SENSOR_OFFSET,
            CricutModel::Explore3 => EXPLORE3_SENSOR_OFFSET,
        }
    }

    /// The sensor's reach in pen X: the carriage's travel less the sensor's
    /// offset.
    pub fn sensor_x_max(self) -> f64 {
        match self {
            CricutModel::Maker3 => SENSOR_X_MAX,
            CricutModel::Explore3 => 335.0 - EXPLORE3_SENSOR_OFFSET.0,
        }
    }

    /// The lowest height for holder 0 (A) or 1 (B).
    pub fn z_min(self, holder: u8) -> f64 {
        match self {
            CricutModel::Maker3 => maker3_z_min(holder),
            // the firmware stops the clamps there, without telling Grbl
            CricutModel::Explore3 => -8.5,
        }
    }

    /// Holder B takes drive-housing tools, scanned with M3.
    pub fn drive_tools(self) -> bool {
        self == CricutModel::Maker3
    }

    /// The registration sensor works (`#YE`), for Print & Cut: on both.
    pub fn print_cut(self) -> bool {
        true
    }

    /// Homes itself at power-on; the Explore 3 needs `$H` first.
    pub fn homes_at_power_on(self) -> bool {
        self == CricutModel::Maker3
    }

    pub fn load_tool(self) -> &'static [&'static str] {
        match self {
            CricutModel::Maker3 => &MAKER3_LOAD_TOOL,
            CricutModel::Explore3 => &EXPLORE3_LOAD_TOOL,
        }
    }
}

/// The pen holder's lowest height: below this its hard stop raises a stall.
pub const MAKER3_Z_MIN: f64 = -9.5;
/// Holder B can go down to the end of its travel.
pub const MAKER3_Z_MIN_B: f64 = -10.0;

/// The lowest height for holder 0 (A, pen) or 1 (B).
pub fn maker3_z_min(holder: u8) -> f64 {
    if holder == 0 { MAKER3_Z_MIN } else { MAKER3_Z_MIN_B }
}

const TOOL_FEED: f64 = 300.0;
const TRAVEL_FEED: f64 = 1500.0;
/// Very low feeds make Y twitch; the firmware limits anything over 1500.
pub const MAKER3_FEED_MIN: f64 = 300.0;
pub const MAKER3_FEED_MAX: f64 = 1500.0;
/// The mat's work area runs from X0 (its right edge) to X301 (its left
/// edge), in either tool's coordinates: the firmware shifts the blade
/// holder's X/Y itself (v75 and later), so the sender adds no tool offset.
pub const MAKER3_WORK_X: f64 = 301.0;
/// How far the mat may be fed back out: the feed button's own eject position.
pub const MAKER3_Y_MIN: f64 = -50.0;
/// Where the carriage parks (in the pen's numbers) so holder B can be reached.
pub const MAKER3_PARK_X: f64 = 150.0;
/// The knife blade's cutting tip trails its turning centre by this much.
pub const KNIFE_TRAIL: f64 = 3.0;
/// A steered tool turns in the material for changes of direction up to this
/// much; past it the cut is lifted and started again.
const STEER_IN_MATERIAL_DEG: f64 = 10.0;
const SLACK: f64 = 0.0005;

/// Park the carriage so the operator can reach holder B: both holders up,
/// steering off, the carriage to the middle (sent with the pen selected).
pub const MAKER3_LOAD_TOOL: [&str; 8] = ["G90 G21", "T1", "G1 Z0 F300", "M5", "T0", "G1 Z0 F300", "G0 X150", "G4 P0"];
/// Identify the drive-housing tool in holder B: home, scan it with `M3`
/// (which reports `[MSG:Tool <name>, ...]`), steering off, park.
pub const MAKER3_DETECT_TOOL: [&str; 14] = ["G90 G21", "T0", "G1 Z0 F300", "T1", "G1 Z0 F300", "M5", "$H", "T1", "M3", "M5", "T0", "G1 Z0 F300", "G0 X150", "G4 P0"];

/// How far each tool's carriage can go in X: the pen 0 to 340, the blade
/// -39 to 301. The pen holder is on the left of the carriage and the blade
/// holder on the right, and the firmware shifts the blade's coordinates by
/// the 39 mm between them, so both ends are the same carriage positions:
/// at 340 the blade is over the work area's left edge (and the pen 39 mm
/// past it), at 0 the pen is over its right edge. With the tool unknown,
/// only where both can go.
/// The Explore 3's X reach: the pen 0 to 335 (at 335 clamp B is over the
/// mat's left edge), the blade -37.6 to 297.4 (the firmware shifts clamp B's
/// coordinates by the 37.6 mm between the clamps). Tool unknown: where both
/// can go.
pub fn explore3_reach(tool: Option<u8>) -> (f64, f64) {
    match tool {
        Some(0) => (0.0, 335.0),
        Some(_) => (-37.6, 297.4),
        None => (0.0, 297.4),
    }
}

pub fn maker3_reach(tool: Option<u8>) -> (f64, f64) {
    match tool {
        Some(0) => (0.0, 340.0),
        Some(_) => (-39.0, 301.0),
        None => (0.0, 301.0),
    }
}

/// A tool for holder B's drive housing (the ones with a gear on top). The
/// machine identifies it from its reflective spots when `M3` scans it. The
/// fine-point blade has no gear or spots, so it's never scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveTool {
    Rotary,
    Knife,
    /// 01 single or 02 double: they have the same spots.
    Score,
    Perf11,
    Wavy31,
    Deboss21,
    Engrave41,
}

impl DriveTool {
    pub const ALL: [DriveTool; 7] = [
        DriveTool::Rotary,
        DriveTool::Knife,
        DriveTool::Score,
        DriveTool::Perf11,
        DriveTool::Wavy31,
        DriveTool::Deboss21,
        DriveTool::Engrave41,
    ];

    pub fn label(self) -> &'static str {
        self.tool().label()
    }

    /// The name the firmware reports after `M3`: `[MSG:Tool <name>, ...]`.
    pub fn reported(self) -> &'static str {
        match self {
            DriveTool::Rotary => "rotary blade",
            DriveTool::Knife => "knife blade",
            DriveTool::Score => "01/02 scoring wheel",
            DriveTool::Perf11 => "11 perforation",
            DriveTool::Wavy31 => "31 wavy blade",
            DriveTool::Deboss21 => "21 fine debossing",
            DriveTool::Engrave41 => "41 engraving",
        }
    }

    pub fn tool(self) -> crate::materials::Tool {
        use crate::materials::Tool;
        match self {
            DriveTool::Rotary => Tool::RotaryBlade,
            DriveTool::Knife => Tool::KnifeBlade,
            DriveTool::Score => Tool::ScoringWheel,
            DriveTool::Perf11 => Tool::PerforationBlade,
            DriveTool::Wavy31 => Tool::WavyBlade,
            DriveTool::Deboss21 => Tool::DebossingTip,
            DriveTool::Engrave41 => Tool::EngravingTip,
        }
    }

    pub fn from_tool(t: crate::materials::Tool) -> Option<DriveTool> {
        DriveTool::ALL.into_iter().find(|d| d.tool() == t)
    }

    /// The tool a report names.
    pub fn from_reported(name: &str) -> Option<DriveTool> {
        DriveTool::ALL.into_iter().find(|d| d.reported() == name)
    }
}

/// Settings for the Maker 3. Heights change whenever a tool is re-seated.
/// Defaults are the ones worked out on copy paper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Maker3Options {
    pub pen_z: f64,
    pub blade_z: f64,
    pub pen_feed: f64,
    pub blade_feed: f64,
    /// Holder B's drive-housing tools: height and speed for each.
    pub rotary_z: f64,
    pub rotary_feed: f64,
    pub knife_z: f64,
    pub knife_feed: f64,
    /// Both scoring wheels (01 single, 02 double).
    pub score_z: f64,
    pub score_feed: f64,
    pub perf_z: f64,
    pub perf_feed: f64,
    pub wavy_z: f64,
    pub wavy_feed: f64,
    pub deboss_z: f64,
    pub deboss_feed: f64,
    pub engrave_z: f64,
    pub engrave_feed: f64,
    /// Shifts plain cuts on the mat, mm, if the mat doesn't sit quite where
    /// it's expected.
    pub shift_x: f64,
    pub shift_y: f64,
    /// A further correction for Print & Cut, mm in the machine's directions,
    /// on top of [`MAKER3_PC_OFFSET`], if cuts always land off the print the
    /// same way. (A new name, so a correction saved before that offset was
    /// built in isn't added to it.)
    pub pc_nudge_x: f64,
    pub pc_nudge_y: f64,
    /// Turn the design half a turn (if a test F comes out upside down).
    pub rotate_180: bool,
    /// The holder last in use (0 pen, 1 holder B), chosen again on connecting.
    pub active_holder: u8,
    /// What's in holder B (None: the fine-point blade): tools stay fitted
    /// between sessions.
    pub holder_b: Option<DriveTool>,
    /// Which machine these settings are for (set when they're fetched).
    #[serde(skip)]
    pub model: CricutModel,
}

impl Default for Maker3Options {
    fn default() -> Self {
        Maker3Options {
            pen_z: -4.0,
            blade_z: -9.0,
            pen_feed: 600.0,
            blade_feed: 450.0,
            // fabric only: on paper it just marks
            rotary_z: -10.0,
            rotary_feed: 300.0,
            knife_z: -9.5,
            knife_feed: 300.0,
            score_z: -9.5,
            score_feed: 300.0,
            perf_z: -9.0,
            perf_feed: 300.0,
            wavy_z: -9.0,
            wavy_feed: 300.0,
            deboss_z: -9.0,
            deboss_feed: 300.0,
            engrave_z: -9.0,
            engrave_feed: 300.0,
            shift_x: 0.0,
            shift_y: 0.0,
            pc_nudge_x: 0.0,
            pc_nudge_y: 0.0,
            rotate_180: false,
            active_holder: 0,
            holder_b: None,
            model: CricutModel::Maker3,
        }
    }
}

/// Where Print & Cut cuts are moved to land on the print, machine mm (+X is
/// the page's left, +Y down the page): 0.35 mm left and 0.45 mm down, found
/// by test cuts on a real machine (2026-11-14), from the sensor and blade
/// offsets and the blade's drag together.
pub const MAKER3_PC_OFFSET: (f64, f64) = (0.35, 0.45);

impl Maker3Options {
    /// The whole Print & Cut correction: the built-in offset plus any nudge.
    pub fn pc_offset(&self) -> kurbo::Vec2 {
        // measured on a Maker 3; an Explore 3 starts from none
        let base = if self.model == CricutModel::Maker3 { MAKER3_PC_OFFSET } else { (0.0, 0.0) };
        kurbo::Vec2::new(base.0 + self.pc_nudge_x, base.1 + self.pc_nudge_y)
    }
}

impl Maker3Options {
    /// Starting settings for the Explore 3: a fine pen drew solid lines at
    /// -4.5, and the fine-point blade cut copy paper cleanly at -8.
    pub fn explore3() -> Self {
        Maker3Options { pen_z: -4.5, blade_z: -8.0, model: CricutModel::Explore3, ..Default::default() }
    }

    /// The work area for the page's mat on this machine: (width, length) mm.
    pub fn bed(&self, mat: crate::doc::MatKind) -> (f64, f64) {
        (self.model.work_x(), maker3_bed(mat).1)
    }
}

/// The work area for the page's mat: (width, length) in mm. The machine takes
/// 12 x 12 in and 12 x 24 in mats; the work area is 301 mm wide.
pub fn maker3_bed(mat: crate::doc::MatKind) -> (f64, f64) {
    (MAKER3_WORK_X, if mat == crate::doc::MatKind::Mat12x24 { 609.6 } else { 304.8 })
}

/// What's doing the work: the pen, the fine-point blade, or a drive tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maker3Tool {
    Pen,
    FinePoint,
    Drive(DriveTool),
}

impl Maker3Tool {
    /// 0 for holder A (`T0`), 1 for holder B (`T1`).
    pub fn holder(self) -> u8 {
        if self == Maker3Tool::Pen { 0 } else { 1 }
    }

    pub fn label(self) -> &'static str {
        match self {
            Maker3Tool::Pen => "pen",
            Maker3Tool::FinePoint => "fine-point blade",
            Maker3Tool::Drive(d) => d.label(),
        }
    }
}

impl Maker3Options {
    /// Page mm (from the mat's top-left corner as you face the machine) to
    /// machine mm, the same for both tools: X = 301 - x, Y = y. +X moves the
    /// carriage left and +Y feeds the mat in.
    pub fn to_machine(&self, p: Point, bed: (f64, f64)) -> Point {
        let (cx, cy) = (bed.0 / 2.0, bed.1 / 2.0);
        let (mut dx, mut dy) = (p.x - cx, p.y - cy);
        if self.rotate_180 {
            (dx, dy) = (-dx, -dy);
        }
        Point::new(cx - dx + self.shift_x, cy + dy + self.shift_y)
    }

    /// A tool's height and speed.
    pub fn setting(&self, t: Maker3Tool) -> (f64, f64) {
        match t {
            Maker3Tool::Pen => (self.pen_z, self.pen_feed),
            Maker3Tool::FinePoint => (self.blade_z, self.blade_feed),
            Maker3Tool::Drive(d) => match d {
                DriveTool::Rotary => (self.rotary_z, self.rotary_feed),
                DriveTool::Knife => (self.knife_z, self.knife_feed),
                DriveTool::Score => (self.score_z, self.score_feed),
                DriveTool::Perf11 => (self.perf_z, self.perf_feed),
                DriveTool::Wavy31 => (self.wavy_z, self.wavy_feed),
                DriveTool::Deboss21 => (self.deboss_z, self.deboss_feed),
                DriveTool::Engrave41 => (self.engrave_z, self.engrave_feed),
            },
        }
    }

    /// A tool's height and speed, to change.
    pub fn setting_mut(&mut self, t: Maker3Tool) -> (&mut f64, &mut f64) {
        match t {
            Maker3Tool::Pen => (&mut self.pen_z, &mut self.pen_feed),
            Maker3Tool::FinePoint => (&mut self.blade_z, &mut self.blade_feed),
            Maker3Tool::Drive(d) => match d {
                DriveTool::Rotary => (&mut self.rotary_z, &mut self.rotary_feed),
                DriveTool::Knife => (&mut self.knife_z, &mut self.knife_feed),
                DriveTool::Score => (&mut self.score_z, &mut self.score_feed),
                DriveTool::Perf11 => (&mut self.perf_z, &mut self.perf_feed),
                DriveTool::Wavy31 => (&mut self.wavy_z, &mut self.wavy_feed),
                DriveTool::Deboss21 => (&mut self.deboss_z, &mut self.deboss_feed),
                DriveTool::Engrave41 => (&mut self.engrave_z, &mut self.engrave_feed),
            },
        }
    }

    /// The machine has no soft limits, so CutCraft keeps each move on the
    /// mat's work area and inside the tool's reach.
    fn check_xy(&self, p: Point, bed: (f64, f64), tool: u8) -> Result<(), String> {
        let name = if tool == 0 { "pen" } else { "blade" };
        let (lo, hi) = self.model.reach(Some(tool));
        let (lo, hi) = (lo.max(0.0), hi.min(bed.0));
        if p.x < lo - SLACK || p.x > hi + SLACK {
            return Err(format!("That would take the {name} to X {:.1}, off the mat's work area (X {lo:.0} to {hi:.0}). Move the design onto the mat, inside {:.0} mm from its left edge.", p.x, bed.0));
        }
        if p.y < -SLACK || p.y > bed.1 + SLACK {
            return Err(format!("That would take the {name} to Y {:.1}, off the {:.0} mm mat. Move the design onto the mat, or check Page setup has the right mat.", p.y, bed.1));
        }
        Ok(())
    }

    fn check_z(&self, z: f64, holder: u8) -> Result<(), String> {
        let min = self.model.z_min(holder);
        if z < min - SLACK || z > SLACK {
            return Err(format!("Z {z:.1} mm is outside the tool holder's travel (0 to {min})."));
        }
        Ok(())
    }
}

/// What a pass is done with. The firmware can't tell a fine-point blade from
/// an empty holder, so the group's Tool decides; drive-housing tools are
/// checked by the machine when the job starts.
fn maker3_tool(pass: &crate::cutjob::JobPass) -> Result<Maker3Tool, String> {
    use crate::materials::{Action, Tool};
    let t = pass.settings.tool;
    if let Some(d) = DriveTool::from_tool(t) {
        return Ok(Maker3Tool::Drive(d));
    }
    if t == Tool::SketchPen || pass.action == Action::Sketch {
        return Ok(Maker3Tool::Pen);
    }
    if matches!(t, Tool::AutoBlade | Tool::RatchetBlade | Tool::DeepCutBlade) && matches!(pass.action, Action::Cut | Action::Perforate) {
        return Ok(Maker3Tool::FinePoint);
    }
    Err(format!("“{}”: the Maker 3 can't do {} with the {}. Use the pen, the fine-point blade or one of holder B's tools.", pass.label, pass.action.label(), t.label()))
}

/// Marks each pass with the holder it uses (A = 1, B = 2) so that a job only
/// splits where a tool has to be swapped by hand: all of holder B's tools
/// share it.
pub fn maker3_holders(mut job: CutJob) -> Result<CutJob, String> {
    for pass in &mut job.passes {
        let t = maker3_tool(pass)?;
        pass.slot = t.holder() + 1;
        if t == Maker3Tool::Pen {
            pass.settings.tool = crate::materials::Tool::SketchPen;
        }
    }
    Ok(job)
}

fn maker3_tool_change(g: &mut Vec<String>, tool: u8) {
    // T doesn't lift anything, and each holder keeps its own height
    g.push(format!("G1 Z0 F{}", num(TOOL_FEED)));
    g.push(format!("T{tool}"));
    g.push(format!("G1 Z0 F{}", num(TOOL_FEED)));
}

/// The comment line that tells the link which tool the next `M3` should find.
pub const EXPECT_TOOL: &str = "; expect tool ";

/// Split a path into the knife's cuts and move each forward by its trailing
/// tip, in machine mm. A change of direction of more than a few degrees
/// ends one cut and starts the next (lifted, so the knife turns in the air).
pub fn knife_cuts(pts: &[Point], trail: f64) -> Vec<Vec<Point>> {
    let mut p: Vec<Point> = vec![];
    for q in pts {
        if p.last().is_none_or(|l| l.distance(*q) > 1e-6) {
            p.push(*q);
        }
    }
    if p.len() < 2 {
        return vec![p];
    }
    let dir = |a: Point, b: Point| (b - a).normalize();
    let limit = STEER_IN_MATERIAL_DEG.to_radians().cos();
    let mut runs: Vec<Vec<Point>> = vec![vec![p[0]]];
    for i in 1..p.len() {
        runs.last_mut().unwrap().push(p[i]);
        if i + 1 < p.len() && dir(p[i - 1], p[i]).dot(dir(p[i], p[i + 1])) < limit {
            runs.push(vec![p[i]]);
        }
    }
    runs.into_iter()
        .map(|r| {
            let n = r.len();
            (0..n)
                .map(|i| {
                    let d = if i == 0 {
                        dir(r[0], r[1])
                    } else if i == n - 1 {
                        dir(r[n - 2], r[n - 1])
                    } else {
                        (dir(r[i - 1], r[i]) + dir(r[i], r[i + 1])).normalize()
                    };
                    r[i] + d * trail
                })
                .collect()
        })
        .collect()
}

/// The job as G-code for the Maker 3, or why it can't run: every move is
/// checked against the mat, since the machine won't check it itself.
pub fn encode_maker3(job: &CutJob, bed: (f64, f64), o: &Maker3Options) -> Result<Vec<String>, String> {
    encode_maker3_mapped(job, bed, o, &|p| o.to_machine(p, bed))
}

/// Like [`encode_maker3`], with `map` taking page mm to machine mm (for
/// Print & Cut, the transform fitted to the printed marks).
pub fn encode_maker3_mapped(job: &CutJob, bed: (f64, f64), o: &Maker3Options, map: &dyn Fn(Point) -> Point) -> Result<Vec<String>, String> {
    // the Explore 3 doesn't home at power-on, and its clamps lose steps
    // silently: it homes for every job
    let home = if o.model.homes_at_power_on() { "$HZ" } else { "$H" };
    let mut g = vec![home.to_string(), "G90 G21".into()];
    let mut cur: Option<Maker3Tool> = None;
    let mut steering = false;
    let go = |g: &mut Vec<String>, m: Point, t: u8, feed: f64| -> Result<(), String> {
        o.check_xy(m, bed, t)?;
        g.push(format!("G1 X{} Y{} F{}", num(m.x), num(m.y), num(feed)));
        Ok(())
    };
    let lift = format!("G1 Z0 F{}", num(TOOL_FEED));
    for pass in job.passes.iter().filter(|p| !p.paths.is_empty()) {
        let t = maker3_tool(pass)?;
        if matches!(t, Maker3Tool::Drive(_)) && !o.model.drive_tools() {
            return Err(format!("“{}”: the {} has no drive housing, so it can only use the pen and the fine-point blade.", pass.label, o.model.label()));
        }
        let h = t.holder();
        if cur != Some(t) {
            if steering {
                g.push("M5".into());
                steering = false;
            }
            match t {
                Maker3Tool::Drive(d) => {
                    // the scan needs X freshly homed, and only happens with
                    // steering off; it reports the tool before its ok, and if
                    // none is found ALARM:9 follows the ok
                    g.push(lift.clone());
                    g.push("$H".into());
                    g.push("T1".into());
                    g.push(lift.clone());
                    g.push("M5".into());
                    g.push(format!("{EXPECT_TOOL}{}", d.reported()));
                    g.push("M3".into());
                    g.push("G4 P0".into());
                    steering = true;
                }
                _ if cur.map(|c| c.holder()) != Some(h) => maker3_tool_change(&mut g, h),
                _ => {}
            }
            cur = Some(t);
        }
        let (z, feed) = o.setting(t);
        o.check_z(z, h)?;
        let feed = feed.clamp(MAKER3_FEED_MIN, MAKER3_FEED_MAX);
        for pl in &pass.paths {
            if pl.pts.is_empty() {
                continue;
            }
            let mut pts: Vec<Point> = pl.pts.iter().map(|p| map(*p)).collect();
            if pl.closed && pts.len() > 2 && pts.last() != pts.first() {
                pts.push(pts[0]);
            }
            let cuts = if t == Maker3Tool::Drive(DriveTool::Knife) { knife_cuts(&pts, KNIFE_TRAIL) } else { vec![pts] };
            for c in cuts {
                go(&mut g, c[0], h, TRAVEL_FEED)?;
                g.push(format!("G1 Z{} F{}", num(z), num(TOOL_FEED)));
                for p in &c[1..] {
                    go(&mut g, *p, h, feed)?;
                }
                g.push(lift.clone());
            }
        }
    }
    if steering {
        g.push("M5".into());
    }
    // finish at Y0 so the mat ejects cleanly, parked (with the pen's numbers)
    // where holder B can be reached; G4 P0's ok means everything's done
    let ended_on_b = cur.is_some_and(|c| c.holder() == 1);
    if ended_on_b {
        g.push("T0".into());
        g.push(lift);
    }
    g.push(format!("G1 X{} Y0 F{}", num(MAKER3_PARK_X), num(TRAVEL_FEED)));
    // and back to the holder it ended on (T moves nothing; both are up)
    if ended_on_b {
        g.push("T1".into());
    }
    g.push("G4 P0".into());
    Ok(g)
}

/// The tools a job uses, in order.
pub fn maker3_tools(job: &CutJob) -> Result<Vec<Maker3Tool>, String> {
    let mut v = vec![];
    for p in job.passes.iter().filter(|p| !p.paths.is_empty()) {
        let t = maker3_tool(p)?;
        if !v.contains(&t) {
            v.push(t);
        }
    }
    Ok(v)
}

/// A tool name as the machine reports it, the way CutCraft shows it.
pub fn reported_label(name: &str) -> String {
    match DriveTool::from_reported(name) {
        Some(d) => d.label().to_lowercase(),
        None => name.to_string(),
    }
}

/// Trace the job's outer box with the tool up, for the job's first tool.
pub fn maker3_frame(job: &CutJob, bed: (f64, f64), o: &Maker3Options) -> Result<Vec<String>, String> {
    let Some(pass) = job.passes.iter().find(|p| !p.paths.is_empty()) else { return Err("Nothing to cut - nothing to frame".into()) };
    let tool = maker3_tool(pass)?.holder();
    let mut pts = job.passes.iter().flat_map(|p| &p.paths).flat_map(|pl| &pl.pts).map(|p| o.to_machine(*p, bed));
    let Some(first) = pts.next() else { return Err("Nothing to cut - nothing to frame".into()) };
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in pts {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    o.check_xy(Point::new(x0, y0), bed, tool)?;
    o.check_xy(Point::new(x1, y1), bed, tool)?;
    let mut g = vec!["G90 G21".to_string(), format!("G1 Z0 F{}", num(TOOL_FEED)), format!("T{tool}"), format!("G1 Z0 F{}", num(TOOL_FEED))];
    for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)] {
        g.push(format!("G1 X{} Y{} F{}", num(x), num(y), num(TRAVEL_FEED)));
    }
    Ok(g)
}

/// A relative move from `mpos` with `tool` selected, if it stays inside that
/// tool's reach and the mat. Uses the forms that have been run on this
/// machine, rather than `$J=`.
pub fn maker3_jog(model: CricutModel, bed: (f64, f64), tool: Option<u8>, mpos: [f64; 3], d: [f64; 3]) -> Result<Vec<String>, String> {
    let (x, y, z) = (mpos[0] + d[0], mpos[1] + d[1], mpos[2] + d[2]);
    let (lo, hi) = model.reach(tool);
    if x < lo - SLACK || x > hi + SLACK {
        return Err(format!("That would reach X {x:.1}, outside this tool's reach (X {lo:.0} to {hi:.0})."));
    }
    if y < MAKER3_Y_MIN - SLACK || y > bed.1 + SLACK {
        return Err(format!("That would reach Y {y:.1}, outside the mat ({MAKER3_Y_MIN:.0} to {:.0}).", bed.1));
    }
    let zmin = model.z_min(tool.unwrap_or(0));
    if z < zmin - SLACK || z > SLACK {
        return Err(format!("That would reach Z {z:.1}, outside the tool holder's travel (0 to {zmin})."));
    }
    if d[2] < 0.0 {
        maker3_can_lower(bed, [x, y, z])?;
    }
    let mut mv = "G1".to_string();
    for (axis, v) in ["X", "Y", "Z"].iter().zip(d) {
        if v != 0.0 {
            mv += &format!(" {axis}{}", num(v));
        }
    }
    let feed = if d[2] != 0.0 { TOOL_FEED } else { TRAVEL_FEED };
    Ok(vec!["G91 G21".into(), format!("{mv} F{}", num(feed)), "G90".into()])
}

/// Lift, change to `tool`, and lift that one too.
pub fn maker3_select_tool(tool: u8) -> Vec<String> {
    let mut g = vec!["G90 G21".to_string()];
    maker3_tool_change(&mut g, tool.min(1));
    g
}

/// Tool up, then over the middle of the mat (the same spot for either tool).
pub fn maker3_go_centre(o: &Maker3Options, bed: (f64, f64)) -> Vec<String> {
    let c = o.to_machine(Point::new(bed.0 / 2.0, bed.1 / 2.0), bed);
    vec!["G90 G21".into(), format!("G1 Z0 F{}", num(TOOL_FEED)), format!("G1 X{} Y{} F{}", num(c.x), num(c.y), num(TRAVEL_FEED))]
}

/// Whether the active tool's tip is over the mat's work area, so it can be
/// lowered: near the ends of the carriage's travel one tool or the other
/// hangs off the mat.
pub fn maker3_can_lower(bed: (f64, f64), mpos: [f64; 3]) -> Result<(), String> {
    let (x, y) = (mpos[0], mpos[1]);
    if x < -SLACK || x > bed.0 + SLACK || y < -SLACK || y > bed.1 + SLACK {
        return Err(format!("The tool is off the mat's work area (X {x:.1} Y {y:.1}) - move it over the mat before lowering it."));
    }
    Ok(())
}

/// Move the active tool (in `holder`) to height `z`, kept inside its travel.
pub fn maker3_height(model: CricutModel, z: f64, holder: u8) -> Vec<String> {
    vec!["G90 G21".into(), format!("G1 Z{} F200", num(z.clamp(model.z_min(holder), 0.0)))]
}

/// The tool-up line.
pub fn maker3_tool_up() -> Vec<String> {
    vec!["G90 G21".into(), format!("G1 Z0 F{}", num(TOOL_FEED))]
}

/// Whether the console may send this. Moves have to go through the checked
/// paths above, `$` settings wouldn't survive a power-off anyway, and a mark
/// scan (`#YE`) answers without an `ok`, so it only runs from Print & Cut.
pub fn maker3_console_ok(line: &str) -> Result<(), String> {
    let up = line.trim().to_ascii_uppercase();
    if matches!(up.as_str(), "$$" | "$G" | "$#" | "$I" | "$H" | "$HZ" | "$HX" | "$X") {
        return Ok(());
    }
    Err(format!("Not sent: {line}. On the Maker 3 the console only sends $$, $G, $#, $I, $H, $HZ, $HX and $X. Moves go through Jog so they stay on the mat."))
}

// ------------------------------------------------------------------ Maker 3 Print & Cut

/// Where the light sensor is relative to the pen: it's over pen point (x, y)
/// when the machine is at (x + 16.1, y + 2.0).
pub const SENSOR_OFFSET: (f64, f64) = (16.1, 2.0);
/// The sensor's reach in pen X (firmware v78 and later).
pub const SENSOR_X_MAX: f64 = 323.9;
/// The Explore 3's sensor (a different chip) sits here from the pen.
pub const EXPLORE3_SENSOR_OFFSET: (f64, f64) = (22.04, -0.83);
/// Scans start this far outside a side of the box, and end this far inside.
const SCAN_REACH: f64 = 12.0;
/// The scan back out starts this far inside where the first scan found it.
const SCAN_BACK: f64 = 4.0;
/// Fit limits: worst corner misfit, and how far the scale may be from 100%.
pub const MAX_MISFIT: f64 = 0.5;
pub const MAX_SCALE_ERROR: f64 = 0.03;

/// One `#YE` scan: along X (0) or Y (1), from and to in tenths of a mm, in
/// pen coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkScan {
    pub axis: u8,
    pub from: i64,
    pub to: i64,
}

impl MarkScan {
    fn new(model: CricutModel, axis: u8, from: f64, to: f64) -> Result<MarkScan, String> {
        let (f, t) = ((from * 10.0).round() as i64, (to * 10.0).round() as i64);
        if f < 0 || t < 0 {
            return Err(format!("A mark scan would start off the {} edge of the mat - move the registration box further in.", if axis == 0 { "right" } else { "top" }));
        }
        if axis == 0 && f.max(t) as f64 > model.sensor_x_max() * 10.0 {
            return Err("A mark scan would go past the light sensor's reach - move the registration box away from the mat's left edge.".into());
        }
        Ok(MarkScan { axis, from: f, to: t })
    }

    pub fn line(&self) -> String {
        format!("#YE {} {} {}", self.axis, self.from, self.to)
    }
}

/// What a `[MARK ...]` reply says.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkReply {
    /// The mark's centre, mm (pen coordinates).
    Found(f64),
    /// The scan ended inside something dark: only the first edge was seen.
    Unfinished,
    None,
    Refused,
    NoSensor,
    /// The axis didn't finish a move; the firmware resets.
    Failed,
}

pub fn parse_mark(line: &str) -> MarkReply {
    let words: Vec<&str> = line.trim().trim_start_matches('[').trim_end_matches(']').split_whitespace().collect();
    match words.get(1).copied() {
        Some("refused") => MarkReply::Refused,
        Some("sensor") => MarkReply::NoSensor,
        _ => match words.get(2).copied() {
            Some("found") => words.get(5).and_then(|c| c.parse().ok()).map(MarkReply::Found).unwrap_or(MarkReply::Unfinished),
            Some("none") => MarkReply::None,
            Some("error") => MarkReply::Failed,
            _ => MarkReply::Refused,
        },
    }
}

/// One place a side of the box is crossed by the sensor.
#[derive(Clone, Debug, PartialEq)]
pub struct Crossing {
    /// "left", "right", "top" or "bottom" (of the page).
    pub side: &'static str,
    /// The axis scanned: 0 X (the left and right sides), 1 Y (top and bottom).
    pub axis: u8,
    /// Where the side's centre line should be on the scanned axis, mm.
    pub nominal: f64,
    /// The pen coordinate of the crossing on the other axis, mm.
    pub along: f64,
    /// Which way is outside the box on the scanned axis: +1 or -1.
    pub outside: f64,
    /// Whose sensor does the scan.
    pub model: CricutModel,
}

impl Crossing {
    /// Put the sensor over where the inward scan starts. The firmware drives
    /// the scanned axis to the start, scans, then drives it back to where it
    /// was, so starting there keeps those trips short (otherwise the carriage
    /// goes all the way back to wherever it was, often home, every scan).
    pub fn premove(&self) -> String {
        let start = self.nominal + SCAN_REACH * self.outside;
        let (x, y) = if self.axis == 0 { (start, self.along) } else { (self.along, start) };
        let s = self.model.sensor_offset();
        format!("G1 X{} Y{} F{}", num(x + s.0), num(y + s.1), num(TRAVEL_FEED))
    }

    /// Scan inwards from outside, so the first dark band is the box.
    pub fn inward(&self) -> Result<MarkScan, String> {
        MarkScan::new(self.model, self.axis, self.nominal + SCAN_REACH * self.outside, self.nominal - SCAN_REACH * self.outside)
    }

    /// Scan back out, starting just inside where the first scan found it.
    pub fn outward(&self, found: f64) -> Result<MarkScan, String> {
        MarkScan::new(self.model, self.axis, found - SCAN_BACK * self.outside, self.nominal + SCAN_REACH * self.outside)
    }

    /// The measured point on the side, in pen coordinates.
    fn point(&self, centre: f64) -> Point {
        if self.axis == 0 { Point::new(centre, self.along) } else { Point::new(self.along, centre) }
    }
}

/// How far from a corner its two sides are crossed, for the corner scans.
const CORNER_IN: f64 = 10.0;

/// Where to cross the sensor box (page centre line `rect`), with the sheet in
/// the mat's top-left corner (nominally X = 301 - x, Y = y):
/// - the whole box: each side at 25% and 75% of its length (8 crossings);
/// - one corner: the left and top sides near the top-left corner (2);
/// - three corners: both sides near the top-left, top-right and bottom-left
///   corners (6), enough to fit a turned, unevenly stretched sheet exactly.
pub fn sensor_box_crossings(model: CricutModel, rect: kurbo::Rect, scan: crate::regmarks::BoxScan) -> Result<Vec<Crossing>, String> {
    use crate::regmarks::BoxScan;
    let x = |px: f64| MAKER3_WORK_X - px;
    let mut v = vec![];
    if scan != BoxScan::Full {
        let (dx, dy) = (CORNER_IN.min(rect.width() / 4.0), CORNER_IN.min(rect.height() / 4.0));
        v.push(Crossing { side: "left", axis: 0, nominal: x(rect.x0), along: rect.y0 + dy, outside: 1.0, model });
        v.push(Crossing { side: "top", axis: 1, nominal: rect.y0, along: x(rect.x0 + dx), outside: -1.0, model });
        if scan == BoxScan::ThreeCorners {
            // top-right
            v.push(Crossing { side: "right", axis: 0, nominal: x(rect.x1), along: rect.y0 + dy, outside: -1.0, model });
            v.push(Crossing { side: "top", axis: 1, nominal: rect.y0, along: x(rect.x1 - dx), outside: -1.0, model });
            // bottom-left
            v.push(Crossing { side: "left", axis: 0, nominal: x(rect.x0), along: rect.y1 - dy, outside: 1.0, model });
            v.push(Crossing { side: "bottom", axis: 1, nominal: rect.y1, along: x(rect.x0 + dx), outside: 1.0, model });
        }
        for c in &v {
            c.inward()?;
            c.outward(c.nominal)?;
        }
        return Ok(v);
    }
    for f in [0.25, 0.75] {
        let py = rect.y0 + rect.height() * f;
        // page-left is +X outside, page-right -X
        v.push(Crossing { side: "left", axis: 0, nominal: x(rect.x0), along: py, outside: 1.0, model });
        v.push(Crossing { side: "right", axis: 0, nominal: x(rect.x1), along: py, outside: -1.0, model });
    }
    for f in [0.25, 0.75] {
        let along = x(rect.x0 + rect.width() * f);
        // page-top is -Y outside, page-bottom +Y
        v.push(Crossing { side: "top", axis: 1, nominal: rect.y0, along, outside: -1.0, model });
        v.push(Crossing { side: "bottom", axis: 1, nominal: rect.y1, along, outside: 1.0, model });
    }
    for c in &v {
        c.inward()?;
        c.outward(c.nominal)?;
    }
    Ok(v)
}

/// A page-to-machine transform fitted to the printed box:
/// X = m0 x + m1 y + m2, Y = m3 x + m4 y + m5.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub m: [f64; 6],
    /// The worst distance between a measured corner and the fit, mm.
    pub misfit: f64,
    /// How the print's x and y came out against the design (1 = 100%).
    pub scale: (f64, f64),
    /// How far the sheet is turned, degrees.
    pub rotation: f64,
    /// What was measured.
    pub scan: crate::regmarks::BoxScan,
}

impl Fit {
    pub fn apply(&self, p: Point) -> Point {
        let m = &self.m;
        Point::new(m[0] * p.x + m[1] * p.y + m[2], m[3] * p.x + m[4] * p.y + m[5])
    }

    pub fn summary(&self) -> String {
        use crate::regmarks::BoxScan;
        match self.scan {
            BoxScan::Corner => format!("Found the sheet's corner: it's {:+.2} mm across and {:+.2} mm down from where it should be", self.m[2] - MAKER3_WORK_X, self.m[5]),
            BoxScan::ThreeCorners => format!("Found the sheet from three corners: turned {:.2}°, scale {:.3} across × {:.3} down", self.rotation, self.scale.0, self.scale.1),
            BoxScan::Full => format!("Found the sheet: turned {:.2}°, scale {:.3} × {:.3}, corners within {:.2} mm", self.rotation, self.scale.0, self.scale.1, self.misfit),
        }
    }
}

fn intersect(a: (Point, Point), b: (Point, Point)) -> Option<Point> {
    let (d1, d2) = (a.1 - a.0, b.1 - b.0);
    let den = d1.cross(d2);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (b.0 - a.0).cross(d2) / den;
    Some(a.0 + d1 * t)
}

/// Least squares for v = a x + b y + c over the points.
fn solve3(pts: &[(Point, f64)]) -> Option<[f64; 3]> {
    let mut n = [[0.0f64; 4]; 3];
    for (p, v) in pts {
        let r = [p.x, p.y, 1.0];
        for i in 0..3 {
            for j in 0..3 {
                n[i][j] += r[i] * r[j];
            }
            n[i][3] += r[i] * v;
        }
    }
    // Gaussian elimination with pivoting
    for c in 0..3 {
        let p = (c..3).max_by(|a, b| n[*a][c].abs().total_cmp(&n[*b][c].abs()))?;
        n.swap(c, p);
        if n[c][c].abs() < 1e-12 {
            return None;
        }
        for r in 0..3 {
            if r != c {
                let k = n[r][c] / n[c][c];
                for j in c..4 {
                    n[r][j] -= k * n[c][j];
                }
            }
        }
    }
    Some([n[0][3] / n[0][0], n[1][3] / n[1][1], n[2][3] / n[2][2]])
}

/// Fit the page-to-machine transform straight from the crossings: each one
/// says a measured machine point lies on a side of the box (page x = the
/// left or right side's x, or page y = the top or bottom's y). Three or more
/// of each give the machine-to-page transform, which is then turned round.
/// No corner is assumed to be where its sides were crossed.
fn fit_crossings(rect: kurbo::Rect, crossings: &[Crossing], centres: &[f64], scan: crate::regmarks::BoxScan) -> Result<Fit, String> {
    let bad = || "Couldn't work out the sheet from the scans".to_string();
    let mut xs = vec![];
    let mut ys = vec![];
    for (c, v) in crossings.iter().zip(centres) {
        let p = c.point(*v);
        match c.side {
            "left" => xs.push((p, rect.x0)),
            "right" => xs.push((p, rect.x1)),
            "top" => ys.push((p, rect.y0)),
            _ => ys.push((p, rect.y1)),
        }
    }
    // page = P · machine
    let px = solve3(&xs).ok_or_else(bad)?;
    let py = solve3(&ys).ok_or_else(bad)?;
    let det = px[0] * py[1] - px[1] * py[0];
    if det.abs() < 1e-12 {
        return Err(bad());
    }
    // machine = P⁻¹ · (page - offset)
    let (a, b, c, d) = (py[1] / det, -px[1] / det, -py[0] / det, px[0] / det);
    let m = [a, b, -(a * px[2] + b * py[2]), c, d, -(c * px[2] + d * py[2])];
    let mut fit = Fit { m, misfit: 0.0, scale: (m[0].hypot(m[3]), m[1].hypot(m[4])), rotation: m[3].atan2(-m[0]).to_degrees(), scan };
    // with more crossings than needed, how far the fit misses them
    fit.misfit = xs.iter().map(|(p, v)| (px[0] * p.x + px[1] * p.y + px[2] - v).abs()).chain(ys.iter().map(|(p, v)| (py[0] * p.x + py[1] * p.y + py[2] - v).abs())).fold(0.0, f64::max);
    for (axis, s) in [("across", fit.scale.0), ("down", fit.scale.1)] {
        if (s - 1.0).abs() > MAX_SCALE_ERROR {
            return Err(format!("The print is {:.1}% of its size {axis} the page - print it at 100% / actual size, with no fit-to-page. Nothing was cut.", s * 100.0));
        }
    }
    Ok(fit)
}

/// Fit the sheet from whichever crossings were scanned (see
/// [`sensor_box_crossings`]): the whole box, three corners or one.
pub fn fit_marks(rect: kurbo::Rect, crossings: &[Crossing], centres: &[f64]) -> Result<Fit, String> {
    use crate::regmarks::BoxScan;
    let found = |side: &str| crossings.iter().zip(centres).find(|(c, _)| c.side == side).map(|(_, v)| *v);
    let corner = |a: &str, b: &str| -> Option<Point> { Some(Point::new(found(a)?, found(b)?)) };
    match crossings.len() {
        8 => fit_sensor_box(rect, crossings, centres),
        2 => {
            // straight and full size: only where the corner is
            let tl = corner("left", "top").ok_or("Couldn't work out the box's corner from the scans")?;
            Ok(Fit { m: [-1.0, 0.0, tl.x + rect.x0, 0.0, 1.0, tl.y - rect.y0], misfit: 0.0, scale: (1.0, 1.0), rotation: 0.0, scan: BoxScan::Corner })
        }
        6 => fit_crossings(rect, crossings, centres, BoxScan::ThreeCorners),
        _ => Err("Couldn't work out the sheet from the scans".into()),
    }
}

/// Fit the transform from the box as designed (`rect`, page) to where the
/// scans found it (`centres`, one per crossing), and check it's believable.
pub fn fit_sensor_box(rect: kurbo::Rect, crossings: &[Crossing], centres: &[f64]) -> Result<Fit, String> {
    let line = |side: &str| -> Option<(Point, Point)> {
        let pts: Vec<Point> = crossings.iter().zip(centres).filter(|(c, _)| c.side == side).map(|(c, v)| c.point(*v)).collect();
        (pts.len() == 2).then(|| (pts[0], pts[1]))
    };
    let bad = || "Couldn't work out the box's corners from the scans".to_string();
    let (l, r, t, b) = (line("left").ok_or_else(bad)?, line("right").ok_or_else(bad)?, line("top").ok_or_else(bad)?, line("bottom").ok_or_else(bad)?);
    let measured = [intersect(l, t), intersect(r, t), intersect(r, b), intersect(l, b)];
    let design = [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)];
    let mut pairs = vec![];
    for (d, m) in design.iter().zip(measured) {
        pairs.push((*d, m.ok_or_else(bad)?));
    }
    let fx = solve3(&pairs.iter().map(|(d, m)| (*d, m.x)).collect::<Vec<_>>()).ok_or_else(bad)?;
    let fy = solve3(&pairs.iter().map(|(d, m)| (*d, m.y)).collect::<Vec<_>>()).ok_or_else(bad)?;
    let mut fit = Fit { m: [fx[0], fx[1], fx[2], fy[0], fy[1], fy[2]], misfit: 0.0, scale: (0.0, 0.0), rotation: 0.0, scan: crate::regmarks::BoxScan::Full };
    fit.misfit = pairs.iter().map(|(d, m)| fit.apply(*d).distance(*m)).fold(0.0, f64::max);
    fit.scale = (fit.m[0].hypot(fit.m[3]), fit.m[1].hypot(fit.m[4]));
    // page x runs towards -X on the machine
    fit.rotation = fit.m[3].atan2(-fit.m[0]).to_degrees();
    if fit.misfit > MAX_MISFIT {
        return Err(format!("The marks don't line up ({:.2} mm off at a corner, {MAX_MISFIT} allowed) - a bad scan or a creased sheet. Nothing was cut.", fit.misfit));
    }
    for (axis, s) in [("across", fit.scale.0), ("down", fit.scale.1)] {
        if (s - 1.0).abs() > MAX_SCALE_ERROR {
            return Err(format!("The print is {:.1}% of its size {axis} the page - print it at 100% / actual size, with no fit-to-page. Nothing was cut.", s * 100.0));
        }
    }
    Ok(fit)
}

// ------------------------------------------------------------------ messages

/// What a GRBL `error:N` means (GRBL 1.1 numbering).
pub fn error_text(n: u32) -> &'static str {
    match n {
        1 => "a G-code word has no letter",
        2 => "a number is missing or badly written",
        3 => "unknown $ command",
        4 => "a value that must be positive is negative",
        5 => "homing isn't enabled ($22)",
        8 => "that $ command only works when the machine is idle",
        9 => "the machine is in alarm or jogging - Home or Unlock first",
        10 => "soft limits need homing enabled",
        11 => "the line is too long",
        15 => "the jog would go past the machine's travel",
        16 => "invalid jog command",
        17 => "laser mode needs a PWM output",
        20 => "unsupported G-code command",
        22 => "no feed rate set",
        33 => "the move's target is invalid",
        _ => "see the GRBL error list",
    }
}

/// What a GRBL `ALARM:N` means.
pub fn alarm_text(n: u32) -> &'static str {
    match n {
        1 => "a limit switch was hit - position lost, home again",
        2 => "the move would go past the machine's travel (soft limit)",
        3 => "reset while moving - position may be lost, home again",
        4 | 5 => "probe failed",
        6 => "homing was reset",
        7 => "homing stopped: the safety door opened",
        8 => "homing couldn't clear the limit switch",
        9 => "homing couldn't find the limit switch",
        _ => "see the GRBL alarm list",
    }
}

/// What an `ALARM:N` means on this machine, and what to do about it.
fn alarm_message(dialect: Dialect, n: u32) -> String {
    if dialect == Dialect::Maker3 {
        let text = match n {
            1 => "a motor stalled or fell behind - click Reset, then Home",
            3 => "reset during a move - position may be off, click Home",
            9 => "homing failed - click Reset, check for obstructions (on an Explore 3, a clamp stuck at the bottom: lift it by hand), then Home",
            _ => "position is unreliable - click Reset, then Home",
        };
        return format!("Alarm {n}: {text}");
    }
    format!("Alarm {n}: {}", alarm_text(n))
}

// ------------------------------------------------------------------ ports

/// A serial port that might be a GRBL controller.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub description: String,
    /// USB vendor and product id, for a USB port.
    pub usb: Option<(u16, u16)>,
}

/// The serial ports on this computer, USB ones first.
pub fn list_ports() -> Vec<Port> {
    let mut out: Vec<(bool, Port)> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        // macOS lists each port twice; the "cu." one is the one to open
        .filter(|p| !p.port_name.starts_with("/dev/tty.") || cfg!(not(target_os = "macos")))
        .map(|p| {
            let (usb, description) = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    let what = u.product.clone().or(u.manufacturer.clone()).unwrap_or_else(|| "USB serial".into());
                    (Some((u.vid, u.pid)), format!("{what} ({:04x}:{:04x})", u.vid, u.pid))
                }
                serialport::SerialPortType::BluetoothPort => (None, "Bluetooth".into()),
                serialport::SerialPortType::PciPort => (None, "Built-in".into()),
                serialport::SerialPortType::Unknown => (None, "Serial port".into()),
            };
            (usb.is_some(), Port { name: p.port_name, description, usb })
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
    out.into_iter().map(|(_, p)| p).collect()
}

// ------------------------------------------------------------------ the link

/// What the machine last reported, for the panel.
#[derive(Clone, Debug, Default)]
pub struct GrblState {
    /// The port is open.
    pub open: bool,
    /// The controller has answered (banner or status report).
    pub answered: bool,
    /// The link has ended (closed, failed or unplugged).
    pub closed: bool,
    pub version: String,
    /// Idle, Run, Hold:0, Jog, Alarm, Door, Check, Home, Sleep.
    pub status: String,
    /// Machine and work positions, mm.
    pub mpos: [f64; 3],
    pub wpos: [f64; 3],
    /// A job is streaming.
    pub busy: bool,
    /// The tool the machine was last told to use (its own reports can't say),
    /// None until one has been chosen or after a restart.
    pub tool: Option<u8>,
    /// Stopped or alarmed: position can't be trusted until it's homed.
    pub needs_home: bool,
    /// Maker 3: motor T is steering a rotary blade (`M3`).
    pub steering: bool,
    /// Maker 3 Print & Cut: what the last look at the printed box found.
    pub last_fit: Option<String>,
    /// Maker 3: the tool the last scan of holder B reported ("not found" if
    /// none), as the machine names it.
    pub detected: Option<String>,
    /// Maker 3: the holder to select when the tool isn't known (on connecting,
    /// or after a restart).
    pub preferred_tool: u8,
    pub error: Option<String>,
    /// The conversation, newest last (sent lines start with "> ").
    pub log: VecDeque<String>,
}

impl GrblState {
    pub fn alarm(&self) -> bool {
        self.status.starts_with("Alarm")
    }
}

/// Which rules the controller plays by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// GRBL 1.1, grblHAL, FluidNC.
    Stock,
    /// Cricut Maker 3 on cutcutgo: one line at a time, a job ends on the
    /// `ok` of its final `G4 P0`, and Ctrl-X needs a re-home.
    Maker3,
}

/// Makes a Print & Cut job's G-code once the sheet has been found.
pub type CutMaker = Box<dyn FnOnce(&Fit) -> Result<Vec<String>, String> + Send>;

enum Cmd {
    Line(String),
    Realtime(u8),
    Job { lines: Vec<String>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool> },
    PrintCut { rect: kurbo::Rect, crossings: Vec<Crossing>, cut: Option<CutMaker>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool> },
    Routine { lines: Vec<String>, detect: bool, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool> },
    Close,
}

/// An open connection to a GRBL controller.
pub struct GrblLink {
    tx: mpsc::Sender<Cmd>,
    pub state: Arc<Mutex<GrblState>>,
    pub port: String,
    pub dialect: Dialect,
}

/// GRBL's receive buffer is 128 bytes; keep one spare.
const RX_BUFFER: usize = 127;
const LOG_LINES: usize = 300;

/// Real-time commands (sent at once, not queued).
pub const STATUS: u8 = b'?';
pub const FEED_HOLD: u8 = b'!';
pub const RESUME: u8 = b'~';
pub const SOFT_RESET: u8 = 0x18;
pub const JOG_CANCEL: u8 = 0x85;

impl GrblLink {
    /// Open `port` on a background thread. Watch `state` for the result.
    pub fn connect(port: &str, baud: u32, dialect: Dialect) -> GrblLink {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(GrblState { status: "Connecting".into(), ..Default::default() }));
        let (p, st) = (port.to_string(), state.clone());
        std::thread::spawn(move || run(&p, baud, dialect, rx, st));
        GrblLink { tx, state, port: port.into(), dialect }
    }

    /// Maker 3: which holder to select when the tool isn't known.
    pub fn prefer_tool(&self, holder: u8) {
        if let Ok(mut s) = self.state.lock() {
            s.preferred_tool = holder.min(1);
        }
    }

    /// For a machine that doesn't home itself at power-on (the Explore 3):
    /// home before anything else, and refuse jobs until that's done.
    pub fn home_first(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.needs_home = true;
        }
        self.send("$H");
    }

    pub fn state(&self) -> GrblState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Send one line (G-code or a `$` command).
    pub fn send(&self, line: &str) {
        let _ = self.tx.send(Cmd::Line(line.trim().to_string()));
    }

    /// Send a real-time command byte.
    pub fn realtime(&self, b: u8) {
        let _ = self.tx.send(Cmd::Realtime(b));
    }

    /// Jog by (dx, dy, dz) mm at `feed` mm/min.
    pub fn jog(&self, d: [f64; 3], feed: f64) {
        let mut s = "$J=G91 G21".to_string();
        for (axis, v) in ["X", "Y", "Z"].iter().zip(d) {
            if v != 0.0 {
                s += &format!(" {axis}{}", num(v));
            }
        }
        self.send(&format!("{s} F{}", num(feed)));
    }

    /// Stream a job. `progress` counts lines; setting `cancel` stops the machine.
    pub fn start_job(&self, lines: Vec<String>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) {
        let _ = self.tx.send(Cmd::Job { lines, progress, cancel });
    }

    /// Maker 3 Print & Cut: scan the printed sensor box (`rect`, its centre
    /// line on the page) at each of `crossings`, fit where the sheet really
    /// is, then run `cut`'s lines for that fit as a job. Without `cut`, only
    /// finds the sheet (to check the marks).
    pub fn print_cut(&self, rect: kurbo::Rect, crossings: Vec<Crossing>, cut: Option<CutMaker>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) {
        let _ = self.tx.send(Cmd::PrintCut { rect, crossings, cut, progress, cancel });
    }

    /// Maker 3: run [`MAKER3_LOAD_TOOL`] or (`detect`) [`MAKER3_DETECT_TOOL`];
    /// `progress` says when it's done and, for a detect, what was found.
    pub fn routine(&self, lines: &[&str], detect: bool, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) {
        let _ = self.tx.send(Cmd::Routine { lines: lines.iter().map(|l| l.to_string()).collect(), detect, progress, cancel });
    }
}

impl Drop for GrblLink {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

struct Job {
    lines: Vec<String>,
    /// The tools each `M3` should find, in order (Maker 3).
    expect: VecDeque<String>,
    next: usize,
    acked: usize,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    /// Everything was accepted; waiting for the machine to finish moving.
    finishing: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Src {
    Job,
    User,
    /// `$H`: its `ok` means the machine knows where it is again.
    Home,
}

/// A two-way byte stream to the controller; a read with nothing to read
/// returns a `TimedOut` error after a short wait.
trait Wire: Read + Write + Send {}
impl<T: Read + Write + Send> Wire for T {}

/// How long the machine may go without taking data before the link is
/// given up on.
const WRITE_PATIENCE: Duration = Duration::from_secs(10);

fn transient(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock)
}

/// Write all of `data`. The port's short timeout applies to writes too, and
/// a busy machine (homing, scanning a tool or a mark, a full queue) may not
/// take bytes for a while: that's waited out, not taken as a lost link.
fn put(port: &mut dyn Wire, data: &[u8]) -> std::io::Result<()> {
    let start = Instant::now();
    let mut off = 0;
    while off < data.len() {
        match port.write(&data[off..]) {
            Ok(n) if n > 0 => off += n,
            Ok(_) => {}
            Err(e) if transient(&e) => {}
            Err(e) => return Err(e),
        }
        if off < data.len() {
            if start.elapsed() > WRITE_PATIENCE {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the machine stopped taking data for 10 seconds"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(())
}

struct Worker {
    port: Box<dyn Wire>,
    dialect: Dialect,
    /// The last tool scan found nothing: the ALARM:9 that follows is that.
    tool_missing: bool,
    state: Arc<Mutex<GrblState>>,
    rx: Vec<u8>,
    /// Lines sent and not yet answered: (bytes, from where).
    pending: VecDeque<(usize, Src)>,
    user: VecDeque<String>,
    job: Option<Job>,
    /// Feed hold sent, reset to follow.
    stopping: Option<Instant>,
    wco: [f64; 3],
}

fn log(state: &Arc<Mutex<GrblState>>, line: String) {
    if let Ok(mut s) = state.lock() {
        s.log.push_back(line);
        while s.log.len() > LOG_LINES {
            s.log.pop_front();
        }
    }
}

fn run(port: &str, baud: u32, dialect: Dialect, rx: mpsc::Receiver<Cmd>, state: Arc<Mutex<GrblState>>) {
    let opened = serialport::new(port, baud).timeout(Duration::from_millis(10)).open();
    let mut port = match opened {
        Ok(p) => p,
        Err(e) => {
            if let Ok(mut s) = state.lock() {
                s.error = Some(format!("Couldn't open {port}: {e}"));
                s.status = "Not connected".into();
                s.closed = true;
            }
            return;
        }
    };
    if dialect == Dialect::Maker3 {
        let _ = port.write_data_terminal_ready(true);
        let _ = port.write_request_to_send(true);
    }
    run_on(Box::new(port), dialect, rx, state)
}

fn run_on(port: Box<dyn Wire>, dialect: Dialect, rx: mpsc::Receiver<Cmd>, state: Arc<Mutex<GrblState>>) {
    if let Ok(mut s) = state.lock() {
        s.open = true;
        s.status = "Waiting for GRBL".into();
    }
    let mut w = Worker { port, dialect, tool_missing: false, state, rx: vec![], pending: VecDeque::new(), user: VecDeque::new(), job: None, stopping: None, wco: [0.0; 3] };
    let start = Instant::now();
    let mut woke = false;
    let mut last_poll = Instant::now();
    let result = (|| -> std::io::Result<()> {
        loop {
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                        if w.job.is_some() {
                            if w.dialect == Dialect::Stock {
                                put(&mut *w.port, &[FEED_HOLD])?;
                                std::thread::sleep(Duration::from_millis(300));
                            }
                            put(&mut *w.port, &[SOFT_RESET])?;
                            w.end_job(Some("disconnected".into()));
                        }
                        return Ok(());
                    }
                    Ok(Cmd::Line(l)) => w.user.push_back(l),
                    Ok(Cmd::Realtime(b)) => {
                        if b == SOFT_RESET {
                            w.pending.clear();
                            w.user.clear();
                        }
                        put(&mut *w.port, &[b])?
                    }
                    Ok(Cmd::Job { lines, progress, cancel }) => w.begin_job(lines, progress, cancel),
                    Ok(Cmd::PrintCut { rect, crossings, cut, progress, cancel }) => w.print_cut(rect, crossings, cut, progress, cancel)?,
                    Ok(Cmd::Routine { lines, detect, progress, cancel }) => w.routine(lines, detect, progress, cancel)?,
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            // Arduino-based boards restart when the port opens and greet us;
            // others (ESP32, grblHAL) are already running: nudge them
            let answered = w.state.lock().map(|s| s.answered).unwrap_or(false);
            if w.dialect == Dialect::Stock && !answered && !woke && start.elapsed() > Duration::from_millis(2500) {
                put(&mut *w.port, b"\r\n\r\n")?;
                woke = true;
            }
            // the Maker 3 homes itself at power-on and doesn't answer until it's done
            let (patience, no_answer) = match w.dialect {
                Dialect::Stock => (8, "No answer from GRBL - check the port and the speed (most use 115200)"),
                Dialect::Maker3 => (60, "No answer from the Maker 3 after a minute - check the port and that the machine is on"),
            };
            if !answered && start.elapsed() > Duration::from_secs(patience) {
                if let Ok(mut s) = w.state.lock() {
                    if s.error.is_none() {
                        s.error = Some(no_answer.into());
                    }
                }
            }
            if last_poll.elapsed() > Duration::from_millis(200) {
                // skipped (not waited for) if the machine is too busy to take it
                match w.port.write(&[STATUS]) {
                    Err(e) if !transient(&e) => return Err(e),
                    _ => {}
                }
                last_poll = Instant::now();
            }
            w.read()?;
            w.check_cancel()?;
            w.stream()?;
        }
    })();
    if let Ok(mut s) = w.state.lock() {
        if let Err(e) = result {
            s.error = Some(format!("Connection lost ({e}). The machine may still finish the moves it had already been sent: let it stop, then Connect again and Home before the next job."));
        }
        s.closed = true;
        s.open = false;
        s.busy = false;
        s.status = "Not connected".into();
    }
    if w.job.is_some() {
        w.end_job(Some("connection lost".into()));
    }
}

impl Worker {
    fn begin_job(&mut self, lines: Vec<String>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) {
        let set = |f: &dyn Fn(&mut Progress)| {
            if let Ok(mut p) = progress.lock() {
                f(&mut p)
            }
        };
        if let Some(e) = self.refusal(true) {
            set(&|p| {
                p.done = true;
                p.error = Some(e.clone());
                p.message = format!("Failed: {e}");
            });
            return;
        }
        let expect: VecDeque<String> = lines.iter().filter_map(|l| l.strip_prefix(EXPECT_TOOL).map(|n| n.trim().to_string())).collect();
        // comments and blank lines aren't sent
        let lines: Vec<String> = lines.into_iter().map(|l| l.split(';').next().unwrap_or("").trim().to_string()).filter(|l| !l.is_empty()).collect();
        let total = lines.len();
        set(&|p| {
            *p = Progress { total, message: "Sending to GRBL…".into(), ..Default::default() };
        });
        if let Ok(mut s) = self.state.lock() {
            s.busy = true;
            s.error = None;
        }
        self.job = Some(Job { lines, expect, next: 0, acked: 0, progress, cancel, finishing: false });
    }

    /// Why a job (or Print & Cut, or with `homed` false a routine that homes
    /// for itself) can't start now, if it can't.
    fn refusal(&self, homed: bool) -> Option<String> {
        let st = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        if self.job.is_some() {
            Some("a job is already running".to_string())
        } else if !st.answered {
            Some("GRBL hasn't answered yet".into())
        } else if st.alarm() {
            Some("GRBL is in alarm - Home or Unlock it first".into())
        } else if self.dialect == Dialect::Maker3 && st.needs_home && homed {
            Some("The machine's position can't be trusted after a stop or alarm - Home it first".into())
        } else if self.dialect == Dialect::Maker3 && st.status != "Idle" {
            Some(format!("The machine is busy ({}) - wait until it's Idle", st.status))
        } else {
            None
        }
    }

    /// Something that wasn't a job went wrong: say so, and free the machine.
    fn fail(&mut self, progress: &Arc<Mutex<Progress>>, e: String) {
        if let Ok(mut p) = progress.lock() {
            p.done = true;
            p.message = if e == "cancelled" { "Stopped - click Home before the next job".into() } else { format!("Failed: {e}") };
            p.error = Some(e.clone());
        }
        if let Ok(mut s) = self.state.lock() {
            s.busy = false;
            if e != "cancelled" {
                s.error = Some(e);
            }
        }
    }

    /// Let any commands already on their way finish first.
    fn settle(&mut self) -> std::io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while (!self.pending.is_empty() || !self.user.is_empty()) && Instant::now() < deadline {
            if self.pending.is_empty() {
                if let Some(l) = self.user.pop_front() {
                    self.write_line(&l, Src::User)?;
                }
            }
            if let Some(l) = self.next_line()? {
                self.on_line(&l)?;
            }
        }
        Ok(())
    }

    /// Maker 3 Load tool / Detect tool: the fixed sequences, one line at a
    /// time. A scan that finds no tool is followed by ALARM:9, which is
    /// cleared (`$X`) before carrying on, as it's not a lost position.
    fn routine(&mut self, lines: Vec<String>, detect: bool, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) -> std::io::Result<()> {
        self.settle()?;
        if let Some(e) = self.refusal(false).or_else(|| (!self.pending.is_empty()).then(|| "the machine is still busy with a command".to_string())) {
            self.fail(&progress, e);
            return Ok(());
        }
        if let Ok(mut p) = progress.lock() {
            *p = Progress { total: lines.len(), ..Default::default() };
        }
        if let Ok(mut s) = self.state.lock() {
            s.busy = true;
            s.error = None;
            if detect {
                s.detected = None;
            }
        }
        self.tool_missing = false;
        for (i, l) in lines.iter().enumerate() {
            if let Ok(mut p) = progress.lock() {
                p.message = match l.as_str() {
                    "M3" => "Scanning the tool in holder B (up to a minute)…".into(),
                    "$H" => "Homing…".into(),
                    _ if detect => "Detecting the tool in holder B…".into(),
                    _ => "Moving the carriage so you can reach holder B…".into(),
                };
            }
            if let Err(e) = self.send_wait(l, &cancel)? {
                self.fail(&progress, e);
                return Ok(());
            }
            if l == "M3" && self.tool_missing {
                // the ALARM:9 comes just after the ok: wait for it
                let deadline = Instant::now() + Duration::from_secs(3);
                while self.tool_missing && Instant::now() < deadline {
                    if let Some(x) = self.next_line()? {
                        self.on_line(&x)?;
                    }
                }
            }
            // the $X that clears it
            while let Some(u) = self.user.pop_front() {
                if let Err(e) = self.send_wait(&u, &cancel)? {
                    self.fail(&progress, e);
                    return Ok(());
                }
            }
            if let Ok(mut p) = progress.lock() {
                p.sent = i + 1;
            }
        }
        let found = self.state.lock().ok().and_then(|s| s.detected.clone());
        let message = if !detect {
            "The carriage is parked: fit or change the tool in holder B, then click Detect tool (not needed for the fine-point blade).".to_string()
        } else {
            match found.as_deref() {
                None | Some("not found") => "No drive-housing tool in holder B: it's empty, or has the fine-point blade (which can't be scanned).".to_string(),
                Some(n) => format!("Holder B has the {}.", reported_label(n)),
            }
        };
        if let Ok(mut s) = self.state.lock() {
            s.busy = false;
            s.error = None;
        }
        if let Ok(mut p) = progress.lock() {
            p.done = true;
            p.sent = p.total;
            p.message = message;
        }
        Ok(())
    }

    /// Maker 3 Print & Cut: find the printed box, fit the sheet, then cut.
    fn print_cut(&mut self, rect: kurbo::Rect, crossings: Vec<Crossing>, cut: Option<CutMaker>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) -> std::io::Result<()> {
        // let any commands already on their way finish first
        let deadline = Instant::now() + Duration::from_secs(20);
        while (!self.pending.is_empty() || !self.user.is_empty()) && Instant::now() < deadline {
            if self.pending.is_empty() {
                if let Some(l) = self.user.pop_front() {
                    self.write_line(&l, Src::User)?;
                }
            }
            if let Some(l) = self.next_line()? {
                self.on_line(&l)?;
            }
        }
        if let Some(e) = self.refusal(true).or_else(|| (!self.pending.is_empty()).then(|| "the machine is still busy with a command".to_string())) {
            self.fail(&progress, e);
            return Ok(());
        }
        if let Ok(mut p) = progress.lock() {
            *p = Progress { total: crossings.len() * 2, message: "Finding the registration box…".into(), ..Default::default() };
        }
        if let Ok(mut s) = self.state.lock() {
            s.busy = true;
            s.error = None;
            s.last_fit = None;
        }
        let fit = match self.find_sheet(rect, &crossings, &progress, &cancel)? {
            Ok(fit) => fit,
            Err(e) => {
                self.fail(&progress, e);
                return Ok(());
            }
        };
        let summary = fit.summary();
        log(&self.state, summary.clone());
        if let Ok(mut s) = self.state.lock() {
            s.last_fit = Some(summary.clone());
            s.busy = false;
        }
        match cut.map(|f| f(&fit)) {
            None => {
                if let Ok(mut p) = progress.lock() {
                    p.done = true;
                    p.sent = p.total;
                    p.message = summary;
                }
            }
            Some(Ok(lines)) => self.begin_job(lines, progress, cancel),
            Some(Err(e)) => self.fail(&progress, e),
        }
        Ok(())
    }

    /// The scans: each crossing inwards, then back out from where it was found.
    fn find_sheet(&mut self, rect: kurbo::Rect, crossings: &[Crossing], progress: &Arc<Mutex<Progress>>, cancel: &Arc<AtomicBool>) -> std::io::Result<Result<Fit, String>> {
        macro_rules! attempt {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                }
            };
        }
        // tools up, and the pen's coordinates, which the scans are given in
        for l in ["$HZ", "G90 G21", "T0"] {
            attempt!(self.send_wait(l, cancel)?);
        }
        let n = crossings.len();
        let mut centres = vec![];
        for (i, c) in crossings.iter().enumerate() {
            if let Ok(mut p) = progress.lock() {
                p.message = format!("Finding the registration box: {} side ({} of {n})…", c.side, i + 1);
            }
            attempt!(self.send_wait(&c.premove(), cancel)?);
            // a scan only goes out once the machine is idle
            attempt!(self.send_wait("G4 P0", cancel)?);
            let first = attempt!(self.scan(&attempt!(c.inward()), c, cancel)?);
            attempt!(self.send_wait("G4 P0", cancel)?);
            let back = attempt!(self.scan(&attempt!(c.outward(first)), c, cancel)?);
            centres.push((first + back) / 2.0);
            if let Ok(mut p) = progress.lock() {
                p.sent = (i + 1) * 2;
            }
        }
        Ok(fit_marks(rect, crossings, &centres))
    }

    /// The next whole line from the machine, if one arrives within a read.
    fn next_line(&mut self) -> std::io::Result<Option<String>> {
        for attempt in 0..2 {
            while let Some(i) = self.rx.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.rx.drain(..=i).collect();
                let line = String::from_utf8_lossy(&line).trim().to_string();
                if !line.is_empty() {
                    return Ok(Some(line));
                }
            }
            if attempt == 0 {
                let mut buf = [0u8; 512];
                match self.port.read(&mut buf) {
                    Ok(n) => self.rx.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(None)
    }

    /// Send one line and wait for its `ok` (Print & Cut's own steps).
    fn send_wait(&mut self, line: &str, cancel: &Arc<AtomicBool>) -> std::io::Result<Result<(), String>> {
        self.write_line(line, Src::User)?;
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if cancel.load(Ordering::Relaxed) {
                self.stop()?;
                return Ok(Err("cancelled".into()));
            }
            if Instant::now() > deadline {
                return Ok(Err(format!("No answer from the machine to {line}")));
            }
            let Some(l) = self.next_line()? else { continue };
            let lower = l.to_lowercase();
            if l == "ok" {
                if let Some((_, Src::Home)) = self.pending.pop_front() {
                    if let Ok(mut s) = self.state.lock() {
                        s.needs_home = false;
                    }
                }
                return Ok(Ok(()));
            }
            if let Some(n) = lower.strip_prefix("error:") {
                self.pending.pop_front();
                log(&self.state, l.clone());
                let n: u32 = n.trim().parse().unwrap_or(0);
                return Ok(Err(format!("The machine refused {line}: error {n}, {}", error_text(n))));
            }
            if lower.starts_with("alarm:") {
                self.on_line(&l)?;
                return Ok(Err(self.state.lock().ok().and_then(|s| s.error.clone()).unwrap_or_else(|| l.clone())));
            }
            self.on_line(&l)?;
        }
    }

    /// One `#YE` scan, sent on its own, and its `[MARK ...]` reply (no `ok`).
    fn scan(&mut self, s: &MarkScan, c: &Crossing, cancel: &Arc<AtomicBool>) -> std::io::Result<Result<f64, String>> {
        let line = s.line();
        put(&mut *self.port, format!("{line}\n").as_bytes())?;
        log(&self.state, format!("> {line}"));
        let deadline = Instant::now() + Duration::from_secs(40);
        let side = c.side;
        loop {
            if cancel.load(Ordering::Relaxed) {
                self.stop()?;
                return Ok(Err("cancelled".into()));
            }
            if Instant::now() > deadline {
                return Ok(Err("The light sensor didn't report back within 40 seconds. Nothing was cut.".into()));
            }
            let Some(l) = self.next_line()? else { continue };
            if l.starts_with("[MARK") {
                log(&self.state, l.clone());
                return Ok(match parse_mark(&l) {
                    MarkReply::Found(v) => Ok(v),
                    MarkReply::Unfinished => Err(format!("The {side} side's scan ended inside something dark - keep other printing at least 15 mm outside the box. Nothing was cut.")),
                    MarkReply::None => Err(format!("The {side} side of the registration box wasn't found - check the sheet is in the mat's top-left corner, printed at 100% on white matt paper. Nothing was cut.")),
                    MarkReply::Refused => Err("The machine refused a mark scan (not idle, or out of the sensor's range). Nothing was cut.".into()),
                    MarkReply::NoSensor => Err("The light sensor didn't answer. Nothing was cut.".into()),
                    MarkReply::Failed => {
                        if let Ok(mut st) = self.state.lock() {
                            st.needs_home = true;
                        }
                        Err("A mark scan couldn't finish its move and the machine reset - Home it before trying again. Nothing was cut.".into())
                    }
                });
            }
            if l.to_lowercase().starts_with("alarm:") {
                self.on_line(&l)?;
                return Ok(Err(self.state.lock().ok().and_then(|s| s.error.clone()).unwrap_or_else(|| l.clone())));
            }
            self.on_line(&l)?;
        }
    }

    fn end_job(&mut self, error: Option<String>) {
        if let Some(job) = self.job.take() {
            if let Ok(mut p) = job.progress.lock() {
                p.done = true;
                match &error {
                    None => {
                        p.sent = p.total;
                        p.message = match self.dialect {
                            Dialect::Stock => "Job finished".into(),
                            Dialect::Maker3 => "Job finished - press the machine's feed button to eject the mat".into(),
                        };
                    }
                    Some(e) => {
                        p.error = Some(e.clone());
                        p.message = match (e.as_str(), self.dialect) {
                            ("cancelled", Dialect::Stock) => "Cancelled - the machine was stopped".into(),
                            ("cancelled", Dialect::Maker3) => "Stopped - click Home before the next job (the tool stays where it stopped until then)".into(),
                            _ => format!("Failed: {e}"),
                        };
                    }
                }
            }
        }
        if let Ok(mut s) = self.state.lock() {
            s.busy = false;
            if let Some(e) = error.filter(|e| e != "cancelled") {
                s.error = Some(e);
            }
        }
    }

    /// Stop now: feed hold, then (once stopped) a reset that empties the
    /// machine's queue without losing its position.
    fn stop(&mut self) -> std::io::Result<()> {
        if self.dialect == Dialect::Maker3 {
            // halts at once and empties the queue, but position needs re-homing
            put(&mut *self.port, &[SOFT_RESET])?;
            self.pending.clear();
            self.user.clear();
            if let Ok(mut s) = self.state.lock() {
                s.needs_home = true;
            }
            return Ok(());
        }
        if self.stopping.is_none() {
            put(&mut *self.port, &[FEED_HOLD])?;
            self.stopping = Some(Instant::now());
        }
        Ok(())
    }

    fn check_cancel(&mut self) -> std::io::Result<()> {
        if self.job.as_ref().is_some_and(|j| j.cancel.load(Ordering::Relaxed)) {
            self.stop()?;
            self.end_job(Some("cancelled".into()));
        }
        if let Some(t) = self.stopping {
            let held = self.state.lock().map(|s| s.status == "Hold:0" || s.status == "Idle").unwrap_or(false);
            if held || t.elapsed() > Duration::from_millis(1500) {
                put(&mut *self.port, &[SOFT_RESET])?;
                self.pending.clear();
                self.user.clear();
                self.stopping = None;
            }
        }
        Ok(())
    }

    fn in_flight(&self) -> usize {
        self.pending.iter().map(|(n, _)| n).sum()
    }

    fn write_line(&mut self, line: &str, src: Src) -> std::io::Result<()> {
        let bytes = format!("{line}\n");
        put(&mut *self.port, bytes.as_bytes())?;
        // a job's own $H stays part of the job (its ok is counted below)
        let src = if src == Src::User && line.eq_ignore_ascii_case("$H") { Src::Home } else { src };
        self.pending.push_back((bytes.len(), src));
        if line == "T0" || line == "T1" {
            if let Ok(mut s) = self.state.lock() {
                s.tool = Some(line.as_bytes()[1] - b'0');
            }
        }
        if line == "M3" || line == "M5" {
            if let Ok(mut s) = self.state.lock() {
                s.steering = line == "M3";
            }
        }
        if line == "M3" {
            self.tool_missing = false;
            if let Some(job) = &self.job {
                if let Ok(mut p) = job.progress.lock() {
                    p.message = "Scanning the tool in holder B (up to a minute)…".into();
                }
            }
        }
        log(&self.state, format!("> {line}"));
        Ok(())
    }

    fn stream(&mut self) -> std::io::Result<()> {
        if self.stopping.is_some() {
            return Ok(());
        }
        // console commands, one at a time; refused while a job runs
        if let Some(l) = self.user.front().cloned() {
            if self.job.is_some() {
                self.user.pop_front();
                log(&self.state, format!("(not sent while the job runs: {l})"));
            } else if self.pending.is_empty() {
                self.user.pop_front();
                self.write_line(&l, Src::User)?;
            }
        }
        let Some(job) = &self.job else { return Ok(()) };
        if job.finishing {
            let idle = self.state.lock().map(|s| s.status == "Idle").unwrap_or(false);
            if idle && self.pending.is_empty() {
                self.end_job(None);
            }
            return Ok(());
        }
        loop {
            let Some(job) = &self.job else { break };
            let Some(line) = job.lines.get(job.next).cloned() else { break };
            let full = match self.dialect {
                Dialect::Stock => self.in_flight() + line.len() + 1 > RX_BUFFER,
                // send-and-wait is the only method tried on this machine
                Dialect::Maker3 => !self.pending.is_empty(),
            };
            if full {
                break;
            }
            self.write_line(&line, Src::Job)?;
            if let Some(job) = &mut self.job {
                job.next += 1;
            }
        }
        Ok(())
    }

    fn read(&mut self) -> std::io::Result<()> {
        let mut buf = [0u8; 512];
        match self.port.read(&mut buf) {
            Ok(n) => self.rx.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
        while let Some(i) = self.rx.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.rx.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if !line.is_empty() {
                self.on_line(&line)?;
            }
        }
        Ok(())
    }

    fn answered(&self) {
        if let Ok(mut s) = self.state.lock() {
            if !s.answered {
                s.answered = true;
                s.error = None;
            }
        }
    }

    fn on_line(&mut self, line: &str) -> std::io::Result<()> {
        if line.starts_with('<') {
            self.answered();
            self.status(line);
            return Ok(());
        }
        let lower = line.to_lowercase();
        if line == "ok" {
            let answered = self.pending.pop_front().map(|p| p.1);
            if answered == Some(Src::Home) {
                if let Ok(mut s) = self.state.lock() {
                    s.needs_home = false;
                }
            }
            if answered == Some(Src::Job) {
                if let Some(job) = &mut self.job {
                    job.acked += 1;
                    if job.lines.get(job.acked - 1).is_some_and(|l| l.eq_ignore_ascii_case("$H")) {
                        if let Ok(mut s) = self.state.lock() {
                            s.needs_home = false;
                        }
                    }
                    let (acked, total) = (job.acked, job.lines.len());
                    if let Ok(mut p) = job.progress.lock() {
                        p.sent = acked;
                        p.message = format!("Cutting… line {acked} of {total}");
                    }
                    if acked >= total {
                        job.finishing = true;
                        if let Ok(mut p) = job.progress.lock() {
                            p.message = "Finishing the last moves…".into();
                        }
                    }
                }
                // here a job ends on the ok of its G4 P0, which waits for every move
                if self.dialect == Dialect::Maker3 && self.job.as_ref().is_some_and(|j| j.finishing) {
                    self.end_job(None);
                }
            }
            return Ok(());
        }
        log(&self.state, line.to_string());
        if let Some(rest) = line.strip_prefix("[MSG:Tool ") {
            // what the scan of holder B found
            let name = rest.split(',').next().unwrap_or("").trim_end_matches(']').trim().to_string();
            self.tool_missing = name == "not found";
            if let Ok(mut s) = self.state.lock() {
                s.detected = Some(name.clone());
                s.steering = line.contains("steering on");
            }
            if let Some(want) = self.job.as_mut().and_then(|j| j.expect.pop_front()) {
                if name != want {
                    let found = if self.tool_missing { "no drive-housing tool (it's empty, or has the fine-point blade)".to_string() } else { format!("the {}", reported_label(&name)) };
                    self.end_job(Some(format!("Holder B has {found}, but this job needs the {}. Nothing more was cut.", reported_label(&want))));
                    if !self.tool_missing {
                        self.user.push_back("M5".into());
                    }
                }
            }
            return Ok(());
        }
        if let Some(n) = lower.strip_prefix("error:") {
            let n: u32 = n.trim().parse().unwrap_or(0);
            let src = self.pending.pop_front().map(|p| p.1);
            if src == Some(Src::Job) {
                let at = self.job.as_ref().map(|j| j.acked).unwrap_or(0);
                let text = self.job.as_ref().and_then(|j| j.lines.get(at)).cloned().unwrap_or_default();
                self.stop()?;
                self.end_job(Some(format!("GRBL refused line {} ({text}): error {n}, {}", at + 1, error_text(n))));
            } else if let Ok(mut s) = self.state.lock() {
                s.error = Some(format!("error {n}: {}", error_text(n)));
            }
        } else if let Some(n) = lower.strip_prefix("alarm:") {
            let n: u32 = n.trim().parse().unwrap_or(0);
            self.pending.clear();
            // a stalled machine goes quiet, so its next status report may never come
            if let Ok(mut s) = self.state.lock() {
                s.status = "Alarm".into();
                s.needs_home = true;
            }
            if let Ok(mut s) = self.state.lock() {
                s.steering = false;
            }
            let missing = n == 9 && std::mem::take(&mut self.tool_missing);
            let msg = if missing {
                // no tool to scan: not a lost position, so clear it and carry on
                if let Ok(mut s) = self.state.lock() {
                    s.needs_home = false;
                }
                self.user.push_front("$X".into());
                "No drive-housing tool was found in holder B (it's empty, or has the fine-point blade, which can't be scanned).".to_string()
            } else {
                alarm_message(self.dialect, n)
            };
            if self.job.is_some() {
                self.end_job(Some(msg));
            } else if let Ok(mut s) = self.state.lock() {
                s.error = Some(msg);
            }
        } else if lower.starts_with("grbl") || lower.contains("fluidnc") {
            // the greeting: the controller has (re)started
            self.answered();
            self.pending.clear();
            if let Ok(mut s) = self.state.lock() {
                s.version = line.to_string();
                s.tool = None;
                s.steering = false;
            }
            if self.job.is_some() && self.stopping.is_none() {
                self.end_job(Some("the controller restarted".into()));
            }
        }
        Ok(())
    }

    fn status(&mut self, line: &str) {
        let body = line.trim_start_matches('<').trim_end_matches('>');
        let mut parts = body.split('|');
        let status = parts.next().unwrap_or("").to_string();
        // the Maker 3 sends no state name while it loads a mat
        let status = if status.is_empty() { "Busy".to_string() } else { status };
        let xyz = |v: &str| -> Option<[f64; 3]> {
            let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (n.len() >= 2).then(|| [n[0], n[1], n.get(2).copied().unwrap_or(0.0)])
        };
        let (mut mpos, mut wpos) = (None, None);
        for p in parts {
            if let Some(v) = p.strip_prefix("MPos:") {
                mpos = xyz(v);
            } else if let Some(v) = p.strip_prefix("WPos:") {
                wpos = xyz(v);
            } else if let Some(v) = p.strip_prefix("WCO:") {
                if let Some(w) = xyz(v) {
                    self.wco = w;
                }
            }
        }
        let wco = self.wco;
        let mut choose_pen = false;
        if let Ok(mut s) = self.state.lock() {
            s.status = status;
            if let Some(m) = mpos {
                s.mpos = m;
                s.wpos = [m[0] - wco[0], m[1] - wco[1], m[2] - wco[2]];
            } else if let Some(w) = wpos {
                s.wpos = w;
                s.mpos = [w[0] + wco[0], w[1] + wco[1], w[2] + wco[2]];
            }
            choose_pen = s.status == "Idle" && s.tool.is_none() && !s.busy;
        }
        // The Maker 3 can't say which tool is selected, and each tool's X
        // reach depends on it. Once it's idle, choose the one last used (the
        // pen at first): lifted first, and nothing else moves.
        if choose_pen && self.dialect == Dialect::Maker3 && self.job.is_none() && self.pending.is_empty() && self.user.is_empty() {
            let t = self.state.lock().map(|s| s.preferred_tool).unwrap_or(0);
            self.user.extend(maker3_select_tool(t));
        }
    }
}
