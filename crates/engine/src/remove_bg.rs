//! Remove background (`image.removeBackground`) and Restore original (`image.restoreOriginal`).
//!
//! The cut-out is made by a small Cloudflare Worker in the user's own Cloudflare account (its code
//! is [`WORKER_JS`]; [`cloudflare::install`] puts it there from a Cloudflare API key the user
//! pastes). The Worker runs Cloudflare Images' foreground segmentation (BiRefNet) and answers with
//! a PNG the size of the image it was sent. Nothing about the user's account or images reaches
//! FigCraft's own servers: the app talks to the user's Worker directly.
//!
//! The Worker's address and access key are the session's [`Connection`], set by the app from its
//! settings ([`Session::set_remove_bg`]); they are never command parameters, so they never reach the
//! command journal or a recorded macro. Desktop only: the browser build can't call the Worker yet.
//!
//! The request runs on a thread of its own: the command returns `{state: "working", job}` at once
//! and the app runs it again with `{job}` once [`Session::remove_bg_ready`] lists the job, which
//! puts the cut-out in the layer as one undo step. Scripts can pass `wait: true` instead.

use figcraft_color::PaintKind;
use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{boolean, get, opt_index, req_targets};
use crate::{EngineError, Result, Session, bad, spec};

#[cfg(not(target_arch = "wasm32"))]
pub mod cloudflare;
#[cfg(not(target_arch = "wasm32"))]
pub mod net;

/// The Worker that removes backgrounds, as FigCraft installs it in the user's Cloudflare account.
///
/// This is JavaScript because Cloudflare Workers run JavaScript. FigCraft never runs it: the app
/// only uploads this text (`cloudflare::install`), the way it would attach a file. It is the one
/// JavaScript file in the repository, an exception the project owner approved for this feature.
pub const WORKER_JS: &str = include_str!("remove_bg/worker.js");

/// The Worker's name in the user's Cloudflare account.
pub const WORKER_NAME: &str = "figcraft-bg";

/// The largest image the Worker takes (Cloudflare Images' input limit for a Worker).
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// The command id.
const CMD: &str = "image.removeBackground";

/// The name the app keeps the access key under, apart from the preferences (the address is a
/// preference).
pub const SECRET_NAME: &str = "remove-background-key";

/// The user's background remover: its address (`https://figcraft-bg.<name>.workers.dev`) and the
/// access key it checks.
#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    pub address: String,
    pub key: String,
}

impl std::fmt::Debug for Connection {
    // Never print the key (logs, panics, test failures).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").field("address", &self.address).field("key", &"…").finish()
    }
}

impl Connection {
    /// The address as the Worker is reached: trimmed, without a trailing slash. Only `https://`
    /// addresses are taken (and `http://` on this computer, for tests and local Workers).
    pub fn new(address: &str, key: &str) -> std::result::Result<Self, String> {
        let address = address.trim().trim_end_matches('/').to_string();
        let key = key.trim().to_string();
        let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| address.starts_with(p));
        if !(address.starts_with("https://") || local) || address.len() < 12 || address.chars().any(char::is_whitespace) {
            return Err("The address must start with https://".into());
        }
        if address.len() > 300 {
            return Err("The address is too long".into());
        }
        if key.is_empty() {
            return Err("Paste the access key".into());
        }
        if key.len() > 200 || !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err("The access key has characters that can't be part of one".into());
        }
        Ok(Self { address, key })
    }
}

/// The remover the environment names, for the CLI, the MCP server and scripts that have no
/// settings: `FIGCRAFT_REMOVER_URL` and `FIGCRAFT_REMOVER_KEY` (the address and key Preferences ▸
/// AI tools ▸ Copy address and key gives). Used only when no remover is set in the session.
pub fn from_env() -> Option<Connection> {
    if cfg!(test) {
        return None;
    }
    let address = std::env::var("FIGCRAFT_REMOVER_URL").ok()?;
    let key = std::env::var("FIGCRAFT_REMOVER_KEY").ok()?;
    Connection::new(&address, &key).ok()
}

/// How a job ended, once its thread is done.
type Outcome = std::result::Result<Vec<u8>, String>;

/// A background removal under way or finished, not yet put in the file.
struct Job {
    id: u64,
    /// The file it was started on (its uid).
    doc: u64,
    node: NodeId,
    /// The fill it was started on.
    index: usize,
    /// The image key the fill showed when it started.
    image: String,
    #[cfg(not(target_arch = "wasm32"))]
    rx: std::sync::mpsc::Receiver<Outcome>,
    outcome: Option<Outcome>,
}

/// The session's background remover and its jobs.
#[derive(Default)]
pub struct State {
    connection: Option<Connection>,
    jobs: Vec<Job>,
    next: u64,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").field("connection", &self.connection).field("jobs", &self.jobs.len()).finish()
    }
}

/// A layer whose background is being removed: for the canvas's "Removing background" label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Working {
    pub job: u64,
    pub doc: u64,
    pub node: NodeId,
}

impl Session {
    /// Set (or clear) the background remover the app's settings name.
    pub fn set_remove_bg(&mut self, connection: Option<Connection>) {
        self.remove_bg.connection = connection;
    }

    /// The background remover set, if any.
    pub fn remove_bg(&self) -> Option<&Connection> {
        self.remove_bg.connection.as_ref()
    }

    /// The removals still waiting for the Worker.
    pub fn remove_bg_working(&self) -> Vec<Working> {
        self.remove_bg.jobs.iter().filter(|j| j.outcome.is_none()).map(|j| Working { job: j.id, doc: j.doc, node: j.node }).collect()
    }

    /// The jobs that finished on the open file, for the app to run `image.removeBackground {job}`
    /// on. Jobs whose file was closed are dropped.
    pub fn remove_bg_ready(&mut self) -> Vec<u64> {
        let open: Vec<u64> = self.docs.iter().map(|d| d.uid).collect();
        self.remove_bg.jobs.retain(|j| open.contains(&j.doc));
        #[cfg(not(target_arch = "wasm32"))]
        for j in &mut self.remove_bg.jobs {
            if j.outcome.is_none() {
                match j.rx.try_recv() {
                    Ok(outcome) => j.outcome = Some(outcome),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => j.outcome = Some(Err("the request stopped unexpectedly".into())),
                }
            }
        }
        let active = self.ds().ok().map(|d| d.uid);
        self.remove_bg.jobs.iter().filter(|j| j.outcome.is_some() && Some(j.doc) == active).map(|j| j.id).collect()
    }

    /// Stop waiting for `job` (Cancel): its answer is dropped, nothing in the file changes.
    pub fn remove_bg_cancel(&mut self, job: u64) -> bool {
        let before = self.remove_bg.jobs.len();
        self.remove_bg.jobs.retain(|j| j.id != job);
        self.remove_bg.jobs.len() != before
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.removeBackground",
            "Remove background",
            "{ids? (one layer), index? (an image fill; default its top visible one), wait? (true: wait for the answer and apply it), job? (apply a finished job)}  cuts the subject out of the layer's image with the background remover connected in Preferences ▸ AI tools (the user's own Cloudflare Worker; desktop only); the picture keeps its size, the layer its place, and the original is kept for image.restoreOriginal → {state: working, job} while the Worker works (run again with {job} once done), or {state: done, id, index, image, width, height}",
            true,
            remove_background
        ),
        spec!(
            "image.restoreOriginal",
            "Restore original image",
            "{ids? (one layer), index? (an image fill; default its top one with an original)}  puts back the image a fill had before Remove background → {id, index, image}",
            true,
            restore_original
        ),
    ]
}

/// The layer and image fill a command acts on: one layer, `index` or its top visible image fill.
fn target(cmd: &str, s: &Session, p: &Value) -> Result<(NodeId, usize, String)> {
    let refuse = |why: &str| EngineError::Disabled(cmd.into(), why.into());
    let ids = req_targets(cmd, s, p)?;
    let [id] = ids[..] else { return Err(refuse("select one layer with an image")) };
    let n = s.doc()?.get(id)?;
    if matches!(n.kind, NodeKind::Text { .. }) {
        return Err(refuse("text can't have its background removed: select a layer filled with an image"));
    }
    let index = match opt_index(cmd, p, "index")? {
        Some(i) => i,
        None => {
            n.fills.iter().rposition(|f| f.visible && matches!(f.kind, PaintKind::Image(_))).ok_or_else(|| refuse("the layer has no image fill"))?
        }
    };
    let Some(PaintKind::Image(img)) = n.fills.get(index).map(|f| &f.kind) else { return Err(bad(cmd, "no image fill at that index")) };
    Ok((id, index, img.image.clone()))
}

fn remove_background(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(job) = get(p, "job").filter(|v| !v.is_null()) {
        let job = job.as_u64().ok_or_else(|| bad(CMD, "`job` must be a job number"))?;
        return finish(s, job, true);
    }
    let (id, index, image) = target(CMD, s, p)?;
    if cfg!(target_arch = "wasm32") {
        return Err(EngineError::Disabled(CMD.into(), "Remove background isn't available in the browser yet: use the desktop app".into()));
    }
    let doc_uid = s.ds()?.uid;
    if let Some(j) = s.remove_bg.jobs.iter().find(|j| j.doc == doc_uid && j.node == id && j.index == index && j.outcome.is_none()) {
        return Ok(json!({"state": "working", "job": j.id}));
    }
    let bytes = s.doc()?.images.get(&image).map(|d| d.bytes.clone()).ok_or_else(|| bad(CMD, "the image's data is missing"))?;
    if bytes.len() > MAX_BYTES {
        return Err(EngineError::Disabled(CMD.into(), "the image is over 20 MB, the most the background remover takes".into()));
    }
    let Some(connection) = s.remove_bg.connection.clone().or_else(from_env) else {
        s.request_ui(CMD, json!({"setup": true}));
        return Err(EngineError::Disabled(CMD.into(), "connect a background remover first, in Preferences under AI tools".into()));
    };
    s.remove_bg.next += 1;
    let job_id = s.remove_bg.next;
    start(s, Job::new(job_id, doc_uid, id, index, image), connection, bytes)?;
    if boolean(p, "wait") == Some(true) {
        return finish(s, job_id, false);
    }
    Ok(json!({"state": "working", "job": job_id}))
}

#[cfg(not(target_arch = "wasm32"))]
impl Job {
    fn new(id: u64, doc: u64, node: NodeId, index: usize, image: String) -> (Self, std::sync::mpsc::Sender<Outcome>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { id, doc, node, index, image, rx, outcome: None }, tx)
    }
}

#[cfg(target_arch = "wasm32")]
impl Job {
    fn new(id: u64, doc: u64, node: NodeId, index: usize, image: String) -> (Self, ()) {
        (Self { id, doc, node, index, image, outcome: None }, ())
    }
}

/// Send the image to the Worker on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start(s: &mut Session, (job, tx): (Job, std::sync::mpsc::Sender<Outcome>), connection: Connection, bytes: std::sync::Arc<Vec<u8>>) -> Result<()> {
    let spawned = std::thread::Builder::new().name("figcraft-remove-background".into()).spawn(move || {
        let _ = tx.send(net::remove(&connection, &bytes));
    });
    if let Err(e) = spawned {
        return Err(EngineError::Other(format!("couldn't start removing the background: {e}")));
    }
    s.remove_bg.jobs.push(job);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn start(_: &mut Session, _: (Job, ()), _: Connection, _: std::sync::Arc<Vec<u8>>) -> Result<()> {
    Err(EngineError::Disabled(CMD.into(), "Remove background isn't available in the browser yet".into()))
}

/// Put job `id`'s cut-out in its layer (`waiting`: while it runs, say so; else wait for it).
fn finish(s: &mut Session, id: u64, waiting: bool) -> Result<Value> {
    let at =
        s.remove_bg.jobs.iter().position(|j| j.id == id).ok_or_else(|| bad(CMD, format!("no background removal {id} (finished or cancelled)")))?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(j) = s.remove_bg.jobs.get_mut(at)
        && j.outcome.is_none()
    {
        if waiting {
            match j.rx.try_recv() {
                Ok(outcome) => j.outcome = Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(json!({"state": "working", "job": id})),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => j.outcome = Some(Err("the request stopped unexpectedly".into())),
            }
        } else {
            j.outcome = Some(j.rx.recv().unwrap_or_else(|_| Err("the request stopped unexpectedly".into())));
        }
    }
    let mut job = s.remove_bg.jobs.remove(at);
    if s.ds()?.uid != job.doc {
        // Not the file it started on: keep it for when that file is in front again.
        s.remove_bg.jobs.push(job);
        return Err(EngineError::Disabled(CMD.into(), "the background was removed in another file: switch to it".into()));
    }
    let png = match job.outcome.take() {
        Some(Ok(png)) => png,
        Some(Err(why)) => return Err(EngineError::Other(why)),
        None => return Err(EngineError::Other("the background remover didn't answer".into())),
    };
    apply(s, &job, png)
}

/// The cut-out goes in place of the image the fill showed when the job started.
fn apply(s: &mut Session, job: &Job, png: Vec<u8>) -> Result<Value> {
    let doc = s.doc_mut()?;
    let node = doc.node(job.node).ok_or_else(|| EngineError::Other("the layer was deleted while its background was being removed".into()))?;
    let showing = |i: usize| node.fills.get(i).is_some_and(|f| matches!(&f.kind, PaintKind::Image(img) if img.image == job.image));
    let index = if showing(job.index) { Some(job.index) } else { (0..node.fills.len()).rev().find(|i| showing(*i)) }
        .ok_or_else(|| EngineError::Other("the image changed while its background was being removed, so nothing was changed".into()))?;
    let (image, width, height) = crate::cmd::create::add_readable_image(doc, png)
        .map_err(|_| EngineError::Other("the background remover sent back something that isn't an image".into()))?;
    let has_subject = doc
        .images
        .get(&image)
        .and_then(|data| figcraft_render::images::get(&image, Some(data), &figcraft_color::ImageFilters::default(), doc.color_profile))
        .is_some_and(|px| px.pixels().iter().any(|p| p.alpha() > 0));
    if !has_subject {
        return Err(EngineError::Other("no clear subject was found, so nothing was changed".into()));
    }
    let n = doc.get_mut(job.node)?;
    if let Some(PaintKind::Image(img)) = n.fills.get_mut(index).map(|f| &mut f.kind) {
        // Removing it again keeps the first original: Restore original goes back to the photo.
        let original = img.original.take().unwrap_or_else(|| img.image.clone());
        img.image.clone_from(&image);
        img.original = Some(original);
    }
    // Like any paint edit, the layer leaves its paint style.
    n.styles.fill = None;
    Ok(json!({"state": "done", "id": job.node, "index": index, "image": image, "width": width, "height": height}))
}

fn restore_original(s: &mut Session, p: &Value) -> Result<Value> {
    const RESTORE: &str = "image.restoreOriginal";
    let refuse = |why: &str| EngineError::Disabled(RESTORE.into(), why.into());
    let ids = req_targets(RESTORE, s, p)?;
    let [id] = ids[..] else { return Err(refuse("select one layer")) };
    let doc = s.doc()?;
    let n = doc.get(id)?;
    let has_original = |i: usize| matches!(n.fills.get(i).map(|f| &f.kind), Some(PaintKind::Image(img)) if img.original.is_some());
    let index = match opt_index(RESTORE, p, "index")? {
        Some(i) if has_original(i) => i,
        Some(_) => return Err(refuse("that fill has no original to restore")),
        None => (0..n.fills.len()).rev().find(|i| has_original(*i)).ok_or_else(|| refuse("the layer has no image with its background removed"))?,
    };
    let original = match n.fills.get(index).map(|f| &f.kind) {
        Some(PaintKind::Image(img)) => img.original.clone().unwrap_or_default(),
        _ => String::new(),
    };
    if !doc.images.contains_key(&original) {
        return Err(refuse("the original image isn't in this file anymore"));
    }
    let n = s.doc_mut()?.get_mut(id)?;
    if let Some(PaintKind::Image(img)) = n.fills.get_mut(index).map(|f| &mut f.kind) {
        img.image.clone_from(&original);
        img.original = None;
    }
    n.styles.fill = None;
    Ok(json!({"id": id, "index": index, "image": original}))
}

/// Whether the layer `id` has an image fill whose background was removed (the Fill panel's
/// Restore original).
pub fn has_original(s: &Session, id: NodeId) -> bool {
    s.doc().ok().and_then(|d| d.node(id)).is_some_and(|n| n.fills.iter().any(|f| matches!(&f.kind, PaintKind::Image(img) if img.original.is_some())))
}

#[cfg(test)]
#[path = "remove_bg/tests.rs"]
mod tests;
