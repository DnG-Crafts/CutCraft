//! A step-by-step start-up log (`startup.log` in the settings folder) and the
//! choice of graphics renderer. Ported from BurnCraft.
//!
//! A release build has no console, so if start-up stops early there would
//! otherwise be nothing to go on. The log's "graphics:" line also says which
//! graphics chip draws the window — "Basic Render Driver" or a `Cpu` device
//! means software drawing, which is slow and uses a lot of processor time.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

fn data_dir() -> PathBuf {
    cutcraft_core::prefs::config_dir()
}

pub fn log_path() -> PathBuf {
    data_dir().join("startup.log")
}

static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Line written just before the GPU renderer starts, and the one written once a frame is shown.
const OPENING_GPU: &str = "opening the window (Wgpu renderer)";
const FIRST_FRAME_NOTE: &str = "first frame drawn: the window is open";

/// Marker file: the GPU renderer crashed on this PC, so use OpenGL from now on.
fn opengl_marker() -> PathBuf {
    data_dir().join("use-opengl")
}

/// Whether to skip the GPU renderer: it crashed here before (remembered), or
/// the previous start died while it was starting up (the log ends at that
/// step). Call before [`begin`].
pub fn prefer_opengl() -> bool {
    if opengl_marker().exists() {
        return true;
    }
    let Ok(previous) = std::fs::read_to_string(log_path()) else { return false };
    let died_starting_gpu = previous.lines().last().is_some_and(|l| l.ends_with(OPENING_GPU)) && !previous.contains(FIRST_FRAME_NOTE);
    if died_starting_gpu {
        let _ = std::fs::write(opengl_marker(), "The GPU renderer crashed while starting on this PC; CutCraft uses OpenGL. Delete this file to try the GPU renderer again.\n");
    }
    died_starting_gpu
}

/// Starts a fresh start-up log for this run.
pub fn begin() {
    STARTED.get_or_init(Instant::now);
    let _ = std::fs::create_dir_all(data_dir());
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = std::fs::write(log_path(), format!("CutCraft {} starting ({stamp})\n", env!("CARGO_PKG_VERSION")));
}

/// Adds a line to the start-up log, with the time since start.
pub fn note(what: &str) {
    let t = STARTED.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(log_path()) {
        let _ = writeln!(f, "{t:7.2}s  {what}");
    }
}

static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// Called every frame; logs the first one (the window is up and drawing).
pub fn frame_shown() {
    if !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        note(FIRST_FRAME_NOTE);
    }
}

/// Logs the graphics adapter and renderer in use.
pub fn log_renderer(cc: &eframe::CreationContext<'_>) {
    if let Some(rs) = &cc.wgpu_render_state {
        let i = rs.adapter.get_info();
        note(&format!("graphics: {} via {:?} ({:?}), driver {} {}", i.name, i.backend, i.device_type, i.driver, i.driver_info));
    }
    if let Some(gl) = &cc.gl {
        use eframe::glow::HasContext;
        // SAFETY: plain string queries on the context eframe created and keeps current.
        let (vendor, renderer, version) = unsafe { (gl.get_parameter_string(eframe::glow::VENDOR), gl.get_parameter_string(eframe::glow::RENDERER), gl.get_parameter_string(eframe::glow::VERSION)) };
        note(&format!("graphics: OpenGL {version}, {renderer} ({vendor})"));
    }
}

/// Picks a graphics adapter for the DirectX 12 / Vulkan renderer, refusing
/// software ones (Windows' "Microsoft Basic Render Driver" draws every frame on
/// the processor). With no hardware adapter the renderer fails to start, and
/// CutCraft tries OpenGL, which usually reaches the graphics chip through its
/// own driver.
pub fn hardware_adapter(adapters: &[eframe::wgpu::Adapter], surface: Option<&eframe::wgpu::Surface<'_>>) -> Result<eframe::wgpu::Adapter, String> {
    use eframe::wgpu::DeviceType;
    let rank = |t: DeviceType| match t {
        DeviceType::DiscreteGpu => 0,
        DeviceType::IntegratedGpu => 1,
        DeviceType::VirtualGpu | DeviceType::Other => 2,
        DeviceType::Cpu => 3,
    };
    adapters
        .iter()
        .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
        .filter(|a| a.get_info().device_type != DeviceType::Cpu)
        .min_by_key(|a| rank(a.get_info().device_type))
        .cloned()
        .ok_or_else(|| {
            let found: Vec<String> = adapters.iter().map(|a| format!("{} ({:?})", a.get_info().name, a.get_info().device_type)).collect();
            format!("no hardware graphics adapter for DirectX 12 / Vulkan (found: {})", if found.is_empty() { "none".into() } else { found.join(", ") })
        })
}

/// Watches for the window redrawing continuously while idle, and notes why in
/// startup.log (ported from BurnCraft) — software drawing makes every frame
/// expensive, so an idle window should draw (almost) nothing.
#[derive(Default)]
pub struct FrameWatch {
    since: Option<f64>,
    idle_frames: u32,
    causes: std::collections::HashMap<String, u32>,
    reports: u32,
}

/// Seconds per check, and idle frames per second above which a check is logged.
const WATCH_WINDOW: f64 = 10.0;
const BUSY_FPS: f64 = 5.0;
/// Most reports per run, so the log stays small.
const MAX_REPORTS: u32 = 3;

impl FrameWatch {
    /// Call once per frame, at the end of the frame.
    pub fn frame(&mut self, ctx: &egui::Context) {
        if self.reports >= MAX_REPORTS {
            return;
        }
        let now = ctx.input(|i| i.time);
        let since = *self.since.get_or_insert(now);
        // frames with input (moving the mouse, typing) are expected; count the others
        let input = ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving() || i.pointer.any_down());
        if !input {
            self.idle_frames += 1;
            for cause in ctx.repaint_causes() {
                *self.causes.entry(cause.to_string()).or_default() += 1;
            }
        }
        if now - since < WATCH_WINDOW {
            return;
        }
        let fps = f64::from(self.idle_frames) / (now - since);
        if fps > BUSY_FPS {
            self.reports += 1;
            let mut causes: Vec<(String, u32)> = self.causes.drain().collect();
            causes.sort_by_key(|c| std::cmp::Reverse(c.1));
            let list: Vec<String> = causes.iter().take(8).map(|(c, n)| format!("{c} ×{n}")).collect();
            note(&format!(
                "redrawing while idle: {fps:.0} frames/s for {:.0} s; asked for by: {}",
                now - since,
                if list.is_empty() { "(nothing recorded)".to_string() } else { list.join("; ") }
            ));
        }
        self.since = Some(now);
        self.idle_frames = 0;
        self.causes.clear();
    }
}

/// Whether a saved window spot is still on a screen (a monitor may have been
/// unplugged since): its title bar must be inside the desktop.
pub fn placement_visible(w: &cutcraft_core::prefs::WindowPlacement) -> bool {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetSystemMetrics(index: i32) -> i32;
        }
        // SAFETY: plain metric queries (SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN).
        let (x, y, cw, ch) = unsafe { (GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79)) };
        let k = w.scale.max(0.5);
        // a point on the title bar, in physical pixels
        let (px, py) = ((w.x + 60.0) * k, (w.y + 12.0) * k);
        px >= x as f32 && py >= y as f32 && px < (x + cw) as f32 && py < (y + ch) as f32
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        true
    }
}
