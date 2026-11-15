//! The production queue: several designs (files, or the design open now),
//! each with its own number of copies, put together as one production run -
//! a document with a page per job, which the Make it screen splits onto mats
//! and cuts one after another.

use crate::doc::{Document, Page};
use anyhow::{Result, bail};
use std::path::Path;
use std::sync::Arc;

/// One design in the queue.
#[derive(Clone, Debug)]
pub struct QueueJob {
    pub name: String,
    /// Its pages, as they are in the design.
    pub pages: Vec<Page>,
    /// How many of it to cut.
    pub copies: u32,
}

impl QueueJob {
    /// A job from the design open now.
    pub fn from_document(name: &str, doc: &Document) -> QueueJob {
        QueueJob { name: name.to_string(), pages: doc.pages.clone(), copies: 1 }
    }

    /// A job from a file: a CutCraft design keeps its pages; anything else
    /// CutCraft imports (SVG, DXF, PDF, FCM…) goes on one page of `size`
    /// (width, height mm) with the given mat.
    pub fn from_file(path: &Path, fonts: Option<Arc<fontdb::Database>>, like: &Page) -> Result<QueueJob> {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Design".into());
        if crate::io::kind_of(path) == crate::io::FileKind::Native {
            let doc = crate::io::load_native(path)?;
            return Ok(QueueJob { name, pages: doc.pages, copies: 1 });
        }
        let (shapes, _) = crate::io::import_with_notes(path, fonts)?;
        if shapes.is_empty() {
            bail!("there's nothing to cut in {name}");
        }
        let mut page = Page::new(&name, like.width, like.height);
        page.mat = like.mat;
        page.shapes = shapes;
        Ok(QueueJob { name, pages: vec![page], copies: 1 })
    }

    /// Shapes in the job (for a quick summary).
    pub fn shape_count(&self) -> usize {
        self.pages.iter().map(|p| p.shapes.len()).sum()
    }
}

/// The jobs as one production run: a page per job page, named after the job,
/// each set to the job's copies, every shape with an id of its own.
pub fn build_run(jobs: &[QueueJob], machine: &str) -> Document {
    let mut doc = Document { pages: vec![], active: 0, next_id: 1, machine: machine.to_string(), params: Default::default() };
    for job in jobs {
        let many = job.pages.len() > 1;
        for p in &job.pages {
            let mut page = p.clone();
            page.name = if many { format!("{} - {}", job.name, p.name) } else { job.name.clone() };
            page.copies = job.copies.max(1);
            for s in &mut page.shapes {
                doc.reid(s);
            }
            doc.pages.push(page);
        }
    }
    if doc.pages.is_empty() {
        doc.pages.push(Page::new("Page 1", 304.8, 304.8));
    }
    doc
}
