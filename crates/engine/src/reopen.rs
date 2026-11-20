//! Fork: the files open when the desktop app quits open again at its next start, each on the page
//! and at the place on the canvas it was left at, the tab in front in front again. Only saved
//! FigCraft files are kept: a new file or an imported one has no file of its own to open again.
use figcraft_doc::NodeId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Session, ViewState};

/// The most files kept to open again.
pub const MAX_OPEN_FILES: usize = 50;

/// A file open in a tab when the app quit ([`Session::remember_open_files`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenFile {
    pub path: String,
    /// Its tab was the one in front.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    /// The page it showed (its node id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u64>,
    /// The canvas's centre (page units) and zoom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<[f64; 3]>,
}

/// What [`Session::reopen_files`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reopened {
    /// How many files opened.
    pub opened: usize,
    /// The files that couldn't be opened (moved, deleted, damaged).
    pub missing: Vec<String>,
}

/// Reads the kept files: an entry that isn't one is left out rather than costing the other
/// preferences, and at most [`MAX_OPEN_FILES`] are kept.
pub(crate) fn read<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<OpenFile>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().into_iter().flatten().filter_map(|e| OpenFile::deserialize(e).ok()).filter(|f| !f.path.is_empty()).take(MAX_OPEN_FILES).collect())
}

impl Session {
    /// Keep the saved files open in tabs, in tab order, to open again at the next start
    /// ([`Self::reopen_files`]). A tab without a file of its own (a new file, an imported one, a
    /// branch, which opens from its main file) isn't kept.
    pub fn remember_open_files(&mut self) {
        let active = self.active;
        self.prefs.open_files = self
            .docs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.branch_of.is_none())
            .filter_map(|(i, d)| {
                Some(OpenFile { path: d.path.clone()?, active: i == active, page: Some(d.page.0), view: Some([d.view.cx, d.view.cy, d.view.zoom]) })
            })
            .take(MAX_OPEN_FILES)
            .collect();
    }

    /// Open the files kept by [`Self::remember_open_files`] again, each on its page and at its
    /// place on the canvas, and bring the tab that was in front to the front. A file already open
    /// (a recovered copy, say) isn't opened twice. The recent files keep their order. The kept
    /// list stays until the app quits again, so a crash before then opens the same files.
    pub fn reopen_files(&mut self) -> Reopened {
        let files = self.prefs.open_files.clone();
        let recents = self.prefs.recent_files.clone();
        let mut out = Reopened::default();
        let mut front = None;
        for f in files.iter().take(MAX_OPEN_FILES) {
            if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(f.path.as_str())) {
                if f.active {
                    front = Some(i);
                }
                continue;
            }
            match self.execute("file.open", &json!({"path": f.path})) {
                Ok(_) => {
                    out.opened += 1;
                    let i = self.active;
                    if let Some(ds) = self.docs.get_mut(i) {
                        if let Some(page) = f.page.map(NodeId).filter(|p| ds.doc.pages.contains(p)) {
                            ds.page = page;
                        }
                        if let Some([cx, cy, zoom]) = f.view.filter(|v| v.iter().all(|x| x.is_finite())) {
                            ds.view.cx = cx;
                            ds.view.cy = cy;
                            ds.view.zoom = zoom.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM);
                        }
                    }
                    if f.active {
                        front = Some(i);
                    }
                }
                Err(e) => {
                    log::info!("couldn't open {} again: {e}", f.path);
                    out.missing.push(f.path.clone());
                }
            }
        }
        if let Some(i) = front.filter(|i| *i < self.docs.len()) {
            self.active = i;
        }
        self.prefs.recent_files = recents;
        out
    }
}
