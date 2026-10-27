//! Where popup windows open: centred on the main window, unless the user has
//! dragged that popup somewhere — then where they left it (remembered between
//! runs in the preferences). The spot is only set on the frame a popup
//! appears, so it can still be dragged while open.

use cutcraft_core::prefs::Prefs;
use std::collections::HashSet;

#[derive(Default)]
pub struct Placement {
    /// Popups shown this frame, and last frame.
    seen: HashSet<String>,
    prev: HashSet<String>,
}

impl Placement {
    /// Positions `w` (the popup called `key`) when it first appears.
    pub fn place<'a>(&mut self, prefs: &Prefs, ctx: &egui::Context, key: &str, w: egui::Window<'a>) -> egui::Window<'a> {
        self.seen.insert(key.to_string());
        // always placed by its centre, so the remembered spot means the same thing every frame
        let w = w.pivot(egui::Align2::CENTER_CENTER);
        if self.prev.contains(key) {
            return w;
        }
        let screen = ctx.content_rect();
        match prefs.dialog_pos.get(key) {
            Some(&[x, y]) if screen.shrink(20.0).contains(egui::pos2(x, y)) => w.current_pos(egui::pos2(x, y)),
            _ => w.current_pos(screen.center()),
        }
    }

    /// Remembers where the user dragged the popup `key` to.
    pub fn moved<R>(&mut self, prefs: &mut Prefs, key: &str, shown: &Option<egui::InnerResponse<R>>) {
        if let Some(r) = shown {
            if r.response.dragged() {
                let c = r.response.rect.center();
                prefs.dialog_pos.insert(key.to_string(), [c.x, c.y]);
            }
            if r.response.drag_stopped() {
                let _ = prefs.save();
            }
        }
    }

    /// Call once at the end of every frame.
    pub fn end_frame(&mut self) {
        self.prev = std::mem::take(&mut self.seen);
    }
}

/// A tidy question popup: no title strip, padded and rounded. Use with
/// [`prompt`] inside.
pub fn prompt_window(ctx: &egui::Context, id: &str) -> egui::Window<'static> {
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::symmetric(26, 22)).corner_radius(egui::CornerRadius::same(12));
    egui::Window::new(id.to_string()).title_bar(false).collapsible(false).resizable(false).frame(frame)
}

/// One button of a [`prompt`]: its label, whether it's the main choice (in the
/// accent colour), and an optional tooltip.
pub struct Choice<'a> {
    pub label: &'a str,
    pub primary: bool,
    pub tip: Option<&'a str>,
}

impl<'a> Choice<'a> {
    pub fn main(label: &'a str) -> Self {
        Choice { label, primary: true, tip: None }
    }
    pub fn other(label: &'a str) -> Self {
        Choice { label, primary: false, tip: None }
    }
    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }
}

/// A centred heading, message lines and a centred row of equal buttons.
/// Returns the index of the button clicked.
pub fn prompt(ui: &mut egui::Ui, heading: &str, lines: &[String], choices: &[Choice]) -> Option<usize> {
    const W: f32 = 400.0;
    const BTN: egui::Vec2 = egui::vec2(112.0, 34.0);
    const GAP: f32 = 10.0;
    ui.set_width(W);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(heading).size(19.0).strong().color(ui.visuals().strong_text_color()));
        ui.add_space(10.0);
        for l in lines {
            ui.label(egui::RichText::new(l).size(14.0));
        }
    });
    ui.add_space(18.0);
    let mut clicked = None;
    let n = choices.len() as f32;
    let total = n * BTN.x + (n - 1.0).max(0.0) * GAP;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.add_space(((W - total) / 2.0).max(0.0));
        for (i, c) in choices.iter().enumerate() {
            let text = egui::RichText::new(c.label).size(14.0);
            let b = if c.primary {
                egui::Button::new(text.strong().color(egui::Color32::WHITE)).fill(crate::chrome::ACCENT)
            } else {
                egui::Button::new(text)
            };
            let mut r = ui.add(b.min_size(BTN).corner_radius(egui::CornerRadius::same(8)));
            if let Some(t) = c.tip {
                r = r.on_hover_text(t);
            }
            if r.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}
