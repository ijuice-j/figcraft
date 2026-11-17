//! File covers (fork): the frame a file shows as its picture on the home screen, chosen with
//! Set as thumbnail as in Figma. The choice is kept on the frame (`figcraft.thumbnail` in its
//! plugin data), so it travels with the file; saving draws that frame, cropped to the cards' 16:9,
//! into the file (see [`crate::cmd::file::cover_thumbnail`]).
//!
//! - `object.setThumbnail` makes a frame, component, instance or section the cover (one per file).
//! - `object.removeThumbnail` takes the cover back off: the file shows its automatic picture again.
//! - `file.addCoverPage` adds a first page named Cover with a 1920 × 1080 frame that is the cover
//!   at once: the file's name as a title, a line for a description and a status tag, all ordinary
//!   layers.

use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::targets;
use crate::{Result, Session, bad, spec};

/// The plugin data key marking the cover frame (value `"true"`).
pub const KEY: &str = "figcraft.thumbnail";

/// Whether a layer of this kind can be a file's cover: frames, components and component sets,
/// instances and sections.
pub fn can_be_thumbnail(kind: &NodeKind) -> bool {
    kind.is_frame_like() || matches!(kind, NodeKind::Section { .. })
}

/// Whether `n` is marked as the file's cover (cheap: for badges drawn every frame).
pub fn is_thumbnail(n: &figcraft_doc::Node) -> bool {
    can_be_thumbnail(&n.kind) && n.plugin_data.get(KEY).map(String::as_str) == Some("true")
}

/// The file's cover frame: the layer marked with [`KEY`] that can be one and sits on one of the
/// document's pages (not a library's hidden page).
pub fn thumbnail_frame(doc: &Document) -> Option<NodeId> {
    doc.ids().find(|id| {
        doc.node(*id).is_some_and(|n| can_be_thumbnail(&n.kind) && n.plugin_data.get(KEY).map(String::as_str) == Some("true"))
            && doc.page_of(*id).is_some_and(|p| doc.pages.contains(&p))
    })
}

/// Take every cover mark off `doc`; whether there was one.
fn clear(doc: &mut Document) -> Result<bool> {
    let marked: Vec<NodeId> = doc.ids().filter(|id| doc.node(*id).is_some_and(|n| n.plugin_data.contains_key(KEY))).collect();
    for id in &marked {
        doc.get_mut(*id)?.plugin_data.remove(KEY);
    }
    Ok(!marked.is_empty())
}

fn set_thumbnail(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "object.setThumbnail";
    let ids = targets(s, p);
    let [id] = ids.as_slice() else {
        return Err(bad(CMD, "select one frame to be the file's thumbnail"));
    };
    let doc = s.doc()?;
    let node = doc.node(*id).ok_or_else(|| bad(CMD, format!("no layer {id}")))?;
    if !can_be_thumbnail(&node.kind) {
        return Err(bad(CMD, "only a frame, component, instance or section can be the file's thumbnail"));
    }
    if !doc.page_of(*id).is_some_and(|pg| doc.pages.contains(&pg)) {
        return Err(bad(CMD, "the layer isn't on one of the file's pages"));
    }
    let id = *id;
    let doc = s.doc_mut()?;
    clear(doc)?;
    doc.get_mut(id)?.plugin_data.insert(KEY.into(), "true".into());
    Ok(json!({"id": id}))
}

fn remove_thumbnail(s: &mut Session, _: &Value) -> Result<Value> {
    let removed = clear(s.doc_mut()?)?;
    Ok(json!({"removed": removed}))
}

/// The cover page's look: background, title, description and tag colours.
const BACKGROUND: &str = "#0F3B2E";
const TITLE: &str = "#FFFFFF";
const DESCRIPTION: &str = "#BFE9D8";
const TAG: &str = "#E4F222";
const TAG_TEXT: &str = "#111111";

fn id_of(v: &Value) -> Result<Value> {
    v.get("id").cloned().ok_or_else(|| crate::EngineError::Other("a layer wasn't made".into()))
}

fn add_cover_page(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.addCoverPage";
    let title = match p.get("title") {
        None | Some(Value::Null) => s.doc()?.name.clone(),
        Some(Value::String(t)) if !t.trim().is_empty() => t.trim().chars().take(200).collect(),
        Some(_) => return Err(bad(CMD, "`title` must be text")),
    };
    s.grouped("Add cover page", |s| {
        let page = id_of(&s.execute("page.add", &json!({"name": "Cover"}))?)?;
        s.execute("page.move", &json!({"id": page, "index": 0}))?;
        let frame = id_of(
            &s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 1920, "height": 1080, "name": "Cover", "props": {"fill": BACKGROUND}}))?,
        )?;
        let title_id = id_of(&s.execute(
            "text.create",
            &json!({"text": title, "x": 160, "y": 330, "width": 1600, "parent": frame, "name": "Title",
                    "props": {"fontSize": 144, "fontStyle": "Bold", "fill": TITLE}, "select": false}),
        )?)?;
        s.execute(
            "text.create",
            &json!({"text": "Add a short description of this file", "x": 160, "y": 560, "width": 1600, "parent": frame, "name": "Description",
                    "props": {"fontSize": 48, "fill": DESCRIPTION}, "select": false}),
        )?;
        let tag = id_of(&s.execute(
            "frame.create",
            &json!({"x": 160, "y": 760, "width": 300, "height": 72, "parent": frame, "name": "Status", "select": false,
                    "props": {"fill": TAG, "cornerRadius": 36, "layout": "horizontal", "padding": 20, "sizingH": "hug", "sizingV": "hug"}}),
        )?)?;
        s.execute(
            "text.create",
            &json!({"text": "IN PROGRESS", "x": 0, "y": 0, "parent": tag, "name": "Status text",
                    "props": {"fontSize": 28, "fontStyle": "Bold", "fill": TAG_TEXT, "letterSpacing": 2}, "select": false}),
        )?;
        s.execute("object.setThumbnail", &json!({"id": frame}))?;
        s.execute("select.set", &json!({"ids": [title_id]}))?;
        if let Ok(ds) = s.ds_mut() {
            ds.zoom_to_fit();
        }
        Ok(json!({"page": page, "frame": frame, "title": title_id}))
    })
}

/// The cover commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "object.setThumbnail",
            "Set as thumbnail",
            "{id? (default the selection: one frame, component, instance or section)}  makes it the file's thumbnail, the cover its card shows on the home screen (one per file: another frame stops being it) → {id}",
            true,
            set_thumbnail
        ),
        spec!(
            "object.removeThumbnail",
            "Remove thumbnail",
            "{}  the file has no chosen thumbnail any more: its card shows the automatic picture of its first page → {removed (whether there was one)}",
            true,
            remove_thumbnail
        ),
        spec!(
            "file.addCoverPage",
            "Add cover page",
            "{title? (default the file's name)}  adds a first page named Cover with a 1920 × 1080 frame set as the thumbnail: a title, a description line and a status tag, ordinary layers to edit; one undo step → {page, frame, title (the title's id, selected)}",
            false,
            add_cover_page
        ),
    ]
}

#[cfg(test)]
#[path = "cover_tests.rs"]
mod tests;
