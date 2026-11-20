//! Fork: the files open when the app quits open again at the next start ([`crate::reopen`]).
use crate::reopen::{MAX_OPEN_FILES, OpenFile};
use crate::{Prefs, Session, ViewState};
use figcraft_doc::NodeId;
use serde_json::json;
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-reopen-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A saved file at `path` with two pages, left showing the second.
fn save_two_pages(s: &mut Session, path: &str) -> NodeId {
    s.execute("file.new", &json!({})).unwrap();
    let second = NodeId(s.execute("page.add", &json!({"name": "Second"})).unwrap()["id"].as_u64().unwrap());
    s.execute("file.save", &json!({"path": path})).unwrap();
    second
}

/// A first run's tabs: `a` and `b` saved, an untitled file in between, `b` in front and scrolled.
fn quit_with_tabs(d: &std::path::Path) -> (Session, String, String, NodeId) {
    let (a, b) = (d.join("A.figcraft").to_string_lossy().to_string(), d.join("B.figcraft").to_string_lossy().to_string());
    let mut s = Session::empty();
    save_two_pages(&mut s, &a);
    s.execute("file.new", &json!({})).unwrap();
    let page = save_two_pages(&mut s, &b);
    let ds = s.ds_mut().unwrap();
    ds.view.cx = 640.0;
    ds.view.cy = -120.0;
    ds.view.zoom = 2.5;
    (s, a, b, page)
}

#[test]
fn quitting_keeps_the_saved_tabs_in_order_and_which_was_in_front() {
    let d = dir("keep");
    let (mut s, a, b, page) = quit_with_tabs(&d);
    s.remember_open_files();
    let kept = &s.prefs.open_files;
    assert_eq!(kept.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), [a.as_str(), b.as_str()], "the untitled tab isn't kept");
    assert!(!kept[0].active && kept[1].active, "B was in front");
    assert_eq!(kept[1].page, Some(page.0));
    assert_eq!(kept[1].view, Some([640.0, -120.0, 2.5]));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_next_start_opens_them_again_where_they_were_left() {
    let d = dir("open");
    let (mut s, a, b, page) = quit_with_tabs(&d);
    s.remember_open_files();
    let recents = s.prefs.recent_files.clone();
    let mut next = Session::empty();
    next.prefs = s.prefs.clone();
    let done = next.reopen_files();
    assert_eq!(done.opened, 2);
    assert!(done.missing.is_empty());
    assert_eq!(next.docs.iter().map(|d| d.path.clone().unwrap()).collect::<Vec<_>>(), [a, b.clone()], "in tab order");
    assert_eq!(next.ds().unwrap().path.as_deref(), Some(b.as_str()), "B in front again");
    let ds = next.ds().unwrap();
    assert_eq!(ds.page, page, "on the page it was left on");
    assert_eq!((ds.view.cx, ds.view.cy, ds.view.zoom), (640.0, -120.0, 2.5), "at the same place and zoom");
    assert_eq!(next.prefs.recent_files, recents, "opening them again doesn't reorder the recent files");
    assert_eq!(next.prefs.open_files.len(), 2, "kept until the next quit, in case the app crashes first");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_file_moved_or_deleted_since_is_skipped_and_named() {
    let d = dir("missing");
    let (mut s, a, b, _) = quit_with_tabs(&d);
    s.remember_open_files();
    std::fs::remove_file(&b).unwrap();
    let mut next = Session::empty();
    next.prefs = s.prefs.clone();
    let done = next.reopen_files();
    assert_eq!(done.opened, 1);
    assert_eq!(done.missing, [b]);
    assert_eq!(next.docs.len(), 1);
    assert_eq!(next.ds().unwrap().path.as_deref(), Some(a.as_str()));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_file_already_open_is_not_opened_twice_but_comes_to_the_front() {
    let d = dir("twice");
    let (mut s, a, b, _) = quit_with_tabs(&d);
    s.remember_open_files();
    let mut next = Session::empty();
    next.prefs = s.prefs.clone();
    // B came back first (a recovered copy, say).
    next.execute("file.open", &json!({"path": b})).unwrap();
    let done = next.reopen_files();
    assert_eq!(done.opened, 1, "only A opens");
    assert_eq!(next.docs.len(), 2);
    assert_eq!(next.docs[1].path.as_deref(), Some(a.as_str()));
    assert_eq!(next.ds().unwrap().path.as_deref(), Some(b.as_str()), "B was in front");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn nothing_kept_opens_nothing() {
    let mut s = Session::empty();
    s.execute("file.new", &json!({})).unwrap();
    s.remember_open_files();
    assert!(s.prefs.open_files.is_empty(), "an untitled file has nothing to open again");
    let mut next = Session::empty();
    next.prefs = s.prefs.clone();
    assert_eq!(next.reopen_files().opened, 0);
    assert!(next.docs.is_empty(), "the app shows the Home screen");
}

#[test]
fn kept_files_survive_the_settings_file_and_junk_in_it_costs_nothing_else() {
    let p = Prefs {
        open_files: vec![OpenFile { path: "/x/A.figcraft".into(), active: true, page: Some(7), view: Some([1.0, 2.0, 3.0]) }],
        ..Prefs::default()
    };
    let back: Prefs = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back.open_files, p.open_files);
    let junk: Prefs = serde_json::from_value(json!({
        "nudge_big": 25.0,
        "open_files": [{"path": "/x/B.figcraft", "view": "far"}, {"path": ""}, 12, {"path": "/x/C.figcraft", "page": 3}],
    }))
    .unwrap();
    assert_eq!(junk.nudge_big, 25.0, "the other preferences still read");
    assert_eq!(junk.open_files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["/x/C.figcraft"]);
    let many: Prefs =
        serde_json::from_value(json!({"open_files": (0..200).map(|i| json!({"path": format!("/x/{i}.figcraft")})).collect::<Vec<_>>()})).unwrap();
    assert_eq!(many.open_files.len(), MAX_OPEN_FILES);
    let not_a_list: Prefs = serde_json::from_value(json!({"open_files": "nope", "nudge_big": 4.0})).unwrap();
    assert!(not_a_list.open_files.is_empty());
    assert_eq!(not_a_list.nudge_big, 4.0);
}

#[test]
fn a_kept_zoom_out_of_range_or_a_page_since_deleted_is_ignored() {
    let d = dir("range");
    let path = d.join("R.figcraft").to_string_lossy().to_string();
    let mut s = Session::empty();
    save_two_pages(&mut s, &path);
    let mut next = Session::empty();
    next.prefs.open_files = vec![OpenFile { path: path.clone(), active: true, page: Some(999_999), view: Some([0.0, 0.0, 1e9]) }];
    next.reopen_files();
    let ds = next.ds().unwrap();
    assert_eq!(ds.view.zoom, ViewState::MAX_ZOOM);
    assert!(ds.doc.pages.contains(&ds.page), "a page that's gone leaves the file's own");
    let _ = std::fs::remove_dir_all(d);
}
