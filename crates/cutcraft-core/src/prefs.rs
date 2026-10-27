//! User preferences and user material presets, stored as JSON in the config dir.

use crate::materials::Material;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    Mm,
    Inch,
}

impl Units {
    pub fn factor(&self) -> f64 {
        match self {
            Units::Mm => 1.0,
            Units::Inch => 1.0 / 25.4,
        }
    }
    pub fn suffix(&self) -> &'static str {
        match self {
            Units::Mm => "mm",
            Units::Inch => "in",
        }
    }
}

/// Where the main window was (outer position and inner size in points, and
/// the display scale they were measured at), and whether it was maximised.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub units: Units,
    pub default_machine: String,
    pub default_font: String,
    pub default_text_size: f64,
    pub nudge_mm: f64,
    pub paste_offset_mm: f64,
    pub dark_mode: bool,
    pub show_rulers: bool,
    pub recent_files: Vec<String>,
    pub autosave_minutes: u32,
    pub user_materials: Vec<Material>,
    pub last_tcp_address: String,
    pub track_enhancing: bool,
    pub spell_dictionary: String,
    pub custom_words: Vec<String>,
    /// Wire language sent to the cutter (GPGL for Silhouette, HP-GL for others).
    pub protocol: crate::gpgl::Protocol,
    /// Which GPGL command variant to emit.
    pub gpgl_dialect: crate::gpgl::GpglDialect,
    /// Edit-bar anchor point (0..9, row-major from the top-left).
    pub anchor: usize,
    /// Line the dragged selection up with other shapes and the page.
    pub smart_guides: bool,
    /// Snap drawing and point edits to other shapes' corners, centres and nodes.
    pub snap_objects: bool,
    /// Draw locked sizes and distances on the canvas.
    pub show_constraints: bool,
    /// Where the user has dragged each popup window (its centre, in points).
    pub dialog_pos: std::collections::HashMap<String, [f32; 2]>,
    /// Print & Cut corrections by machine id (from the calibration wizard).
    pub print_cut_calibration: std::collections::HashMap<String, crate::calibration::Calibration>,
    /// Block ads in the built-in browser.
    pub browser_adblock: bool,
    /// The main window's last position and size, restored at start.
    pub window: Option<WindowPlacement>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            units: Units::Mm,
            default_machine: "cameo5".into(),
            default_font: "Arial".into(),
            default_text_size: 20.0,
            nudge_mm: 1.0,
            paste_offset_mm: 5.0,
            dark_mode: true,
            show_rulers: true,
            recent_files: vec![],
            autosave_minutes: 5,
            user_materials: vec![],
            last_tcp_address: "192.168.1.50:9100".into(),
            track_enhancing: false,
            spell_dictionary: String::new(),
            custom_words: vec![],
            protocol: crate::gpgl::Protocol::default(),
            gpgl_dialect: crate::gpgl::GpglDialect::default(),
            anchor: 0,
            smart_guides: true,
            snap_objects: true,
            show_constraints: true,
            dialog_pos: Default::default(),
            print_cut_calibration: Default::default(),
            browser_adblock: true,
            window: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("CutCraft")
}

impl Prefs {
    pub fn load() -> Prefs {
        std::fs::read(config_dir().join("prefs.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let d = config_dir();
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("prefs.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
    pub fn add_recent(&mut self, p: &str) {
        self.recent_files.retain(|x| x != p);
        self.recent_files.insert(0, p.into());
        self.recent_files.truncate(10);
    }
}
