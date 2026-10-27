//! Mail merge: fill `{{Field}}` placeholders in text shapes from CSV rows and
//! lay out one copy per row, adding pages when the current page is full.

use crate::doc::{Document, Page, Shape, ShapeId, ShapeKind};
use crate::ops;
use crate::text::FontLibrary;
use anyhow::{Result, bail};
use kurbo::{Affine, Vec2};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// RFC-4180-style CSV parser (quoted fields, doubled quotes, CRLF). Also accepts `;` or tab
/// as delimiter when the header line contains no commas.
pub fn parse_csv(text: &str) -> Result<Table> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or("");
    let delim = if first.contains(',') {
        ','
    } else if first.contains(';') {
        ';'
    } else if first.contains('\t') {
        '\t'
    } else {
        ','
    };
    let mut records: Vec<Vec<String>> = vec![];
    let mut rec = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == delim {
            rec.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            rec.push(std::mem::take(&mut field));
            if rec.iter().any(|f| !f.trim().is_empty()) {
                records.push(std::mem::take(&mut rec));
            } else {
                rec.clear();
            }
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        records.push(rec);
    }
    if records.is_empty() {
        bail!("The CSV file is empty");
    }
    let headers: Vec<String> = records.remove(0).into_iter().map(|h| h.trim().to_string()).collect();
    Ok(Table { headers, rows: records })
}

/// Replace `{{Header}}` (case-insensitive) with the row's values.
pub fn fill(template: &str, t: &Table, row: &[String]) -> String {
    let mut s = template.to_string();
    for (i, h) in t.headers.iter().enumerate() {
        let v = row.get(i).map(|x| x.as_str()).unwrap_or("");
        for pat in [format!("{{{{{h}}}}}"), format!("{{{{{}}}}}", h.to_lowercase()), format!("{{{{{}}}}}", h.to_uppercase())] {
            s = s.replace(&pat, v);
        }
    }
    s
}

/// Built-in merge fields, filled per copy (ported from BurnCraft's variable text,
/// kept in CutCraft's `{{…}}` syntax): `{{serial}}`, `{{serial:0000}}`
/// (zero-padded to the pattern's width), `{{row}}` (1-based copy number),
/// `{{date}}` and `{{time}}`. The date and time are supplied by the caller so
/// this crate needs no clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub serial_start: i64,
    pub increment: i64,
    pub date: String,
    pub time: String,
}

impl Default for Fields {
    fn default() -> Self {
        Fields { serial_start: 1, increment: 1, date: String::new(), time: String::new() }
    }
}

/// Names reserved for built-in fields (not treated as CSV columns).
pub const BUILTIN_FIELDS: [&str; 4] = ["serial", "row", "date", "time"];

/// Fill the built-in fields for copy `n` (0-based). Unknown `{{…}}` are left as written.
pub fn fill_fields(template: &str, n: usize, f: &Fields) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(a) = rest.find("{{") {
        let Some(len) = rest[a + 2..].find("}}") else { break };
        out.push_str(&rest[..a]);
        let inner = &rest[a + 2..a + 2 + len];
        let (name, fmt) = inner.split_once(':').unwrap_or((inner, ""));
        let serial = f.serial_start + n as i64 * f.increment;
        let value = match name.trim().to_ascii_lowercase().as_str() {
            "serial" if fmt.is_empty() => Some(serial.to_string()),
            "serial" => Some(format!("{serial:0width$}", width = fmt.len())),
            "row" => Some((n + 1).to_string()),
            "date" => Some(f.date.clone()),
            "time" => Some(f.time.clone()),
            _ => None,
        };
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[a..a + 2 + len + 2]),
        }
        rest = &rest[a + 2 + len + 2..];
    }
    out.push_str(rest);
    out
}

/// Placeholders used in the template shapes.
pub fn placeholders(shapes: &[Shape]) -> Vec<String> {
    let mut out = vec![];
    for s in shapes {
        for l in s.leaves() {
            if let ShapeKind::Text { data, .. } = &l.kind {
                let mut rest = data.text.as_str();
                while let Some(a) = rest.find("{{") {
                    if let Some(b) = rest[a..].find("}}") {
                        let name = rest[a + 2..a + b].to_string();
                        if !out.contains(&name) {
                            out.push(name);
                        }
                        rest = &rest[a + b + 2..];
                    } else {
                        break;
                    }
                }
            }
        }
    }
    out
}

fn fill_shape(s: &mut Shape, t: &Table, row: &[String], n: usize, fields: &Fields, fonts: &FontLibrary) {
    match &mut s.kind {
        ShapeKind::Text { data, outline } => {
            // CSV columns first (a column called "serial" wins), then built-ins.
            data.text = fill_fields(&fill(&data.text, t, row), n, fields);
            *outline = fonts.layout(data);
        }
        ShapeKind::Group(ch) => ch.iter_mut().for_each(|c| fill_shape(c, t, row, n, fields, fonts)),
        _ => {}
    }
}

/// Merge: one copy of the selected template per CSV row, laid out in a grid with `gap` mm.
/// Returns the number of copies created. The template itself is removed.
pub fn merge(doc: &mut Document, sel: &[ShapeId], table: &Table, fields: &Fields, fonts: &FontLibrary, gap: f64, margin: f64) -> Result<usize> {
    if sel.is_empty() {
        bail!("Select the template shapes first");
    }
    if table.rows.is_empty() {
        bail!("The CSV has no data rows");
    }
    let bb = ops::selection_bbox(doc, sel).ok_or_else(|| anyhow::anyhow!("Empty selection"))?;
    let template: Vec<Shape> = doc.page().shapes.iter().filter(|s| sel.contains(&s.id)).cloned().collect();
    doc.page_mut().shapes.retain(|s| !sel.contains(&s.id));
    let base_page = doc.page().clone();
    let (w, h) = (bb.width() + gap, bb.height() + gap);
    let cols = (((base_page.width - 2.0 * margin + gap) / w).floor() as usize).max(1);
    let rows_per_page = (((base_page.height - 2.0 * margin + gap) / h).floor() as usize).max(1);
    let per_page = cols * rows_per_page;
    let start_page = doc.active;
    for (n, row) in table.rows.iter().enumerate() {
        let page_no = n / per_page;
        let slot = n % per_page;
        let target = start_page + page_no;
        if target >= doc.pages.len() {
            let mut p = Page::new(&format!("Merge {}", page_no + 1), base_page.width, base_page.height);
            p.mat = base_page.mat;
            p.layers = base_page.layers.clone();
            p.regmarks = base_page.regmarks.clone();
            doc.pages.push(p);
        }
        let (cx, cy) = ((slot % cols) as f64, (slot / cols) as f64);
        let d = Vec2::new(margin + cx * w - bb.x0, margin + cy * h - bb.y0);
        for t in &template {
            let mut s = t.clone();
            doc.reid(&mut s);
            fill_shape(&mut s, table, row, n, fields, fonts);
            s.transform = Affine::translate(d) * s.transform;
            doc.pages[target].shapes.push(s);
        }
    }
    Ok(table.rows.len())
}

/// Numbered copies without a CSV: `count` copies of the template with the
/// built-in fields (`{{serial}}`, `{{date}}`…) filled in — e.g. numbered tags.
pub fn merge_count(doc: &mut Document, sel: &[ShapeId], count: usize, fields: &Fields, fonts: &FontLibrary, gap: f64, margin: f64) -> Result<usize> {
    if count == 0 {
        bail!("Choose how many copies to make");
    }
    let table = Table { headers: vec![], rows: vec![vec![]; count] };
    merge(doc, sel, &table, fields, fonts, gap, margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_csv() {
        let t = parse_csv("Name,City\r\n\"Smith, Jo\",\"Paris \"\"FR\"\"\"\nAl,Rome\n\n").unwrap();
        assert_eq!(t.headers, vec!["Name", "City"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0], vec!["Smith, Jo", "Paris \"FR\""]);
        assert_eq!(fill("Hi {{Name}} from {{city}}", &t, &t.rows[1]), "Hi Al from Rome");
        let f = Fields { serial_start: 7, increment: 2, date: "2026-10-26".into(), time: "09:30".into() };
        assert_eq!(fill_fields("No. {{serial}} / {{serial:0000}} #{{row}} {{date}} {{time}} {{Name}}", 1, &f), "No. 9 / 0009 #2 2026-10-26 09:30 {{Name}}");
        assert_eq!(fill_fields("{{serial:00}}", 0, &Fields { serial_start: 123, ..Default::default() }), "123", "padding never truncates");
    }

    #[test]
    fn merge_creates_pages() {
        let fonts = FontLibrary::new();
        let mut d = Document::default();
        let data = crate::doc::TextData { text: "{{Name}}".into(), ..Default::default() };
        let id = d.alloc_id();
        d.page_mut().shapes.push(Shape {
            id,
            name: String::new(),
            layer: 0,
            transform: Affine::IDENTITY,
            style: Default::default(),
            kind: ShapeKind::Text { outline: fonts.layout(&data), data },
            locked: false,
            hidden: false,
            effects: Vec::new(),
        });
        // big fake bbox: use a 100x100 rect group instead when no fonts are installed
        let rect = d.add_path(crate::geom::rect_path(kurbo::Rect::new(0.0, 0.0, 140.0, 140.0)), Default::default(), 0);
        let t = Table { headers: vec!["Name".into()], rows: (0..5).map(|i| vec![format!("N{i}")]).collect() };
        let n = merge(&mut d, &[id, rect], &t, &Fields::default(), &fonts, 5.0, 5.0).unwrap();
        assert_eq!(n, 5);
        assert_eq!(d.pages.len(), 2); // 4 per 12" page, 1 on the next
        assert!(placeholders(&d.pages[0].shapes).is_empty());
    }
}
