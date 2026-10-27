fn main() {
    // `has_browser`: this build has the built-in browser (Windows and macOS
    // always, Linux with the linux-browser feature).
    println!("cargo::rustc-check-cfg=cfg(has_browser)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let linux_browser = std::env::var_os("CARGO_FEATURE_LINUX_BROWSER").is_some();
    if os == "windows" || os == "macos" || (os == "linux" && linux_browser) {
        println!("cargo:rustc-cfg=has_browser");
    }

    // Embed the application icon into the Windows .exe so Explorer/taskbar show it.
    #[cfg(windows)]
    if os == "windows" {
        println!("cargo:rerun-if-changed=assets/cutcraft.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/cutcraft.ico");
        if let Err(e) = res.compile() {
            // Don't fail the build if no resource compiler is available.
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
