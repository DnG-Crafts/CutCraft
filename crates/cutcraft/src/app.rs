//! Application state, actions, menus and keyboard shortcuts.

use crate::canvas::{CanvasState, View};
use crate::panels::{Panel, PanelState};
use crate::render::RenderCache;
use cutcraft_core::cutjob::{self, JobPlan};
use cutcraft_core::device::Progress;
use cutcraft_core::doc::{Document, Page, Rgba, Shape, ShapeId, ShapeKind, Style, TextData};
use cutcraft_core::kurbo::{Affine, Point, Vec2};
use cutcraft_core::ops::{self, AlignMode, Arrange};
use cutcraft_core::prefs::Prefs;
use cutcraft_core::text::FontLibrary;
use cutcraft_core::{io, machines};
use egui::{Key, KeyboardShortcut, Modifiers};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Node,
    Pan,
    Zoom,
    Line,
    Polyline,
    Curve,
    Freehand,
    Rect,
    RoundRect,
    Ellipse,
    Polygon,
    Star,
    Text,
    Knife,
    Eraser,
    Corner,
    ImageEraser,
    Measure,
    Dimension,
    Trim,
    ShapeBuilder,
}

impl Tool {
    /// Every tool and its tooltip.
    pub const ALL: [(Tool, &'static str); 22] = [
        (Tool::Select, "Select (V)"),
        (Tool::Node, "Edit points (A)"),
        (Tool::Pan, "Pan (H)"),
        (Tool::Zoom, "Zoom (Z) - click to zoom in, shift-click out"),
        (Tool::Line, "Line (L)"),
        (Tool::Polyline, "Polygon / polyline (P) - click points, double-click or Enter to finish"),
        (Tool::Curve, "Curve (C) - click points, double-click or Enter to finish"),
        (Tool::Freehand, "Freehand (F)"),
        (Tool::Rect, "Rectangle (R) - shift for square"),
        (Tool::RoundRect, "Rounded rectangle"),
        (Tool::Ellipse, "Ellipse (E) - shift for circle"),
        (Tool::Polygon, "Regular polygon"),
        (Tool::Star, "Star"),
        (Tool::Text, "Text (T)"),
        (Tool::Knife, "Knife (K) - drag a line through shapes"),
        (Tool::Eraser, "Eraser (size: Edit ▸ Drawing tools ▸ Eraser radius)"),
        (Tool::Corner, "Round corner - click a corner of a selected shape to round or chamfer it (size & style in Edit ▸ Drawing tools)"),
        (Tool::ImageEraser, "Image eraser - rub out parts of a picture (size: Edit ▸ Drawing tools ▸ Eraser radius)"),
        (Tool::Measure, "Measure (M) - drag between two points to see the distance and angle; nothing is added"),
        (Tool::Dimension, "Dimension - drag between two points to add a labelled dimension (never cut or printed)"),
        (Tool::Trim, "Trim - click a piece of outline to delete it back to the nearest lines crossing it"),
        (Tool::ShapeBuilder, "Shape builder (Shift+M) - select overlapping shapes, then drag across the areas to merge them into one shape; Alt-drag or Alt-click deletes areas"),
    ];
}

/// A readable undo-step name for an action: `FitIntoBox` → "Fit into box".
pub fn action_label(a: &Action) -> String {
    let dbg = format!("{a:?}");
    let name = dbg.split(['(', ' ', '{']).next().unwrap_or("Edit");
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Every command reachable from menus, toolbar and shortcuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    New,
    Open,
    Import,
    Save,
    SaveAs,
    ExportSvg,
    ExportSvgCut,
    ExportDxf,
    ExportGcode,
    ExportPdf,
    ExportGpgl,
    ExportPlt,
    ExportFcm,
    ExportDmpl,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PasteInPlace,
    Duplicate,
    Delete,
    SelectAll,
    Deselect,
    Group,
    Ungroup,
    Arrange(Arrange),
    FlipH,
    FlipV,
    Rotate(f64),
    Align(AlignMode),
    DistributeH,
    DistributeV,
    Weld,
    Subtract,
    SubtractAll,
    Intersect,
    Exclude,
    /// Weld live, keeping the parts (ungroup to get them back).
    Unite,
    /// Turn the selection into one printed picture with an outer cut line.
    Flatten,
    Unflatten,
    Divide,
    /// Split a text into one editable text per letter.
    UngroupLetters,
    /// Split a text into one editable text per line.
    UngroupLines,
    MakeCompound,
    ReleaseCompound,
    ConvertToPath,
    FitIntoBox,
    ClearGuides,
    ZoomIn,
    ZoomOut,
    ZoomFitPage,
    ZoomFitSelection,
    AddPage,
    DeletePage,
    Nudge(f64, f64),
    ShowPanel(Panel),
    /// The Prepare screen: mats by material, then cut each.
    OpenPrepare,
    /// The Home gallery of recent designs.
    Home,
}

pub struct App {
    pub doc: Document,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Snapshots, each with the name of the edit made after it (History panel).
    pub undo: Vec<(Document, String)>,
    pub redo: Vec<(Document, String)>,
    /// Name for the next undo step, set while an action runs.
    pub next_label: Option<String>,
    /// Name of the side panel being drawn, the fallback label for its edits.
    pub label_ctx: Option<&'static str>,
    pub sel: Vec<ShapeId>,
    pub clipboard: Vec<Shape>,
    pub tool: Tool,
    pub view: View,
    pub fonts: FontLibrary,
    pub fontdb: Arc<fontdb::Database>,
    pub prefs: Prefs,
    pub panel: Panel,
    pub ps: PanelState,
    pub status: String,
    pub cache: RenderCache,
    pub textures: std::collections::HashMap<usize, egui::TextureHandle>,
    /// Keeps the canvas picture on the graphics card.
    pub gpu: crate::gpu::Gpu,
    pub cs: CanvasState,
    pub rev: u64,
    pub plan: JobPlan,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    pub sending: bool,
    /// A thread is watching `progress` and waking the window when it changes.
    pub progress_watch: Arc<AtomicBool>,
    /// Background backup of unsaved work (crash recovery).
    pub autosaver: crate::recovery::AutoSaver,
    /// A backup from a session that didn't close properly, awaiting an answer.
    pub recovery: Option<crate::recovery::Pending>,
    /// Constraints that can't be met right now (Parameters panel).
    pub constraint_errors: Vec<String>,
    /// The Prepare ("Make it") screen, while it's open.
    pub prepare: Option<crate::prepare_ui::PrepareView>,
    /// The image upload wizard, while open.
    pub upload: Option<crate::wizards::UploadWizard>,
    /// The Print & Cut calibration wizard, while open.
    pub calib: Option<crate::calibrate::Wizard>,
    /// Picking a colour from the screen.
    pub eyedrop: crate::eyedrop::State,
    /// Icons and fonts from the web.
    pub web: crate::webui::WebState,
    /// The built-in browser, while open, and a site to open it at.
    #[cfg_attr(not(has_browser), allow(dead_code))]
    pub browser: Option<crate::browser::Browser>,
    pub browser_request: Option<crate::browser::Site>,
    /// The production queue (Make ▸ Queue).
    pub queue: Vec<cutcraft_core::queue::QueueJob>,
    /// The Home gallery, while shown.
    pub home: Option<crate::wizards::Home>,
    /// Parts of a job still to send after a tool change: (part, tool, carriage, last).
    /// Where popup windows open.
    pub place: crate::dialogs::Placement,
    /// Notes in startup.log when the idle window keeps redrawing.
    pub frame_watch: crate::startup::FrameWatch,
    /// The title last given to the window.
    pub window_title: String,
    /// The main window's spot and size, kept up to date to save on exit.
    pub window_place: Option<cutcraft_core::prefs::WindowPlacement>,
    pub tool_queue: Vec<(cutcraft_core::cutjob::CutJob, cutcraft_core::materials::Tool, u8, bool)>,
    pub confirm_quit: bool,
    pub quitting: bool,
    pub show_about: bool,
    pub show_shortcuts: bool,
    /// The last merged edit (label, time) so a run of field edits is one undo step.
    pub merge: Option<(&'static str, f64)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs = Prefs::load();
        install_symbol_fonts(&cc.egui_ctx);
        crate::chrome::apply_theme(&cc.egui_ctx, prefs.dark_mode);
        let fonts = FontLibrary::new();
        let fontdb = Arc::new(fonts.database().clone());
        let mut doc = Document::default();
        doc.machine = prefs.default_machine.clone();
        let m = machines::by_id(&doc.machine);
        doc.page_mut().mat = m.default_mat;
        if let Some((w, h)) = m.default_mat.size_mm() {
            doc.page_mut().width = w;
            doc.page_mut().height = h;
        }
        let mut ps = PanelState::default();
        ps.text.family = prefs.default_font.clone();
        ps.text.size = prefs.default_text_size;
        let mut app = App {
            doc,
            path: None,
            dirty: false,
            undo: vec![],
            redo: vec![],
            next_label: None,
            label_ctx: None,
            sel: vec![],
            clipboard: vec![],
            tool: Tool::Select,
            view: View::default(),
            fonts,
            fontdb,
            prefs,
            panel: Panel::Page,
            ps,
            status: "Welcome to CutCraft".into(),
            cache: RenderCache::default(),
            textures: Default::default(),
            gpu: crate::gpu::Gpu::new(cc),
            cs: CanvasState::default(),
            rev: 1,
            plan: JobPlan::default(),
            progress: Arc::new(Mutex::new(Progress::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            sending: false,
            progress_watch: Arc::new(AtomicBool::new(false)),
            autosaver: crate::recovery::AutoSaver::new(),
            recovery: None,
            constraint_errors: Vec::new(),
            prepare: None,
            upload: None,
            calib: None,
            eyedrop: Default::default(),
            web: Default::default(),
            browser: None,
            browser_request: None,
            queue: Vec::new(),
            home: None,
            tool_queue: Vec::new(),
            place: Default::default(),
            frame_watch: Default::default(),
            window_title: String::new(),
            window_place: None,
            confirm_quit: false,
            quitting: false,
            show_about: false,
            show_shortcuts: false,
            merge: None,
        };
        // open a file passed on the command line
        match std::env::args().nth(1).as_deref() {
            Some("--sample") => {
                app.doc = sample_document(&app.fonts);
                app.sel = app.doc.page().shapes.iter().take(1).map(|s| s.id).collect();
            }
            Some(p) => app.open_path(PathBuf::from(p)),
            None => {}
        }
        app.recovery = app.autosaver.pending();
        if std::env::args().nth(1).is_none() && !app.prefs.recent_files.is_empty() {
            app.home = Some(crate::wizards::Home::new(&app));
        }
        app
    }

    // ------------------------------------------------------------ undo

    /// Record the current state before a modification.
    /// Like [`checkpoint`](Self::checkpoint), but consecutive edits with the same
    /// `label` within a second share one undo step (dragging or typing in a
    /// numeric field). Ported from BurnCraft.
    pub fn checkpoint_merged(&mut self, label: &'static str, now: f64) {
        let merging = matches!(self.merge, Some((l, t)) if l == label && now - t < 1.0);
        if !merging {
            self.checkpoint_named(label);
        }
        self.merge = Some((label, now));
        self.dirty = true;
        self.rev += 1;
    }

    /// Record the current state, naming the step after the running action or
    /// the panel being drawn.
    pub fn checkpoint(&mut self) {
        let label = self.next_label.clone().or(self.label_ctx.map(String::from)).unwrap_or_else(|| "Edit".into());
        self.checkpoint_named(label);
    }

    /// Record the current state as an undo step called `label`.
    pub fn checkpoint_named(&mut self, label: impl Into<String>) {
        self.merge = None;
        self.undo.push((self.doc.clone(), label.into()));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
        self.rev += 1;
    }

    /// Bump the render revision without an undo step (live drags).
    pub fn touch(&mut self) {
        self.rev += 1;
    }

    pub fn undo(&mut self) {
        if let Some((d, l)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.doc, d), l));
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if let Some((d, l)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.doc, d), l));
            self.after_history();
        }
    }

    /// Undo or redo until `steps` undo steps remain (History panel).
    pub fn history_jump(&mut self, steps: usize) {
        while self.undo.len() > steps && !self.undo.is_empty() {
            self.undo();
        }
        while self.undo.len() < steps && !self.redo.is_empty() {
            self.redo();
        }
    }

    fn after_history(&mut self) {
        self.sel.retain(|id| self.doc.page().shape(*id).is_some());
        self.dirty = true;
        self.rev += 1;
    }

    /// Run an editing operation with undo; `f` returns the new selection (or None to keep it).
    pub fn edit(&mut self, f: impl FnOnce(&mut Document, &[ShapeId]) -> Option<Vec<ShapeId>>) {
        self.checkpoint();
        let sel = self.sel.clone();
        if let Some(s) = f(&mut self.doc, &sel) {
            self.sel = s;
        }
        self.sel.retain(|id| self.doc.page().shape(*id).is_some());
    }

    pub fn editable_sel(&self) -> Vec<ShapeId> {
        let p = self.doc.page();
        self.sel.iter().copied().filter(|id| p.shape(*id).map(|s| p.is_editable(s)).unwrap_or(false)).collect()
    }

    pub fn current_layer(&self) -> u32 {
        let p = self.doc.page();
        let l = self.ps.current_layer;
        if p.layer(l).is_some() { l } else { p.layers.first().map(|l| l.id).unwrap_or(0) }
    }

    /// The current layer's colour: the outline colour new shapes get.
    pub fn layer_colour(&self) -> Rgba {
        self.doc.page().layer(self.current_layer()).map(|l| l.color).unwrap_or(Rgba::RED)
    }

    pub fn default_style(&self) -> Style {
        Style { stroke: Some(self.layer_colour()), ..Style::default() }
    }

    pub fn add_shape(&mut self, s: Shape) -> ShapeId {
        let layer = self.current_layer();
        let id = self.doc.add_shape(s, layer);
        self.sel = vec![id];
        id
    }

    /// Re-layout a text shape's outline after its data changed.
    pub fn relayout_text(&mut self, id: ShapeId) {
        let fonts = &self.fonts;
        if let Some(s) = self.doc.page_mut().shape_mut(id) {
            if let ShapeKind::Text { data, outline } = &mut s.kind {
                *outline = fonts.layout(data);
            }
        }
        self.rev += 1;
    }

    pub fn create_text(&mut self, at: Point, data: TextData) -> ShapeId {
        let outline = self.fonts.layout(&data);
        let s = Shape {
            id: 0,
            name: String::new(),
            layer: 0,
            transform: Affine::translate(at.to_vec2()),
            style: Style { fill: Some(Rgba(40, 40, 40, 255)), stroke: Some(self.layer_colour()), ..Style::default() },
            kind: ShapeKind::Text { data, outline },
            locked: false,
            hidden: false,
            effects: Vec::new(),
        };
        self.checkpoint();
        self.add_shape(s)
    }

    // ------------------------------------------------------------ files

    pub fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or("Untitled".into());
        format!("{}{} - CutCraft", if self.dirty { "• " } else { "" }, name)
    }

    pub fn open_path(&mut self, p: PathBuf) {
        match io::kind_of(&p) {
            io::FileKind::Native => match io::load_native(&p) {
                Ok(d) => {
                    self.doc = d;
                    self.path = Some(p.clone());
                    self.undo.clear();
                    self.redo.clear();
                    self.sel.clear();
                    self.dirty = false;
                    self.rev += 1;
                    self.view.fit_requested = true;
                    self.prefs.add_recent(&p.to_string_lossy());
                    let _ = self.prefs.save();
                    self.status = format!("Opened {}", p.display());
                }
                Err(e) => self.status = format!("Open failed: {e}"),
            },
            _ => self.import_path(p),
        }
    }

    pub fn import_path(&mut self, p: PathBuf) {
        if crate::wizards::start_upload(self, p.clone()) {
            return;
        }
        match io::import_with_notes(&p, Some(self.fontdb.clone())) {
            Ok((shapes, notes)) if !shapes.is_empty() => {
                self.checkpoint();
                let layer = self.current_layer();
                let mut ids = vec![];
                for mut s in shapes {
                    self.doc.reid(&mut s);
                    ids.push(self.doc.add_shape(s, layer));
                }
                // keep imports on the page
                if let Some(bb) = ops::selection_bbox(&self.doc, &ids) {
                    if bb.x0 < 0.0 || bb.y0 < 0.0 || bb.x1 > self.doc.page().width || bb.y1 > self.doc.page().height {
                        ops::move_to(&mut self.doc, &ids, 10.0, 10.0);
                    }
                }
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.status = format!("Imported {} shape(s) from {name}", ids.len());
                if !notes.is_empty() {
                    self.status += &format!(" - note: {}", notes.join("; "));
                }
                self.sel = ids;
            }
            Ok((_, notes)) => self.status = if notes.is_empty() { "Nothing to import".into() } else { format!("Nothing to import - {}", notes.join("; ")) },
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    pub fn save(&mut self, save_as: bool) {
        let path = if save_as || self.path.is_none() {
            rfd::FileDialog::new().add_filter("CutCraft", &[io::NATIVE_EXT]).set_file_name("design.cutcraft").save_file()
        } else {
            self.path.clone()
        };
        if let Some(p) = path {
            match io::save_native(&self.doc, &p) {
                Ok(()) => {
                    self.path = Some(p.clone());
                    self.dirty = false;
                    self.prefs.add_recent(&p.to_string_lossy());
                    let _ = self.prefs.save();
                    self.status = format!("Saved {}", p.display());
                }
                Err(e) => self.status = format!("Save failed: {e}"),
            }
        }
    }

    fn export_text(&mut self, ext: &str, label: &str, content: String) {
        if let Some(p) = rfd::FileDialog::new().add_filter(label, &[ext]).set_file_name(format!("design.{ext}")).save_file() {
            self.status = match std::fs::write(&p, content) {
                Ok(()) => format!("Exported {}", p.display()),
                Err(e) => format!("Export failed: {e}"),
            };
        }
    }

    pub fn export_job(&mut self, proto: cutcraft_core::gpgl::Protocol) {
        use cutcraft_core::gpgl::Protocol;
        cutjob::sync_plan(&mut self.plan, self.doc.page());
        if proto == Protocol::Fcm {
            let (page, plan) = (self.doc.page().clone(), self.plan.clone());
            self.save_fcm(&page, &plan, "design.fcm");
            return;
        }
        let job = cutjob::generate(self.doc.page(), &self.plan);
        if job.is_empty() {
            self.status = "Nothing to cut".into();
            return;
        }
        let h = self.doc.page().height;
        match proto {
            Protocol::Hpgl => self.export_text("plt", "HP-GL", cutcraft_core::gpgl::encode_hpgl(&job, h)),
            Protocol::Dmpl => self.export_text("dmpl", "DMPL", cutcraft_core::gpgl::encode_dmpl(&job, h)),
            Protocol::Fcm => unreachable!("saved above"),
            Protocol::Gpgl => {
                let bytes = cutcraft_core::gpgl::encode(&job, &crate::panels::job_options(self));
                if let Some(p) = rfd::FileDialog::new().add_filter("GPGL", &["gpgl"]).set_file_name("job.gpgl").save_file() {
                    self.status = match std::fs::write(&p, bytes) {
                        Ok(()) => format!("Exported {}", p.display()),
                        Err(e) => format!("Export failed: {e}"),
                    };
                }
            }
        }
    }

    /// Ungroup text to letters (or lines): each becomes its own editable text,
    /// exactly where it was.
    fn split_text(&mut self, letters: bool) {
        let texts: Vec<(ShapeId, TextData, Affine, Style, u32)> = self
            .sel
            .iter()
            .filter_map(|id| self.doc.page().shape(*id))
            .filter_map(|s| match &s.kind {
                ShapeKind::Text { data, .. } => Some((s.id, data.clone(), s.transform, s.style.clone(), s.layer)),
                _ => None,
            })
            .collect();
        if texts.is_empty() {
            self.status = "Select a text first".into();
            return;
        }
        let mut made = vec![];
        let mut first = true;
        for (id, data, t, style, layer) in texts {
            let parts: Vec<(String, Affine)> = if letters {
                self.fonts.letter_placements(&data).into_iter().map(|(c, a)| (c.to_string(), a)).collect()
            } else {
                self.fonts.line_placements(&data)
            };
            if parts.len() < 2 {
                continue;
            }
            if first {
                self.checkpoint_named(if letters { "Ungroup to letters" } else { "Ungroup to lines" });
                first = false;
            }
            for (text, at) in parts {
                let d = TextData { text, curve: if letters { 0.0 } else { data.curve }, on_path: None, ..data.clone() };
                let outline = self.fonts.layout(&d);
                let s = Shape { id: 0, name: String::new(), layer, transform: t * at, style: style.clone(), kind: ShapeKind::Text { data: d, outline }, locked: false, hidden: false, effects: Vec::new() };
                made.push(self.doc.add_shape(s, layer));
            }
            self.doc.page_mut().shapes.retain(|s| s.id != id);
        }
        self.status = if made.is_empty() {
            format!("Nothing to split - the text is a single {}", if letters { "letter" } else { "line" })
        } else {
            format!("{} {} - each is its own text now", made.len(), if letters { "letters" } else { "lines" })
        };
        if !made.is_empty() {
            self.sel = made;
        }
    }

    /// Brother ScanNCut: save the page as an `.fcm` file to open in
    /// CanvasWorkspace or a USB stick.
    pub fn save_fcm(&mut self, page: &Page, plan: &JobPlan, default_name: &str) {
        let (bytes, summary) = match cutcraft_core::fcm::export(page, plan) {
            Ok(r) => r,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        let name = self.path.as_ref().and_then(|p| p.file_stem()).map(|s| format!("{}.fcm", s.to_string_lossy())).unwrap_or_else(|| default_name.into());
        let Some(p) = rfd::FileDialog::new().add_filter("Brother ScanNCut", &["fcm"]).set_file_name(name).save_file() else { return };
        self.status = match std::fs::write(&p, bytes) {
            Ok(()) => {
                let mut s = format!(
                    "Saved {} - {} piece(s): {} cut, {} drawn line(s). Open it in CanvasWorkspace to cut",
                    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    summary.pieces,
                    summary.cut_paths,
                    summary.draw_paths
                );
                if summary.outside_cut_area {
                    s += " · note: part of the design is outside the mat's cut area";
                }
                if summary.skipped_stipple {
                    s += " · stipple groups skipped";
                }
                s
            }
            Err(e) => format!("Couldn't save: {e}"),
        };
    }

    /// Brother ScanNCut: send the page straight to the machine over USB.
    pub fn send_fcm_usb(&mut self, page: &Page, plan: &JobPlan) {
        if self.sending {
            return;
        }
        let bytes = match cutcraft_core::fcm::export(page, plan) {
            Ok((b, _)) => b,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        self.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        *self.progress.lock().unwrap() = cutcraft_core::device::Progress::default();
        self.sending = true;
        cutcraft_core::brother_usb::spawn_send(bytes, self.progress.clone(), self.cancel.clone());
        self.status = "Sending to the ScanNCut…".into();
    }

    // ------------------------------------------------------------ actions

    pub fn do_action(&mut self, a: Action, ctx: &egui::Context) {
        self.next_label = Some(action_label(&a));
        self.run_action(a, ctx);
        self.next_label = None;
    }

    fn run_action(&mut self, a: Action, ctx: &egui::Context) {
        use Action::*;
        let gap = self.prefs.paste_offset_mm;
        match a {
            New => {
                if self.dirty && !confirm("Discard unsaved changes?") {
                    return;
                }
                let machine = self.doc.machine.clone();
                self.doc = Document::default();
                self.doc.machine = machine;
                self.path = None;
                self.undo.clear();
                self.redo.clear();
                self.sel.clear();
                self.dirty = false;
                self.rev += 1;
                self.view.fit_requested = true;
            }
            Open => {
                if self.dirty && !confirm("Discard unsaved changes?") {
                    return;
                }
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("All supported", &[&[io::NATIVE_EXT][..], &io::IMPORT_EXTS[..]].concat())
                    .pick_file()
                {
                    if io::kind_of(&p) == io::FileKind::Native {
                        self.open_path(p);
                    } else {
                        self.doc = Document::default();
                        self.path = None;
                        self.undo.clear();
                        self.import_path(p);
                        self.dirty = false;
                    }
                }
            }
            Import => {
                if let Some(ps) = rfd::FileDialog::new()
                    .add_filter("Vector & images", &[&io::IMPORT_EXTS[..], &[io::NATIVE_EXT][..]].concat())
                    .pick_files()
                {
                    for p in ps {
                        self.import_path(p);
                    }
                }
            }
            Save => self.save(false),
            SaveAs => self.save(true),
            ExportSvg => {
                let s = io::export_svg(self.doc.page(), false);
                self.export_text("svg", "SVG", s)
            }
            ExportSvgCut => {
                let s = io::export_svg(self.doc.page(), true);
                self.export_text("svg", "SVG", s)
            }
            ExportDxf => {
                let s = io::export_dxf(self.doc.page());
                self.export_text("dxf", "DXF", s)
            }
            ExportPdf => {
                if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name("print.pdf").save_file() {
                    self.status = match io::export_print_pdf(self.doc.page(), &p) {
                        Ok(()) => format!("Print file written: {} - print it at 100% scale", p.display()),
                        Err(e) => format!("PDF export failed: {e}"),
                    };
                }
            }
            ExportGcode => {
                let s = io::export_gcode(self.doc.page(), &io::GcodeOptions::default());
                self.export_text("gcode", "G-code", s)
            }
            ExportGpgl => self.export_job(cutcraft_core::gpgl::Protocol::Gpgl),
            ExportPlt => self.export_job(cutcraft_core::gpgl::Protocol::Hpgl),
            ExportFcm => self.export_job(cutcraft_core::gpgl::Protocol::Fcm),
            ExportDmpl => self.export_job(cutcraft_core::gpgl::Protocol::Dmpl),
            Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Undo => self.undo(),
            Redo => self.redo(),
            Copy => {
                self.copy();
                crate::webui::note_copy(self);
            }
            Cut => {
                self.copy();
                self.edit(|d, s| Some(ops::delete(d, s)));
            }
            Paste if crate::webui::paste_svg(self) => {}
            Paste => {
                let clip = self.clipboard.clone();
                let layer = self.current_layer();
                self.edit(|d, _| Some(ops::paste(d, &clip, Vec2::new(gap, gap), layer)));
                for c in self.clipboard.iter_mut() {
                    c.transform = Affine::translate((gap, gap)) * c.transform;
                }
            }
            PasteInPlace => {
                let clip = self.clipboard.clone();
                let layer = self.current_layer();
                self.edit(|d, _| Some(ops::paste(d, &clip, Vec2::ZERO, layer)));
            }
            Duplicate => {
                let s = self.editable_sel();
                self.edit(|d, _| Some(ops::duplicate(d, &s, Vec2::new(gap, gap))));
            }
            Delete => {
                if self.tool == Tool::Node && crate::canvas::delete_node(self) {
                    return;
                }
                let s = self.editable_sel();
                self.edit(|d, _| Some(ops::delete(d, &s)));
            }
            SelectAll => {
                let p = self.doc.page();
                self.sel = p.shapes.iter().filter(|s| p.is_editable(s)).map(|s| s.id).collect();
            }
            Deselect => self.sel.clear(),
            Group => self.edit(|d, s| Some(ops::group(d, s))),
            Ungroup => self.edit(|d, s| Some(ops::ungroup(d, s))),
            Arrange(h) => self.edit(|d, s| {
                ops::arrange(d, s, h);
                None
            }),
            FlipH => self.edit(|d, s| {
                ops::flip(d, s, true);
                None
            }),
            FlipV => self.edit(|d, s| {
                ops::flip(d, s, false);
                None
            }),
            Rotate(deg) => self.edit(|d, s| {
                ops::rotate(d, s, deg);
                None
            }),
            Align(m) => self.edit(|d, s| {
                ops::align(d, s, m);
                None
            }),
            DistributeH => self.edit(|d, s| {
                ops::distribute(d, s, true);
                None
            }),
            DistributeV => self.edit(|d, s| {
                ops::distribute(d, s, false);
                None
            }),
            Weld => self.need(2, |d, s| ops::weld(d, s)),
            ClearGuides => {
                if !self.doc.page().guides.is_empty() {
                    self.checkpoint();
                    self.doc.page_mut().guides.clear();
                }
            }
            Home => {
                self.home = Some(crate::wizards::Home::new(self));
            }
            OpenPrepare => {
                let view = crate::prepare_ui::PrepareView::new(self, false);
                if view.prep.mats.is_empty() {
                    self.status = "Nothing to cut yet - draw or import something, and give it a cut style (Style ▸ Fill & Line)".into();
                } else {
                    let n = view.prep.mats.len();
                    self.status = format!("{n} mat{} ready - pick each material, then cut them one at a time", if n == 1 { "" } else { "s" });
                    self.prepare = Some(view);
                }
            }
            FitIntoBox => self.need(2, |d, s| {
                // the largest selected shape is the box; the rest scale into it
                if let Some((frame, items)) = ops::frame_and_items(d, s) {
                    ops::fit_into(d, &items, frame, 2.0);
                }
                s.to_vec()
            }),
            Subtract => self.need(2, |d, s| ops::subtract(d, s)),
            SubtractAll => self.need(2, |d, s| ops::subtract_all(d, s)),
            Intersect => self.need(2, |d, s| ops::intersect(d, s)),
            Exclude => self.need(2, |d, s| ops::exclude(d, s)),
            Unite => self.need(2, |d, s| ops::unite(d, s)),
            Flatten => self.need(1, |d, s| cutcraft_core::printflat::flatten(d, s)),
            Unflatten => self.need(1, |d, s| cutcraft_core::printflat::unflatten(d, s)),
            Divide => self.need(2, |d, s| ops::divide(d, s)),
            UngroupLetters => self.split_text(true),
            UngroupLines => self.split_text(false),
            MakeCompound => self.need(2, |d, s| ops::make_compound(d, s)),
            ReleaseCompound => self.need(1, |d, s| ops::release_compound(d, s)),
            ConvertToPath => self.need(1, |d, s| ops::convert_to_path(d, s)),
            ZoomIn => self.view.zoom_by(1.25, None),
            ZoomOut => self.view.zoom_by(0.8, None),
            ZoomFitPage => self.view.fit_requested = true,
            ZoomFitSelection => {
                if let Some(bb) = ops::selection_bbox(&self.doc, &self.sel) {
                    self.view.fit_rect = Some(bb);
                }
            }
            AddPage => {
                self.checkpoint();
                let cur = self.doc.page();
                let mut p = Page::new(&format!("Page {}", self.doc.pages.len() + 1), cur.width, cur.height);
                p.mat = cur.mat;
                self.doc.pages.push(p);
                self.doc.active = self.doc.pages.len() - 1;
                self.sel.clear();
            }
            DeletePage => {
                if self.doc.pages.len() > 1 && confirm("Delete this page?") {
                    self.checkpoint();
                    let a = self.doc.active;
                    self.doc.pages.remove(a);
                    self.doc.active = a.min(self.doc.pages.len() - 1);
                    self.sel.clear();
                }
            }
            Nudge(dx, dy) => {
                let s = self.editable_sel();
                if !s.is_empty() {
                    self.edit(|d, _| {
                        ops::move_by(d, &s, Vec2::new(dx, dy));
                        None
                    });
                }
            }
            ShowPanel(p) => self.panel = p,
        }
    }

    fn need(&mut self, n: usize, f: impl FnOnce(&mut Document, &[ShapeId]) -> Vec<ShapeId>) {
        let s = self.editable_sel();
        if s.len() < n {
            self.status = format!("Select at least {n} unlocked shape(s)");
            return;
        }
        self.edit(|d, _| Some(f(d, &s)));
    }

    fn copy(&mut self) {
        let p = self.doc.page();
        self.clipboard = p.shapes.iter().filter(|s| self.sel.contains(&s.id)).cloned().collect();
        self.status = format!("Copied {} shape(s)", self.clipboard.len());
    }

    // ------------------------------------------------------------ menus

    pub fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        use Action::*;
        let mut act = None;
        let mut item = |ui: &mut egui::Ui, label: &str, sc: &str, a: Action| {
            let b = egui::Button::new(label).shortcut_text(sc);
            if ui.add(b).clicked() {
                act = Some(a);
                ui.close();
            }
        };
        // Collect the top-level menu buttons so we can add classic "hover to
        // switch the open menu" behaviour, which egui 0.36 doesn't do by default.
        let mut tops: Vec<egui::Response> = Vec::new();
        macro_rules! top {
            ($ui:ident, $label:expr, $body:expr) => {
                tops.push($ui.menu_button($label, $body).response);
            };
        }
        egui::MenuBar::new().ui(ui, |ui| {
            top!(ui, "File", |ui| {
                item(ui, "New", "Ctrl+N", New);
                item(ui, "Home (recent designs)", "", Home);
                item(ui, "Open…", "Ctrl+O", Open);
                ui.menu_button("Open recent", |ui| {
                    for r in self.prefs.recent_files.clone() {
                        if ui.button(&r).clicked() {
                            self.open_path(PathBuf::from(&r));
                            ui.close();
                        }
                    }
                });
                item(ui, "Import / Merge…", "Ctrl+I", Import);
                ui.separator();
                item(ui, "Save", "Ctrl+S", Save);
                item(ui, "Save As…", "Ctrl+Shift+S", SaveAs);
                ui.separator();
                item(ui, "Export SVG…", "", ExportSvg);
                item(ui, "Export SVG (cut lines only)…", "", ExportSvgCut);
                item(ui, "Export DXF…", "", ExportDxf);
                item(ui, "Export G-code…", "", ExportGcode);
                item(ui, "Export Print & Cut PDF…", "", ExportPdf);
                item(ui, "Export cut job (GPGL)…", "", ExportGpgl);
                item(ui, "Export cut job (PLT / HP-GL)…", "", ExportPlt);
                item(ui, "Export for Brother ScanNCut (.fcm)…", "", ExportFcm);
                item(ui, "Export cut job (DMPL)…", "", ExportDmpl);
                ui.separator();
                item(ui, "Preferences…", "", ShowPanel(Panel::Prefs));
                item(ui, "Quit", "Ctrl+Q", Quit);
            });
            top!(ui, "Edit", |ui| {
                item(ui, "Undo", "Ctrl+Z", Undo);
                item(ui, "Redo", "Ctrl+Y", Redo);
                ui.separator();
                item(ui, "Cut", "Ctrl+X", Cut);
                item(ui, "Copy", "Ctrl+C", Copy);
                item(ui, "Paste", "Ctrl+V", Paste);
                item(ui, "Paste in place", "Ctrl+Shift+V", PasteInPlace);
                item(ui, "Duplicate", "Ctrl+D", Duplicate);
                item(ui, "Delete", "Del", Delete);
                ui.separator();
                item(ui, "Select all", "Ctrl+A", SelectAll);
                item(ui, "Deselect", "Esc", Deselect);
            });
            top!(ui, "View", |ui| {
                item(ui, "Zoom in", "+", ZoomIn);
                item(ui, "Zoom out", "-", ZoomOut);
                item(ui, "Fit page", "Ctrl+0", ZoomFitPage);
                item(ui, "Fit selection", "Ctrl+1", ZoomFitSelection);
                ui.separator();
                let g = &mut self.doc.pages[self.doc.active].grid;
                ui.checkbox(&mut g.visible, "Show grid");
                ui.checkbox(&mut g.snap, "Snap to grid");
                ui.checkbox(&mut self.prefs.show_rulers, "Show rulers").on_hover_text("Drag from a ruler to place a guide; drag a guide back onto a ruler to remove it");
                item(ui, "Clear guides", "", ClearGuides);
                ui.checkbox(&mut self.prefs.smart_guides, "Smart guides").on_hover_text("While moving, line shapes up with other shapes and the page");
                ui.checkbox(&mut self.prefs.show_constraints, "Show constraints").on_hover_text("Draw locked sizes and distances (Parameters panel) as blue dimensions");
                ui.checkbox(&mut self.prefs.snap_objects, "Snap to objects").on_hover_text("Snap drawing and point edits to other shapes' corners, centres and nodes");
                ui.checkbox(&mut self.cs.show_cut_lines_only, "Show cut lines only");
                ui.menu_button("Symmetry", |ui| {
                    use crate::canvas::SymmetryMode;
                    let page_centre = self.doc.page().rect().center();
                    let sel_centre = ops::selection_bbox(&self.doc, &self.sel).map(|b| b.center());
                    let sym = &mut self.cs.symmetry;
                    ui.weak("Shapes you draw are repeated as you draw them.");
                    for m in SymmetryMode::ALL {
                        if ui.radio_value(&mut sym.mode, m, m.label()).clicked() && sym.center == Point::ORIGIN {
                            sym.center = page_centre;
                        }
                    }
                    if sym.mode == SymmetryMode::Radial {
                        ui.horizontal(|ui| {
                            ui.label("Copies");
                            ui.add(egui::DragValue::new(&mut sym.count).range(2..=64));
                        });
                        ui.checkbox(&mut sym.mirror, "Mirror alternate slices").on_hover_text("Kaleidoscope: every other slice is a mirror image");
                    }
                    ui.separator();
                    if ui.button("Centre on the page").clicked() {
                        sym.center = page_centre;
                    }
                    if ui.add_enabled(sel_centre.is_some(), egui::Button::new("Centre on the selection")).clicked() {
                        sym.center = sel_centre.unwrap_or(page_centre);
                    }
                });
            });
            top!(ui, "Object", |ui| {
                item(ui, "Group", "Ctrl+G", Group);
                item(ui, "Ungroup", "Ctrl+Shift+G", Ungroup);
                item(ui, "Fit into box", "", FitIntoBox);
                item(ui, "Flatten (print then cut)", "", Flatten);
                item(ui, "Unflatten", "", Unflatten);
                item(ui, "Ungroup text to letters", "", UngroupLetters);
                item(ui, "Ungroup text to lines", "", UngroupLines);
                ui.menu_button("Arrange", |ui| {
                    item(ui, "Bring to front", "Ctrl+Shift+]", Arrange(ops::Arrange::Front));
                    item(ui, "Bring forward", "Ctrl+]", Arrange(ops::Arrange::Forward));
                    item(ui, "Send backward", "Ctrl+[", Arrange(ops::Arrange::Backward));
                    item(ui, "Send to back", "Ctrl+Shift+[", Arrange(ops::Arrange::Back));
                });
                ui.menu_button("Align", |ui| {
                    item(ui, "Left", "", Align(AlignMode::Left));
                    item(ui, "Centre", "", Align(AlignMode::CenterH));
                    item(ui, "Right", "", Align(AlignMode::Right));
                    item(ui, "Top", "", Align(AlignMode::Top));
                    item(ui, "Middle", "", Align(AlignMode::Middle));
                    item(ui, "Bottom", "", Align(AlignMode::Bottom));
                    item(ui, "Centre both", "", Align(AlignMode::Center));
                    item(ui, "Distribute horizontally", "", DistributeH);
                    item(ui, "Distribute vertically", "", DistributeV);
                });
                ui.menu_button("Rotate", |ui| {
                    for d in [45.0, 90.0, 180.0, 270.0, -45.0, -90.0] {
                        item(ui, &format!("{d}°"), "", Rotate(d));
                    }
                });
                item(ui, "Flip horizontal", "", FlipH);
                item(ui, "Flip vertical", "", FlipV);
                ui.separator();
                ui.menu_button("Modify", |ui| {
                    item(ui, "Weld", "Ctrl+E", Weld);
                    item(ui, "Subtract", "", Subtract);
                    item(ui, "Subtract all", "", SubtractAll);
                    item(ui, "Intersect", "", Intersect);
                    item(ui, "Exclude", "", Exclude);
                    item(ui, "Unite (keep parts)", "", Unite);
                    item(ui, "Divide (slice)", "", Divide);
                });
                ui.menu_button("Compound path", |ui| {
                    item(ui, "Make", "Ctrl+8", MakeCompound);
                    item(ui, "Release", "Ctrl+Shift+8", ReleaseCompound);
                });
                item(ui, "Convert to path", "", ConvertToPath);
                ui.separator();
                item(ui, "Offset…", "", ShowPanel(Panel::Modify));
                item(ui, "Path tools (hull, lattice, knockout…)…", "", ShowPanel(Panel::PathTools));
                item(ui, "Replicate…", "", ShowPanel(Panel::Replicate));
                item(ui, "Trace…", "", ShowPanel(Panel::Trace));
                item(ui, "Warp / conical warp…", "", ShowPanel(Panel::Warp));
                item(ui, "Distort (arch, bulge, shadow, extrude…)…", "", ShowPanel(Panel::Distort));
                item(ui, "Stipple, emboss & pop-up…", "", ShowPanel(Panel::Effects));
            });
            top!(ui, "Tools", |ui| {
                item(ui, "PixScan (photo to true size)…", "", ShowPanel(Panel::PixScan));
                item(ui, "Image tools (adjust, photo → lines, shadow box)…", "", ShowPanel(Panel::Image));
                item(ui, "Barcode / QR code…", "", ShowPanel(Panel::Barcode));
                item(ui, "Art: shapes, templates, signs, monogram…", "", ShowPanel(Panel::Art));
                item(ui, "Dieline / packaging…", "", ShowPanel(Panel::Dieline));
                item(ui, "Script (Rhai generator)…", "", ShowPanel(Panel::Script));
                item(ui, "Mail merge…", "", ShowPanel(Panel::MailMerge));
                item(ui, "Font creation…", "", ShowPanel(Panel::Fonts));
                item(ui, "Spelling (select a text)…", "", ShowPanel(Panel::Text));
            });
            top!(ui, "Page", |ui| {
                item(ui, "Page setup…", "", ShowPanel(Panel::Page));
                item(ui, "Add page", "", AddPage);
                item(ui, "Delete page", "", DeletePage);
                item(ui, "Registration marks…", "", ShowPanel(Panel::PrintCut));
                item(ui, "Tile / slice to pages…", "", ShowPanel(Panel::Effects));
            });
            top!(ui, "Send", |ui| {
                item(ui, "Send to cutter…", "Ctrl+Enter", ShowPanel(Panel::Send));
                item(ui, "Export cut job (GPGL)…", "", ExportGpgl);
            });
            top!(ui, "Help", |ui| {
                if ui.button("Load sample design").clicked() {
                    if !self.dirty || confirm("Discard unsaved changes?") {
                        self.doc = sample_document(&self.fonts);
                        self.path = None;
                        self.undo.clear();
                        self.sel.clear();
                        self.rev += 1;
                        self.view.fit_requested = true;
                    }
                    ui.close();
                }
                if ui.button("Keyboard shortcuts").clicked() {
                    self.show_shortcuts = true;
                    ui.close();
                }
                if ui.button("About CutCraft").clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
        // Classic menu-bar behaviour: if a menu is open, hovering another top-level
        // menu switches to it (open_id opens one popup and closes the others).
        let ctx = ui.ctx();
        let any_open = tops.iter().any(|r| egui::Popup::is_id_open(ctx, r.id.with("popup")));
        if any_open {
            if let Some(r) = tops.iter().find(|r| r.hovered()) {
                let pid = r.id.with("popup");
                if !egui::Popup::is_id_open(ctx, pid) {
                    egui::Popup::open_id(ctx, pid);
                }
            }
        }
        act
    }

    pub fn shortcuts(&mut self, ctx: &egui::Context) -> Option<Action> {
        use Action::*;
        if ctx.egui_wants_keyboard_input() {
            return None;
        }
        let c = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let table: &[(Modifiers, Key, Action)] = &[
            (c, Key::N, New),
            (c, Key::O, Open),
            (c, Key::I, Import),
            (cs, Key::S, SaveAs),
            (c, Key::S, Save),
            (c, Key::Q, Quit),
            (cs, Key::Z, Redo),
            (c, Key::Z, Undo),
            (c, Key::Y, Redo),
            (c, Key::X, Cut),
            (c, Key::C, Copy),
            (cs, Key::V, PasteInPlace),
            (c, Key::V, Paste),
            (c, Key::D, Duplicate),
            (c, Key::A, SelectAll),
            (cs, Key::G, Ungroup),
            (c, Key::G, Group),
            (c, Key::E, Weld),
            (cs, Key::Num8, ReleaseCompound),
            (c, Key::Num8, MakeCompound),
            (c, Key::Num0, ZoomFitPage),
            (c, Key::Num1, ZoomFitSelection),
            (cs, Key::CloseBracket, Arrange(ops::Arrange::Front)),
            (cs, Key::OpenBracket, Arrange(ops::Arrange::Back)),
            (c, Key::CloseBracket, Arrange(ops::Arrange::Forward)),
            (c, Key::OpenBracket, Arrange(ops::Arrange::Backward)),
            (c, Key::Enter, ShowPanel(Panel::Send)),
        ];
        let mut found = None;
        ctx.input_mut(|i| {
            for (m, k, a) in table {
                if i.consume_shortcut(&KeyboardShortcut::new(*m, *k)) {
                    found = Some(*a);
                    return;
                }
            }
        });
        if found.is_some() {
            return found;
        }
        let n = self.prefs.nudge_mm;
        ctx.input(|i| {
            let big = if i.modifiers.shift { 10.0 } else { 1.0 };
            if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                found = Some(Delete);
            } else if i.key_pressed(Key::ArrowLeft) {
                found = Some(Nudge(-n * big, 0.0));
            } else if i.key_pressed(Key::ArrowRight) {
                found = Some(Nudge(n * big, 0.0));
            } else if i.key_pressed(Key::ArrowUp) {
                found = Some(Nudge(0.0, -n * big));
            } else if i.key_pressed(Key::ArrowDown) {
                found = Some(Nudge(0.0, n * big));
            } else if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                found = Some(ZoomIn);
            } else if i.key_pressed(Key::Minus) {
                found = Some(ZoomOut);
            } else if !i.modifiers.command && !i.modifiers.alt {
                let tool = if i.modifiers.shift && i.key_pressed(Key::M) {
                    Some(Tool::ShapeBuilder)
                } else if i.key_pressed(Key::V) {
                    Some(Tool::Select)
                } else if i.key_pressed(Key::A) {
                    Some(Tool::Node)
                } else if i.key_pressed(Key::H) {
                    Some(Tool::Pan)
                } else if i.key_pressed(Key::Z) {
                    Some(Tool::Zoom)
                } else if i.key_pressed(Key::L) {
                    Some(Tool::Line)
                } else if i.key_pressed(Key::P) {
                    Some(Tool::Polyline)
                } else if i.key_pressed(Key::C) {
                    Some(Tool::Curve)
                } else if i.key_pressed(Key::F) {
                    Some(Tool::Freehand)
                } else if i.key_pressed(Key::R) {
                    Some(Tool::Rect)
                } else if i.key_pressed(Key::E) {
                    Some(Tool::Ellipse)
                } else if i.key_pressed(Key::T) {
                    Some(Tool::Text)
                } else if i.key_pressed(Key::K) {
                    Some(Tool::Knife)
                } else if i.key_pressed(Key::M) {
                    Some(Tool::Measure)
                } else {
                    None
                };
                if let Some(t) = tool {
                    self.tool = t;
                }
            }
        });
        found
    }

    /// While a job is sending, checks its progress 10 times a second and
    /// redraws only when it changed (a redraw per check is costly where the
    /// window is drawn in software). Ported from BurnCraft's position polling.
    fn watch_progress(&mut self, ctx: &egui::Context) {
        if self.progress_watch.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return; // already watching
        }
        let (progress, alive, ctx) = (self.progress.clone(), self.progress_watch.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("progress-watch".into()).spawn(move || {
            let mut last = progress.lock().map(|p| p.clone()).unwrap_or_default();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let now = progress.lock().map(|p| p.clone()).unwrap_or_default();
                if now != last {
                    ctx.request_repaint();
                    last = now;
                }
                if last.done {
                    break;
                }
            }
            alive.store(false, std::sync::atomic::Ordering::Release);
        });
        if spawned.is_err() {
            self.progress_watch.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    /// Remembers the window's normal (not maximised or minimised) spot and size.
    fn track_window(&mut self, ctx: &egui::Context) {
        let (outer, inner, maximized, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.maximized.unwrap_or(false), v.minimized.unwrap_or(false))
        });
        if minimized {
            return;
        }
        let scale = ctx.pixels_per_point();
        match (maximized, outer, inner) {
            // maximised: keep the last normal spot, just note the state
            (true, _, _) => {
                if let Some(w) = &mut self.window_place {
                    w.maximized = true;
                } else if let Some(o) = outer {
                    self.window_place = Some(cutcraft_core::prefs::WindowPlacement { x: o.min.x, y: o.min.y, w: 1400.0, h: 820.0, scale, maximized: true });
                }
            }
            (false, Some(o), Some(i)) => {
                self.window_place = Some(cutcraft_core::prefs::WindowPlacement { x: o.min.x, y: o.min.y, w: i.width(), h: i.height(), scale, maximized: false });
            }
            _ => {}
        }
    }

    fn autosave(&mut self) {
        let every = std::time::Duration::from_secs(self.prefs.autosave_minutes as u64 * 60);
        self.autosaver.tick(&self.doc, self.rev, self.dirty, self.path.as_deref(), every);
    }

    /// Loads a left-behind backup as an unsaved design.
    fn restore(&mut self, p: crate::recovery::Pending) {
        match io::load_native(&p.design) {
            Ok(d) => {
                self.doc = d;
                self.path = p.info.file.clone();
                self.undo.clear();
                self.redo.clear();
                self.sel.clear();
                self.dirty = true;
                self.rev += 1;
                self.view.fit_requested = true;
                self.autosaver.discard(&p);
                self.status = "Recovered unsaved work - save it to keep it".into();
            }
            Err(e) => self.status = format!("Couldn't read the backup: {e}"),
        }
    }

    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = self.recovery.clone() else { return };
        let what = match &p.info.file {
            Some(f) => format!("your changes to {}", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            None => "an unsaved design".into(),
        };
        use crate::dialogs::{Choice, prompt, prompt_window};
        let shown = self.place.place(&self.prefs, ctx, "recover", prompt_window(ctx, "Recover unsaved work?")).show(ctx, |ui| {
            let lines = ["CutCraft didn't close properly last time.".to_string(), format!("A backup of {what} from {} was found.", p.info.age())];
            let choice = prompt(ui, "Recover unsaved work?", &lines, &[
                Choice::main("Restore").tip("Open the backup as an unsaved design"),
                Choice::other("Discard").tip("Delete the backup"),
                Choice::other("Later").tip("Keep the backup and ask again next time"),
            ]);
            match choice {
                Some(0) => {
                    if !self.dirty || confirm("Replace the current design with the backup?") {
                        self.recovery = None;
                        self.restore(p.clone());
                    }
                }
                Some(1) => {
                    self.autosaver.discard(&p);
                    self.recovery = None;
                }
                Some(_) => self.recovery = None,
                None => {}
            }
        });
        self.place.moved(&mut self.prefs, "recover", &shown);
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.show_about {
            let shown = self.place.place(&self.prefs, ctx, "about", egui::Window::new("About CutCraft").collapsible(false).resizable(false).open(&mut self.show_about)).show(ctx, |ui| {
                ui.heading("CutCraft 0.1");
                ui.label("Design and cut software for desktop vinyl/paper cutters.");
                ui.label("Written in Rust with egui, kurbo, geo and lyon.");
                ui.label("Cutter protocol support follows public GPGL documentation - always run a test cut first.");
                ui.add_space(6.0);
                ui.label("CutCraft is free software: you can share and change it under the GNU General Public License, version 3 or later. It comes with no warranty.");
                ui.hyperlink_to("Read the licence", "https://www.gnu.org/licenses/gpl-3.0.html");
            });
            self.place.moved(&mut self.prefs, "about", &shown);
        }
        if self.show_shortcuts {
            let shown = self.place.place(&self.prefs, ctx, "shortcuts", egui::Window::new("Keyboard shortcuts").open(&mut self.show_shortcuts)).show(ctx, |ui| {
                egui::Grid::new("sc").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("V / A / H / Z", "Select / edit points / pan / zoom"),
                        ("L / P / C / F", "Line / polyline / curve / freehand"),
                        ("R / E / T / K", "Rectangle / ellipse / text / knife"),
                        ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
                        ("Ctrl+C / X / V / D", "Copy / cut / paste / duplicate"),
                        ("Ctrl+G / Ctrl+Shift+G", "Group / ungroup"),
                        ("Ctrl+E", "Weld"),
                        ("Ctrl+8 / Ctrl+Shift+8", "Make / release compound path"),
                        ("Arrows (+Shift)", "Nudge 1 mm (10 mm)"),
                        ("Ctrl+0 / Ctrl+1", "Fit page / fit selection"),
                        ("Mouse wheel / middle drag", "Zoom / pan"),
                        ("Ctrl+Enter", "Send to cutter"),
                    ] {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
            self.place.moved(&mut self.prefs, "shortcuts", &shown);
        }
        if self.confirm_quit {
            use crate::dialogs::{Choice, prompt, prompt_window};
            let shown = self.place.place(&self.prefs, ctx, "unsaved", prompt_window(ctx, "Unsaved changes")).show(ctx, |ui| {
                let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "this design".into());
                let lines = [format!("Do you want to save your changes to {name}?"), "If you don't, they'll be lost.".to_string()];
                match prompt(ui, "Save changes before closing?", &lines, &[Choice::main("Save"), Choice::other("Don't save"), Choice::other("Cancel")]) {
                    Some(0) => {
                        self.save(false);
                        if !self.dirty {
                            self.quitting = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        self.confirm_quit = false;
                    }
                    Some(1) => {
                        self.quitting = true;
                        self.confirm_quit = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Some(_) => self.confirm_quit = false,
                    None => {}
                }
            });
            self.place.moved(&mut self.prefs, "unsaved", &shown);
        }
    }
}

/// Add system symbol/emoji fonts as fallbacks so toolbar glyphs render.
fn install_symbol_fonts(ctx: &egui::Context) {
    let candidates = [
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut added = false;
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("symbols{i}");
            fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.clone());
            }
            added = true;
        }
    }
    if added {
        ctx.set_fonts(fonts);
    }
}

/// A small design that exercises the main features.
pub fn sample_document(fonts: &FontLibrary) -> Document {
    use cutcraft_core::geom;
    use cutcraft_core::kurbo::Rect;
    let mut d = Document::default();
    let red = Style::default();
    let blue = Style { fill: Some(Rgba(120, 170, 235, 255)), stroke: Some(Rgba(30, 90, 220, 255)), ..Style::default() };
    let green = Style { fill: Some(Rgba(150, 215, 140, 255)), stroke: Some(Rgba(40, 150, 60, 255)), ..Style::default() };
    d.add_path(geom::rounded_rect_path(Rect::new(20.0, 20.0, 120.0, 90.0), 8.0), blue.clone(), 0);
    d.add_path(geom::ellipse_path(Rect::new(140.0, 20.0, 220.0, 100.0)), green.clone(), 0);
    d.add_path(geom::star_path(Rect::new(230.0, 20.0, 290.0, 80.0), 5, 0.45), red.clone(), 0);
    // weld of two circles
    let a = geom::circle_path(Point::new(60.0, 150.0), 30.0);
    let b = geom::circle_path(Point::new(95.0, 150.0), 30.0);
    d.add_path(geom::boolean(&[a], &[b], geom::BoolOp::Union), green, 0);
    // offset outline around a polygon
    let poly = geom::polygon_path(Rect::new(160.0, 120.0, 240.0, 190.0), 6);
    d.add_path(geom::offset(&[poly.clone()], 4.0, geom::CornerStyle::Round), Style { stroke: Some(Rgba(255, 140, 0, 255)), ..Style::default() }, 0);
    d.add_path(poly, blue, 0);
    let data = TextData { text: "CutCraft".into(), size: 28.0, ..TextData::default() };
    let outline = fonts.layout(&data);
    let id = d.alloc_id();
    d.page_mut().shapes.push(Shape {
        id,
        name: String::new(),
        layer: 0,
        transform: Affine::translate((25.0, 215.0)),
        style: Style { fill: Some(Rgba(40, 40, 40, 255)), ..Style::default() },
        kind: ShapeKind::Text { data, outline },
        locked: false,
        hidden: false,
        effects: Vec::new(),
    });
    d
}

pub fn confirm(msg: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title("CutCraft")
        .set_description(msg)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // close handling
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }
        // only when it changes: every window command makes egui draw another
        // frame, so sending it each frame redrew the window non-stop (high
        // processor use while idle, worst with software drawing)
        let title = self.title();
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        // keep the design to its locked sizes and distances
        crate::paramspanel::enforce(self);

        // dropped files
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for p in dropped {
            self.import_path(p);
        }

        let mut action = None;
        egui::Panel::top("menu").show(ui, |ui| {
            action = self.menu_bar(ui);
        });
        // a finished send, wherever it was started
        if self.sending && self.progress.lock().map(|p| p.done).unwrap_or(false) {
            self.sending = false;
            self.status = self.progress.lock().map(|p| p.message.clone()).unwrap_or_default();
        }
        if self.home.is_some() {
            crate::wizards::home_screen(self, ui);
        } else if self.prepare.is_some() {
            // the Prepare screen replaces the design workspace until "Back to design";
            // its status bar is just the message (page tabs, zoom and selection don't apply)
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status);
                });
            });
            crate::prepare_ui::show(self, &ctx, ui);
        } else {
            if action.is_none() {
                action = self.shortcuts(&ctx);
            }
            egui::Panel::top("toolbar").show(ui, |ui| {
                ui.add_space(2.0);
                if let Some(a) = crate::chrome::top_toolbar(self, ui) {
                    action = Some(a);
                }
                ui.add_space(2.0);
            });
            egui::Panel::top("editbar").show(ui, |ui| {
                crate::editbar::edit_bar(self, ui);
            });
            egui::Panel::left("tools").resizable(false).exact_size(crate::chrome::TOOL_STRIP_WIDTH).show(ui, |ui| {
                crate::chrome::tool_strip(self, ui);
            });
            egui::Panel::right("rail").resizable(false).exact_size(crate::chrome::RAIL_WIDTH).show(ui, |ui| {
                crate::chrome::rail(self, ui);
            });
            egui::Panel::right("panels").resizable(true).default_size(340.0).min_size(280.0).show(ui, |ui| {
                if let Some(a) = crate::panels::show(self, ui) {
                    action = Some(a);
                }
            });
            egui::Panel::bottom("status").show(ui, |ui| {
                crate::canvas::status_bar(self, ui);
            });
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                crate::canvas::show(self, ui);
            });
        }
        if let Some(a) = action {
            self.do_action(a, &ctx);
        }
        self.dialogs(&ctx);
        self.recovery_dialog(&ctx);
        crate::wizards::upload_window(self, &ctx);
        crate::wizards::tool_change_window(self, &ctx);
        crate::calibrate::window(self, &ctx);
        crate::eyedrop::tick(self, &ctx);
        crate::browser::tick(self, &ctx, frame);
        crate::panels2::pixscan_window(self, &ctx);
        self.autosave();
        crate::startup::frame_shown();
        self.place.end_frame();
        self.frame_watch.frame(&ctx);
        self.track_window(&ctx);
        if self.sending {
            self.watch_progress(&ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosaver.finish();
        if self.window_place.is_some() {
            self.prefs.window = self.window_place;
        }
        let _ = self.prefs.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_labels_read_as_words() {
        assert_eq!(action_label(&Action::FitIntoBox), "Fit into box");
        assert_eq!(action_label(&Action::Undo), "Undo");
    }
}
