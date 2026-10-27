//! Round-trips Brother's sample files (not included: they are copyright their
//! owners). Point `FCM_SAMPLES` at a folder of .fcm files, e.g. fcmlib's
//! `tests/samples`, and run `cargo test -p fcmlib -- --ignored`.

use fcmlib::FcmFile;

fn fcm_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            fcm_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("fcm")) {
            out.push(p);
        }
    }
}

#[test]
#[ignore = "needs FCM_SAMPLES"]
fn every_sample_reads_and_writes_back_identically() {
    let dir = std::env::var("FCM_SAMPLES").expect("set FCM_SAMPLES");
    let mut files = vec![];
    fcm_files(std::path::Path::new(&dir), &mut files);
    assert!(!files.is_empty(), "no .fcm files under {dir}");
    let mut failed = vec![];
    for f in &files {
        let original = std::fs::read(f).unwrap();
        match FcmFile::from_bytes(&original).and_then(|p| p.to_bytes()) {
            Ok(back) if back == original => {}
            Ok(_) => failed.push(format!("{}: written back differently", f.display())),
            Err(e) => failed.push(format!("{}: {e}", f.display())),
        }
    }
    println!("{} of {} files round-trip", files.len() - failed.len(), files.len());
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
