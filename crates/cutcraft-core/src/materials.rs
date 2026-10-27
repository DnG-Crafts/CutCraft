//! Material presets (cut settings). Values are CutCraft's own starting points;
//! always do a test cut. User presets are stored as JSON next to preferences.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    AutoBlade,
    RatchetBlade,
    DeepCutBlade,
    KraftBlade,
    RotaryBlade,
    SketchPen,
    EmbossTool,
    PunchTool,
    StippleTool,
}

impl Tool {
    pub const ALL: [Tool; 9] = [
        Tool::AutoBlade,
        Tool::RatchetBlade,
        Tool::DeepCutBlade,
        Tool::KraftBlade,
        Tool::RotaryBlade,
        Tool::SketchPen,
        Tool::EmbossTool,
        Tool::PunchTool,
        Tool::StippleTool,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Tool::AutoBlade => "AutoBlade",
            Tool::RatchetBlade => "Ratchet blade",
            Tool::DeepCutBlade => "Deep-cut blade",
            Tool::KraftBlade => "Kraft blade",
            Tool::RotaryBlade => "Rotary blade",
            Tool::SketchPen => "Sketch pen",
            Tool::EmbossTool => "Emboss / score tool",
            Tool::PunchTool => "Punch tool",
            Tool::StippleTool => "Stipple / etching tool",
        }
    }
    pub fn is_blade(&self) -> bool {
        matches!(self, Tool::AutoBlade | Tool::RatchetBlade | Tool::DeepCutBlade | Tool::KraftBlade | Tool::RotaryBlade)
    }
    /// Swivel-blade tip offset in mm (0 for pens and rotary).
    pub fn blade_offset(&self) -> f64 {
        match self {
            Tool::AutoBlade | Tool::RatchetBlade => 0.27,
            Tool::DeepCutBlade | Tool::KraftBlade => 0.45,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Cut,
    Sketch,
    Score,
    Emboss,
    Perforate,
    /// Each small closed shape becomes a single tool plunge (dot).
    Stipple,
    None,
}

impl Action {
    pub const ALL: [Action; 7] =
        [Action::Cut, Action::Sketch, Action::Score, Action::Emboss, Action::Perforate, Action::Stipple, Action::None];
    pub fn label(&self) -> &'static str {
        match self {
            Action::Cut => "Cut",
            Action::Sketch => "Sketch",
            Action::Score => "Score",
            Action::Emboss => "Emboss",
            Action::Perforate => "Perforate",
            Action::Stipple => "Stipple (dots)",
            Action::None => "No action",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutSettings {
    pub tool: Tool,
    /// 1..=33
    pub force: u8,
    /// 1..=10 (cm/s on most machines)
    pub speed: u8,
    /// Blade exposure 0..=10 (AutoBlade/ratchet).
    pub depth: u8,
    pub passes: u8,
    /// Extend closed contours past their start, mm.
    pub overcut: f64,
    /// Apply swivel-blade compensation in software.
    pub blade_compensation: bool,
}

impl Default for CutSettings {
    fn default() -> Self {
        CutSettings { tool: Tool::AutoBlade, force: 10, speed: 5, depth: 3, passes: 1, overcut: 0.0, blade_compensation: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub category: String,
    pub settings: CutSettings,
    #[serde(default)]
    pub user: bool,
}

fn m(name: &str, cat: &str, tool: Tool, force: u8, speed: u8, depth: u8, passes: u8) -> Material {
    Material {
        name: name.into(),
        category: cat.into(),
        settings: CutSettings { tool, force, speed, depth, passes, overcut: 0.0, blade_compensation: false },
        user: false,
    }
}

pub fn builtin() -> Vec<Material> {
    use Tool::*;
    vec![
        m("Copy paper (20 lb)", "Paper", AutoBlade, 7, 8, 1, 1),
        m("Cardstock (65 lb)", "Cardstock", AutoBlade, 20, 4, 3, 1),
        m("Heavy cardstock (110 lb)", "Cardstock", AutoBlade, 30, 3, 5, 2),
        m("Glitter cardstock", "Cardstock", AutoBlade, 33, 3, 6, 2),
        m("Adhesive vinyl", "Vinyl", AutoBlade, 10, 5, 1, 1),
        m("Heat transfer vinyl (smooth)", "Heat Transfer", AutoBlade, 8, 8, 2, 1),
        m("Heat transfer vinyl (flock)", "Heat Transfer", AutoBlade, 14, 5, 4, 1),
        m("Printable sticker paper", "Sticker", AutoBlade, 15, 5, 2, 1),
        m("Clear sticker paper", "Sticker", AutoBlade, 15, 5, 2, 1),
        m("Kraft paper", "Paper", AutoBlade, 15, 5, 3, 1),
        m("Chipboard (1 mm)", "Board", KraftBlade, 33, 1, 10, 4),
        m("Felt", "Fabric", RotaryBlade, 33, 3, 0, 1),
        m("Bonded fabric", "Fabric", AutoBlade, 25, 3, 6, 2),
        m("Stencil material", "Film", AutoBlade, 18, 4, 3, 1),
        m("Magnet sheet", "Board", DeepCutBlade, 33, 2, 8, 2),
        m("Sketch pen", "Pen", SketchPen, 10, 5, 0, 1),
        m("Score cardstock", "Tools", EmbossTool, 20, 3, 0, 1),
        m("Emboss metal sheet", "Tools", EmbossTool, 33, 2, 0, 1),
        m("Stipple aluminium", "Tools", StippleTool, 30, 5, 0, 1),
    ]
}
