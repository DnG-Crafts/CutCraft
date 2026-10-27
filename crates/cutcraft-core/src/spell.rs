//! Spell-checking with Hunspell dictionaries (.aff + .dic).
//!
//! CutCraft doesn't ship dictionaries; it finds ones already installed (LibreOffice,
//! OpenOffice, CutCraft's own `dictionaries` folder) or uses files you choose.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct Speller {
    dict: spellbook::Dictionary,
    pub name: String,
    pub custom: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Misspelling {
    pub word: String,
    /// Byte range in the checked text.
    pub start: usize,
    pub end: usize,
}

impl Speller {
    /// Load from the `.aff` path (the `.dic` must sit next to it).
    pub fn load(aff: &Path) -> Result<Speller> {
        let dic = aff.with_extension("dic");
        let a = read_text(aff)?;
        let d = read_text(&dic)?;
        let dict = spellbook::Dictionary::new(&a, &d).map_err(|e| anyhow::anyhow!("dictionary parse error: {e:?}"))?;
        Ok(Speller { dict, name: aff.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), custom: BTreeSet::new() })
    }

    pub fn check_word(&self, w: &str) -> bool {
        if w.chars().all(|c| !c.is_alphabetic()) || self.custom.contains(&w.to_lowercase()) {
            return true;
        }
        self.dict.check(w)
    }

    pub fn suggest(&self, w: &str) -> Vec<String> {
        let mut v = vec![];
        self.dict.suggest(w, &mut v);
        v.truncate(8);
        v
    }

    pub fn check_text(&self, text: &str) -> Vec<Misspelling> {
        words(text).into_iter().filter(|(_, _, w)| !self.check_word(w)).map(|(s, e, w)| Misspelling { word: w.to_string(), start: s, end: e }).collect()
    }
}

/// Dictionaries may be UTF-8 or Latin-1; decode leniently.
fn read_text(p: &Path) -> Result<String> {
    let b = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    Ok(match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|b| *b as char).collect(),
    })
}

/// Split text into words: (start, end, word). Apostrophes inside words are kept.
pub fn words(text: &str) -> Vec<(usize, usize, &str)> {
    let mut out = vec![];
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let inner_apos = (c == '\'' || c == '’')
            && start.is_some()
            && chars.get(k + 1).map(|(_, n)| n.is_alphabetic()).unwrap_or(false);
        if c.is_alphabetic() || inner_apos {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            out.push((s, i, &text[s..i]));
        }
    }
    if let Some(s) = start {
        out.push((s, text.len(), &text[s..]));
    }
    out
}

/// Look for installed Hunspell dictionaries.
pub fn find_dictionaries() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![crate::prefs::config_dir().join("dictionaries")];
    for pf in ["C:\\Program Files", "C:\\Program Files (x86)"] {
        roots.push(PathBuf::from(pf).join("LibreOffice\\share\\extensions"));
        roots.push(PathBuf::from(pf).join("OpenOffice 4\\share\\extensions"));
    }
    roots.push(PathBuf::from("/usr/share/hunspell"));
    roots.push(PathBuf::from("/usr/share/myspell/dicts"));
    roots.push(PathBuf::from("/Library/Spelling"));
    if let Some(h) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(h).join("Library/Spelling"));
    }
    let mut out = vec![];
    for r in roots {
        scan(&r, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn scan(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 3 {
            scan(&p, depth + 1, out);
        } else if p.extension().map(|x| x == "aff").unwrap_or(false) && p.with_extension("dic").exists() {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words() {
        let w: Vec<&str> = words("Don't cut-me, 3 times!").into_iter().map(|w| w.2).collect();
        assert_eq!(w, vec!["Don't", "cut", "me", "times"]);
    }

    #[test]
    fn checks_with_tiny_dictionary() {
        let dir = std::env::temp_dir().join("cc_dict");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.aff"), "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n").unwrap();
        std::fs::write(dir.join("t.dic"), "3\nhello\nworld\ncraft\n").unwrap();
        let mut s = Speller::load(&dir.join("t.aff")).unwrap();
        let m = s.check_text("hello wrold craft");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].word, "wrold");
        assert!(s.suggest("wrold").iter().any(|x| x == "world"));
        s.custom.insert("wrold".into());
        assert!(s.check_text("wrold").is_empty());
    }
}
