//! Sending an `.fcm` file straight to a Brother ScanNCut over USB, as the
//! machine's "receive data" screen expects it (measured from USB captures of
//! a real transfer, 2026-10-26):
//!
//! 1. a vendor control read: `bmRequestType 0xC1`, `bRequest 0xA0`, one byte
//!    back (the machine's state);
//! 2. on bulk OUT, each as its own transfer:
//!    `@BCCL`, ` `, `EXECUTE`, ` `, `FUNCTION`, `=`, `FCMDOWNLOAD`, ` `,
//!    `SIZE`, `=`, `<n>`, ` `, `DATA`, `=`, the `n` file bytes, `\r`;
//! 3. the machine answers on bulk IN, one word per transfer: `@BCCL ACK\r`
//!    once it has the file, then
//!    `@BCCL COMPLETE FUNCTION=FCMDOWNLOAD REASON=<why>\r`, where `<why>` is
//!    `SUCCESS`, or `NOTACCEPT` when it isn't on a screen that takes files
//!    (the home screen does);
//! 4. `@BCCL ACK\r` (same split) to acknowledge that.
//!
//! The state byte read in step 1 was 1 both when the machine took the file
//! and when it refused it. Each send is noted in `brother_usb.log` in the
//! settings folder.

use crate::device::{Progress, UsbDevice};
use anyhow::{Result, anyhow, bail};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Brother Industries' USB vendor id.
pub const VID: u16 = 0x04F9;

/// Product ids seen on ScanNCut machines.
pub const KNOWN_PIDS: &[u16] = &[0x210F];

/// The bulk OUT transfers that hand `fcm` to the machine.
pub fn download_command(fcm: &[u8]) -> Vec<Vec<u8>> {
    let size = fcm.len().to_string();
    let mut parts: Vec<Vec<u8>> = ["@BCCL", " ", "EXECUTE", " ", "FUNCTION", "=", "FCMDOWNLOAD", " ", "SIZE", "=", &size, " ", "DATA", "="]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    parts.push(fcm.to_vec());
    parts.push(b"\r".to_vec());
    parts
}

/// The bulk OUT transfers acknowledging the machine's answer.
pub fn ack_command() -> Vec<Vec<u8>> {
    ["@BCCL", " ", "ACK", "\r"].iter().map(|s| s.as_bytes().to_vec()).collect()
}

/// Brother devices plugged in now, as `brother:<vid>:<pid>` paths.
pub fn list() -> Vec<UsbDevice> {
    let Ok(devs) = nusb::list_devices() else { return vec![] };
    devs.filter(|d| d.vendor_id() == VID)
        .map(|d| UsbDevice {
            path: format!("brother:{:04x}:{:04x}", d.vendor_id(), d.product_id()),
            description: d.product_string().unwrap_or("ScanNCut Machine").to_string(),
            winusb: false,
        })
        .collect()
}

struct Link {
    iface: nusb::Interface,
    inum: u8,
    ep_out: u8,
    /// Reads kept waiting on the machine's bulk IN the whole time, as
    /// CanvasWorkspace does: the machine may hold off taking more data until
    /// its answer has been collected.
    replies: nusb::transfer::Queue<nusb::transfer::RequestBuffer>,
    in_size: usize,
    rt: tokio::runtime::Runtime,
}

/// Reads kept queued on bulk IN.
const QUEUED_READS: usize = 2;

impl Link {
    /// Opens the first Brother machine (a known ScanNCut first) and claims its
    /// bulk interface.
    fn open() -> Result<(Link, String)> {
        use nusb::transfer::{Direction, EndpointType};
        let mut devs: Vec<_> = nusb::list_devices()?.filter(|d| d.vendor_id() == VID).collect();
        devs.sort_by_key(|d| !KNOWN_PIDS.contains(&d.product_id()));
        let info = devs.into_iter().next().ok_or_else(|| anyhow!("No ScanNCut found - check the USB cable and that the machine is on"))?;
        let label = format!("{} ({:04x}:{:04x})", info.product_string().unwrap_or("ScanNCut"), info.vendor_id(), info.product_id());
        let dev = info.open().map_err(|e| anyhow!("Couldn't open the ScanNCut ({e}) - is CanvasWorkspace or another program using it?"))?;
        let cfg = dev.active_configuration().map_err(|e| anyhow!("no active USB configuration: {e}"))?;
        let mut found = None;
        'outer: for grp in cfg.interfaces() {
            for alt in grp.alt_settings() {
                let (mut out, mut inp) = (None, None);
                for ep in alt.endpoints().filter(|e| e.transfer_type() == EndpointType::Bulk) {
                    match ep.direction() {
                        Direction::Out if out.is_none() => out = Some(ep.address()),
                        Direction::In if inp.is_none() => inp = Some((ep.address(), ep.max_packet_size())),
                        _ => {}
                    }
                }
                if let (Some(o), Some((i, n))) = (out, inp) {
                    found = Some((grp.interface_number(), o, i, n));
                    break 'outer;
                }
            }
        }
        let (inum, ep_out, ep_in, n) = found.ok_or_else(|| anyhow!("the ScanNCut has no bulk USB interface"))?;
        let iface = dev.claim_interface(inum).map_err(|e| anyhow!("Couldn't claim the ScanNCut ({e}) - is CanvasWorkspace or another program using it?"))?;
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build()?;
        // big enough for a whole answer, and a whole number of packets
        let in_size = n.max(64) * 8;
        let mut replies = iface.bulk_in_queue(ep_in);
        for _ in 0..QUEUED_READS {
            replies.submit(nusb::transfer::RequestBuffer::new(in_size));
        }
        Ok((Link { iface, inum, ep_out, replies, in_size, rt }, label))
    }

    fn state(&self) -> Result<Option<u8>> {
        use nusb::transfer::{ControlIn, ControlType, Recipient};
        let fut = self.iface.control_in(ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Interface, request: 0xA0, value: 0, index: self.inum as u16, length: 1 });
        match self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), fut).await }) {
            Ok(c) => {
                c.status.map_err(|e| anyhow!("the ScanNCut didn't answer its status request ({e:?})"))?;
                Ok(c.data.first().copied())
            }
            Err(_) => bail!("the ScanNCut didn't answer its status request"),
        }
    }

    fn write(&self, data: Vec<u8>) -> Result<()> {
        let fut = self.iface.bulk_out(self.ep_out, data);
        match self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), fut).await }) {
            Ok(c) => c.status.map_err(|e| anyhow!("USB write failed: {e:?}")),
            Err(_) => bail!("the ScanNCut isn't reading - it may be asleep or busy. Wake it, go to its home screen, then turn it off and on if it still won't take the file"),
        }
    }

    /// Whatever the machine sent within `timeout` (empty if nothing). The read
    /// stays queued if nothing came, and a finished one is replaced.
    fn read(&mut self, timeout: Duration) -> Vec<u8> {
        let Link { replies, rt, .. } = self;
        match rt.block_on(async { tokio::time::timeout(timeout, replies.next_complete()).await }) {
            Ok(c) => {
                self.replies.submit(nusb::transfer::RequestBuffer::new(self.in_size));
                if c.status.is_ok() { c.data } else { vec![] }
            }
            Err(_) => vec![],
        }
    }
}

/// One line from the machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// It has the file.
    Ack,
    /// It's done with the file: `reason` is `SUCCESS` when it took it.
    Complete { reason: String },
    Other(String),
}

/// Reads one `\r`-ended line from the machine.
pub fn parse_reply(line: &[u8]) -> Reply {
    let text = String::from_utf8_lossy(line);
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        ["@BCCL", "ACK", ..] => Reply::Ack,
        ["@BCCL", "COMPLETE", rest @ ..] => {
            let reason = rest.iter().find_map(|w| w.strip_prefix("REASON=")).unwrap_or("").to_string();
            Reply::Complete { reason }
        }
        _ => Reply::Other(words.join(" ")),
    }
}

/// What a `REASON=` means for the user; `None` for success.
pub fn refusal(reason: &str) -> Option<String> {
    match reason {
        "SUCCESS" => None,
        "NOTACCEPT" => Some("The ScanNCut didn't accept the file - go to the home screen on the machine and send again.".into()),
        "" => Some("The ScanNCut didn't take the file.".into()),
        r => Some(format!("The ScanNCut didn't take the file ({r}).")),
    }
}

/// Sends the file; returns the machine's name and its `REASON=`.
fn transfer(fcm: &[u8], progress: &Arc<Mutex<Progress>>, cancel: &Arc<AtomicBool>) -> Result<(String, String)> {
    let set = |f: &dyn Fn(&mut Progress)| {
        if let Ok(mut p) = progress.lock() {
            f(&mut p)
        }
    };
    let t0 = Instant::now();
    let step = |what: &str| log(&format!("  {:>6} ms  {what}", t0.elapsed().as_millis()));
    let (mut link, machine) = Link::open()?;
    step(&format!("opened {machine}"));
    set(&|p| p.message = format!("Connected to {machine}"));
    let state = link.state()?;
    step(&format!("state {state:?}"));
    // drop anything left over from an earlier transfer
    loop {
        let old = link.read(Duration::from_millis(50));
        if old.is_empty() {
            break;
        }
        step(&format!("dropped old reply {:?}", String::from_utf8_lossy(&old)));
    }
    set(&|p| p.message = "Sending…".into());
    for part in download_command(fcm) {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if part.len() <= 16 {
            if let Err(e) = link.write(part.clone()) {
                step(&format!("writing {:?} failed: {e}", String::from_utf8_lossy(&part)));
                return Err(e);
            }
            set(&|p| p.sent += part.len());
            continue;
        }
        // the file, a packet at a time: the same packets on the wire as one
        // big transfer, but it shows how far the machine got if it stops
        let mut taken = 0;
        for chunk in part.chunks(512) {
            if let Err(e) = link.write(chunk.to_vec()) {
                step(&format!("the machine stopped taking the file after {taken} of {} bytes: {e}", part.len()));
                return Err(e);
            }
            taken += chunk.len();
            set(&|p| p.sent += chunk.len());
        }
    }
    step("all sent");
    set(&|p| p.message = "Waiting for the ScanNCut…".into());
    let start = Instant::now();
    let mut buf = vec![];
    let reason = 'wait: loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if start.elapsed() > Duration::from_secs(30) {
            bail!("the ScanNCut didn't answer - check it's on the home screen and send again");
        }
        buf.extend(link.read(Duration::from_millis(200)));
        while let Some(end) = buf.iter().position(|&b| b == b'\r') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            step(&format!("reply {:?}", String::from_utf8_lossy(&line).trim_end()));
            match parse_reply(&line) {
                Reply::Ack => set(&|p| p.message = "The ScanNCut has the file…".into()),
                Reply::Complete { reason } => break 'wait reason,
                Reply::Other(s) => log(&format!("unexpected reply: {s:?}")),
            }
        }
    };
    for part in ack_command() {
        link.write(part)?;
    }
    Ok((machine, reason))
}

fn log(line: &str) {
    use std::io::Write;
    let path = crate::prefs::config_dir().join("brother_usb.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{when} {line}");
    }
}

/// Sends `fcm` to the machine on a background thread, reporting through
/// `progress` like the cutter sends do.
pub fn spawn_send(fcm: Vec<u8>, progress: Arc<Mutex<Progress>>, cancel: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Ok(mut p) = progress.lock() {
            p.total = download_command(&fcm).iter().map(Vec::len).sum();
            p.message = "Connecting…".into();
        }
        let result = transfer(&fcm, &progress, &cancel);
        match &result {
            Ok((machine, reason)) => log(&format!("sent {} bytes to {machine}: REASON={reason}", fcm.len())),
            Err(e) => log(&format!("send of {} bytes failed: {e}", fcm.len())),
        }
        if let Ok(mut p) = progress.lock() {
            p.done = true;
            match result.map(|(_, reason)| refusal(&reason)) {
                Ok(None) => p.message = "Sent - the design is on the ScanNCut's screen, ready to cut".into(),
                Ok(Some(e)) => {
                    p.message = e.clone();
                    p.error = Some(e);
                }
                Err(e) => {
                    p.message = format!("Failed: {e}");
                    p.error = Some(e.to_string());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends the file named by `FCM_SEND` to a connected ScanNCut and prints
    /// the result (hardware check; the log has the steps).
    #[test]
    #[ignore = "needs the machine"]
    fn send_file_to_machine() {
        let path = std::env::var("FCM_SEND").expect("set FCM_SEND to an .fcm file");
        let progress = Arc::new(Mutex::new(Progress::default()));
        spawn_send(std::fs::read(path).unwrap(), progress.clone(), Arc::new(AtomicBool::new(false))).join().unwrap();
        let p = progress.lock().unwrap();
        println!("result: {} / error: {:?}", p.message, p.error);
    }

    #[test]
    fn download_matches_the_capture() {
        let parts = download_command(b"#FCM0100xyz");
        let words: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        assert_eq!(words, [&b"@BCCL"[..], b" ", b"EXECUTE", b" ", b"FUNCTION", b"=", b"FCMDOWNLOAD", b" ", b"SIZE", b"=", b"11", b" ", b"DATA", b"=", b"#FCM0100xyz", b"\r"]);
        assert_eq!(ack_command().concat(), b"@BCCL ACK\r");
    }

    #[test]
    fn replies_from_the_capture() {
        assert_eq!(parse_reply(b"@BCCL ACK\r"), Reply::Ack);
        assert_eq!(parse_reply(b"@BCCL COMPLETE FUNCTION=FCMDOWNLOAD REASON=SUCCESS\r"), Reply::Complete { reason: "SUCCESS".into() });
        assert_eq!(parse_reply(b"@BCCL COMPLETE FUNCTION=FCMDOWNLOAD REASON=NOTACCEPT\r"), Reply::Complete { reason: "NOTACCEPT".into() });
        assert_eq!(refusal("SUCCESS"), None);
        assert!(refusal("NOTACCEPT").unwrap().contains("home screen"));
    }
}
