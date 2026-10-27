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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutjob::{GroupMode, JobPlan};
    use crate::geom;
    use kurbo::Rect;

    fn design(n: usize) -> Document {
        let mut d = Document::default();
        for i in 0..n {
            d.add_path(geom::rect_path(Rect::new(10.0 + i as f64 * 30.0, 10.0, 30.0 + i as f64 * 30.0, 30.0)), Default::default(), 0);
        }
        d
    }

    #[test]
    fn jobs_become_pages_with_their_copies_and_fresh_ids() {
        let mut a = QueueJob::from_document("Coasters", &design(2));
        a.copies = 3;
        let b = QueueJob::from_document("Tags", &design(1));
        let run = build_run(&[a, b], "cameo5");
        assert_eq!(run.pages.len(), 2);
        assert_eq!((run.pages[0].name.as_str(), run.pages[0].copies), ("Coasters", 3));
        assert_eq!((run.pages[1].name.as_str(), run.pages[1].copies), ("Tags", 1));
        // ids don't clash between jobs
        let mut ids: Vec<_> = run.pages.iter().flat_map(|p| p.shapes.iter().map(|s| s.id)).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        // the Make it screen lays out each job's copies: 2 shapes × 3 + 1 shape
        let mut plan = JobPlan { mode: GroupMode::Simple, ..JobPlan::default() };
        crate::prepare::sync_plan_all(&mut plan, &run);
        let prep = crate::prepare::prepare(&run, &plan, false);
        let items: usize = prep.mats.iter().map(|m| m.items.len()).sum();
        assert_eq!(items, 7);
        assert!(prep.mats.iter().any(|m| m.page == 0) && prep.mats.iter().any(|m| m.page == 1));
    }
}
