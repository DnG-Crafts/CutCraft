//! The Send panel's GRBL section: pick a serial port, connect, see where the
//! machine is, jog it, set the origin, frame the job and talk to it directly.

use crate::app::App;
use cutcraft_core::grbl::{self, Dialect, GrblLink, Head};
use cutcraft_core::{cutjob, machines};
use egui::{Color32, DragValue, RichText, Ui};
use std::sync::atomic::Ordering;

const BAUDS: [u32; 7] = [9600, 19200, 38400, 57600, 115200, 230400, 250000];

/// The open link, if it's still alive and the controller has answered.
fn ready(app: &App) -> Option<&GrblLink> {
    app.ps.x.grbl.as_ref().filter(|l| {
        let s = l.state();
        s.answered && !s.closed
    })
}

pub fn section(app: &mut App, ui: &mut Ui) {
    let maker3 = grbl::is_maker3(&app.doc.machine);
    let dialect = if maker3 { Dialect::Maker3 } else { Dialect::Stock };
    // a connection made for another machine plays by that machine's rules
    if !app.sending && app.ps.x.grbl.as_ref().is_some_and(|l| l.dialect != dialect) {
        app.ps.x.grbl = None;
    }
    ui.heading(if maker3 { "Cricut Maker 3 connection" } else { "GRBL connection" });
    if app.ps.x.grbl_scanned != Some(dialect) {
        app.ps.x.grbl_scanned = Some(dialect);
        scan(app, maker3);
    }
    let connected = app.ps.x.grbl.as_ref().is_some_and(|l| !l.state().closed);
    ui.add_enabled_ui(!connected, |ui| {
        ui.horizontal(|ui| {
            ui.label("Port");
            let shown = app.ps.x.grbl_ports.iter().find(|p| p.name == app.prefs.grbl.port).map(|p| format!("{} - {}", p.name, p.description)).unwrap_or_else(|| app.prefs.grbl.port.clone());
            egui::ComboBox::from_id_salt("grbl_port").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                for p in &app.ps.x.grbl_ports {
                    ui.selectable_value(&mut app.prefs.grbl.port, p.name.clone(), format!("{} - {}", p.name, p.description));
                }
                if app.ps.x.grbl_ports.is_empty() {
                    ui.weak("No serial ports found");
                }
            });
            if ui.button("⟳").on_hover_text("Look for serial ports again").clicked() {
                scan(app, maker3);
            }
        });
        ui.horizontal(|ui| {
            if !maker3 {
                ui.label("Speed");
                egui::ComboBox::from_id_salt("grbl_baud").selected_text(app.prefs.grbl.baud.to_string()).show_ui(ui, |ui| {
                    for b in BAUDS {
                        ui.selectable_value(&mut app.prefs.grbl.baud, b, b.to_string());
                    }
                });
            }
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut app.prefs.grbl.port).desired_width(110.0)).on_hover_text("Type a port name if it isn't listed, e.g. COM5 or /dev/ttyACM0");
        });
    });
    ui.horizontal(|ui| {
        if !connected {
            let can = !app.prefs.grbl.port.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(RichText::new("Connect").strong())).clicked() {
                let baud = if maker3 { 115200 } else { app.prefs.grbl.baud };
                app.ps.x.grbl = Some(GrblLink::connect(app.prefs.grbl.port.trim(), baud, dialect));
                let _ = app.prefs.save();
            }
        } else if ui.add_enabled(!app.sending, egui::Button::new("Disconnect")).clicked() {
            app.ps.x.grbl = None;
        }
    });

    match app.ps.x.grbl.take() {
        Some(link) => {
            live(app, ui, &link);
            app.ps.x.grbl = Some(link);
        }
        None => {
            ui.small(if maker3 {
                "Hold the machine's Power button for 3 s and let it home, then ⟳ and Connect. The port only exists once the machine is on."
            } else {
                "Connect the machine by USB, pick its port and Connect. Most GRBL boards use 115200."
            });
        }
    }
    if maker3 {
        maker3_settings(app, ui);
    } else {
        settings(app, ui);
    }
}

/// Look for serial ports and pick the likely one.
fn scan(app: &mut App, maker3: bool) {
    app.ps.x.grbl_ports = grbl::list_ports();
    if maker3 {
        if let Some(p) = app.ps.x.grbl_ports.iter_mut().find(|p| p.usb == Some(grbl::MAKER3_USB)) {
            p.description = "Cricut Maker 3".into();
            app.prefs.grbl.port = p.name.clone();
            return;
        }
    }
    if app.prefs.grbl.port.is_empty() {
        if let Some(p) = app.ps.x.grbl_ports.first() {
            app.prefs.grbl.port = p.name.clone();
        }
    }
}

/// The connected machine: status, buttons, jog, console.
fn live(app: &mut App, ui: &mut Ui, link: &GrblLink) {
    let st = link.state();
    let maker3 = link.dialect == Dialect::Maker3;
    if !st.closed {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    // ---- where it is and what it's doing
    let colour = match st.status.split(':').next().unwrap_or("") {
        "Idle" => Color32::from_rgb(90, 180, 90),
        "Run" | "Jog" | "Home" => Color32::from_rgb(80, 150, 230),
        "Hold" | "Door" => Color32::from_rgb(230, 170, 50),
        "Alarm" => Color32::from_rgb(230, 80, 80),
        _ => ui.visuals().weak_text_color(),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("● {}", st.status)).color(colour).strong());
        if !st.version.is_empty() {
            ui.weak(&st.version);
        }
        if maker3 {
            ui.weak(match st.tool {
                Some(0) => "Tool: pen (T0)",
                Some(_) if st.steering => "Tool: rotary (T1, steering on)",
                Some(_) => "Tool: holder B (T1)",
                None => "Tool: choosing the pen…",
            });
        }
    });
    if st.answered {
        ui.monospace(format!("X {:>8.3}  Y {:>8.3}  Z {:>7.3}", st.wpos[0], st.wpos[1], st.wpos[2]))
            .on_hover_text(format!("Work position (from your origin). Machine position: X {:.3} Y {:.3} Z {:.3}", st.mpos[0], st.mpos[1], st.mpos[2]));
    }
    if let Some(e) = &st.error {
        ui.colored_label(Color32::LIGHT_RED, e);
    }
    if st.alarm() && !maker3 {
        ui.small("In alarm: Home the machine, or Unlock if it has no limit switches (make sure the position is right first).");
    }
    if maker3 && !st.answered && !st.closed {
        ui.small("The machine homes itself at power-on and doesn't answer until it's done (up to a minute).");
    }
    if !st.answered || st.closed {
        return;
    }
    let busy = st.busy || app.sending;
    if maker3 {
        maker3_controls(app, ui, link, &st);
        console(app, ui, link, &st, busy, true);
        return;
    }

    // ---- machine buttons
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(!busy, egui::Button::new("⌂ Home")).on_hover_text("Run the homing cycle ($H) - needs limit switches").clicked() {
            link.send("$H");
        }
        if ui.add_enabled(!busy, egui::Button::new("Unlock")).on_hover_text("Clear the alarm without homing ($X)").clicked() {
            link.send("$X");
        }
        if ui.button("⏸ Hold").on_hover_text("Pause the motion (feed hold)").clicked() {
            link.realtime(grbl::FEED_HOLD);
        }
        if ui.button("▶ Resume").on_hover_text("Carry on after a hold").clicked() {
            link.realtime(grbl::RESUME);
        }
        if ui.button("Reset").on_hover_text("Soft reset: stops at once and clears the controller's queue").clicked() {
            link.realtime(grbl::SOFT_RESET);
        }
    });

    // ---- jog
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            ui.label("Jog");
            for s in [0.1, 1.0, 10.0, 50.0] {
                ui.selectable_value(&mut app.ps.x.grbl_step, s, format!("{s} mm"));
            }
        });
        let step = app.ps.x.grbl_step;
        let feed = app.ps.x.grbl_jog_feed;
        let mut jog = None;
        egui::Grid::new("grbl_jog").spacing([4.0, 4.0]).show(ui, |ui| {
            let b = |ui: &mut Ui, t: &str| ui.add_sized([44.0, 28.0], egui::Button::new(t)).clicked();
            ui.label("");
            if b(ui, "Y+") {
                jog = Some([0.0, step, 0.0]);
            }
            ui.label("");
            ui.label("");
            if b(ui, "Z+") {
                jog = Some([0.0, 0.0, step]);
            }
            ui.end_row();
            if b(ui, "X-") {
                jog = Some([-step, 0.0, 0.0]);
            }
            if ui.add_sized([44.0, 28.0], egui::Button::new("■")).on_hover_text("Stop jogging").clicked() {
                link.realtime(grbl::JOG_CANCEL);
            }
            if b(ui, "X+") {
                jog = Some([step, 0.0, 0.0]);
            }
            ui.label("");
            ui.label("");
            ui.end_row();
            ui.label("");
            if b(ui, "Y-") {
                jog = Some([0.0, -step, 0.0]);
            }
            ui.label("");
            ui.label("");
            if b(ui, "Z-") {
                jog = Some([0.0, 0.0, -step]);
            }
            ui.end_row();
        });
        if let Some(d) = jog {
            link.jog(d, feed);
        }
        ui.horizontal(|ui| {
            ui.label("Jog speed");
            ui.add(DragValue::new(&mut app.ps.x.grbl_jog_feed).range(10.0..=20000.0).speed(10.0).suffix(" mm/min"));
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Set origin here").on_hover_text("Make where the tool is now X0 Y0 - the page's corner goes here").clicked() {
                link.send("G10 L20 P1 X0 Y0");
            }
            if app.prefs.grbl.head == Head::ZAxis && ui.button("Set Z0 here").on_hover_text("Make the tool's height now Z0 (touch the tip on the material first)").clicked() {
                link.send("G10 L20 P1 Z0");
            }
            if ui.button("Go to origin").clicked() {
                link.send(&grbl::tool_up(&app.prefs.grbl));
                link.send("G90 G0 X0 Y0");
            }
            if ui.button("Tool up").clicked() {
                link.send(&grbl::tool_up(&app.prefs.grbl));
            }
            if ui.button("Frame the job").on_hover_text("Move round the job's outer box with the tool up (laser off), to check where it will land").clicked() {
                let job = cutjob::generate(app.doc.page(), &app.plan);
                match grbl::frame(&job, app.doc.page().height, &app.prefs.grbl) {
                    Some(lines) => lines.iter().for_each(|l| link.send(l)),
                    None => app.status = "Nothing to cut - nothing to frame".into(),
                }
            }
        });
    });

    console(app, ui, link, &st, busy, false);
}

/// The conversation with the machine, and a line to type into.
fn console(app: &mut App, ui: &mut Ui, link: &GrblLink, st: &grbl::GrblState, busy: bool, maker3: bool) {
    egui::CollapsingHeader::new("Console").show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
            for l in &st.log {
                if l.starts_with("> ") {
                    ui.monospace(RichText::new(l).weak());
                } else {
                    ui.monospace(l);
                }
            }
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.ps.x.grbl_cmd).hint_text("G-code or $ command, e.g. $$").desired_width(200.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled(!busy, egui::Button::new("Send")).clicked() || enter && !busy) && !app.ps.x.grbl_cmd.trim().is_empty() {
                match if maker3 { grbl::maker3_console_ok(&app.ps.x.grbl_cmd) } else { Ok(()) } {
                    Ok(()) => link.send(&app.ps.x.grbl_cmd),
                    Err(e) => app.status = e,
                }
                app.ps.x.grbl_cmd.clear();
                r.request_focus();
            }
        });
        ui.small("$$ lists the machine's settings, $I its build info, $# its offsets.");
    });
}

/// The machine's own settings: how the tool goes down, feeds, origin.
fn settings(app: &mut App, ui: &mut Ui) {
    let o = &mut app.prefs.grbl;
    let before = o.clone();
    egui::CollapsingHeader::new("Machine settings").show(ui, |ui| {
        egui::Grid::new("grbl_set").num_columns(2).show(ui, |ui| {
            ui.label("Tool");
            egui::ComboBox::from_id_salt("grbl_head").selected_text(Head::ALL.iter().find(|(h, _)| *h == o.head).map(|(_, n)| *n).unwrap_or("")).show_ui(ui, |ui| {
                for (h, n) in Head::ALL {
                    ui.selectable_value(&mut o.head, h, n);
                }
            });
            ui.end_row();
            match o.head {
                Head::ZAxis => {
                    ui.label("Z up / down");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut o.z_up).speed(0.1).suffix(" mm"));
                        ui.add(DragValue::new(&mut o.z_down).speed(0.05).suffix(" mm"));
                    });
                    ui.end_row();
                    ui.label("Plunge speed");
                    ui.add(DragValue::new(&mut o.plunge_feed).range(10.0..=5000.0).speed(10.0).suffix(" mm/min"));
                    ui.end_row();
                }
                Head::Servo => {
                    ui.label("Servo up / down (S)");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut o.servo_up).range(0.0..=10000.0));
                        ui.add(DragValue::new(&mut o.servo_down).range(0.0..=10000.0));
                    });
                    ui.end_row();
                    ui.label("Wait for servo");
                    ui.add(DragValue::new(&mut o.servo_wait_ms).range(0..=2000).suffix(" ms"));
                    ui.end_row();
                }
                Head::Laser => {
                    ui.label("Full power (S)");
                    ui.add(DragValue::new(&mut o.s_max).range(1.0..=100000.0)).on_hover_text("The controller's $30 setting, usually 1000");
                    ui.end_row();
                }
            }
            ui.label("Feed");
            ui.vertical(|ui| {
                ui.checkbox(&mut o.feed_from_speed, "From each group's Speed");
                if o.feed_from_speed {
                    ui.add(DragValue::new(&mut o.feed_per_speed).range(10.0..=5000.0).speed(5.0).prefix("Speed × ").suffix(" mm/min"));
                } else {
                    ui.add(DragValue::new(&mut o.feed).range(10.0..=50000.0).speed(10.0).suffix(" mm/min"));
                }
            });
            ui.end_row();
            ui.label("Origin");
            ui.vertical(|ui| {
                ui.radio_value(&mut o.origin_bottom_left, true, "Front-left, Y away from you");
                ui.radio_value(&mut o.origin_bottom_left, false, "Back-left, Y towards you");
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut o.return_to_origin, "Go back to the origin when done");
            ui.end_row();
        });
        if o.head == Head::Laser {
            ui.small("Each group's Force is its laser power (Force ÷ the machine's top Force × full power). Turn on laser mode in GRBL ($32=1).");
        }
    });
    if *o != before {
        let _ = app.prefs.save();
    }
}

/// The Maker 3's buttons: home, pause, jog, heights.
fn maker3_controls(app: &mut App, ui: &mut Ui, link: &GrblLink, st: &grbl::GrblState) {
    let running = st.busy || app.sending;
    let idle = !running && st.status == "Idle";
    let send_all = |lines: Vec<String>| lines.iter().for_each(|l| link.send(l));
    if st.needs_home {
        ui.colored_label(Color32::from_rgb(230, 170, 50), "Home the machine before the next job: its position can't be trusted after a stop or alarm.");
    }
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(!running && (idle || st.alarm()), egui::Button::new("⌂ Home")).on_hover_text("Lifts both tools and finds the end of X ($H). The mat's Y position isn't homed.").clicked() {
            link.send("$H");
        }
        if ui.add_enabled(idle, egui::Button::new("Home tools")).on_hover_text("Lifts both holders only ($HZ). Do this after fitting a tool: fitting pushes the holder.").clicked() {
            link.send("$HZ");
        }
        if ui.button("⏸ Pause").on_hover_text("Slows to a stop part-way along the line. The tool stays where it is.").clicked() {
            link.realtime(grbl::FEED_HOLD);
        }
        if ui.button("▶ Resume").on_hover_text("Carries on from where it paused").clicked() {
            link.realtime(grbl::RESUME);
        }
        if ui.button("Reset").on_hover_text("Stops every motor at once and clears the queue (Ctrl-X). Also clears a stall alarm. Home afterwards.").clicked() {
            link.realtime(grbl::SOFT_RESET);
        }
    });
    ui.small("In a real emergency use the mains switch: these are commands over USB, and the Power button only works when the machine is idle.");
    ui.small("To load, push the mat against the rollers until the feed button flashes, then press it; press it again to eject. CutCraft can't do this or see whether a mat is in.");
    if let Some(fit) = &st.last_fit {
        ui.small(fit);
    }

    // what a scan of holder B found becomes the tool shown for it
    if st.detected != app.ps.x.grbl_detected_seen {
        match st.detected.as_deref() {
            // the two scoring wheels read the same: keep the one chosen
            Some(n) => match grbl::DriveTool::from_reported(n) {
                Some(grbl::DriveTool::Score01) if app.ps.x.grbl_b_tool == Some(grbl::DriveTool::Score02) => {}
                Some(d) => app.ps.x.grbl_b_tool = Some(d),
                None if n == "not found" => app.ps.x.grbl_b_tool = None,
                None => {}
            },
            None => {}
        }
        app.ps.x.grbl_detected_seen = st.detected.clone();
    }
    let o = app.prefs.maker3.clone();
    let bed = grbl::maker3_bed(app.doc.page().mat);
    let now = ui.input(|i| i.time);
    // the reported position lags a jog, and each jog is checked against it
    let settled = idle && now >= app.ps.x.grbl_jog_until;
    ui.add_enabled_ui(settled, |ui| {
        ui.horizontal(|ui| {
            ui.label("Active tool");
            for (t, name, tip) in [(0u8, "Pen (A)", "Holder A (T0)"), (1, "Holder B", "Holder B (T1): the fine-point blade or a drive-housing tool")] {
                if ui.selectable_label(st.tool == Some(t), name).on_hover_text(format!("{tip}. Lifts the current tool and changes to this one.")).clicked() && st.tool != Some(t) {
                    send_all(grbl::maker3_select_tool(t));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("In holder B");
            let shown = app.ps.x.grbl_b_tool.map(|d| d.label()).unwrap_or("Fine-point blade");
            egui::ComboBox::from_id_salt("m3_btool")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.ps.x.grbl_b_tool, None, "Fine-point blade");
                    for d in grbl::DriveTool::ALL {
                        ui.selectable_value(&mut app.ps.x.grbl_b_tool, Some(d), d.label());
                    }
                })
                .response
                .on_hover_text("Which tool is in holder B, for finding and saving its height. Detect tool fills this in for the drive-housing tools.");
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Load tool").on_hover_text("Lifts both holders, turns steering off and moves the carriage to the middle, so you can reach holder B").clicked() {
                start_routine(app, link, &grbl::MAKER3_LOAD_TOOL, false);
            }
            if ui.button("Detect tool").on_hover_text("Homes, then scans the tool in holder B and says which it is (30-40 s). For the drive-housing tools (with a gear on top): the fine-point blade can't be scanned.").clicked() {
                start_routine(app, link, &grbl::MAKER3_DETECT_TOOL, true);
            }
            match st.detected.as_deref() {
                Some("not found") => {
                    ui.weak("Last scan: no drive-housing tool");
                }
                Some(n) => {
                    ui.label(RichText::new(format!("Holder B: {}", grbl::reported_label(n))).strong());
                }
                None => {}
            }
        });
        ui.horizontal(|ui| {
            ui.label("Jog");
            for s in [1.0, 5.0, 10.0, 25.0] {
                ui.selectable_value(&mut app.ps.x.grbl_step, s, format!("{s} mm"));
            }
        });
        let step = app.ps.x.grbl_step;
        let mut jog = None;
        egui::Grid::new("m3_jog").spacing([4.0, 4.0]).show(ui, |ui| {
            let b = |ui: &mut Ui, t: &str| ui.add_sized([58.0, 28.0], egui::Button::new(t)).clicked();
            ui.label("");
            if b(ui, "Mat in") {
                jog = Some([0.0, step, 0.0]);
            }
            ui.label("");
            ui.label("");
            if b(ui, "Tool up") {
                jog = Some([0.0, 0.0, step]);
            }
            ui.end_row();
            if b(ui, "◀ Left") {
                jog = Some([step, 0.0, 0.0]);
            }
            ui.label("");
            if b(ui, "Right ▶") {
                jog = Some([-step, 0.0, 0.0]);
            }
            ui.label("");
            ui.label("");
            ui.end_row();
            ui.label("");
            if b(ui, "Mat out") {
                jog = Some([0.0, -step, 0.0]);
            }
            ui.label("");
            ui.label("");
            if b(ui, "Tool down") {
                jog = Some([0.0, 0.0, -step]);
            }
            ui.end_row();
        });
        if let Some(d) = jog {
            match grbl::maker3_jog(bed, st.tool, st.mpos, d) {
                Ok(lines) => {
                    send_all(lines);
                    app.ps.x.grbl_jog_until = now + 0.6;
                }
                Err(e) => app.status = e,
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Go to mat centre").on_hover_text("Tool up, then the active tool's tip over the middle of the mat's work area (the same spot for either tool)").clicked() {
                send_all(grbl::maker3_go_centre(&o, bed));
            }
            if ui.button("Make this the mat centre").on_hover_text("If Go to mat centre doesn't land on the middle of the mat, jog the tool's tip there and click: plain cuts shift to match").clicked() {
                let centre = grbl::Maker3Options { shift_x: 0.0, shift_y: 0.0, ..o.clone() }.to_machine(cutcraft_core::kurbo::Point::new(bed.0 / 2.0, bed.1 / 2.0), bed);
                (app.prefs.maker3.shift_x, app.prefs.maker3.shift_y) = (st.mpos[0] - centre.x, st.mpos[1] - centre.y);
                let _ = app.prefs.save();
                app.status = format!("Plain cuts now shift X {:+.2} Y {:+.2} mm", app.prefs.maker3.shift_x, app.prefs.maker3.shift_y);
            }
            if ui.button("Lift tool").clicked() {
                send_all(grbl::maker3_tool_up());
            }
            if ui.button("Frame the job").on_hover_text("Moves round the job's outer box with the tool up, to check where it will land").clicked() {
                let job = cutjob::generate(app.doc.page(), &app.plan);
                match grbl::maker3_frame(&job, bed, &o) {
                    Ok(lines) => send_all(lines),
                    Err(e) => app.status = e,
                }
            }
            let page = app.doc.page();
            if let Some(rect) = page.regmarks.sensor_box(page.width, page.height) {
                if ui.button("Find the printed sheet").on_hover_text("Scans the printed registration box and shows where the sheet is, without cutting (16 scans, about 3 minutes)").clicked() {
                    match grbl::sensor_box_crossings(rect) {
                        Ok(crossings) => {
                            app.cancel.store(false, Ordering::Relaxed);
                            *app.progress.lock().unwrap() = Default::default();
                            link.print_cut(rect, crossings, None, app.progress.clone(), app.cancel.clone());
                            app.sending = true;
                        }
                        Err(e) => app.status = e,
                    }
                }
            }
        });

        egui::CollapsingHeader::new("Find a tool height").show(ui, |ui| {
            ui.small("Heights change every time a tool is re-seated. Fit the tool, click Home tools, make it the Active tool above (and say what's in holder B), Go to mat centre, then lower until it touches.");
            let z = st.mpos[2];
            ui.add_enabled_ui(st.tool.is_some(), |ui| {
                ui.horizontal(|ui| {
                    let mut to = None;
                    if ui.button("Lower 1").clicked() {
                        to = Some(z - 1.0);
                    }
                    if ui.button("Lower 0.5").clicked() {
                        to = Some(z - 0.5);
                    }
                    if ui.button("Raise 0.5").clicked() {
                        to = Some((z + 0.5).min(0.0));
                    }
                    if let Some(to) = to {
                        // only over the mat: near the ends of travel a tool hangs off it
                        match if to < z { grbl::maker3_can_lower(bed, st.mpos) } else { Ok(()) } {
                            Ok(()) => {
                                send_all(grbl::maker3_height(to, st.tool.unwrap_or(0)));
                                app.ps.x.grbl_jog_until = now + 0.6;
                            }
                            Err(e) => app.status = e,
                        }
                    }
                    ui.monospace(format!("Z {z:.2}"));
                });
                let which = if st.tool == Some(0) { grbl::Maker3Tool::Pen } else { app.ps.x.grbl_b_tool.map(grbl::Maker3Tool::Drive).unwrap_or(grbl::Maker3Tool::FinePoint) };
                let name = which.label();
                if ui.button(format!("Save Z {z:.2} as the {name} height")).clicked() {
                    *app.prefs.maker3.setting_mut(which).0 = z;
                    let _ = app.prefs.save();
                    app.status = format!("{name} height set to Z {z:.2}");
                }
            });
            ui.small("Pen: where it first marks. Blade: just past contact, deeper until one pass cuts through on fresh material (going over an old line flatters it).");
        });
    });
}

/// The Maker 3's settings: heights, feeds, where the design sits.
fn maker3_settings(app: &mut App, ui: &mut Ui) {
    let bed = grbl::maker3_bed(app.doc.page().mat);
    let o = &mut app.prefs.maker3;
    let before = o.clone();
    egui::CollapsingHeader::new("Machine settings").show(ui, |ui| {
        egui::Grid::new("m3_set").num_columns(2).show(ui, |ui| {
            ui.label("");
            ui.weak("Height and speed");
            ui.end_row();
            use grbl::Maker3Tool;
            // the two scoring wheels share a setting
            let tools = [Maker3Tool::Pen, Maker3Tool::FinePoint].into_iter().chain(grbl::DriveTool::ALL.into_iter().filter(|d| *d != grbl::DriveTool::Score02).map(Maker3Tool::Drive));
            for tool in tools {
                let label = match tool {
                    Maker3Tool::Pen => "Pen".to_string(),
                    Maker3Tool::Drive(grbl::DriveTool::Score01) => "Scoring wheels (01 / 02)".to_string(),
                    t => {
                        let l = t.label();
                        l[..1].to_uppercase() + &l[1..]
                    }
                };
                ui.label(label);
                ui.horizontal(|ui| {
                    let zmin = grbl::maker3_z_min(tool.holder());
                    let (z, f) = o.setting_mut(tool);
                    ui.add(DragValue::new(z).range(zmin..=0.0).speed(0.05).prefix("Z ").suffix(" mm"));
                    ui.add(DragValue::new(f).range(grbl::MAKER3_FEED_MIN..=grbl::MAKER3_FEED_MAX).speed(10.0).suffix(" mm/min"));
                });
                ui.end_row();
            }
            let shift = |ui: &mut Ui, label: &str, tip: &str, x: &mut f64, y: &mut f64| {
                ui.label(label).on_hover_text(tip);
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(x).speed(0.05).fixed_decimals(2).prefix("X ").suffix(" mm"));
                    ui.add(DragValue::new(y).speed(0.05).fixed_decimals(2).prefix("Y ").suffix(" mm"));
                });
                ui.end_row();
            };
            shift(ui, "Shift plain cuts", "If plain cuts always land a little off on the mat", &mut o.shift_x, &mut o.shift_y);
            shift(ui, "Print & Cut correction", "If Print & Cut cuts always land off the print the same way (+X is towards the mat's left)", &mut o.pc_shift_x, &mut o.pc_shift_y);
            ui.label("Work area");
            ui.label(format!("{:.0} x {:.1} mm (the mat in Page setup)", bed.0, bed.1));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut o.rotate_180, "Turn the design half a turn");
            ui.end_row();
        });
        ui.small("The page's top-left corner is the top-left of the mat's work area (X301 Y0 on the machine), the same for both tools: the firmware allows for where each holder sits. The machine has no limits of its own, so CutCraft keeps every move on the work area. Needs cutcutgo bringup v78 or later.");
        ui.small("Test cut draws a letter F at the page centre: if it isn't upright and readable, this tells you the design needs turning.");
    });
    if *o != before {
        let _ = app.prefs.save();
    }
}

/// The tools a job needs, and their heights, for the check before it runs.
fn tool_checklist(job: &cutjob::CutJob, o: &grbl::Maker3Options) -> String {
    let mut s = String::new();
    for t in grbl::maker3_tools(job).unwrap_or_default() {
        let z = o.setting(t).0;
        s += &match t {
            grbl::Maker3Tool::Pen => format!("- Pen in holder A (height Z{z:.2})\n"),
            grbl::Maker3Tool::FinePoint => format!("- Fine-point blade in holder B (height Z{z:.2})\n"),
            grbl::Maker3Tool::Drive(d) => format!("- {} in holder B (height Z{z:.2}) - the machine scans it first, and stops if it's a different tool\n", d.label()),
        };
    }
    s
}

/// Start Load tool or Detect tool.
fn start_routine(app: &mut App, link: &GrblLink, lines: &[&str], detect: bool) {
    app.cancel.store(false, Ordering::Relaxed);
    *app.progress.lock().unwrap() = Default::default();
    link.routine(lines, detect, app.progress.clone(), app.cancel.clone());
    app.sending = true;
}

/// Say why a job can't run, in the status bar and the Send panel.
fn refuse(app: &mut App, e: String) {
    app.status = e.clone();
    *app.progress.lock().unwrap() = cutcraft_core::device::Progress { done: true, message: format!("Failed: {e}"), error: Some(e), ..Default::default() };
}

/// Check a whole Maker 3 job before any of it runs, and mark which holder
/// each part uses so it only stops where a tool has to be swapped by hand.
pub fn maker3_job(app: &mut App, job: cutjob::CutJob) -> Option<cutjob::CutJob> {
    let bed = grbl::maker3_bed(app.doc.page().mat);
    match grbl::encode_maker3(&job, bed, &app.prefs.maker3).and_then(|_| grbl::maker3_holders(job)) {
        Ok(job) => Some(job),
        Err(e) => {
            refuse(app, e);
            None
        }
    }
}

/// Print & Cut on the Maker 3: find the printed box, then cut through the fit.
fn print_cut_maker3(app: &mut App, job: &cutjob::CutJob) {
    let page = app.doc.page();
    let Some(rect) = page.regmarks.sensor_box(page.width, page.height) else {
        return refuse(app, "The Maker 3 finds a printed box round the artwork: turn on Page ▸ Registration marks (it uses the box style for this machine).".into());
    };
    let crossings = match grbl::sensor_box_crossings(rect) {
        Ok(c) => c,
        Err(e) => return refuse(app, e),
    };
    let (bed, o, job) = (grbl::maker3_bed(page.mat), app.prefs.maker3.clone(), job.clone());
    // the cut must fit where the sheet should be, before 3 minutes of scanning
    let nominal = |p: cutcraft_core::kurbo::Point| cutcraft_core::kurbo::Point::new(grbl::MAKER3_WORK_X - p.x + o.pc_shift_x, p.y + o.pc_shift_y);
    if let Err(e) = grbl::encode_maker3_mapped(&job, bed, &o, &nominal) {
        return refuse(app, e);
    }
    let mut ask = String::from("Ready for Print & Cut on the Cricut Maker 3?\n\n- Printed at 100% / actual size, on white matt paper\n- The sheet in the mat's top-left corner, and the mat loaded with the feed button\n");
    ask += &tool_checklist(&job, &o);
    ask += "- Hands clear of the carriage\n\nCutCraft scans the printed box first (16 scans, about 3 minutes), and only cuts if the sheet is found.";
    if !crate::app::confirm(&ask) {
        return;
    }
    let Some(link) = &app.ps.x.grbl else { return };
    let cut: grbl::CutMaker = Box::new(move |fit: &grbl::Fit| {
        let map = |p| {
            let m = fit.apply(p);
            cutcraft_core::kurbo::Point::new(m.x + o.pc_shift_x, m.y + o.pc_shift_y)
        };
        grbl::encode_maker3_mapped(&job, bed, &o, &map)
    });
    app.cancel.store(false, Ordering::Relaxed);
    *app.progress.lock().unwrap() = Default::default();
    link.print_cut(rect, crossings, Some(cut), app.progress.clone(), app.cancel.clone());
    app.sending = true;
    app.status = "Finding the printed sheet…".into();
}

/// Run a job on the Maker 3: check it, make sure the operator is ready, send.
fn send_maker3(app: &mut App, job: &cutjob::CutJob) {
    let bed = grbl::maker3_bed(app.doc.page().mat);
    let lines = match grbl::encode_maker3(job, bed, &app.prefs.maker3) {
        Ok(l) => l,
        Err(e) => return refuse(app, e),
    };
    let mut ask = String::from("Ready to run on the Cricut Maker 3?\n\n- Mat loaded with the machine's feed button\n");
    ask += &tool_checklist(job, &app.prefs.maker3);
    ask += "- Hands clear of the carriage";
    if !crate::app::confirm(&ask) {
        return;
    }
    let Some(link) = &app.ps.x.grbl else { return };
    app.cancel.store(false, Ordering::Relaxed);
    *app.progress.lock().unwrap() = Default::default();
    link.start_job(lines, app.progress.clone(), app.cancel.clone());
    app.sending = true;
    app.status = "Sending to the Maker 3…".into();
}

/// Send a cut job to the connected GRBL machine (with `print_cut`, finding the
/// printed sheet first, on the Maker 3).
pub fn send(app: &mut App, job: &cutjob::CutJob, print_cut: bool) {
    let maker3 = grbl::is_maker3(&app.doc.machine);
    let wanted = if maker3 { Dialect::Maker3 } else { Dialect::Stock };
    let Some(link) = ready(app).filter(|l| l.dialect == wanted) else {
        app.status = format!("Connect to the machine first (Send panel ▸ {} connection)", if maker3 { "Cricut Maker 3" } else { "GRBL" });
        return;
    };
    if maker3 {
        if print_cut {
            print_cut_maker3(app, job);
        } else {
            send_maker3(app, job);
        }
        return;
    }
    let m = machines::by_id(&app.doc.machine);
    let lines = grbl::encode(job, app.doc.page().height, m.max_force, &app.prefs.grbl);
    app.cancel.store(false, Ordering::Relaxed);
    *app.progress.lock().unwrap() = Default::default();
    link.start_job(lines, app.progress.clone(), app.cancel.clone());
    app.sending = true;
    app.status = "Sending G-code to GRBL…".into();
}
