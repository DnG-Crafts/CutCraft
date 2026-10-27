//! Bluetooth LE transport.
//!
//! Desktop cutters with Bluetooth expose a vendor-specific GATT "serial" service.
//! Its UUIDs are not publicly documented, so CutCraft discovers it: after
//! connecting it picks a writable characteristic that lives in the same
//! (non-standard) service as a notifying one, and streams GPGL through it.
//! You can override the choice with explicit characteristic UUIDs.

use crate::device::Transport;
use anyhow::{Context, Result, anyhow, bail};
use btleplug::api::{CharPropFlags, Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification, WriteType};
use btleplug::platform::{Manager, Peripheral};
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct BleDevice {
    pub id: String,
    pub name: String,
    pub rssi: Option<i16>,
    pub likely_cutter: bool,
}

fn cache() -> &'static Mutex<HashMap<String, Peripheral>> {
    static C: OnceLock<Mutex<HashMap<String, Peripheral>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread().enable_all().build()?)
}

const CUTTER_NAMES: [&str; 5] = ["CAMEO", "PORTRAIT", "CURIO", "SILHOUETTE", "GRAPHTEC"];

/// Scan for nearby BLE devices for `secs` seconds. Cutters are listed first.
pub fn scan(secs: u64) -> Result<Vec<BleDevice>> {
    scan_inner(secs).map_err(|e| {
        let s = format!("{e:#}");
        if s.contains("0x800710DF") || s.to_lowercase().contains("not ready") || s.to_lowercase().contains("powered off") {
            anyhow!("Bluetooth is turned off or this PC has no Bluetooth adapter")
        } else {
            e
        }
    })
}

fn scan_inner(secs: u64) -> Result<Vec<BleDevice>> {
    let rt = runtime()?;
    rt.block_on(async {
        let manager = Manager::new().await.context("Bluetooth is not available")?;
        let adapter = manager.adapters().await?.into_iter().next().ok_or_else(|| anyhow!("No Bluetooth adapter found"))?;
        adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let _ = adapter.stop_scan().await;
        let mut out = vec![];
        let mut c = cache().lock().unwrap();
        for p in adapter.peripherals().await? {
            let props = p.properties().await.ok().flatten();
            let name = props.as_ref().and_then(|p| p.local_name.clone()).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let id = p.id().to_string();
            let up = name.to_uppercase();
            out.push(BleDevice {
                id: id.clone(),
                likely_cutter: CUTTER_NAMES.iter().any(|n| up.contains(n)),
                name,
                rssi: props.and_then(|p| p.rssi),
            });
            c.insert(id, p);
        }
        out.sort_by_key(|d| (!d.likely_cutter, d.name.clone()));
        Ok(out)
    })
}

/// Standard GATT services that are never the data channel.
fn is_standard(u: &Uuid) -> bool {
    let s = u.to_string();
    // 0000xxxx-0000-1000-8000-00805f9b34fb with xxxx in the SIG-assigned 0x18xx range
    s.ends_with("-0000-1000-8000-00805f9b34fb") && s.starts_with("000018")
}

pub struct BleTransport {
    rt: tokio::runtime::Runtime,
    periph: Peripheral,
    write: Characteristic,
    write_type: WriteType,
    notes: Option<Pin<Box<dyn Stream<Item = ValueNotification> + Send>>>,
    pub chunk: usize,
    name: String,
    pending: Vec<u8>,
}

impl BleTransport {
    pub fn connect(dev: &BleDevice, write_uuid: Option<Uuid>, notify_uuid: Option<Uuid>, chunk: usize) -> Result<Self> {
        let periph = cache().lock().unwrap().get(&dev.id).cloned().ok_or_else(|| anyhow!("Scan again - device not in range"))?;
        let rt = runtime()?;
        let (write, notify) = rt.block_on(async {
            if !periph.is_connected().await.unwrap_or(false) {
                periph.connect().await.context("connect failed")?;
            }
            periph.discover_services().await?;
            let chars: Vec<Characteristic> = periph.characteristics().into_iter().collect();
            let writable = |c: &Characteristic| c.properties.intersects(CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE);
            let notifying = |c: &Characteristic| c.properties.intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE);
            let w = match write_uuid {
                Some(u) => chars.iter().find(|c| c.uuid == u).cloned(),
                None => {
                    // prefer a vendor service that has both a writable and a notifying characteristic
                    chars
                        .iter()
                        .filter(|c| writable(c) && !is_standard(&c.service_uuid))
                        .max_by_key(|c| {
                            let pair = chars.iter().any(|n| n.service_uuid == c.service_uuid && notifying(n));
                            (pair, c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE))
                        })
                        .cloned()
                }
            }
            .ok_or_else(|| anyhow!("No writable data characteristic found on this device"))?;
            let n = match notify_uuid {
                Some(u) => chars.iter().find(|c| c.uuid == u).cloned(),
                None => chars.iter().find(|c| c.service_uuid == w.service_uuid && notifying(c)).cloned(),
            };
            if let Some(n) = &n {
                periph.subscribe(n).await?;
            }
            anyhow::Ok((w, n))
        })?;
        let notes = if notify.is_some() { Some(rt.block_on(periph.notifications())?) } else { None };
        let write_type = if write.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) { WriteType::WithoutResponse } else { WriteType::WithResponse };
        Ok(BleTransport { rt, periph, write, write_type, notes, chunk: chunk.clamp(20, 512), name: dev.name.clone(), pending: vec![] })
    }

    pub fn describe(&self) -> String {
        format!("write {} ({:?})", self.write.uuid, self.write_type)
    }
}

impl Transport for BleTransport {
    fn name(&self) -> String {
        format!("Bluetooth {}", self.name)
    }

    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        for c in data.chunks(self.chunk) {
            self.rt.block_on(self.periph.write(&self.write, c, self.write_type)).context("Bluetooth write failed")?;
            if self.write_type == WriteType::WithoutResponse {
                // give the device's small buffer time to drain
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        let Some(stream) = self.notes.as_mut() else { return Ok(vec![]) };
        let wid = self.write.service_uuid;
        let r = self.rt.block_on(async {
            match tokio::time::timeout(timeout, stream.next()).await {
                Ok(Some(n)) => Ok(n.value),
                Ok(None) => bail!("Bluetooth connection closed"),
                Err(_) => Ok(vec![]),
            }
        });
        let _ = wid;
        r
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        let _ = self.rt.block_on(self.periph.disconnect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_services_are_recognised() {
        assert!(is_standard(&Uuid::parse_str("00001800-0000-1000-8000-00805f9b34fb").unwrap()));
        assert!(is_standard(&Uuid::parse_str("0000180a-0000-1000-8000-00805f9b34fb").unwrap()));
        assert!(!is_standard(&Uuid::parse_str("6e400001-b5a3-f393-e0a9-e50e24dcca9e").unwrap()));
    }
}
