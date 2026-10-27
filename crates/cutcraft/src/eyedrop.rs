//! Screen eyedropper: pick a colour from anywhere on screen, even outside
//! CutCraft's window. While picking, the mouse and Esc are read straight from
//! the system each frame, and a clear layer over CutCraft's own window stops
//! the click from doing anything else there.

use crate::app::App;
use egui::Color32;

/// What a picked colour is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Fill,
    Line,
}

#[derive(Default)]
pub struct State {
    picking: Option<Picking>,
    /// A colour picked and not yet used by the Fill & Line panel.
    picked: Option<(Target, Color32)>,
    /// The colour under the pointer right now, while picking.
    pub live: Option<Color32>,
}

struct Picking {
    target: Target,
    /// Ready for the pick click (the button that started picking was let go).
    armed: bool,
    /// Picked: wait for the button to come up before stepping aside, so the
    /// click doesn't land on whatever is under it.
    done: bool,
}

/// Whether a screen eyedropper works here.
pub fn available() -> bool {
    cfg!(windows)
}

impl State {
    pub fn start(&mut self, target: Target) {
        self.picking = Some(Picking { target, armed: false, done: false });
        self.live = None;
    }

    pub fn active(&self) -> Option<Target> {
        self.picking.as_ref().filter(|p| !p.done).map(|p| p.target)
    }

    /// The colour picked for `target`, once.
    pub fn take(&mut self, target: Target) -> Option<Color32> {
        match self.picked {
            Some((t, c)) if t == target => {
                self.picked = None;
                Some(c)
            }
            _ => None,
        }
    }
}

/// Runs the eyedropper for this frame (call once per frame).
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let Some(p) = &mut app.eyedrop.picking else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    let down = sys::mouse_down();
    if p.done {
        if !down {
            app.eyedrop.picking = None;
        }
    } else if sys::esc_down() {
        app.eyedrop.picking = None;
        app.status = "Colour picking cancelled".into();
        return;
    } else {
        app.eyedrop.live = sys::colour_under_pointer();
        if !p.armed {
            p.armed = !down;
        } else if down {
            p.done = true;
            if let Some(c) = app.eyedrop.live {
                app.eyedrop.picked = Some((p.target, c));
                app.status = format!("Picked #{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
            }
        }
    }
    // cover CutCraft's window so the click only picks
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("eyedrop-cover")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if let (Some(pos), Some(c), false) = (ui.ctx().pointer_hover_pos(), app.eyedrop.live, app.eyedrop.picking.as_ref().is_some_and(|p| p.done)) {
            // the colour under the pointer, and how to finish
            let at = (pos + egui::vec2(18.0, 18.0)).min(rect.max - egui::vec2(230.0, 40.0));
            let r = egui::Rect::from_min_size(at, egui::vec2(220.0, 34.0));
            let painter = ui.painter();
            painter.rect_filled(r, 6.0, ui.visuals().window_fill);
            painter.rect_stroke(r, 6.0, ui.visuals().window_stroke, egui::StrokeKind::Inside);
            let sw = egui::Rect::from_min_size(r.min + egui::vec2(6.0, 6.0), egui::vec2(22.0, 22.0));
            painter.rect_filled(sw, 3.0, c);
            painter.text(sw.right_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("#{:02x}{:02x}{:02x} - click to pick, Esc cancels", c.r(), c.g(), c.b()), egui::FontId::proportional(12.0), ui.visuals().text_color());
        }
    });
}

#[cfg(windows)]
mod sys {
    use egui::Color32;

    #[repr(C)]
    struct Pt {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(p: *mut Pt) -> i32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetDC(wnd: isize) -> isize;
        fn ReleaseDC(wnd: isize, dc: isize) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetPixel(dc: isize, x: i32, y: i32) -> u32;
    }

    const VK_LBUTTON: i32 = 0x01;
    const VK_ESCAPE: i32 = 0x1B;
    const CLR_INVALID: u32 = 0xFFFF_FFFF;

    pub fn mouse_down() -> bool {
        // SAFETY: reads the key's state; no pointers involved.
        unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0 }
    }

    pub fn esc_down() -> bool {
        // SAFETY: as above.
        unsafe { GetAsyncKeyState(VK_ESCAPE) as u16 & 0x8000 != 0 }
    }

    /// The screen pixel under the mouse pointer.
    pub fn colour_under_pointer() -> Option<Color32> {
        let mut p = Pt { x: 0, y: 0 };
        // SAFETY: `p` is a valid POINT to fill; the whole-screen DC is released
        // straight after reading one pixel from it.
        unsafe {
            if GetCursorPos(&mut p) == 0 {
                return None;
            }
            let dc = GetDC(0);
            if dc == 0 {
                return None;
            }
            let c = GetPixel(dc, p.x, p.y);
            ReleaseDC(0, dc);
            // COLORREF is 0x00BBGGRR
            (c != CLR_INVALID).then(|| Color32::from_rgb((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8))
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use egui::Color32;
    pub fn mouse_down() -> bool {
        false
    }
    pub fn esc_down() -> bool {
        true
    }
    pub fn colour_under_pointer() -> Option<Color32> {
        None
    }
}
