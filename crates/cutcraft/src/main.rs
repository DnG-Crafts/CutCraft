#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod browser;
mod calibrate;
mod canvas;
mod dialogs;
mod chrome;
mod editbar;
mod eyedrop;
mod icon;
mod icons;
mod panels;
mod panels2;
mod paramspanel;
mod prepare_ui;
mod queue_ui;
mod recovery;
mod render;
mod webui;
mod wizards;
mod startup;

/// `software_ok` lets the DirectX 12 / Vulkan renderer use a software adapter
/// (the last resort).
fn options(renderer: eframe::Renderer, software_ok: bool) -> eframe::NativeOptions {
    // reopen where it was last closed, if that's still on a screen
    let saved = cutcraft_core::prefs::Prefs::load().window.filter(startup::placement_visible);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("CutCraft")
        .with_icon(std::sync::Arc::new(icon::icon_data()))
        .with_inner_size([1400.0, 820.0])
        .with_min_inner_size([900.0, 600.0])
        .with_drag_and_drop(true);
    if let Some(w) = saved {
        viewport = viewport.with_position([w.x, w.y]).with_inner_size([w.w.max(900.0), w.h.max(600.0)]).with_maximized(w.maximized);
    }
    let selector: Option<eframe::egui_wgpu::NativeAdapterSelectorMethod> = if software_ok { None } else { Some(std::sync::Arc::new(startup::hardware_adapter)) };
    eframe::NativeOptions {
        renderer,
        viewport,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(eframe::egui_wgpu::WgpuSetupCreateNew {
                native_adapter_selector: selector,
                ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn run(renderer: eframe::Renderer, software_ok: bool) -> eframe::Result {
    startup::note(&format!("opening the window ({renderer:?} renderer{})", if software_ok { ", software drawing allowed" } else { "" }));
    eframe::run_native(
        "CutCraft",
        options(renderer, software_ok),
        Box::new(|cc| {
            startup::log_renderer(cc);
            let app = app::App::new(cc);
            startup::note("ready");
            Ok(Box::new(app))
        }),
    )
}

fn main() {
    // decide before the log is overwritten: did the GPU renderer kill the last start?
    let prefer_opengl = startup::prefer_opengl();
    startup::begin();
    if prefer_opengl {
        startup::note("using OpenGL: the GPU renderer crashed while starting on this PC before");
    }
    // On Windows the GPU renderer uses DirectX 12 only: Vulkan pulls in add-ons
    // ("layers") from overlays and capture tools, a common cause of start-up crashes.
    if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }
    // The Linux browser (WebKitGTK) can only sit inside an X11 window, so on
    // a Wayland desktop with XWayland, open the window through X11.
    // CUTCRAFT_WAYLAND=1 keeps native Wayland (no browser).
    if cfg!(all(target_os = "linux", has_browser))
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_some()
        && std::env::var_os("CUTCRAFT_WAYLAND").is_none()
    {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        startup::note("Wayland desktop: opening through XWayland so the built-in browser works (CUTCRAFT_WAYLAND=1 to keep Wayland)");
    }
    // ...and GTK must use X11 too: without WAYLAND_DISPLAY it would still
    // find Wayland's default socket.
    if cfg!(all(target_os = "linux", has_browser)) && std::env::var_os("CUTCRAFT_WAYLAND").is_none() && std::env::var_os("GDK_BACKEND").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    // WebKitGTK's DMA-BUF renderer often draws nothing in an embedded window
    // (XWayland, NVIDIA): use its plain one.
    if cfg!(all(target_os = "linux", has_browser)) && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    // CUTCRAFT_RENDERER=gl goes straight to OpenGL (older chips, virtual machines, remote desktop).
    let force_gl = std::env::var("CUTCRAFT_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("gl") || v.eq_ignore_ascii_case("opengl"));
    // Order: DirectX 12 / Vulkan on a hardware graphics chip, then OpenGL, then —
    // so CutCraft still opens on a PC with no working graphics driver — software drawing.
    let software = |earlier: String| {
        startup::note(&format!("{earlier}; using software drawing (slow)"));
        run(eframe::Renderer::Wgpu, true).map_err(|last| format!("{earlier}\nSoftware: {last}"))
    };
    let result = if force_gl || prefer_opengl {
        run(eframe::Renderer::Glow, false).map_err(|e| format!("OpenGL: {e}")).or_else(software)
    } else {
        run(eframe::Renderer::Wgpu, false)
            .or_else(|first| {
                startup::note(&format!("GPU renderer failed: {first}; trying OpenGL"));
                run(eframe::Renderer::Glow, false).map_err(|second| format!("DirectX/Vulkan: {first}\nOpenGL: {second}"))
            })
            .or_else(software)
    };
    startup::note(if result.is_ok() { "closed normally" } else { "the graphics could not start" });
    if let Err(e) = result {
        rfd::MessageDialog::new()
            .set_title("CutCraft")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!(
                "CutCraft couldn't start its graphics.\n\n{e}\n\nUpdating your graphics driver usually fixes this. CutCraft needs DirectX 12, Vulkan or OpenGL 2 or newer.\n\nDetails: {}",
                startup::log_path().display()
            ))
            .show();
    }
}
