//! The window chrome: theme, the icon toolbar under the menu, the left tool
//! strip, and the category rail beside the side panel. Layout ideas from
//! BurnCraft (icon toolbar, two-column tool strip, titled panels) and Cricut
//! Design Space (a category rail and one clear "Make it" button).

use crate::app::{Action, App, Tool};
use crate::icons::{self, Icon};
use crate::panels::Panel;
use cutcraft_core::ops::AlignMode;
use egui::{Color32, CornerRadius, RichText, Stroke, Ui};

/// CutCraft's accent blue.
pub const ACCENT: Color32 = Color32::from_rgb(47, 128, 237);

/// Softer neutral greys, rounded widgets, one accent colour and a little more
/// room than egui's defaults — in both themes.
pub fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let g = |l: u8| Color32::from_gray(l);
    if dark {
        v.panel_fill = g(32);
        v.window_fill = g(36);
        v.extreme_bg_color = g(22);
        v.faint_bg_color = g(38);
        v.code_bg_color = g(26);
        v.window_stroke = Stroke::new(1.0, g(52));
        v.widgets.noninteractive.bg_fill = g(32);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, g(48));
        v.widgets.inactive.bg_fill = g(46);
        v.widgets.inactive.weak_bg_fill = g(44);
        v.widgets.hovered.bg_fill = g(58);
        v.widgets.hovered.weak_bg_fill = g(56);
        v.widgets.active.bg_fill = g(66);
        v.widgets.active.weak_bg_fill = g(64);
        v.weak_text_color = Some(g(150));
    } else {
        v.panel_fill = g(244);
        v.window_fill = g(250);
        v.extreme_bg_color = g(226);
        v.faint_bg_color = g(236);
        v.window_stroke = Stroke::new(1.0, g(210));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, g(218));
        v.widgets.inactive.bg_fill = g(228);
        v.widgets.inactive.weak_bg_fill = g(230);
        v.widgets.hovered.bg_fill = g(216);
        v.widgets.hovered.weak_bg_fill = g(218);
        v.weak_text_color = Some(g(105));
    }
    v.selection.bg_fill = if dark { ACCENT.gamma_multiply(0.55) } else { Color32::from_rgb(170, 200, 245) };
    v.selection.stroke = Stroke::new(1.0, if dark { Color32::from_rgb(150, 195, 255) } else { Color32::from_rgb(20, 70, 150) });
    v.hyperlink_color = ACCENT;
    let r = CornerRadius::same(5);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(7.0, 5.0);
        s.spacing.button_padding = egui::vec2(7.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}

// ------------------------------------------------------------------ top toolbar

const TOP: f32 = 28.0;

/// The icon toolbar under the menu, with the "Make it" button on the right.
pub fn top_toolbar(app: &App, ui: &mut Ui) -> Option<Action> {
    let mut act = None;
    let has_sel = !app.sel.is_empty();
    let two = app.sel.len() >= 2;
    let (undo, redo, clip) = (!app.undo.is_empty(), !app.redo.is_empty(), !app.clipboard.is_empty());
    ui.horizontal(|ui| {
        let mut cmd = |ui: &mut Ui, icon: Icon, a: Action, enabled: bool, tip: &str| {
            if icons::button(ui, icon, TOP, false, enabled, tip).clicked() {
                act = Some(a);
            }
        };
        ui.spacing_mut().item_spacing.x = 2.0;
        cmd(ui, Icon::New, Action::New, true, "New (Ctrl+N)");
        cmd(ui, Icon::Open, Action::Open, true, "Open (Ctrl+O)");
        cmd(ui, Icon::Save, Action::Save, true, "Save (Ctrl+S)");
        cmd(ui, Icon::Import, Action::Import, true, "Import SVG, DXF, PDF, images… (Ctrl+I)");
        cmd(ui, Icon::Export, Action::ExportSvg, true, "Export SVG");
        ui.separator();
        cmd(ui, Icon::Undo, Action::Undo, undo, "Undo (Ctrl+Z)");
        cmd(ui, Icon::Redo, Action::Redo, redo, "Redo (Ctrl+Y)");
        ui.separator();
        cmd(ui, Icon::Cut, Action::Cut, has_sel, "Cut (Ctrl+X)");
        cmd(ui, Icon::Copy, Action::Copy, has_sel, "Copy (Ctrl+C)");
        cmd(ui, Icon::Paste, Action::Paste, clip, "Paste (Ctrl+V)");
        cmd(ui, Icon::Delete, Action::Delete, has_sel, "Delete (Del)");
        ui.separator();
        cmd(ui, Icon::ZoomFit, Action::ZoomFitPage, true, "Fit page (Ctrl+0)");
        cmd(ui, Icon::ZoomIn, Action::ZoomIn, true, "Zoom in (+)");
        cmd(ui, Icon::ZoomOut, Action::ZoomOut, true, "Zoom out (−)");
        cmd(ui, Icon::ZoomSelection, Action::ZoomFitSelection, has_sel, "Fit selection (Ctrl+1)");
        ui.separator();
        cmd(ui, Icon::Group, Action::Group, two, "Group (Ctrl+G)");
        cmd(ui, Icon::Ungroup, Action::Ungroup, has_sel, "Ungroup (Ctrl+Shift+G)");
        cmd(ui, Icon::MirrorH, Action::FlipH, has_sel, "Flip horizontally");
        cmd(ui, Icon::MirrorV, Action::FlipV, has_sel, "Flip vertically");
        ui.separator();
        for (icon, m, tip) in [
            (Icon::AlignLeft, AlignMode::Left, "Align left"),
            (Icon::AlignHCenter, AlignMode::CenterH, "Align centres horizontally"),
            (Icon::AlignRight, AlignMode::Right, "Align right"),
            (Icon::AlignTop, AlignMode::Top, "Align top"),
            (Icon::AlignVCenter, AlignMode::Middle, "Align middles vertically"),
            (Icon::AlignBottom, AlignMode::Bottom, "Align bottom"),
            (Icon::AlignCenters, AlignMode::Center, "Align centres"),
        ] {
            cmd(ui, icon, Action::Align(m), has_sel, &format!("{tip} (one shape: to the page)"));
        }
        cmd(ui, Icon::DistributeH, Action::DistributeH, app.sel.len() >= 3, "Distribute horizontally");
        cmd(ui, Icon::DistributeV, Action::DistributeV, app.sel.len() >= 3, "Distribute vertically");
        ui.separator();
        cmd(ui, Icon::Weld, Action::Weld, two, "Weld: join overlapping shapes into one outline");
        cmd(ui, Icon::Subtract, Action::Subtract, two, "Subtract: cut the top shape out of the one below");
        cmd(ui, Icon::Intersect, Action::Intersect, two, "Intersect: keep only the overlap");
        cmd(ui, Icon::Exclude, Action::Exclude, two, "Exclude: remove only where shapes overlap");
        cmd(ui, Icon::Unite, Action::Unite, two, "Unite: cut as one outline but keep the parts (ungroup to separate)");
        cmd(ui, Icon::Flatten, Action::Flatten, has_sel, "Flatten: make one printed picture with a single outer cut line (Print Then Cut)");

        // the one primary action, as in Cricut Design Space
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let make = egui::Button::new(RichText::new("  Make it  ").strong().size(15.0).color(Color32::WHITE))
                .fill(ACCENT)
                .corner_radius(CornerRadius::same(14))
                .min_size(egui::vec2(96.0, 26.0));
            if ui.add(make).on_hover_text("Preview the mats, choose each material and cut them one by one (or save them for a Brother ScanNCut)").clicked() {
                act = Some(Action::OpenPrepare);
            }
        });
    });
    act
}

// ------------------------------------------------------------------ left tool strip

const TOOL: f32 = 32.0;

/// Tools in groups (separated by lines), two to a row.
const TOOL_GROUPS: &[&[Tool]] = &[
    &[Tool::Select, Tool::Node, Tool::Pan, Tool::Zoom],
    &[Tool::Line, Tool::Polyline, Tool::Curve, Tool::Freehand],
    &[Tool::Rect, Tool::RoundRect, Tool::Ellipse, Tool::Polygon, Tool::Star, Tool::Text],
    &[Tool::Knife, Tool::Trim, Tool::Eraser, Tool::ImageEraser, Tool::Corner],
    &[Tool::Measure, Tool::Dimension],
];

pub const TOOL_STRIP_WIDTH: f32 = 2.0 * TOOL + 16.0;

pub fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Select,
        Tool::Node => Icon::Nodes,
        Tool::Pan => Icon::Pan,
        Tool::Zoom => Icon::ZoomIn,
        Tool::Line => Icon::Line,
        Tool::Polyline => Icon::Polyline,
        Tool::Curve => Icon::Curve,
        Tool::Freehand => Icon::Freehand,
        Tool::Rect => Icon::Rect,
        Tool::RoundRect => Icon::RoundRect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Polygon => Icon::Polygon,
        Tool::Star => Icon::Star,
        Tool::Text => Icon::Text,
        Tool::Knife => Icon::Knife,
        Tool::Eraser => Icon::Eraser,
        Tool::Corner => Icon::Corner,
        Tool::ImageEraser => Icon::ImageEraser,
        Tool::Measure => Icon::Measure,
        Tool::Dimension => Icon::Dimension,
        Tool::Trim => Icon::Trim,
    }
}

pub fn tool_strip(app: &mut App, ui: &mut Ui) {
    ui.add_space(6.0);
    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
        for (gi, group) in TOOL_GROUPS.iter().enumerate() {
            if gi > 0 {
                ui.add_space(2.0);
                ui.separator();
                ui.add_space(2.0);
            }
            for row in group.chunks(2) {
                ui.horizontal(|ui| {
                    ui.add_space(3.0);
                    for &t in row {
                        let tip = Tool::ALL.iter().find(|(x, _)| *x == t).map(|(_, tip)| *tip).unwrap_or("");
                        if icons::button(ui, tool_icon(t), TOOL, app.tool == t, true, tip).clicked() {
                            app.tool = t;
                            app.cs.poly.clear();
                            if t == Tool::Text {
                                app.panel = Panel::Text;
                            }
                        }
                    }
                });
            }
        }
    });
}

// ------------------------------------------------------------------ category rail

/// The side panel's categories, each a handful of panels.
pub const CATEGORIES: &[(&str, Icon, &[Panel])] = &[
    ("Design", Icon::Page, &[Panel::Page, Panel::Layers, Panel::History, Panel::Params]),
    ("Edit", Icon::Transform, &[Panel::Transform, Panel::Replicate, Panel::Modify, Panel::PathTools, Panel::Warp, Panel::Distort, Panel::DrawTools]),
    ("Style", Icon::Style, &[Panel::Style, Panel::Colours, Panel::Effects, Panel::Art, Panel::Icons, Panel::Dieline]),
    ("Text", Icon::Text, &[Panel::Text, Panel::Fonts, Panel::WebFonts, Panel::Barcode, Panel::MailMerge]),
    ("Image", Icon::Image, &[Panel::Image, Panel::Trace, Panel::PixScan]),
    ("Make", Icon::Make, &[Panel::Send, Panel::PrintCut, Panel::Queue]),
    ("More", Icon::More, &[Panel::Script, Panel::Prefs]),
];

pub fn category_of(p: Panel) -> usize {
    CATEGORIES.iter().position(|(_, _, ps)| ps.contains(&p)).unwrap_or(0)
}

pub const RAIL_WIDTH: f32 = 64.0;

/// The vertical category rail at the window's right edge.
pub fn rail(app: &mut App, ui: &mut Ui) {
    let cur = category_of(app.panel);
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (i, (name, icon, panels)) in CATEGORIES.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(RAIL_WIDTH - 8.0, 50.0), egui::Sense::click());
            let v = ui.visuals();
            let selected = i == cur;
            let bg = if selected {
                v.widgets.active.weak_bg_fill
            } else if resp.hovered() {
                v.widgets.hovered.weak_bg_fill
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, 6.0, bg);
            if selected {
                let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 8.0), egui::vec2(3.0, rect.height() - 16.0));
                ui.painter().rect_filled(bar, 2.0, ACCENT);
            }
            let ink = if selected { v.strong_text_color() } else { v.text_color() };
            let ir = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 8.0), egui::vec2(22.0, 22.0));
            icons::paint(ui.painter(), ir, *icon, ink, ACCENT);
            ui.painter().text(rect.center_bottom() - egui::vec2(0.0, 6.0), egui::Align2::CENTER_BOTTOM, *name, egui::FontId::proportional(11.0), ink);
            if resp.on_hover_text(panels.iter().map(|p| panel_name(*p)).collect::<Vec<_>>().join(" · ")).clicked() && !selected {
                app.panel = app.ps.cat_panel[i].filter(|p| panels.contains(p)).unwrap_or(panels[0]);
            }
        }
    });
}

pub fn panel_icon(p: Panel) -> Icon {
    match p {
        Panel::Page => Icon::Page,
        Panel::Layers => Icon::Layers,
        Panel::History => Icon::History,
        Panel::Params => Icon::Params,
        Panel::Transform => Icon::Transform,
        Panel::Replicate => Icon::GridArray,
        Panel::Modify => Icon::Offset,
        Panel::PathTools => Icon::PathTools,
        Panel::Warp => Icon::Warp,
        Panel::Distort => Icon::CircularArray,
        Panel::DrawTools => Icon::Polygon,
        Panel::Icons => Icon::Star,
        Panel::WebFonts => Icon::Fonts,
        Panel::Queue => Icon::Layers,
        Panel::Style => Icon::Style,
        Panel::Colours => Icon::Colours,
        Panel::Effects => Icon::Effects,
        Panel::Art => Icon::Art,
        Panel::Dieline => Icon::Dieline,
        Panel::Text => Icon::Text,
        Panel::Fonts => Icon::Fonts,
        Panel::Barcode => Icon::Barcode,
        Panel::MailMerge => Icon::Mail,
        Panel::Image => Icon::Image,
        Panel::Trace => Icon::Trace,
        Panel::PixScan => Icon::Camera,
        Panel::Send => Icon::Make,
        Panel::PrintCut => Icon::PrintCut,
        Panel::Script => Icon::Script,
        Panel::Prefs => Icon::Settings,
    }
}

/// A rounded tab with an icon and a label.
fn pill(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let size = egui::vec2(galley.size().x + 36.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let v = ui.visuals();
    let bg = if on {
        ACCENT
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        v.widgets.inactive.weak_bg_fill
    };
    ui.painter().rect_filled(rect, 12.0, bg);
    let ink = if on { Color32::WHITE } else { v.text_color() };
    let ir = egui::Rect::from_center_size(rect.left_center() + egui::vec2(15.0, 0.0), egui::vec2(14.0, 14.0));
    icons::paint(ui.painter(), ir, icon, ink, if on { Color32::WHITE } else { ACCENT });
    ui.painter().galley(egui::pos2(rect.left() + 27.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    resp
}

pub fn panel_name(p: Panel) -> &'static str {
    Panel::ALL.iter().find(|(x, _)| *x == p).map(|(_, n)| *n).unwrap_or("")
}

/// The side panel's header: pill tabs for the category's panels.
pub fn panel_header(app: &mut App, ui: &mut Ui) {
    let ci = category_of(app.panel);
    let (_, _, panels) = CATEGORIES[ci];
    app.ps.cat_panel[ci] = Some(app.panel);
    ui.add_space(6.0);
    if panels.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for &p in panels {
                if pill(ui, panel_icon(p), panel_name(p), app.panel == p).clicked() {
                    app.panel = p;
                }
            }
        });
        ui.add_space(4.0);
    }
    ui.separator();
    ui.add_space(4.0);
}
