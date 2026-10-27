//! Cutter connections and job streaming.
//!
//! Transports: write-to-file, TCP (network cutters / print servers), Bluetooth,
//! Windows WinUSB / printer-class USB, and the cross-platform `nusb` bulk USB
//! transport (Linux / macOS / Windows).

use crate::gpgl;
use anyhow::{Result, anyhow, bail};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub trait Transport: Send {
    fn name(&self) -> String;
    fn write_all(&mut self, data: &[u8]) -> Result<()>;
    /// Read whatever is available, waiting up to `timeout`.
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>>;
    /// Whether the far end is a real device that answers status queries.
    fn interactive(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------ file

pub struct FileTransport {
    path: std::path::PathBuf,
    file: std::fs::File,
}

impl FileTransport {
    pub fn create(path: impl Into<std::path::PathBuf>) -> Result<Self> {
        let path = path.into();
        let file = std::fs::File::create(&path)?;
        Ok(FileTransport { path, file })
    }
}

impl Transport for FileTransport {
    fn name(&self) -> String {
        format!("File {}", self.path.display())
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.file.write_all(data)?;
        Ok(())
    }
    fn read(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        Ok(vec![])
    }
    fn interactive(&self) -> bool {
        false
    }
}

// ------------------------------------------------------------------ tcp

pub struct TcpTransport {
    addr: String,
    sock: std::net::TcpStream,
}

impl TcpTransport {
    pub fn connect(addr: &str) -> Result<Self> {
        let sa = std::net::ToSocketAddrs::to_socket_addrs(addr)?.next().ok_or_else(|| anyhow!("bad address"))?;
        let sock = std::net::TcpStream::connect_timeout(&sa, Duration::from_secs(5))?;
        sock.set_nodelay(true).ok();
        Ok(TcpTransport { addr: addr.into(), sock })
    }
}

impl Transport for TcpTransport {
    fn name(&self) -> String {
        format!("TCP {}", self.addr)
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.sock.write_all(data)?;
        Ok(())
    }
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.sock.set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        let mut buf = [0u8; 256];
        match self.sock.read(&mut buf) {
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }
}

// ------------------------------------------------------------------ USB (Windows)

#[derive(Clone, Debug, PartialEq)]
pub struct UsbDevice {
    pub path: String,
    pub description: String,
    /// `true` if this interface should be driven through WinUSB bulk pipes;
    /// `false` for the USB printer-class (write-file) interface.
    pub winusb: bool,
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows_sys::Win32::Devices::Usb::*;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::core::GUID;

    const GUID_DEVINTERFACE_USBPRINT: GUID = GUID::from_u128(0x28d78fad_5a12_11d1_ae5b_0000f803a8c2);
    // Registered by usbhub for every USB device PDO; a path under this GUID can
    // be opened with CreateFile and handed to WinUsb_Initialize when WinUSB.sys
    // is the device's loaded function driver.
    const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID::from_u128(0xa5dcbf10_6530_11d2_901f_00c04fb951ed);

    /// All device-interface paths registered under `guid`.
    fn enumerate(guid: &GUID) -> Vec<String> {
        let mut paths = vec![];
        unsafe {
            let h = SetupDiGetClassDevsW(guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if h as isize == INVALID_HANDLE_VALUE as isize {
                return paths;
            }
            let mut i = 0;
            loop {
                let mut data: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
                if SetupDiEnumDeviceInterfaces(h, std::ptr::null(), guid, i, &mut data) == 0 {
                    break;
                }
                i += 1;
                let mut need = 0u32;
                SetupDiGetDeviceInterfaceDetailW(h, &data, std::ptr::null_mut(), 0, &mut need, std::ptr::null_mut());
                if need == 0 {
                    continue;
                }
                let mut buf = vec![0u64; (need as usize).div_ceil(8)];
                let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(h, &data, detail, need, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                    continue;
                }
                let p = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                paths.push(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
            }
            SetupDiDestroyDeviceInfoList(h);
        }
        paths
    }

    pub fn list() -> Vec<UsbDevice> {
        let mut out: Vec<UsbDevice> = enumerate(&GUID_DEVINTERFACE_USBPRINT)
            .into_iter()
            .map(|path| {
                let vendor = match crate::machines::match_usb_path(&path) {
                    Some(m) => format!("Silhouette {} (printer-class)", m.name),
                    None if path.to_lowercase().contains("vid_0b4d") => "Graphtec/Silhouette (printer-class)".into(),
                    None => "USB printer-class device".into(),
                };
                UsbDevice { description: vendor, path, winusb: false }
            })
            .collect();
        out.sort_by_key(|d| !d.path.to_lowercase().contains("vid_0b4d"));
        out
    }

    pub fn list_winusb() -> Vec<UsbDevice> {
        let mut out: Vec<UsbDevice> = enumerate(&GUID_DEVINTERFACE_USB_DEVICE)
            .into_iter()
            .map(|path| {
                let vendor = match crate::machines::match_usb_path(&path) {
                    Some(m) => format!("Silhouette {} (WinUSB)", m.name),
                    None if path.to_lowercase().contains("vid_0b4d") => "Graphtec/Silhouette (WinUSB)".into(),
                    None => "USB device (WinUSB)".into(),
                };
                UsbDevice { description: vendor, path, winusb: true }
            })
            .collect();
        out.sort_by_key(|d| !d.path.to_lowercase().contains("vid_0b4d"));
        out
    }

    // ---- printer-class (write-file) interface ----

    pub struct Handle(pub HANDLE);
    unsafe impl Send for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn open(path: &str) -> Result<Handle> {
        let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            bail!("cannot open USB device (is another program using it?)");
        }
        Ok(Handle(h))
    }

    pub fn write(h: &Handle, data: &[u8]) -> Result<()> {
        let mut off = 0;
        while off < data.len() {
            let mut n = 0u32;
            let ok = unsafe {
                WriteFile(h.0, data[off..].as_ptr(), (data.len() - off).min(4096) as u32, &mut n, std::ptr::null_mut())
            };
            if ok == 0 {
                bail!("USB write failed (error {})", unsafe { GetLastError() });
            }
            off += n as usize;
        }
        Ok(())
    }

    pub fn read(h: &Handle) -> Result<Vec<u8>> {
        let mut buf = [0u8; 256];
        let mut n = 0u32;
        let ok = unsafe { ReadFile(h.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Ok(vec![]);
        }
        Ok(buf[..n as usize].to_vec())
    }

    // ---- WinUSB bulk interface ----

    /// Open file handle + WinUSB interface handle + discovered bulk pipes.
    pub struct WinUsb {
        file: HANDLE,
        h: WINUSB_INTERFACE_HANDLE,
        out_pipe: u8,
        in_pipe: u8,
    }
    unsafe impl Send for WinUsb {}

    impl Drop for WinUsb {
        fn drop(&mut self) {
            unsafe {
                if !self.h.is_null() {
                    WinUsb_Free(self.h);
                }
                if self.file != INVALID_HANDLE_VALUE {
                    CloseHandle(self.file);
                }
            }
        }
    }

    pub fn winusb_open(path: &str) -> Result<WinUsb> {
        let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // WinUSB requires the handle to be opened for overlapped I/O.
        let file = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if file == INVALID_HANDLE_VALUE {
            bail!("cannot open USB device (is another program using it?)");
        }
        let mut dev = WinUsb { file, h: std::ptr::null_mut(), out_pipe: 0, in_pipe: 0 };
        if unsafe { WinUsb_Initialize(file, &mut dev.h) } == 0 {
            bail!("WinUsb_Initialize failed (error {}). This device is not bound to the WinUSB driver - use the printer-class interface, or install a WinUSB driver (e.g. via Zadig) for it.", unsafe { GetLastError() });
        }
        // Discover the first bulk OUT and bulk IN pipe on the default setting.
        let mut iface: USB_INTERFACE_DESCRIPTOR = unsafe { std::mem::zeroed() };
        if unsafe { WinUsb_QueryInterfaceSettings(dev.h, 0, &mut iface) } == 0 {
            bail!("WinUsb_QueryInterfaceSettings failed (error {})", unsafe { GetLastError() });
        }
        let (mut found_out, mut found_in) = (false, false);
        for i in 0..iface.bNumEndpoints {
            let mut pipe: WINUSB_PIPE_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { WinUsb_QueryPipe(dev.h, 0, i, &mut pipe) } == 0 {
                continue;
            }
            if pipe.PipeType != UsbdPipeTypeBulk {
                continue;
            }
            // High bit of the pipe id marks the direction: 1 = IN, 0 = OUT.
            if pipe.PipeId & 0x80 != 0 {
                if !found_in {
                    dev.in_pipe = pipe.PipeId;
                    found_in = true;
                }
            } else if !found_out {
                dev.out_pipe = pipe.PipeId;
                found_out = true;
            }
        }
        if !found_out {
            bail!("no bulk OUT pipe found on this WinUSB interface");
        }
        if found_in {
            // Bound read timeout so status reads don't block forever.
            let ms: u32 = 2000;
            unsafe {
                WinUsb_SetPipePolicy(
                    dev.h,
                    dev.in_pipe,
                    PIPE_TRANSFER_TIMEOUT,
                    std::mem::size_of::<u32>() as u32,
                    &ms as *const u32 as *const core::ffi::c_void,
                );
            }
        }
        Ok(dev)
    }

    pub fn winusb_write(dev: &WinUsb, data: &[u8]) -> Result<()> {
        let mut off = 0;
        while off < data.len() {
            let mut n = 0u32;
            let chunk = &data[off..];
            let ok = unsafe {
                WinUsb_WritePipe(dev.h, dev.out_pipe, chunk.as_ptr(), chunk.len() as u32, &mut n, std::ptr::null())
            };
            if ok == 0 {
                bail!("WinUSB write failed (error {})", unsafe { GetLastError() });
            }
            if n == 0 {
                bail!("WinUSB write stalled");
            }
            off += n as usize;
        }
        Ok(())
    }

    pub fn winusb_read(dev: &WinUsb) -> Result<Vec<u8>> {
        if dev.in_pipe == 0 {
            return Ok(vec![]);
        }
        let mut buf = [0u8; 256];
        let mut n = 0u32;
        let ok = unsafe {
            WinUsb_ReadPipe(dev.h, dev.in_pipe, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null())
        };
        if ok == 0 {
            return Ok(vec![]);
        }
        Ok(buf[..n as usize].to_vec())
    }
}

pub fn list_usb() -> Vec<UsbDevice> {
    #[cfg(windows)]
    {
        win::list()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

/// WinUSB-capable USB devices (bulk-pipe interface). Windows only.
pub fn list_winusb() -> Vec<UsbDevice> {
    #[cfg(windows)]
    {
        win::list_winusb()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

pub struct UsbTransport {
    dev: UsbDevice,
    #[cfg(windows)]
    h: win::Handle,
}

impl UsbTransport {
    pub fn open(dev: &UsbDevice) -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(UsbTransport { dev: dev.clone(), h: win::open(&dev.path)? })
        }
        #[cfg(not(windows))]
        {
            let _ = dev;
            bail!("USB transport is only implemented on Windows in this build")
        }
    }
}

impl Transport for UsbTransport {
    fn name(&self) -> String {
        format!("USB {}", self.dev.description)
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        #[cfg(windows)]
        {
            win::write(&self.h, data)
        }
        #[cfg(not(windows))]
        {
            let _ = data;
            bail!("unsupported")
        }
    }
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let start = Instant::now();
        loop {
            #[cfg(windows)]
            let r = win::read(&self.h)?;
            #[cfg(not(windows))]
            let r: Vec<u8> = vec![];
            if !r.is_empty() || start.elapsed() >= timeout {
                return Ok(r);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// ------------------------------------------------------------------ WinUSB (Windows)

pub struct WinUsbTransport {
    dev: UsbDevice,
    #[cfg(windows)]
    h: win::WinUsb,
}

impl WinUsbTransport {
    pub fn open(dev: &UsbDevice) -> Result<Self> {
        #[cfg(windows)]
        {
            Ok(WinUsbTransport { dev: dev.clone(), h: win::winusb_open(&dev.path)? })
        }
        #[cfg(not(windows))]
        {
            let _ = dev;
            bail!("WinUSB transport is only implemented on Windows in this build")
        }
    }
}

impl Transport for WinUsbTransport {
    fn name(&self) -> String {
        format!("WinUSB {}", self.dev.description)
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        #[cfg(windows)]
        {
            win::winusb_write(&self.h, data)
        }
        #[cfg(not(windows))]
        {
            let _ = data;
            bail!("unsupported")
        }
    }
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let start = Instant::now();
        loop {
            #[cfg(windows)]
            let r = win::winusb_read(&self.h)?;
            #[cfg(not(windows))]
            let r: Vec<u8> = vec![];
            if !r.is_empty() || start.elapsed() >= timeout {
                return Ok(r);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// ------------------------------------------------------------------ nusb (cross-platform bulk USB)

/// Enumerate likely cutters via `nusb` (works on Linux, macOS and Windows).
/// Returns entries whose `path` is `nusb:<vid>:<pid>` for [`NusbTransport::open`].
pub fn list_nusb() -> Vec<UsbDevice> {
    let mut out = vec![];
    let Ok(devs) = nusb::list_devices() else { return out };
    for d in devs {
        let (vid, pid) = (d.vendor_id(), d.product_id());
        let model = crate::machines::match_usb_path(&format!("vid_{vid:04x}&pid_{pid:04x}"));
        // Show Graphtec/Silhouette devices and anything matching a known profile.
        if vid != crate::machines::GRAPHTEC_VID && model.is_none() {
            continue;
        }
        let name = match model {
            Some(m) => format!("Silhouette {} (USB)", m.name),
            None => d.product_string().unwrap_or("Graphtec/Silhouette (USB)").to_string(),
        };
        out.push(UsbDevice { path: format!("nusb:{vid:04x}:{pid:04x}"), description: name, winusb: false });
    }
    out
}

/// A bulk-USB connection through the cross-platform `nusb` crate.
pub struct NusbTransport {
    label: String,
    iface: nusb::Interface,
    ep_out: u8,
    ep_in: u8, // 0 = no bulk IN endpoint
    rt: tokio::runtime::Runtime,
}

impl NusbTransport {
    /// Open the first device matching `vid:pid` and claim its bulk interface.
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        use nusb::transfer::{Direction, EndpointType};
        let info = nusb::list_devices()?
            .find(|d| d.vendor_id() == vid && d.product_id() == pid)
            .ok_or_else(|| anyhow!("no USB device {vid:04x}:{pid:04x} found (check cable, power and permissions)"))?;
        let label = format!("{} {vid:04x}:{pid:04x}", info.product_string().unwrap_or("USB cutter"));
        let dev = info.open()?;
        let cfg = dev.active_configuration().map_err(|e| anyhow!("no active USB configuration: {e}"))?;

        // Find the first interface alt-setting that has a bulk OUT endpoint.
        let mut found: Option<(u8, u8, u8)> = None;
        'outer: for grp in cfg.interfaces() {
            let inum = grp.interface_number();
            for alt in grp.alt_settings() {
                let (mut out_ep, mut in_ep) = (None, None);
                for ep in alt.endpoints() {
                    if ep.transfer_type() != EndpointType::Bulk {
                        continue;
                    }
                    match ep.direction() {
                        Direction::Out if out_ep.is_none() => out_ep = Some(ep.address()),
                        Direction::In if in_ep.is_none() => in_ep = Some(ep.address()),
                        _ => {}
                    }
                }
                if let Some(o) = out_ep {
                    found = Some((inum, o, in_ep.unwrap_or(0)));
                    break 'outer;
                }
            }
        }
        let (inum, ep_out, ep_in) = found.ok_or_else(|| anyhow!("device has no bulk OUT endpoint"))?;
        let iface = dev.claim_interface(inum)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|e| anyhow!("runtime: {e}"))?;
        Ok(NusbTransport { label, iface, ep_out, ep_in, rt })
    }
}

impl Transport for NusbTransport {
    fn name(&self) -> String {
        format!("USB {}", self.label)
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        for chunk in data.chunks(4096) {
            let comp = self.rt.block_on(self.iface.bulk_out(self.ep_out, chunk.to_vec()));
            comp.status.map_err(|e| anyhow!("USB write failed: {e:?}"))?;
        }
        Ok(())
    }
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        use nusb::transfer::RequestBuffer;
        if self.ep_in == 0 {
            std::thread::sleep(timeout.min(Duration::from_millis(50)));
            return Ok(vec![]);
        }
        let fut = self.iface.bulk_in(self.ep_in, RequestBuffer::new(256));
        let res = self.rt.block_on(async { tokio::time::timeout(timeout.max(Duration::from_millis(1)), fut).await });
        match res {
            Ok(comp) if comp.status.is_ok() => Ok(comp.data),
            _ => Ok(vec![]), // timeout or transfer error → treat as no data
        }
    }
}

// ------------------------------------------------------------------ cutter protocol

/// Read until ETX or timeout.
pub fn read_response(t: &mut dyn Transport, timeout: Duration) -> Result<Vec<u8>> {
    let start = Instant::now();
    let mut acc = vec![];
    while start.elapsed() < timeout {
        let r = t.read(Duration::from_millis(100))?;
        acc.extend(r);
        if acc.contains(&gpgl::ETX) {
            break;
        }
    }
    Ok(acc)
}

pub fn query_status(t: &mut dyn Transport) -> Result<gpgl::Status> {
    t.write_all(&gpgl::STATUS)?;
    Ok(gpgl::parse_status(&read_response(t, Duration::from_secs(2))?))
}

pub fn query_firmware(t: &mut dyn Transport) -> Result<String> {
    t.write_all(&gpgl::cmd("FG"))?;
    let r = read_response(t, Duration::from_secs(2))?;
    Ok(String::from_utf8_lossy(&r).trim_matches(|c: char| c == '\u{3}' || c.is_whitespace()).to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub sent: usize,
    pub total: usize,
    pub message: String,
    pub done: bool,
    pub error: Option<String>,
}

fn set_msg(progress: &Arc<Mutex<Progress>>, f: impl Fn(&mut Progress)) {
    if let Ok(mut p) = progress.lock() {
        f(&mut p)
    }
}

/// Send the init handshake and wait until the cutter reports ready (media loaded).
fn wait_until_ready(t: &mut dyn Transport, progress: &Arc<Mutex<Progress>>, cancel: &Arc<AtomicBool>) -> Result<()> {
    if !t.interactive() {
        return Ok(());
    }
    t.write_all(&gpgl::INIT)?;
    std::thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        match query_status(t)? {
            gpgl::Status::Ready => break,
            gpgl::Status::NoMedia => set_msg(progress, |p| p.message = "Load media into the cutter…".into()),
            gpgl::Status::Moving => set_msg(progress, |p| p.message = "Cutter busy…".into()),
            gpgl::Status::Paused => set_msg(progress, |p| p.message = "Cutter paused…".into()),
            gpgl::Status::Unknown(s) => {
                if start.elapsed() > Duration::from_secs(5) {
                    set_msg(progress, |p| p.message = format!("No status reply ({s}); sending anyway"));
                    break;
                }
            }
        }
        if start.elapsed() > Duration::from_secs(300) {
            bail!("timed out waiting for the cutter");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// Stream job bytes in chunks, updating progress and honouring cancellation.
fn stream_chunks(t: &mut dyn Transport, data: &[u8], progress: &Arc<Mutex<Progress>>, cancel: &Arc<AtomicBool>) -> Result<()> {
    for chunk in data.chunks(1024) {
        if cancel.load(Ordering::Relaxed) {
            let _ = t.write_all(&gpgl::cmd("H")); // ask the device to stop and home
            bail!("cancelled");
        }
        t.write_all(chunk)?;
        set_msg(progress, |p| p.sent += chunk.len());
    }
    Ok(())
}

/// Read a `mark_status;` reply, accumulating until a `;` arrives or `timeout`.
fn read_mark_status(t: &mut dyn Transport, timeout: Duration) -> Result<Option<gpgl::MarkStatus>> {
    let start = Instant::now();
    let mut acc = vec![];
    while start.elapsed() < timeout {
        acc.extend(t.read(Duration::from_millis(100))?);
        if acc.contains(&b';') {
            return Ok(gpgl::parse_mark_status(&acc));
        }
    }
    Ok(None)
}

/// Run the documented interactive registration-mark search: for each mark, send
/// its `DETECT` command then poll `mark_status;` until the device answers found
/// (`=4`) or failed (`=5`). The firmware applies the alignment correction once
/// the marks are located.
fn run_regmark_search(
    t: &mut dyn Transport,
    marks: &[gpgl::MarkStep],
    progress: &Arc<Mutex<Progress>>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let n = marks.len();
    for (i, step) in marks.iter().enumerate() {
        let label = step.label.clone();
        set_msg(progress, |p| p.message = format!("Aligning: searching {label} mark ({}/{n})…", i + 1));
        t.write_all(&step.detect)?;
        let start = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = t.write_all(&gpgl::cmd("H"));
                bail!("cancelled");
            }
            t.write_all(gpgl::MARK_STATUS.as_bytes())?;
            match read_mark_status(t, Duration::from_secs(2))? {
                Some(gpgl::MarkStatus::Found) => break,
                Some(gpgl::MarkStatus::Failed) => {
                    bail!("registration mark not found ({}). Check the printout and that the marks are unobscured.", step.label)
                }
                _ => {}
            }
            if start.elapsed() > Duration::from_secs(60) {
                bail!("timed out searching for the {} registration mark", step.label);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    Ok(())
}

fn finish(progress: &Arc<Mutex<Progress>>, result: Result<()>) {
    set_msg(progress, |p| {
        p.done = true;
        match &result {
            Ok(()) => p.message = "Job sent".into(),
            Err(e) => {
                p.error = Some(e.to_string());
                p.message = format!("Failed: {e}");
            }
        }
    });
}

/// Stream a GPGL job to the device on a background thread.
pub fn spawn_send(
    t: Box<dyn Transport>,
    data: Vec<u8>,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    spawn_send_lazy(Box::new(move || Ok(t)), data, progress, cancel)
}

/// Like `spawn_send`, but opens the connection on the worker thread (for slow links such as Bluetooth).
pub fn spawn_send_lazy(
    open: Box<dyn FnOnce() -> Result<Box<dyn Transport>> + Send>,
    data: Vec<u8>,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let total = data.len();
        set_msg(&progress, |p| {
            p.total = total;
            p.message = "Connecting…".into();
        });
        let result = (|| -> Result<()> {
            let mut t = open()?;
            set_msg(&progress, |p| p.message = format!("Connected to {}", t.name()));
            wait_until_ready(t.as_mut(), &progress, &cancel)?;
            set_msg(&progress, |p| p.message = "Sending…".into());
            stream_chunks(t.as_mut(), &data, &progress, &cancel)
        })();
        finish(&progress, result);
    })
}

/// Stream a staged Print & Cut job: setup, then the interactive registration-mark
/// search, then the cut body. Opens the connection on the worker thread.
pub fn spawn_send_staged_lazy(
    open: Box<dyn FnOnce() -> Result<Box<dyn Transport>> + Send>,
    staged: gpgl::Staged,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let total = staged.setup.len() + staged.body.len();
        set_msg(&progress, |p| {
            p.total = total;
            p.message = "Connecting…".into();
        });
        let result = (|| -> Result<()> {
            let mut t = open()?;
            set_msg(&progress, |p| p.message = format!("Connected to {}", t.name()));
            wait_until_ready(t.as_mut(), &progress, &cancel)?;
            stream_chunks(t.as_mut(), &staged.setup, &progress, &cancel)?;
            if !staged.marks.is_empty() {
                if !t.interactive() {
                    bail!("registration-mark search needs a two-way connection (USB/Bluetooth/network), not file output");
                }
                run_regmark_search(t.as_mut(), &staged.marks, &progress, &cancel)?;
            }
            set_msg(&progress, |p| p.message = "Cutting…".into());
            stream_chunks(t.as_mut(), &staged.body, &progress, &cancel)
        })();
        finish(&progress, result);
    })
}

/// Convenience wrapper around [`spawn_send_staged_lazy`] for an already-open transport.
pub fn spawn_send_staged(
    t: Box<dyn Transport>,
    staged: gpgl::Staged,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    spawn_send_staged_lazy(Box::new(move || Ok(t)), staged, progress, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_transport_streams_all_bytes() {
        let dir = std::env::temp_dir().join("cutcraft_test_job.gpgl");
        let t = FileTransport::create(&dir).unwrap();
        let p = Arc::new(Mutex::new(Progress::default()));
        spawn_send(Box::new(t), b"FN0\x03H\x03".to_vec(), p.clone(), Arc::new(AtomicBool::new(false))).join().unwrap();
        assert!(p.lock().unwrap().done);
        assert_eq!(std::fs::read(&dir).unwrap(), b"FN0\x03H\x03");
    }
}
