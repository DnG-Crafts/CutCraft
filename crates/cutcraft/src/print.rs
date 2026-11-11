//! File ▸ Print: the page sent to a printer at its true size - what the
//! Print & Cut PDF has (pictures, filled shapes, registration marks).
//!
//! Windows: the usual print dialog, then the page drawn through the printer
//! driver, lined up with the paper's own corner (the printer's unprintable
//! margin taken off), so it prints at 100%. macOS and Linux: a small dialog
//! listing the system's printers, then the PDF sent with CUPS's `lp`, with
//! scaling off.

use crate::app::App;

/// The printer dialog on macOS and Linux.
#[derive(Default)]
pub struct PrintDialog {
    printers: Vec<String>,
    printer: usize,
    copies: u32,
    note: String,
}

/// Starts printing the current page.
pub fn start(app: &mut App) {
    if app.doc.page().shapes.iter().all(|s| !app.doc.page().is_visible(s)) && !app.doc.page().regmarks.enabled {
        app.status = "Nothing to print on this page".into();
        return;
    }
    #[cfg(windows)]
    {
        app.status = match windows_print(app.doc.page(), &app.title()) {
            Ok(Some(note)) => note,
            Ok(None) => "Printing cancelled".into(),
            Err(e) => format!("Couldn't print: {e}"),
        };
    }
    #[cfg(not(windows))]
    {
        let printers = cups_printers();
        app.print_dialog = Some(PrintDialog { note: if printers.is_empty() { "No printers found - add one in your system's printer settings.".into() } else { String::new() }, printers, printer: 0, copies: 1 });
    }
}

/// The printer dialog (macOS and Linux), while it's open.
pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.print_dialog.take() else { return };
    let mut open = true;
    let mut send = false;
    egui::Window::new("Print").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
        if d.printers.is_empty() {
            ui.label(&d.note);
            return;
        }
        egui::Grid::new("print").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Printer");
            egui::ComboBox::from_id_salt("printer").selected_text(&d.printers[d.printer.min(d.printers.len() - 1)]).show_ui(ui, |ui| {
                for (i, p) in d.printers.iter().enumerate() {
                    ui.selectable_value(&mut d.printer, i, p);
                }
            });
            ui.end_row();
            ui.label("Copies");
            ui.add(egui::DragValue::new(&mut d.copies).range(1..=99));
            ui.end_row();
        });
        ui.small("Prints at 100% (actual size) - right for Print & Cut. Set the paper size in the printer's settings to match the page.");
        if !d.note.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(220, 80, 60), &d.note);
        }
        if ui.button("Print").clicked() {
            send = true;
        }
    });
    if send {
        match cups_print(app.doc.page(), &d.printers[d.printer.min(d.printers.len() - 1)], d.copies) {
            Ok(()) => {
                app.status = format!("Sent to {}", d.printers[d.printer.min(d.printers.len() - 1)]);
                return;
            }
            Err(e) => d.note = format!("Couldn't print: {e}"),
        }
    }
    if open {
        app.print_dialog = Some(d);
    }
}

/// The system's printers (CUPS), the default first.
#[cfg_attr(windows, allow(dead_code))]
fn cups_printers() -> Vec<String> {
    let run = |args: &[&str]| std::process::Command::new("lpstat").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut list: Vec<String> = run(&["-e"]).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    if list.is_empty() {
        // older CUPS: "printer NAME is idle…"
        list = run(&["-p"]).lines().filter_map(|l| l.strip_prefix("printer ")).filter_map(|l| l.split_whitespace().next()).map(String::from).collect();
    }
    // "system default destination: NAME"
    if let Some(def) = run(&["-d"]).split(':').nth(1).map(|s| s.trim().to_string()) {
        if let Some(i) = list.iter().position(|p| *p == def) {
            let p = list.remove(i);
            list.insert(0, p);
        }
    }
    list
}

#[cfg_attr(windows, allow(dead_code))]
fn cups_print(page: &cutcraft_core::doc::Page, printer: &str, copies: u32) -> anyhow::Result<()> {
    let path = std::env::temp_dir().join("cutcraft-print.pdf");
    cutcraft_core::io::export_print_pdf(page, &path)?;
    let out = std::process::Command::new("lp")
        .args(["-d", printer, "-n", &copies.max(1).to_string(), "-o", "print-scaling=none", "-o", "fit-to-page=false", "-t", "CutCraft"])
        .arg(&path)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Shows the Windows print dialog and prints the page. None if cancelled,
/// else what happened.
#[cfg(windows)]
fn windows_print(page: &cutcraft_core::doc::Page, title: &str) -> anyhow::Result<Option<String>> {
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::UI::Controls::Dialogs::*;

    // SAFETY: plain Win32 printing on the UI thread; every handle the dialog
    // returns is released below, and the bitmap outlives the drawing call.
    unsafe {
        // start from the default printer's settings, turned to suit the page
        let mut pd = PRINTDLGW { lStructSize: std::mem::size_of::<PRINTDLGW>() as u32, Flags: PD_RETURNDEFAULT, ..Default::default() };
        if PrintDlgW(&mut pd).as_bool() && !pd.hDevMode.is_invalid() {
            let dm = GlobalLock(pd.hDevMode) as *mut DEVMODEW;
            if !dm.is_null() {
                (*dm).dmFields |= DM_ORIENTATION;
                (*dm).Anonymous1.Anonymous1.dmOrientation = if page.width > page.height { DMORIENT_LANDSCAPE } else { DMORIENT_PORTRAIT } as i16;
                let _ = GlobalUnlock(pd.hDevMode);
            }
        }
        pd.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
        pd.nCopies = 1;
        let chosen = PrintDlgW(&mut pd).as_bool();
        let free = |h: HGLOBAL| {
            if !h.is_invalid() {
                let _ = GlobalFree(Some(h));
            }
        };
        if !chosen || pd.hDC.is_invalid() {
            free(pd.hDevMode);
            free(pd.hDevNames);
            return Ok(None);
        }
        let dc = pd.hDC;
        let copies = pd.nCopies.max(1);
        let result = print_to_dc(dc, page, title, copies, None);
        let _ = DeleteDC(dc);
        free(pd.hDevMode);
        free(pd.hDevNames);
        let (paper_w, paper_h) = result?;
        let fits = page.width <= paper_w + 0.5 && page.height <= paper_h + 0.5;
        Ok(Some(if fits {
            "Sent to the printer at 100% - for Print & Cut, check the printer isn't set to scale or fit to page".into()
        } else {
            format!("Sent to the printer - but the page ({:.0} x {:.0} mm) is bigger than the paper ({paper_w:.0} x {paper_h:.0} mm), so only part of it prints", page.width, page.height)
        }))
    }
}

/// Prints `page` on the printer device context `dc` (`copies` times, or to
/// the file `output` for a printer that writes files); returns the paper
/// size (mm).
#[cfg(windows)]
unsafe fn print_to_dc(dc: windows::Win32::Graphics::Gdi::HDC, page: &cutcraft_core::doc::Page, title: &str, copies: u16, output: Option<&std::path::Path>) -> anyhow::Result<(f64, f64)> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
    use windows::core::PCWSTR;
    // SAFETY: the caller's printer DC; the bitmap and strings outlive the calls.
    unsafe {
        let cap = |i| GetDeviceCaps(Some(dc), i);
        let (dpi_x, dpi_y) = (cap(LOGPIXELSX) as f64, cap(LOGPIXELSY) as f64);
        let (off_x, off_y) = (cap(PHYSICALOFFSETX), cap(PHYSICALOFFSETY));
        let paper = (cap(PHYSICALWIDTH) as f64 / dpi_x.max(1.0) * 25.4, cap(PHYSICALHEIGHT) as f64 / dpi_y.max(1.0) * 25.4);

        // the page at up to 300 dpi; the driver scales it to the printer's own
        let (img, _) = cutcraft_core::printflat::render_print(page, dpi_x.max(dpi_y).min(300.0));
        let (w, h) = (img.width() as i32, img.height() as i32);
        let bgra: Vec<u8> = img.pixels().flat_map(|p| [p.0[2], p.0[1], p.0[0], 255]).collect();
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
        let (dest_w, dest_h) = ((page.width / 25.4 * dpi_x).round() as i32, (page.height / 25.4 * dpi_y).round() as i32);

        let name: Vec<u16> = format!("CutCraft - {title}").encode_utf16().chain([0]).collect();
        let out: Option<Vec<u16>> = output.map(|p| p.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect());
        let doc = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(name.as_ptr()),
            lpszOutput: out.as_ref().map_or(PCWSTR::null(), |o| PCWSTR(o.as_ptr())),
            ..Default::default()
        };
        if StartDocW(dc, &doc) <= 0 {
            anyhow::bail!("the printer didn't start the job");
        }
        let mut result = Ok(paper);
        for _ in 0..copies.max(1) {
            if StartPage(dc) <= 0 {
                result = Err(anyhow::anyhow!("the printer didn't start the page"));
                break;
            }
            // the page's corner on the paper's corner, not the printable area's
            if StretchDIBits(dc, -off_x, -off_y, dest_w, dest_h, 0, 0, w, h, Some(bgra.as_ptr() as *const _), &bmi, DIB_RGB_COLORS, SRCCOPY) == 0 {
                result = Err(anyhow::anyhow!("the printer driver didn't take the picture"));
            }
            EndPage(dc);
        }
        EndDoc(dc);
        result
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// Prints a sample page with Windows' own "Microsoft Print to PDF"
    /// printer, to the file in CUTCRAFT_PRINT_TO (run by hand).
    #[test]
    #[ignore = "needs the Microsoft Print to PDF printer"]
    fn prints_to_a_pdf_printer() {
        use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC};
        use windows::core::{PCWSTR, w};
        let Ok(out) = std::env::var("CUTCRAFT_PRINT_TO") else { return };
        let mut doc = cutcraft_core::doc::Document::default();
        let red = cutcraft_core::doc::Style { fill: Some(cutcraft_core::doc::Rgba(220, 30, 30, 255)), ..Default::default() };
        // a 100 x 50 mm red box, 20 mm from the paper's top-left corner
        doc.add_path(cutcraft_core::geom::rect_path(cutcraft_core::kurbo::Rect::new(20.0, 20.0, 120.0, 70.0)), red, 0);
        {
            let p = doc.page_mut();
            p.width = 210.0;
            p.height = 297.0;
        }
        unsafe {
            let dc = CreateDCW(w!("WINSPOOL"), w!("Microsoft Print to PDF"), PCWSTR::null(), None);
            assert!(!dc.is_invalid(), "no Microsoft Print to PDF printer");
            let paper = super::print_to_dc(dc, doc.page(), "test", 1, Some(std::path::Path::new(&out))).unwrap();
            let _ = DeleteDC(dc);
            println!("paper {paper:?}");
        }
    }
}
