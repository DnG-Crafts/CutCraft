//! The Queue panel (Make ▸ Queue): line up several designs with their copies
//! and cut them as one production run on the Make it screen.

use crate::app::{Action, App};
use cutcraft_core::queue::{self, QueueJob};
use egui::{DragValue, RichText, Ui};

pub fn queue_panel(app: &mut App, ui: &mut Ui) -> Option<Action> {
    ui.heading("Production queue");
    ui.label("Line up several designs, each with its own number of copies, and cut them in one run.");
    ui.horizontal_wrapped(|ui| {
        if ui.button("➕ Add files…").on_hover_text("CutCraft designs, SVG, DXF, PDF, FCM… - several at once").clicked() {
            if let Some(files) = rfd::FileDialog::new().add_filter("Designs", &cutcraft_core::io::IMPORT_EXTS.iter().copied().chain(["cutcraft"]).collect::<Vec<_>>()).pick_files() {
                let like = app.doc.page().clone();
                let mut failed = vec![];
                for f in files {
                    match QueueJob::from_file(&f, Some(app.fontdb.clone()), &like) {
                        Ok(job) => app.queue.push(job),
                        Err(e) => failed.push(format!("{}: {e}", f.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())),
                    }
                }
                app.status = if failed.is_empty() { format!("{} job{} in the queue", app.queue.len(), if app.queue.len() == 1 { "" } else { "s" }) } else { format!("Couldn't add {}", failed.join("; ")) };
            }
        }
        if ui.button("➕ Add this design").on_hover_text("A snapshot of the design open now").clicked() {
            let base = app.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "This design".into());
            // the same design twice: number the second
            let same = app.queue.iter().filter(|j| j.name == base || j.name.starts_with(&format!("{base} ("))).count();
            let name = if same == 0 { base } else { format!("{base} ({})", same + 1) };
            let job = QueueJob::from_document(&name, &app.doc);
            app.queue.push(job);
        }
    });
    ui.separator();

    if app.queue.is_empty() {
        ui.weak("The queue is empty.");
        return None;
    }
    let (mut up, mut down, mut remove) = (None, None, None);
    let n = app.queue.len();
    egui::Grid::new("queue").num_columns(4).striped(true).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label(RichText::new("Job").strong());
        ui.label(RichText::new("Copies").strong());
        ui.label("");
        ui.label("");
        ui.end_row();
        for (i, job) in app.queue.iter_mut().enumerate() {
            let pages = job.pages.len();
            ui.label(&job.name).on_hover_text(format!("{} shape{} on {pages} page{}", job.shape_count(), if job.shape_count() == 1 { "" } else { "s" }, if pages == 1 { "" } else { "s" }));
            ui.add(DragValue::new(&mut job.copies).range(1..=500));
            ui.horizontal(|ui| {
                if ui.add_enabled(i > 0, egui::Button::new("↑").small()).on_hover_text("Cut earlier").clicked() {
                    up = Some(i);
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).on_hover_text("Cut later").clicked() {
                    down = Some(i);
                }
            });
            if ui.button("✕").on_hover_text("Take it out of the queue").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = up {
        app.queue.swap(i, i - 1);
    }
    if let Some(i) = down {
        app.queue.swap(i, i + 1);
    }
    if let Some(i) = remove {
        app.queue.remove(i);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(RichText::new("✂ Cut the queue").strong()).on_hover_text("Opens the jobs as one production run on the Make it screen - each job's mats are labelled with its name").clicked() {
            cut_queue(app);
        }
        if ui.button("Clear").clicked() {
            app.queue.clear();
        }
    });
    ui.small("The run opens as a new, unsaved design (one page per job), so your design stays as it is on disk.");
    None
}

/// Opens the queue as one production run on the Make it screen.
fn cut_queue(app: &mut App) {
    if app.dirty && !crate::app::confirm("The production run replaces the design on screen. Discard its unsaved changes?") {
        return;
    }
    let run = queue::build_run(&app.queue, &app.doc.machine);
    let packed = run.pages.iter().any(|p| p.copies > 1);
    app.doc = run;
    app.path = None;
    app.undo.clear();
    app.redo.clear();
    app.sel.clear();
    app.dirty = true;
    app.rev += 1;
    app.view.fit_requested = true;
    let view = crate::prepare_ui::PrepareView::new(app, packed);
    if view.prep.mats.is_empty() {
        app.status = "Nothing in the queue has a cut style - give its shapes one (Style ▸ Fill & Line)".into();
    } else {
        let mats = view.prep.mats.len();
        app.status = format!("Production run: {} job{} on {mats} mat{} - cut them one at a time", app.queue.len(), if app.queue.len() == 1 { "" } else { "s" }, if mats == 1 { "" } else { "s" });
        app.prepare = Some(view);
    }
}
