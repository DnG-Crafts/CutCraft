//! Bringing artwork and fonts in from the web: icons from Iconify's open
//! API, open-licensed fonts from Fontsource's API, and any SVG by its link.
//!
//! Everything here blocks while it downloads, so call it from a background
//! thread.

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

/// Largest download accepted (an SVG or a font file).
const MAX_BYTES: u64 = 20 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("CutCraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The body of `url`.
pub fn get(url: &str) -> Result<Vec<u8>> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("that isn't a web link");
    }
    let resp = agent().get(url).call().map_err(|e| anyhow!("couldn't download it: {e}"))?;
    let mut out = vec![];
    resp.into_body().into_reader().take(MAX_BYTES + 1).read_to_end(&mut out)?;
    if out.len() as u64 > MAX_BYTES {
        bail!("that file is too big");
    }
    Ok(out)
}

/// Whether a link the browser is following is a file to bring in rather
/// than a page: a site's download link (SVG Repo's `/download/…`), or a
/// link straight to an SVG or font file.
pub fn is_file_link(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
    let path = path.splitn(4, '/').nth(3).unwrap_or("");
    path.starts_with("download/") || path.contains("/download/") || [".svg", ".ttf", ".otf"].iter().any(|e| path.ends_with(e))
}

/// What a web page posts (to `window.ipc`) to hand CutCraft a file: starts
/// with this, then JSON.
pub const PAGE_FILE_PREFIX: &str = "cutcraft-file:";

/// A script for the page that downloads `url` as the page itself would
/// (its own session, nothing added) and posts the file to `window.ipc` -
/// for the macOS web view, which shows a file it can display even when the
/// site sends it as a download.
pub fn page_fetch_script(url: &str) -> String {
    let u = serde_json::to_string(url).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(async () => {{
  const url = {u};
  const send = (o) => window.ipc.postMessage('{PAGE_FILE_PREFIX}' + JSON.stringify(Object.assign({{ url }}, o)));
  try {{
    const r = await fetch(url, {{ credentials: 'include' }});
    if (!r.ok) throw new Error('the site answered ' + r.status);
    const b = new Uint8Array(await r.arrayBuffer());
    if (b.length > {MAX_BYTES}) throw new Error('that file is too big');
    let s = '';
    for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
    send({{ name: r.headers.get('content-disposition') || '', data: btoa(s) }});
  }} catch (e) {{
    send({{ error: String(e && e.message || e) }});
  }}
}})();"#
    )
}

/// The file in a page's message from [`page_fetch_script`]: its name (from
/// the server, or the link) and bytes.
pub fn page_file(message: &str) -> Result<(String, Vec<u8>)> {
    #[derive(Deserialize)]
    struct Posted {
        url: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        data: String,
        error: Option<String>,
    }
    let json = message.strip_prefix(PAGE_FILE_PREFIX).ok_or_else(|| anyhow!("not a file"))?;
    let p: Posted = serde_json::from_str(json)?;
    if let Some(e) = p.error {
        bail!("couldn't download it: {e}");
    }
    let bytes = crate::doc::b64::decode(&p.data).map_err(|e| anyhow!(e))?;
    let name = disposition_name(&p.name).unwrap_or_else(|| p.url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string());
    Ok((safe_file_name(&name), bytes))
}

/// The file name in a Content-Disposition header
/// (`attachment; filename="heart.svg"`).
fn disposition_name(header: &str) -> Option<String> {
    header.split(';').map(str::trim).find_map(|p| p.strip_prefix("filename=")).map(|n| n.trim_matches('"').to_string()).filter(|n| !n.is_empty())
}

/// A name that's safe to save under: no folders, nothing hidden.
fn safe_file_name(name: &str) -> String {
    let n = name.rsplit(['/', '\\']).next().unwrap_or("").trim().trim_start_matches('.');
    let n: String = n.chars().filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    if n.is_empty() { "download".into() } else { n }
}

fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    Ok(serde_json::from_slice(&get(url)?)?)
}

/// Whether `text` looks like SVG markup (as copied from a website).
pub fn looks_like_svg(text: &str) -> bool {
    let t = text.trim_start();
    (t.starts_with("<svg") || t.starts_with("<?xml") || t.starts_with("<!--")) && t.contains("<svg") && t.contains("</svg>")
}

/// Turns a page link into the file's own link where a site is known to
/// wrap its files in a page (SVG Repo's "show" pages).
pub fn svg_file_url(url: &str) -> String {
    let u = url.trim();
    // https://www.svgrepo.com/svg/12345/heart → https://www.svgrepo.com/show/12345/heart.svg
    if let Some(rest) = u.split("svgrepo.com/svg/").nth(1) {
        let rest = rest.trim_end_matches('/');
        return format!("https://www.svgrepo.com/show/{rest}.svg");
    }
    u.to_string()
}

/// Downloads an SVG by its link (a page link to an SVG Repo icon works too).
pub fn fetch_svg(url: &str) -> Result<String> {
    let bytes = get(&svg_file_url(url))?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("that link isn't an SVG file"))?;
    if !looks_like_svg(&text) {
        bail!("that link isn't an SVG file - open the icon's page and use its download or \"copy SVG\" link");
    }
    Ok(text)
}

// ------------------------------------------------------------------ Iconify

/// One icon found by a search.
#[derive(Clone, Debug)]
pub struct Icon {
    /// `set:name`, e.g. `mdi:heart`.
    pub id: String,
    /// The whole SVG.
    pub svg: String,
    /// The icon set's name and licence.
    pub set_name: String,
    pub license: String,
}

#[derive(Deserialize)]
struct Search {
    icons: Vec<String>,
}

#[derive(Deserialize)]
struct SetData {
    #[serde(default = "sixteen")]
    width: f64,
    #[serde(default = "sixteen")]
    height: f64,
    icons: HashMap<String, IconData>,
    #[serde(default)]
    aliases: HashMap<String, Alias>,
}

#[derive(Deserialize)]
struct IconData {
    body: String,
    width: Option<f64>,
    height: Option<f64>,
}

#[derive(Deserialize)]
struct Alias {
    parent: String,
}

fn sixteen() -> f64 {
    16.0
}

#[derive(Deserialize)]
struct SetInfo {
    name: String,
    license: Option<SetLicense>,
}

#[derive(Deserialize)]
struct SetLicense {
    title: String,
}

/// Searches Iconify's open icon sets. Icon sets with colours of their own
/// (emoji and the like) are left out - they don't cut well.
pub fn search_icons(query: &str, limit: usize) -> Result<Vec<Icon>> {
    let q: String = query.trim().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    if q.trim().is_empty() {
        return Ok(vec![]);
    }
    let found: Search = get_json(&format!("https://api.iconify.design/search?query={}&limit={}&palette=false", q.trim().replace(' ', "+"), limit.clamp(32, 999)))?;
    // fetch each set's icons in one request
    let mut by_set: Vec<(String, Vec<String>)> = vec![];
    for id in &found.icons {
        let Some((set, name)) = id.split_once(':') else { continue };
        match by_set.iter_mut().find(|(s, _)| s == set) {
            Some((_, v)) => v.push(name.to_string()),
            None => by_set.push((set.to_string(), vec![name.to_string()])),
        }
    }
    let prefixes: Vec<&str> = by_set.iter().map(|(s, _)| s.as_str()).collect();
    let info: HashMap<String, SetInfo> = if prefixes.is_empty() { HashMap::new() } else { get_json(&format!("https://api.iconify.design/collections?prefixes={}", prefixes.join(",")))? };
    let mut out = vec![];
    for (set, names) in by_set {
        let data: SetData = get_json(&format!("https://api.iconify.design/{set}.json?icons={}", names.join(",")))?;
        let (set_name, license) = match info.get(&set) {
            Some(i) => (i.name.clone(), i.license.as_ref().map(|l| l.title.clone()).unwrap_or_else(|| "see the icon set".into())),
            None => (set.clone(), "see the icon set".into()),
        };
        for name in names {
            let key = data.aliases.get(&name).map(|a| a.parent.clone()).unwrap_or(name.clone());
            let Some(icon) = data.icons.get(&key) else { continue };
            let (w, h) = (icon.width.unwrap_or(data.width), icon.height.unwrap_or(data.height));
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" color="black">{}</svg>"#, icon.body);
            out.push(Icon { id: format!("{set}:{name}"), svg, set_name: set_name.clone(), license: license.clone() });
        }
    }
    // keep the search's own order (best matches first)
    out.sort_by_key(|i| found.icons.iter().position(|f| *f == i.id).unwrap_or(usize::MAX));
    out.truncate(limit);
    Ok(out)
}

// ------------------------------------------------------------------ Fontsource

/// An open-licensed font family from Fontsource.
#[derive(Clone, Debug, Deserialize)]
pub struct WebFont {
    pub id: String,
    pub family: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub weights: Vec<u32>,
    #[serde(default)]
    pub styles: Vec<String>,
    #[serde(default)]
    pub subsets: Vec<String>,
}

/// Every font Fontsource lists that covers Latin text.
pub fn font_list() -> Result<Vec<WebFont>> {
    let mut all: Vec<WebFont> = get_json("https://api.fontsource.org/v1/fonts")?;
    all.retain(|f| f.subsets.iter().any(|s| s == "latin"));
    all.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    Ok(all)
}

/// The TTF files for a font: regular, plus bold and italic where it has them.
pub fn font_files(f: &WebFont) -> Result<Vec<(String, Vec<u8>)>> {
    let mut want: Vec<(u32, &str)> = vec![];
    let regular = if f.weights.contains(&400) { 400 } else { *f.weights.iter().min_by_key(|w| (**w as i32 - 400).abs()).unwrap_or(&400) };
    let normal = f.styles.iter().any(|s| s == "normal");
    want.push((regular, if normal { "normal" } else { "italic" }));
    if f.weights.contains(&700) && normal {
        want.push((700, "normal"));
    }
    if normal && f.styles.iter().any(|s| s == "italic") {
        want.push((regular, "italic"));
    }
    let mut out = vec![];
    for (w, s) in want {
        let url = format!("https://cdn.jsdelivr.net/fontsource/fonts/{}@latest/latin-{w}-{s}.ttf", f.id);
        let bytes = get(&url)?;
        if ttf_parser::Face::parse(&bytes, 0).is_err() {
            bail!("the font file for {} didn't download properly", f.family);
        }
        out.push((format!("{}-{w}-{s}.ttf", f.id), bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_svg_text_and_svgrepo_page_links() {
        assert!(looks_like_svg("<?xml version=\"1.0\"?>\n<svg viewBox=\"0 0 1 1\"></svg>"));
        assert!(looks_like_svg("  <svg xmlns=\"x\"><path d=\"M0 0\"/></svg>"));
        assert!(!looks_like_svg("hello"));
        assert_eq!(svg_file_url("https://www.svgrepo.com/svg/12345/heart"), "https://www.svgrepo.com/show/12345/heart.svg");
        assert_eq!(svg_file_url("https://example.com/a.svg"), "https://example.com/a.svg");
    }

    #[test]
    fn tells_file_links_from_pages() {
        assert!(is_file_link("https://www.svgrepo.com/download/533360/heart.svg"));
        assert!(is_file_link("https://www.fontrepo.com/font/1/a-bebedera.ttf"));
        assert!(is_file_link("https://example.com/files/Logo.SVG?v=2"));
        assert!(!is_file_link("https://www.svgrepo.com/svg/533360/heart"));
        assert!(!is_file_link("https://www.svgrepo.com/"));
        assert!(!is_file_link("https://www.fontrepo.com/font/1/a-bebedera"));
        assert!(!is_file_link("blob:https://www.svgrepo.com/1234"));
        assert_eq!(disposition_name("attachment; filename=heart-svgrepo-com.svg").as_deref(), Some("heart-svgrepo-com.svg"));
        assert_eq!(disposition_name("attachment; filename=\"a b.ttf\"").as_deref(), Some("a b.ttf"));
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name(".hidden"), "hidden");
        assert_eq!(safe_file_name(""), "download");
    }

    /// Talks to the real services (run by hand).
    #[test]
    #[ignore = "needs the internet"]
    fn finds_icons_and_fonts() {
        let icons = search_icons("heart", 40).unwrap();
        assert!(!icons.is_empty());
        assert!(icons.iter().all(|i| i.svg.contains("<svg")), "{:?}", icons.first());
        println!("{} icons, first {} ({}, {})", icons.len(), icons[0].id, icons[0].set_name, icons[0].license);
        let fonts = font_list().unwrap();
        let abel = fonts.iter().find(|f| f.id == "abel").unwrap();
        let files = font_files(abel).unwrap();
        println!("{} fonts; Abel: {:?}", fonts.len(), files.iter().map(|f| (&f.0, f.1.len())).collect::<Vec<_>>());
    }

    #[test]
    fn reads_a_file_the_page_posted() {
        let js = page_fetch_script("https://www.svgrepo.com/download/533360/heart.svg");
        assert!(js.contains("\"https://www.svgrepo.com/download/533360/heart.svg\"") && js.contains(PAGE_FILE_PREFIX));
        let msg = format!("{PAGE_FILE_PREFIX}{{\"url\":\"https://www.svgrepo.com/download/533360/heart.svg\",\"name\":\"attachment; filename=heart-svgrepo-com.svg\",\"data\":\"PHN2Zy8+\"}}");
        let (name, bytes) = page_file(&msg).unwrap();
        assert_eq!((name.as_str(), bytes.as_slice()), ("heart-svgrepo-com.svg", b"<svg/>".as_slice()));
        // no name from the server: the link's
        let msg = format!("{PAGE_FILE_PREFIX}{{\"url\":\"https://x.org/f/a.ttf?v=1\",\"name\":\"\",\"data\":\"\"}}");
        assert_eq!(page_file(&msg).unwrap().0, "a.ttf");
        let msg = format!("{PAGE_FILE_PREFIX}{{\"url\":\"https://x.org/a.svg\",\"error\":\"the site answered 404\"}}");
        assert!(page_file(&msg).unwrap_err().to_string().contains("404"));
        assert!(page_file("blocked").is_err());
    }
}
