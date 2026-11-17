//! File covers in the editor (fork): Set as thumbnail and Remove thumbnail on a frame's right-click
//! menu, and the Design panel's Thumbnail section on the cover.

use egui::Pos2;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::present_tests::id;

fn cover(u: &Ui) -> Option<figcraft_doc::NodeId> {
    figcraft_engine::cover::thumbnail_frame(u.app.session.doc().unwrap())
}

fn menu_on(u: &mut Ui, f: figcraft_doc::NodeId) -> Vec<egui::Shape> {
    u.app.run("select.set", json!({"ids": [f]}));
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![f]));
    u.frame(vec![]);
    flat(&u.frame_output(vec![]).shapes)
}

#[test]
fn a_frames_menu_sets_it_as_the_thumbnail_and_then_offers_to_remove_it() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 320, "height": 180, "name": "Cover"})).unwrap());
    let r = id(&s.execute("shape.rectangle", &json!({"x": 400, "y": 0, "width": 50, "height": 50})).unwrap());
    let mut u = Ui::new(s);
    let shapes = menu_on(&mut u, f);
    assert!(has_text(&shapes, "Set as thumbnail") && !has_text(&shapes, "Remove thumbnail"));
    let at = text_center(&shapes, "Set as thumbnail");
    u.button(at, true);
    u.button(at, false);
    assert_eq!(cover(&u), Some(f));

    let shapes = menu_on(&mut u, f);
    assert!(has_text(&shapes, "Remove thumbnail") && !has_text(&shapes, "Set as thumbnail"), "the cover offers to remove it");
    let at = text_center(&shapes, "Remove thumbnail");
    u.button(at, true);
    u.button(at, false);
    assert_eq!(cover(&u), None);

    let shapes = menu_on(&mut u, r);
    assert!(!has_text(&shapes, "Set as thumbnail"), "a rectangle can't be a cover");
}

#[test]
fn the_cover_shows_a_thumbnail_section_that_removes_it() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 320, "height": 180, "name": "Cover"})).unwrap());
    let other = id(&s.execute("frame.create", &json!({"x": 400, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("object.setThumbnail", &json!({"id": f})).unwrap();
    let mut u = Ui::new(s);
    u.app.run("select.set", json!({"ids": [other]}));
    u.frame(vec![]);
    assert!(!has_text(&flat(&u.frame_output(vec![]).shapes), "Thumbnail"), "only on the cover");

    u.app.run("select.set", json!({"ids": [f]}));
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Thumbnail"));
    let at = text_center(&shapes, "Remove thumbnail");
    u.button(at, true);
    u.button(at, false);
    assert_eq!(cover(&u), None);
    u.frame(vec![]);
    assert!(!has_text(&flat(&u.frame_output(vec![]).shapes), "Remove thumbnail"), "the section goes with it");
}
