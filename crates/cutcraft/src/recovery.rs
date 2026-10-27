//! Auto-save and crash recovery, ported from BurnCraft.
//!
//! While there are unsaved changes the design is copied to a backup in the
//! config folder's `recovery` directory (on a background thread). Saving, or a
//! clean exit, deletes it; a backup found at startup means a session didn't
//! close properly, and its design can be restored.
//!
//! Each running CutCraft keeps its own backup next to a lock file it holds open
//! and locked, so a second window never mistakes the first one's backup for a
//! crash: a backup counts as left behind only when its lock can be taken.

use cutcraft_core::doc::Document;
use cutcraft_core::io;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What a backup was a copy of.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecoveryInfo {
    /// The design file it belongs to, if it had been saved before.
    pub file: Option<PathBuf>,
    /// When it was written (seconds since 1970).
    pub saved_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl RecoveryInfo {
    /// "3 minutes ago"-style age.
    pub fn age(&self) -> String {
        let mins = now_secs().saturating_sub(self.saved_at) / 60;
        match mins {
            0 => "less than a minute ago".into(),
            1 => "1 minute ago".into(),
            m if m < 120 => format!("{m} minutes ago"),
            m if m < 48 * 60 => format!("{} hours ago", m / 60),
            m => format!("{} days ago", m / (24 * 60)),
        }
    }
}

/// A backup left behind by a session that didn't close properly.
#[derive(Debug, Clone)]
pub struct Pending {
    pub design: PathBuf,
    pub info: RecoveryInfo,
    session: String,
}

pub struct AutoSaver {
    dir: Option<PathBuf>,
    /// This session's file stem inside `dir`.
    session: String,
    /// Held (and locked) for the life of the session.
    lock: Option<File>,
    /// Revision last backed up, and when.
    saved_revision: Option<u64>,
    last: Option<Instant>,
    /// A backup of this session exists on disk.
    has_backup: bool,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl AutoSaver {
    pub fn new() -> Self {
        Self::in_dir(Some(cutcraft_core::prefs::config_dir().join("recovery")))
    }

    pub fn in_dir(dir: Option<PathBuf>) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session = format!("session-{}-{}-{n}", std::process::id(), now_secs());
        let lock = dir.as_ref().and_then(|d| {
            std::fs::create_dir_all(d).ok()?;
            let f = File::create(d.join(format!("{session}.lock"))).ok()?;
            f.try_lock().ok()?;
            Some(f)
        });
        Self { dir, session, lock, saved_revision: None, last: None, has_backup: false, worker: None }
    }

    fn files(&self, session: &str) -> Option<(PathBuf, PathBuf, PathBuf)> {
        let d = self.dir.as_ref()?;
        Some((d.join(format!("{session}.cutcraft")), d.join(format!("{session}.json")), d.join(format!("{session}.lock"))))
    }

    /// The newest backup left by a session that is no longer running. Stale
    /// lock files of sessions that never backed anything up are tidied away.
    pub fn pending(&self) -> Option<Pending> {
        let dir = self.dir.as_ref()?;
        let mut found: Vec<Pending> = vec![];
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(session) = name.strip_suffix(".lock") else { continue };
            if session == self.session {
                continue;
            }
            let (design, info_path, lock_path) = self.files(session)?;
            // still running? (its lock is held)
            let alive = File::options().write(true).open(&lock_path).map(|f| f.try_lock().is_err()).unwrap_or(false);
            if alive {
                continue;
            }
            let info = std::fs::read(&info_path).ok().and_then(|b| serde_json::from_slice::<RecoveryInfo>(&b).ok());
            match info {
                Some(info) if design.exists() => found.push(Pending { design, info, session: session.to_string() }),
                _ => {
                    for p in [design, info_path, lock_path] {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
        }
        found.into_iter().max_by_key(|p| p.info.saved_at)
    }

    /// Deletes a left-behind backup (once restored or declined).
    pub fn discard(&self, p: &Pending) {
        if let Some((design, info, lock)) = self.files(&p.session) {
            for f in [design, info, lock] {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Backs up `doc` if it has changes that haven't been backed up, at most
    /// once per `interval`; removes the backup once the design is saved.
    /// `dirty` means the document differs from its file (or was never saved).
    pub fn tick(&mut self, doc: &Document, revision: u64, dirty: bool, file: Option<&Path>, interval: Duration) {
        if !dirty {
            if self.has_backup {
                self.clear();
            }
            return;
        }
        if interval.is_zero() || doc.pages.iter().all(|p| p.shapes.is_empty()) || self.saved_revision == Some(revision) {
            return;
        }
        if self.last.is_some_and(|t| t.elapsed() < interval) || self.worker.as_ref().is_some_and(|w| !w.is_finished()) {
            return;
        }
        self.save_now(doc, revision, file);
    }

    /// Writes the backup now (in the background).
    pub fn save_now(&mut self, doc: &Document, revision: u64, file: Option<&Path>) {
        let Some((design, info_path, _)) = self.files(&self.session) else { return };
        // Cheap: image data is shared (Arc), so this copies only the shapes.
        let doc = doc.clone();
        let info = RecoveryInfo { file: file.map(Path::to_path_buf), saved_at: now_secs() };
        self.saved_revision = Some(revision);
        self.last = Some(Instant::now());
        self.has_backup = true;
        self.worker = std::thread::Builder::new()
            .name("autosave".into())
            .spawn(move || {
                if let Some(dir) = design.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if io::save_native(&doc, &design).is_ok() {
                    let _ = std::fs::write(info_path, serde_json::to_vec(&info).unwrap_or_default());
                }
            })
            .ok();
    }

    /// Removes this session's backup (after saving, or a clean exit).
    pub fn clear(&mut self) {
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some((design, info, _)) = self.files(&self.session) {
            let _ = std::fs::remove_file(design);
            let _ = std::fs::remove_file(info);
        }
        self.saved_revision = None;
        self.has_backup = false;
    }

    /// Clean exit: removes the backup and releases the lock.
    pub fn finish(&mut self) {
        self.clear();
        if self.lock.take().is_some() {
            if let Some((_, _, lock)) = self.files(&self.session) {
                let _ = std::fs::remove_file(lock);
            }
        }
    }

    /// Waits for a backup in progress (for tests).
    #[cfg(test)]
    fn wait(&mut self) {
        if let Some(w) = self.worker.take() {
            w.join().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutcraft_core::doc::Style;
    use cutcraft_core::geom;
    use cutcraft_core::kurbo::Rect;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cutcraft-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn one_shape() -> Document {
        let mut doc = Document::default();
        doc.add_path(geom::rect_path(Rect::new(0.0, 0.0, 20.0, 20.0)), Style::default(), 0);
        doc
    }

    const NOW: Duration = Duration::from_secs(0);
    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_crashed_sessions_backup_is_found_and_a_running_ones_is_not() {
        let dir = temp_dir("recovery");
        let doc = one_shape();
        let mut crashed = AutoSaver::in_dir(Some(dir.clone()));
        crashed.tick(&Document::default(), 1, true, None, MIN);
        crashed.tick(&doc, 2, false, None, MIN);
        crashed.wait();
        assert!(!crashed.has_backup, "empty or saved designs aren't backed up");
        crashed.tick(&doc, 2, true, Some(Path::new("C:/designs/sign.cutcraft")), MIN);
        crashed.wait();

        // A second window while the first still runs: nothing to recover.
        let other = AutoSaver::in_dir(Some(dir.clone()));
        assert!(other.pending().is_none());

        // The first one "crashes" (its lock goes away without cleanup).
        drop(crashed.lock.take());
        let p = other.pending().expect("a backup");
        assert_eq!(p.info.file.as_deref(), Some(Path::new("C:/designs/sign.cutcraft")));
        assert_eq!(io::load_native(&p.design).unwrap().page().shapes.len(), 1);
        assert_eq!(p.info.age(), "less than a minute ago");
        other.discard(&p);
        assert!(other.pending().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_or_exiting_removes_the_backup() {
        let dir = temp_dir("recovery-clean");
        let doc = one_shape();
        let mut s = AutoSaver::in_dir(Some(dir.clone()));
        s.tick(&doc, 1, true, None, NOW); // zero interval = autosave off
        assert!(!s.has_backup);
        s.tick(&doc, 1, true, None, MIN);
        s.wait();
        assert!(s.has_backup);
        s.tick(&doc, 1, false, None, MIN); // saved
        assert!(!s.has_backup);
        s.tick(&doc, 2, true, None, Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        s.finish();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "clean exit leaves nothing behind");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod plant_tests {
    use super::*;

    /// Leaves a backup in the real recovery folder, as a crash would (for checking the restore prompt).
    #[test]
    #[ignore = "diagnostic"]
    fn plant_backup() {
        let mut saver = AutoSaver::new();
        let mut doc = Document::default();
        doc.add_path(cutcraft_core::geom::rect_path(cutcraft_core::kurbo::Rect::new(60.0, 60.0, 240.0, 180.0)), Default::default(), 0);
        saver.save_now(&doc, 1, Some(Path::new("C:/designs/shop-sign.cutcraft")));
        saver.wait();
        drop(saver.lock.take()); // "crash": the lock is released without cleanup
        println!("planted in {:?}", saver.dir);
    }
}

#[cfg(test)]
mod guide_demo {
    /// Writes a design with ruler guides to the temp folder (for a visual check).
    #[test]
    #[ignore = "diagnostic"]
    fn write_guide_demo() {
        use cutcraft_core::doc::{Document, Guide, Style};
        let mut doc = Document::default();
        doc.add_path(cutcraft_core::geom::rect_path(cutcraft_core::kurbo::Rect::new(50.0, 50.0, 150.0, 120.0)), Style::default(), 0);
        for (vertical, pos) in [(true, 50.0), (true, 150.0), (false, 50.0), (false, 200.0)] {
            doc.page_mut().guides.push(Guide { vertical, pos });
        }
        let p = std::env::temp_dir().join("cutcraft-guides-demo.cutcraft");
        cutcraft_core::io::save_native(&doc, &p).unwrap();
        println!("wrote {}", p.display());
    }
}
