//! A web browser inside CutCraft, for finding designs and fonts on sites
//! like SVG Repo and FONT Repo: whatever you download in it comes straight
//! into CutCraft - SVGs into the design, fonts into the font list, anything
//! else CutCraft opens (pictures, PDFs, DXF…) through the usual import.
//!
//! It's the system's own web view set into CutCraft's window: Edge WebView2
//! on Windows, WKWebView on macOS, and WebKitGTK on Linux (built with the
//! `linux-browser` feature, in an X11 session).

use crate::app::App;
use std::path::PathBuf;

/// Sites with a button to open them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    SvgRepo,
    FontRepo,
}

impl Site {
    pub fn url(self) -> &'static str {
        match self {
            Site::SvgRepo => "https://www.svgrepo.com/",
            Site::FontRepo => "https://www.fontrepo.com/",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Site::SvgRepo => "SVG Repo",
            Site::FontRepo => "FONT Repo",
        }
    }
}

pub fn available() -> bool {
    cfg!(has_browser)
}

/// A button that opens `site` in the browser; greyed out, saying why, in a
/// build without it.
pub fn site_button(app: &mut App, ui: &mut egui::Ui, site: Site, hover: &str) {
    let r = ui
        .add_enabled(available(), egui::Button::new(format!("🌐 Browse {}", site.name())))
        .on_hover_text(hover)
        .on_disabled_hover_text("This copy of CutCraft was built without the browser. On Linux, install libwebkit2gtk-4.1-dev and libgtk-3-dev and build it again (the browser is on by default; --no-default-features leaves it out).");
    if r.clicked() {
        open(app, site);
    }
}

/// What the page told us.
#[allow(dead_code)]
enum Msg {
    Started(String),
    Finished(Option<PathBuf>, bool),
    Popup(String),
    /// A file link to download through the page (macOS).
    Fetch(String),
}

#[cfg(has_browser)]
pub struct Browser {
    view: wry::WebView,
    _context: wry::WebContext,
    rx: std::sync::mpsc::Receiver<Msg>,
    note: String,
    shown: Option<egui::Rect>,
    /// Ad blocking on, and how many requests it has refused (shared with
    /// the page's request handler).
    adblock: std::sync::Arc<std::sync::atomic::AtomicBool>,
    blocked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(not(has_browser))]
pub struct Browser;

/// Opens the browser at `site` (next frame).
pub fn open(app: &mut App, site: Site) {
    app.browser_request = Some(site);
}

/// Runs the browser for this frame: opens it when asked, keeps it over its
/// spot in the window, and brings in finished downloads.
#[cfg(has_browser)]
pub fn tick(app: &mut App, ctx: &egui::Context, frame: &eframe::Frame) {
    #[cfg(target_os = "linux")]
    if app.browser.is_some() {
        // WebKitGTK runs on GTK's event loop, so give it its turn
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
    }
    if let Some(site) = app.browser_request.take() {
        match &app.browser {
            Some(b) => {
                let _ = b.view.load_url(site.url());
            }
            None => match build(frame, site.url(), app.prefs.browser_adblock) {
                Ok(b) => app.browser = Some(b),
                Err(e) => app.status = format!("Couldn't open the browser: {e}"),
            },
        }
    }
    let Some(b) = &mut app.browser else { return };
    // downloads and pop-ups arrive from the page between frames; on Linux the
    // page itself only loads and draws while GTK gets its turn each frame
    let every = if cfg!(target_os = "linux") { 15 } else { 250 };
    ctx.request_repaint_after(std::time::Duration::from_millis(every));
    let mut arrived = vec![];
    while let Ok(m) = b.rx.try_recv() {
        match m {
            Msg::Started(name) => b.note = format!("Downloading {name}…"),
            Msg::Finished(Some(p), true) => arrived.push(p),
            Msg::Finished(_, _) => b.note = "The download didn't finish".into(),
            Msg::Popup(url) => {
                let _ = b.view.load_url(&url);
            }
            Msg::Fetch(url) => {
                b.note = format!("Downloading {}…", url.rsplit('/').next().unwrap_or("the file"));
                let _ = b.view.evaluate_script(&cutcraft_core::web::page_fetch_script(&url));
            }
        }
    }

    let mut open = true;
    let mut go: Option<&'static str> = None;
    let mut script: Option<&'static str> = None;
    let area = ctx.content_rect().shrink2(egui::vec2(40.0, 36.0));
    let mut spot = None;
    let mut adblock = b.adblock.load(std::sync::atomic::Ordering::Relaxed);
    let blocked = b.blocked.load(std::sync::atomic::Ordering::Relaxed);
    let mut rebuild = None;
    egui::Window::new("Browser")
        .open(&mut open)
        .default_rect(area)
        .min_size(egui::vec2(480.0, 320.0))
        .collapsible(false)
        .resizable(true)
        .constrain(true)
        .show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("◀").on_hover_text("Back").clicked() {
                script = Some("history.back()");
            }
            if ui.button("▶").on_hover_text("Forward").clicked() {
                script = Some("history.forward()");
            }
            if ui.button("⟳").on_hover_text("Reload").clicked() {
                script = Some("location.reload()");
            }
            ui.separator();
            for site in [Site::SvgRepo, Site::FontRepo] {
                if ui.button(site.name()).clicked() {
                    go = Some(site.url());
                }
            }
            ui.separator();
            ui.checkbox(&mut adblock, "Block ads").on_hover_text(format!(
                "Refuses requests to advertising and tracking networks ({blocked} blocked so far). Add your own in adblock.txt in CutCraft's settings folder."
            ));
            ui.separator();
            ui.label("Click a Download button - the file comes straight into CutCraft.");
        });
        if !b.note.is_empty() {
            ui.label(&b.note);
        }
        // leave a strip under the page for the resize grip
        let (r, _) = ui.allocate_exact_size(ui.available_size() - egui::vec2(0.0, 18.0), egui::Sense::hover());
        spot = Some(r);
    });
    if adblock != b.adblock.load(std::sync::atomic::Ordering::Relaxed) {
        b.adblock.store(adblock, std::sync::atomic::Ordering::Relaxed);
        app.prefs.browser_adblock = adblock;
        let _ = app.prefs.save();
        if cfg!(windows) {
            script = Some("location.reload()");
        } else {
            // there the blocker is part of the page's start-up script, so
            // open the page afresh in a new web view
            rebuild = Some(b.view.url().unwrap_or_else(|_| Site::SvgRepo.url().into()));
        }
    }
    if let Some(r) = spot {
        if b.shown != Some(r) {
            let z = ctx.zoom_factor();
            let _ = b.view.set_bounds(wry::Rect {
                position: wry::dpi::LogicalPosition::new((r.min.x * z) as f64, (r.min.y * z) as f64).into(),
                size: wry::dpi::LogicalSize::new((r.width() * z) as f64, (r.height() * z) as f64).into(),
            });
            b.shown = Some(r);
        }
    }
    if let Some(url) = go {
        let _ = b.view.load_url(url);
    }
    if let Some(js) = script {
        let _ = b.view.evaluate_script(js);
    }
    for p in arrived {
        let msg = bring_in(app, p);
        if let Some(b) = &mut app.browser {
            b.note = msg;
        }
    }
    if let Some(url) = rebuild {
        close(app);
        match build(frame, &url, adblock) {
            Ok(b) => app.browser = Some(b),
            Err(e) => app.status = format!("Couldn't open the browser: {e}"),
        }
    }
    if !open {
        close(app);
    }
}

/// Takes the browser off the screen and closes it.
#[cfg(has_browser)]
fn close(app: &mut App) {
    let Some(b) = app.browser.take() else { return };
    let _ = b.view.set_visible(false);
    #[cfg(not(target_os = "linux"))]
    drop(b);
    // On Linux wry destroys the web view's X11 window before the GTK window
    // inside it, so when GTK closes that it asks X about a window that's gone,
    // and GDK's default answer to the X error is to end the program. Trap X
    // errors while it all happens, and give GTK the turns it needs to send
    // the requests on (nothing else will, now the browser is gone).
    #[cfg(target_os = "linux")]
    #[allow(deprecated)]
    {
        let display = gtk::gdk::Display::default();
        gtk::gdk::error_trap_push();
        drop(b);
        for _ in 0..3 {
            if let Some(d) = &display {
                d.sync();
            }
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
        }
        if let Some(d) = &display {
            d.sync();
        }
        gtk::gdk::error_trap_pop_ignored();
    }
}

#[cfg(not(has_browser))]
pub fn tick(app: &mut App, _ctx: &egui::Context, _frame: &eframe::Frame) {
    if let Some(site) = app.browser_request.take() {
        app.status = format!("This build has no built-in browser - open {} ({}) in your web browser", site.name(), site.url());
    }
}

#[cfg(has_browser)]
fn build(frame: &eframe::Frame, url: &str, adblock_on: bool) -> anyhow::Result<Browser> {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    #[cfg(target_os = "linux")]
    {
        use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let raw = frame.window_handle().map_err(|e| anyhow::anyhow!("{e}"))?.as_raw();
        if !matches!(raw, RawWindowHandle::Xlib(_)) {
            anyhow::bail!("on Linux it needs X11 - CutCraft is running on Wayland (install XWayland, or unset CUTCRAFT_WAYLAND)");
        }
        gtk::init().map_err(|e| anyhow::anyhow!("GTK didn't start: {e}"))?;
        // the web view goes into an X11 window, so GTK has to be on X11 as well
        // (wry panics otherwise)
        use gtk::prelude::ObjectExt;
        let gdk = gtk::gdk::Display::default().map(|d| d.type_().name().to_string()).unwrap_or_default();
        if gdk != "GdkX11Display" {
            anyhow::bail!("GTK is using {} rather than X11 - start CutCraft with GDK_BACKEND=x11", if gdk.is_empty() { "no display" } else { gdk.as_str() });
        }
    }
    let data = cutcraft_core::prefs::config_dir().join("browser");
    let mut context = wry::WebContext::new(Some(data));
    let downloads = std::env::temp_dir().join("cutcraft-downloads");
    std::fs::create_dir_all(&downloads)?;
    let (tx, rx) = channel();
    let (tx2, tx3) = (tx.clone(), tx.clone());
    #[cfg(target_os = "macos")]
    let (tx4, tx5, downloads5) = (tx.clone(), tx.clone(), downloads.clone());
    // where each download is going (macOS doesn't say when it finishes)
    let going: Arc<Mutex<Vec<(String, PathBuf)>>> = Arc::default();
    let going2 = going.clone();
    let adblock = Arc::new(AtomicBool::new(adblock_on));
    let blocked = Arc::new(AtomicUsize::new(0));
    #[allow(unused_mut)]
    let mut builder = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_bounds(wry::Rect { position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(), size: wry::dpi::LogicalSize::new(10.0, 10.0).into() })
        .with_download_started_handler(move |url, path| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "download".into());
            *path = downloads.join(&name);
            // an earlier download of the same file
            let _ = std::fs::remove_file(&*path);
            if let Ok(mut g) = going.lock() {
                g.push((url, path.clone()));
            }
            let _ = tx.send(Msg::Started(name));
            true
        })
        .with_download_completed_handler(move |url, path, ok| {
            let known = going2.lock().ok().and_then(|mut g| g.iter().rposition(|(u, _)| *u == url).map(|i| g.remove(i).1));
            let _ = tx2.send(Msg::Finished(path.or(known), ok));
        })
        // links that open a new window open here instead
        .with_new_window_req_handler(move |url, _| {
            let _ = tx3.send(Msg::Popup(url));
            wry::NewWindowResponse::Deny
        });
    // macOS's web view shows a file it can display (an SVG) even when the
    // site sends it as a download, so there CutCraft stops file links and
    // has the page download them instead, handing the file over by message
    #[cfg(target_os = "macos")]
    {
        builder = builder.with_navigation_handler(move |url| {
            if cutcraft_core::web::is_file_link(&url) {
                let _ = tx4.send(Msg::Fetch(url));
                return false;
            }
            true
        });
    }
    // macOS and Linux: the page blocks its own ad requests and says so
    #[cfg(not(windows))]
    {
        if adblock_on {
            builder = builder.with_initialization_script(cutcraft_core::adblock::Blocker::load().page_script());
        }
        let count = blocked.clone();
        builder = builder.with_ipc_handler(move |req| {
            let body = req.body();
            if body == "blocked" {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            #[cfg(target_os = "macos")]
            if body.starts_with(cutcraft_core::web::PAGE_FILE_PREFIX) {
                let saved = cutcraft_core::web::page_file(body).and_then(|(name, bytes)| {
                    let p = downloads5.join(name);
                    std::fs::write(&p, bytes)?;
                    Ok(p)
                });
                let _ = tx5.send(match saved {
                    Ok(p) => Msg::Finished(Some(p), true),
                    Err(_) => Msg::Finished(None, false),
                });
            }
        });
    }
    let view = builder.build_as_child(frame)?;
    #[cfg(windows)]
    install_adblock(&view, adblock.clone(), blocked.clone()).map_err(|e| anyhow::anyhow!("ad blocking: {e}"))?;
    Ok(Browser { view, _context: context, rx, note: String::new(), shown: None, adblock, blocked })
}

/// Refuses the page's requests to ad and tracking networks (while `on`),
/// counting them, and turns on WebView2's strict tracking prevention.
#[cfg(windows)]
fn install_adblock(
    view: &wry::WebView,
    on: std::sync::Arc<std::sync::atomic::AtomicBool>,
    blocked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> windows::core::Result<()> {
    use std::sync::atomic::Ordering;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{WebResourceRequestedEventHandler, take_pwstr};
    use windows::core::{HSTRING, Interface, PWSTR};
    use wry::WebViewExtWindows;
    let webview = view.webview();
    let env = view.environment();
    let list = cutcraft_core::adblock::Blocker::load();
    // SAFETY: WebView2 COM calls on the UI thread that owns the web view; the
    // handler runs on that thread too.
    unsafe {
        if let Ok(profile) = webview.cast::<ICoreWebView2_13>().and_then(|w| w.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile3>()) {
            let _ = profile.SetPreferredTrackingPreventionLevel(COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_STRICT);
        }
        let all = HSTRING::from("*");
        // every request, including from frames inside the page where supported
        match webview.cast::<ICoreWebView2_22>() {
            Ok(w22) => w22.AddWebResourceRequestedFilterWithRequestSourceKinds(&all, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL)?,
            Err(_) => webview.AddWebResourceRequestedFilter(&all, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?,
        }
        let mut token = 0i64;
        webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                if !on.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                if list.blocks(&take_pwstr(uri)) {
                    let response = env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked"), &HSTRING::new())?;
                    args.SetResponse(&response)?;
                    blocked.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

/// Brings a downloaded file into CutCraft; says what happened.
#[cfg(has_browser)]
fn bring_in(app: &mut App, p: PathBuf) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let msg = match ext.as_str() {
        "svg" => match std::fs::read_to_string(&p) {
            Ok(svg) => {
                let size = app.web.size;
                match crate::webui::add_svg(app, &svg, size, name.trim_end_matches(".svg")) {
                    Ok(()) => format!("Added {name} to the design"),
                    Err(e) => e,
                }
            }
            Err(e) => format!("Couldn't read {name}: {e}"),
        },
        "ttf" | "otf" => match std::fs::read(&p).map_err(anyhow::Error::from).and_then(|d| app.fonts.add_font(&name, d)) {
            Ok(families) => {
                crate::webui::fonts_changed(app);
                format!("Added the font {} - pick it in the Text panel's font list. Check its licence before selling what you make.", families.join(", "))
            }
            Err(e) => format!("{name}: {e}"),
        },
        _ => {
            app.import_path(p.clone());
            format!("Imported {name}")
        }
    };
    app.status = msg.clone();
    msg
}
