use std::io::Cursor;

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::{KEY, thumbnail_frame};
use crate::{EngineError, Session};

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn frame(s: &mut Session, x: f64, w: f64, h: f64) -> NodeId {
    id(&s.execute("frame.create", &json!({"x": x, "y": 0, "width": w, "height": h, "props": {"fill": "#2266CC"}})).unwrap())
}

fn bad(r: Result<Value, EngineError>, needle: &str) {
    match r {
        Err(EngineError::BadParams { msg, .. }) => assert!(msg.contains(needle), "{msg} should say {needle}"),
        other => panic!("expected a refusal saying {needle}, got {other:?}"),
    }
}

#[test]
fn one_frame_is_the_thumbnail_and_it_can_be_moved_removed_and_undone() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let a = frame(&mut s, 0.0, 400.0, 300.0);
    let b = frame(&mut s, 500.0, 400.0, 300.0);
    s.execute("object.setThumbnail", &json!({"id": a})).unwrap();
    assert_eq!(thumbnail_frame(s.doc().unwrap()), Some(a));
    s.execute("object.setThumbnail", &json!({"id": b})).unwrap();
    assert_eq!(thumbnail_frame(s.doc().unwrap()), Some(b), "another frame takes it over");
    assert!(!s.doc().unwrap().node(a).unwrap().plugin_data.contains_key(KEY));

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(thumbnail_frame(s.doc().unwrap()), Some(a), "one undo step");
    let removed = s.execute("object.removeThumbnail", &json!({})).unwrap();
    assert_eq!(removed["removed"], true);
    assert_eq!(thumbnail_frame(s.doc().unwrap()), None);
    assert_eq!(s.execute("object.removeThumbnail", &json!({})).unwrap()["removed"], false);

    // Only frames and their kin, one at a time.
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 500, "width": 10, "height": 10})).unwrap());
    bad(s.execute("object.setThumbnail", &json!({"id": r})), "only a frame");
    bad(s.execute("object.setThumbnail", &json!({"ids": [a, b]})), "select one frame");
    s.execute("select.set", &json!({"ids": []})).unwrap();
    bad(s.execute("object.setThumbnail", &json!({})), "select one frame");
    let doc = s.doc_mut().unwrap();
    let page = doc.first_page().unwrap();
    let section = doc.create(page, figcraft_doc::NodeKind::Section { contents_hidden: false }, "Section", 0.0, 900.0, 800.0, 400.0).unwrap();
    s.execute("object.setThumbnail", &json!({"id": section})).unwrap();
    assert_eq!(thumbnail_frame(s.doc().unwrap()), Some(section), "a section can be the cover");
}

#[test]
fn a_saved_cover_is_cropped_to_the_cards_and_marked_while_the_automatic_picture_is_not() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let tall = frame(&mut s, 0.0, 800.0, 1000.0);
    let bytes = crate::cmd::file::file_bytes(s.ds().unwrap()).unwrap();
    let auto = figcraft_format::thumbnail::archive_preview(Cursor::new(&bytes), 2000);
    assert!(auto.picture.is_some() && !auto.cover, "no cover chosen: the automatic picture");

    s.execute("object.setThumbnail", &json!({"id": tall})).unwrap();
    let bytes = crate::cmd::file::file_bytes(s.ds().unwrap()).unwrap();
    let preview = figcraft_format::thumbnail::archive_preview(Cursor::new(&bytes), 2000);
    assert!(preview.cover, "marked as the cover");
    let pic = preview.picture.unwrap();
    assert_eq!((pic.width, pic.height), (crate::cmd::file::COVER_W, 405), "16:9 from the middle of a tall frame");
    // The frame's own fill everywhere: the crop stays inside it.
    let px = |x: u32, y: u32| &pic.rgba[((y * pic.width + x) * 4) as usize..][..3];
    assert_eq!(px(0, 0), [0x22, 0x66, 0xCC]);
    assert_eq!(px(pic.width - 1, pic.height - 1), [0x22, 0x66, 0xCC]);

    // A small frame is drawn larger, up to four times its size.
    let small = frame(&mut s, 2000.0, 160.0, 90.0);
    s.execute("object.setThumbnail", &json!({"id": small})).unwrap();
    let png = crate::cmd::file::cover_thumbnail(s.doc().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (640, 360));
}

#[test]
fn a_cover_page_comes_first_with_a_thumbnail_frame_titled_after_the_file_in_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Checkout flow"})).unwrap();
    let pages_before = s.doc().unwrap().pages.len();
    let made = s.execute("file.addCoverPage", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.pages.len(), pages_before + 1);
    let first = doc.pages[0];
    assert_eq!(doc.node(first).unwrap().name, "Cover");
    let cover = id(&json!({"id": made["frame"]}));
    let node = doc.node(cover).unwrap();
    assert_eq!((node.width, node.height), (1920.0, 1080.0));
    assert_eq!(doc.page_of(cover), Some(first));
    assert_eq!(thumbnail_frame(doc), Some(cover));
    let title = id(&json!({"id": made["title"]}));
    match &doc.node(title).unwrap().kind {
        figcraft_doc::NodeKind::Text(t) => assert_eq!(t.characters, "Checkout flow"),
        other => panic!("the title is text, not {other:?}"),
    }
    assert_eq!(s.selection(), vec![title], "the title is selected, ready to edit");

    s.execute("edit.undo", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.pages.len(), pages_before, "one undo step takes it all away");
    assert_eq!(thumbnail_frame(doc), None);
    bad(s.execute("file.addCoverPage", &json!({"title": 5})), "must be text");
}
