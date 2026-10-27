//! Icons and fonts from the web: the Icons panel (search Iconify's open icon
//! sets, or bring in any SVG by its link or from the clipboard) and the Web
//! fonts panel (Fontsource's open-licensed fonts, or a font file from
//! anywhere). Downloads run on background threads.

use crate::app::{Action, App};
use cutcraft_core::kurbo::{Affine, BezPath, Shape as _};
use cutcraft_core::{io, ops, web};
use egui::{RichText, Ui};
use std::sync::{Arc, Mutex};

/// Work done on a background thread: empty until it finishes.
type Job<T> = Arc<Mutex<Option<Result<T, String>>>>;

fn spawn<T: Send + 'static>(ctx: &egui::Context, work: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Job<T> {
    let job: Job<T> = Arc::new(Mutex::new(None));
    let (out, ctx) = (job.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = work().map_err(|e| e.to_string());
        if let Ok(mut o) = out.lock() {
            *o = Some(r);
        }
        ctx.request_repaint();
    });
    job
}

/// The result once a job is done (taking it out).
fn finished<T>(job: &mut Option<Job<T>>) -> Option<Result<T, String>> {
    let r = job.as_ref()?.lock().ok()?.take()?;
    *job = None;
    Some(r)
}

/// An icon found by a search, with an outline for its preview.
pub struct FoundIcon {
    icon: web::Icon,
    preview: BezPath,
}

pub struct WebState {
    query: String,
    icons: Vec<FoundIcon>,
    icon_job: Option<Job<Vec<FoundIcon>>>,
    link: String,
    link_job: Option<Job<String>>,
    /// The longer side of an icon or SVG added (mm).
    pub size: f64,
    note: String,
    fonts: Option<Vec<web::WebFont>>,
    fonts_job: Option<Job<Vec<web::WebFont>>>,
    font_filter: String,
    font_category: String,
    adding: Option<(String, Job<Vec<(String, Vec<u8>)>>)>,
    font_note: String,
    /// The system clipboard's text when CutCraft last copied shapes (so
    /// Ctrl+V only takes SVG from the clipboard if it's newer).
    clip_at_copy: Option<String>,
}

impl Default for WebState {
    fn default() -> Self {
        WebState {
            query: String::new(),
            icons: vec![],
            icon_job: None,
            link: String::new(),
            link_job: None,
            size: 60.0,
            note: String::new(),
            fonts: None,
            fonts_job: None,
            font_filter: String::new(),
            font_category: String::new(),
            adding: None,
            font_note: String::new(),
            clip_at_copy: None,
        }
    }
}

// ------------------------------------------------------------------ SVG in

/// Adds an SVG to the design as cuttable shapes (strokes made into
/// outlines), its longer side `size` mm, grouped at the centre of the page.
pub fn add_svg(app: &mut App, svg: &str, size: f64, name: &str) -> Result<(), String> {
    let shapes = io::import_svg_for_cutting(svg.as_bytes(), size).map_err(|e| format!("Couldn't use that SVG: {e}"))?;
    let c = app.doc.page().rect().center();
    let all: Vec<BezPath> = shapes.iter().flat_map(|s| s.world_paths()).collect();
    let middle = cutcraft_core::geom::paths_bbox(&all).map(|b| b.center()).unwrap_or_default();
    let layer = app.current_layer();
    let name = name.to_string();
    app.edit(move |doc, _| {
        let mut ids = vec![];
        for mut s in shapes {
            s.transform = Affine::translate(c - middle) * s.transform;
            ids.push(doc.add_shape(s, layer));
        }
        let ids = if ids.len() > 1 { ops::group(doc, &ids) } else { ids };
        if let Some(s) = ids.first().and_then(|id| doc.page_mut().shape_mut(*id)) {
            s.name = name;
        }
        Some(ids)
    });
    Ok(())
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Remembers the clipboard as it is when CutCraft copies shapes.
pub fn note_copy(app: &mut App) {
    app.web.clip_at_copy = clipboard_text();
}

/// Ctrl+V: if the clipboard holds SVG copied since CutCraft last copied
/// shapes (from SVG Repo's "Copy SVG", say), add that instead.
pub fn paste_svg(app: &mut App) -> bool {
    let Some(text) = clipboard_text() else { return false };
    if !web::looks_like_svg(&text) || app.web.clip_at_copy.as_deref() == Some(text.as_str()) {
        return false;
    }
    let size = app.web.size;
    match add_svg(app, &text, size, "Pasted SVG") {
        Ok(()) => app.status = "Pasted the SVG from the clipboard".into(),
        Err(e) => app.status = e,
    }
    // pasting again takes it again only if it's copied again
    app.web.clip_at_copy = Some(text);
    true
}

// ------------------------------------------------------------------ Icons panel

pub fn icons_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    let units = app.prefs.units;
    ui.heading("Icons");
    // finished downloads
    if let Some(r) = finished(&mut app.web.icon_job) {
        match r {
            Ok(v) => {
                app.web.note = if v.is_empty() { "No icons found - try another word".into() } else { String::new() };
                app.web.icons = v;
            }
            Err(e) => app.web.note = format!("Search failed: {e}"),
        }
    }
    if let Some(r) = finished(&mut app.web.link_job) {
        let size = app.web.size;
        match r.and_then(|svg| add_svg(app, &svg, size, "SVG from the web")) {
            Ok(()) => app.status = "Added the SVG from the link".into(),
            Err(e) => app.web.note = e,
        }
    }

    ui.horizontal(|ui| {
        ui.label("Size");
        crate::panels2::len_field(ui, &mut app.web.size, units);
    })
    .response
    .on_hover_text("The longer side of an icon or SVG you add");

    ui.add_space(4.0);
    ui.label(RichText::new("Search open icon sets").strong());
    let mut go = false;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut app.web.query).hint_text("heart, star, cat…").desired_width(190.0));
        go |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        go |= ui.add_enabled(app.web.icon_job.is_none(), egui::Button::new("Search")).clicked();
    });
    if go && !app.web.query.trim().is_empty() {
        let q = app.web.query.clone();
        app.web.note = "Searching…".into();
        app.web.icon_job = Some(spawn(ui.ctx(), move || {
            let found = web::search_icons(&q, 64)?;
            Ok(found
                .into_iter()
                .filter_map(|icon| {
                    let shapes = io::import_svg(icon.svg.as_bytes(), None).ok()?;
                    let mut preview = BezPath::new();
                    for s in shapes {
                        for p in s.world_paths() {
                            preview.extend(p.elements().iter().copied());
                        }
                    }
                    // previews are drawn from a simplified outline: make it big enough to stay smooth
                    let b = preview.bounding_box();
                    if b.area() <= 0.0 {
                        return None;
                    }
                    let preview = Affine::scale(100.0 / b.width().max(b.height())) * preview;
                    Some(FoundIcon { icon, preview })
                })
                .collect())
        }));
    }
    if app.web.icon_job.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Searching…");
        });
    } else if !app.web.note.is_empty() {
        ui.label(&app.web.note);
    }
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for (i, f) in app.web.icons.iter().enumerate() {
            let tip = format!("{}\n{} - {} licence", f.icon.id, f.icon.set_name, f.icon.license);
            if crate::panels2::art_tile(ui, f.icon.id.split(':').nth(1).unwrap_or(&f.icon.id), &f.preview).on_hover_text(tip).clicked() {
                picked = Some(i);
            }
        }
    });
    if let Some(i) = picked {
        let (svg, id, set, licence) = {
            let f = &app.web.icons[i].icon;
            (f.svg.clone(), f.id.clone(), f.set_name.clone(), f.license.clone())
        };
        let size = app.web.size;
        app.status = match add_svg(app, &svg, size, &id) {
            Ok(()) => format!("Added {id} from {set} ({licence} licence)"),
            Err(e) => e,
        };
    }
    if !app.web.icons.is_empty() {
        ui.small("Icons come from Iconify's open icon sets. Hover one for its set and licence - some licences ask you to credit the author.");
    }

    ui.separator();
    ui.label(RichText::new("Any SVG from the web").strong());
    crate::browser::site_button(app, ui, crate::browser::Site::SvgRepo, "Opens SVG Repo in CutCraft - click an icon's Download button and it comes straight into your design");
    ui.small("Works with SVG Repo and any other site: paste the SVG's link (or its page on SVG Repo), or copy the SVG (SVG Repo's “Copy SVG”) and paste it.");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.web.link).hint_text("https://…").desired_width(190.0));
        if ui.add_enabled(app.web.link_job.is_none() && !app.web.link.trim().is_empty(), egui::Button::new("Import")).clicked() {
            let url = app.web.link.trim().to_string();
            app.web.note = String::new();
            app.web.link_job = Some(spawn(ui.ctx(), move || web::fetch_svg(&url)));
        }
    });
    if app.web.link_job.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Downloading…");
        });
    }
    if ui.button("📋 Paste SVG").on_hover_text("Add SVG code copied from a website (Ctrl+V on the canvas does this too)").clicked() {
        match clipboard_text() {
            Some(t) if web::looks_like_svg(&t) => {
                let size = app.web.size;
                app.status = match add_svg(app, &t, size, "Pasted SVG") {
                    Ok(()) => "Pasted the SVG from the clipboard".into(),
                    Err(e) => e,
                };
            }
            _ => app.web.note = "The clipboard doesn't hold SVG code - use the site's “Copy SVG” button first".into(),
        }
    }
    ui.small("Check each design's licence before selling what you make with it.");
    None
}

// ------------------------------------------------------------------ Web fonts panel

const CATEGORIES: [(&str, &str); 6] = [("", "All"), ("sans-serif", "Sans serif"), ("serif", "Serif"), ("display", "Display"), ("handwriting", "Handwriting"), ("monospace", "Monospace")];

pub fn fonts_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Web fonts");
    ui.label("Add fonts to CutCraft without installing them in Windows.");

    if ui.button("➕ Add font file…").on_hover_text("A .ttf or .otf you've downloaded - from FONT Repo, Google Fonts, dafont or anywhere").clicked() {
        if let Some(files) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf"]).pick_files() {
            let mut added = vec![];
            for f in files {
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font.ttf".into());
                match std::fs::read(&f).map_err(anyhow::Error::from).and_then(|d| app.fonts.add_font(&name, d)) {
                    Ok(n) => added.extend(n),
                    Err(e) => app.web.font_note = format!("{name}: {e}"),
                }
            }
            if !added.is_empty() {
                fonts_changed(app);
                added.dedup();
                app.web.font_note = format!("Added {} - pick it in the Text panel's font list", added.join(", "));
            }
        }
    }
    crate::browser::site_button(app, ui, crate::browser::Site::FontRepo, "Opens FONT Repo in CutCraft - click a font's download link and it's added straight away");
    ui.small("Check a font's licence before selling what you make with it - many free fonts are for personal use only.");
    ui.separator();

    ui.label(RichText::new("Open-licensed fonts (Fontsource)").strong());
    ui.small("Over 2,000 fonts, nearly all under the SIL Open Font License - free to use, including for things you sell.");
    if let Some(r) = finished(&mut app.web.fonts_job) {
        match r {
            Ok(v) => app.web.fonts = Some(v),
            Err(e) => app.web.font_note = format!("Couldn't get the font list: {e}"),
        }
    }
    if app.web.fonts.is_none() && app.web.fonts_job.is_none() {
        if ui.button("Show fonts").clicked() {
            app.web.fonts_job = Some(spawn(ui.ctx(), web::font_list));
        }
    }
    if app.web.fonts_job.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Getting the font list…");
        });
    }
    let done = app.web.adding.as_ref().and_then(|(family, job)| Some((family.clone(), job.lock().ok()?.take()?)));
    if let Some((family, r)) = done {
        app.web.adding = None;
        {
            match r {
                Ok(files) => {
                    let mut ok = true;
                    for (name, data) in files {
                        if let Err(e) = app.fonts.add_font(&name, data) {
                            app.web.font_note = format!("{family}: {e}");
                            ok = false;
                        }
                    }
                    if ok {
                        fonts_changed(app);
                        app.web.font_note = format!("Added {family} - pick it in the Text panel's font list");
                    }
                }
                Err(e) => app.web.font_note = format!("Couldn't download {family}: {e}"),
            }
        }
    } else if let Some((family, _)) = &app.web.adding {
        let family = family.clone();
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Downloading {family}…"));
        });
    }
    if !app.web.font_note.is_empty() {
        ui.label(&app.web.font_note);
    }
    let Some(fonts) = &app.web.fonts else { return None };
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.web.font_filter).hint_text("search fonts").desired_width(150.0));
        egui::ComboBox::from_id_salt("webfont-cat").selected_text(CATEGORIES.iter().find(|c| c.0 == app.web.font_category).map(|c| c.1).unwrap_or("All")).show_ui(ui, |ui| {
            for (k, label) in CATEGORIES {
                ui.selectable_value(&mut app.web.font_category, k.to_string(), label);
            }
        });
    });
    let filter = app.web.font_filter.to_lowercase();
    let matches: Vec<&web::WebFont> = fonts
        .iter()
        .filter(|f| app.web.font_category.is_empty() || f.category == app.web.font_category)
        .filter(|f| filter.is_empty() || f.family.to_lowercase().contains(&filter))
        .collect();
    ui.small(format!("{} font{}", matches.len(), if matches.len() == 1 { "" } else { "s" }));
    let mut add = None;
    let busy = app.web.adding.is_some();
    egui::Grid::new("webfonts").num_columns(3).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
        for f in matches.iter().take(300) {
            let have = app.fonts.families().iter().any(|n| n == &f.family);
            ui.label(&f.family).on_hover_text(format!("{} · {} licence · weights {:?}", f.category, f.license, f.weights));
            ui.small(&f.category);
            if have {
                ui.small("✔ added");
            } else if ui.add_enabled(!busy, egui::Button::new("Add").small()).clicked() {
                add = Some((*f).clone());
            }
            ui.end_row();
        }
    });
    if matches.len() > 300 {
        ui.small("Showing the first 300 - search to narrow it down.");
    }
    if let Some(f) = add {
        let family = f.family.clone();
        app.web.font_note = String::new();
        app.web.adding = Some((family, spawn(ui.ctx(), move || web::font_files(&f))));
    }
    None
}

/// After fonts are added: the SVG importer sees them too.
pub(crate) fn fonts_changed(app: &mut App) {
    app.fontdb = Arc::new(app.fonts.database().clone());
}
