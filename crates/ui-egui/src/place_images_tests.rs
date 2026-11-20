//! Place image's loaded cursor, driven as a user does: pick several images, then click, drag,
//! Place all or Escape on the canvas. Positions and fills are checked in the engine document.
use crate::layers_drag_tests::Ui;
use crate::services::{ManyContentsFuture, ManyFuture};
use crate::{DialogFuture, Services};
use egui::{Event, Key, Modifiers, Pos2};
use figcraft_color::PaintKind;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

fn png(width: u32, height: u32) -> Vec<u8> {
    figcraft_render::encode_png(&figcraft_render::Pixmap::new(width, height).unwrap()).unwrap()
}

/// The browser: the picker hands over several files' names and contents.
struct Pictures(Vec<(&'static str, Vec<u8>)>);

impl Services for Pictures {
    fn pick_open_many_contents(&mut self, _title: &str, _exts: &[&str]) -> Option<ManyContentsFuture> {
        let all = self.0.iter().map(|(n, b)| Ok((n.to_string(), b.clone()))).collect();
        Some(Box::pin(std::future::ready(Some(all))))
    }
}

/// The desktop: the picker hands over paths.
struct Paths(Vec<std::path::PathBuf>);

impl Services for Paths {
    fn pick_open_many(&mut self, _title: &str, _exts: &[&str]) -> ManyFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
}

/// A platform with only the single-file picker: Place image still loads that one.
struct OnePath(std::path::PathBuf);

impl Services for OnePath {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
}

/// A session with an empty page, three images (40×20, 30×30, 10×50) loaded on the cursor.
fn loaded(s: Session) -> Ui {
    let mut u = Ui::new(s);
    u.app.services = Box::new(Pictures(vec![("Wide.png", png(40, 20)), ("Square.png", png(30, 30)), ("Tall.png", png(10, 50))]));
    u.app.place_dialog();
    u.frame(vec![]);
    u
}

fn left(u: &Ui) -> usize {
    u.app.ui.place.as_ref().map_or(0, |l| l.len())
}

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![
            Event::PointerMoved(p),
            Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
        ]);
    }
    u.frame(vec![]);
}

fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.frame(vec![Event::PointerMoved(from)]);
    u.button(from, true);
    for i in 1..=5 {
        u.frame(vec![Event::PointerMoved(from + (to - from) * i as f32 / 5.0)]);
    }
    u.button(to, false);
    u.frame(vec![]);
}

fn key(u: &mut Ui, k: Key) {
    u.frame(vec![
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
    u.frame(vec![]);
}

/// The image layers on the page, by name.
fn images(u: &Ui) -> Vec<(String, NodeId)> {
    let doc = u.app.session.doc().unwrap();
    doc.nodes()
        .filter(|n| matches!(n.kind, NodeKind::Rectangle) && n.fills.iter().any(|f| matches!(f.kind, PaintKind::Image(_))))
        .map(|n| (n.name.clone(), n.id))
        .collect()
}

fn bounds(u: &Ui, id: NodeId) -> (f64, f64, f64, f64) {
    let b = u.app.session.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0, b.width(), b.height())
}

#[test]
fn picked_images_load_onto_the_cursor_without_placing_anything() {
    let u = loaded(Session::new());
    assert_eq!(left(&u), 3);
    assert!(images(&u).is_empty(), "nothing lands until a click");
}

#[test]
fn each_click_places_the_next_image_at_its_size_with_its_corner_at_the_click() {
    let mut u = loaded(Session::new());
    let p = u.app.to_screen(100.0, 100.0);
    click(&mut u, p);
    assert_eq!(left(&u), 2);
    let placed = images(&u);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].0, "Wide", "named after the file");
    let (x, y, w, h) = bounds(&u, placed[0].1);
    assert!((x - 100.0).abs() < 0.5 && (y - 100.0).abs() < 0.5, "{x}, {y}");
    assert_eq!((w, h), (40.0, 20.0));
    let p = u.app.to_screen(300.0, 100.0);
    click(&mut u, p);
    let p = u.app.to_screen(500.0, 100.0);
    click(&mut u, p);
    assert_eq!(images(&u).iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Wide", "Square", "Tall"]);
    assert!(u.app.ui.place.is_none(), "the cursor is empty once the last is placed");
}

#[test]
fn a_click_on_a_rectangle_fills_it() {
    let mut s = Session::new();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 200, "height": 200})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = loaded(s);
    let p = u.app.to_screen(500.0, 400.0);
    click(&mut u, p);
    let n = u.app.session.doc().unwrap().node(r).unwrap();
    assert!(n.fills.iter().any(|f| matches!(f.kind, PaintKind::Image(_))), "the rectangle took the image");
    assert_eq!(images(&u).len(), 1, "no new layer: the filled rectangle is the only image");
    assert_eq!(left(&u), 2);
}

#[test]
fn a_drag_places_the_image_at_the_dragged_width_in_proportion() {
    let mut u = loaded(Session::new());
    let z = u.app.session.ds().unwrap().view.zoom;
    let from = u.app.to_screen(100.0, 100.0);
    drag(&mut u, from, from + egui::vec2((200.0 * z) as f32, (37.0 * z) as f32));
    let placed = images(&u);
    assert_eq!(placed.len(), 1);
    let (x, y, w, h) = bounds(&u, placed[0].1);
    assert!((x - 100.0).abs() < 0.5 && (y - 100.0).abs() < 0.5, "{x}, {y}");
    assert!((w - 200.0).abs() < 0.5 && (h - 100.0).abs() < 0.5, "40×20 dragged to 200 wide: {w}×{h}");
}

#[test]
fn escape_or_delete_discards_the_rest_and_keeps_the_selection() {
    for k in [Key::Escape, Key::Delete] {
        let mut s = Session::new();
        let r = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
        let mut u = loaded(s);
        key(&mut u, k);
        assert!(u.app.ui.place.is_none(), "{k:?}");
        assert_eq!(u.app.session.selection(), vec![r], "{k:?} discards the images, not the selection");
        assert!(u.app.session.doc().unwrap().node(r).is_some(), "{k:?} deletes no layer");
        assert!(images(&u).is_empty());
    }
}

#[test]
fn place_all_lines_the_rest_up_selected_in_one_undo_step() {
    let mut u = loaded(Session::new());
    let p = u.app.to_screen(100.0, 100.0);
    click(&mut u, p);
    let before = u.app.session.undo_depth().0;
    crate::place_images::place_all(&mut u.app);
    u.frame(vec![]);
    assert!(u.app.ui.place.is_none());
    let placed = images(&u);
    assert_eq!(placed.len(), 3);
    let (square, tall) = (bounds(&u, placed[1].1), bounds(&u, placed[2].1));
    assert_eq!(square.1, tall.1, "top-aligned");
    assert_eq!(tall.0 - (square.0 + square.2), 20.0, "20 apart");
    assert_eq!(u.app.session.selection(), vec![placed[1].1, placed[2].1]);
    assert_eq!(u.app.session.undo_depth().0, before + 1, "one undo step");
    u.app.run("edit.undo", json!({}));
    assert_eq!(images(&u).len(), 1);
}

#[test]
fn the_cursor_shows_a_crosshair_and_how_many_are_left() {
    let mut u = loaded(Session::new());
    let p = u.app.to_screen(200.0, 200.0);
    u.frame(vec![Event::PointerMoved(p)]);
    let out = u.frame_output(vec![Event::PointerMoved(p)]);
    assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Crosshair);
    let texts = texts(&out.shapes);
    assert!(texts.iter().any(|t| t == "3"), "the count badge: {texts:?}");
    assert!(texts.iter().any(|t| t == "3 images to place"), "the banner: {texts:?}");
}

#[test]
fn another_file_in_front_drops_the_loaded_images() {
    let mut u = loaded(Session::new());
    u.app.run("file.new", json!({}));
    u.frame(vec![]);
    assert!(u.app.ui.place.is_none());
}

#[test]
fn dev_mode_drops_the_loaded_images_and_a_click_places_nothing() {
    let mut u = loaded(Session::new());
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let p = u.app.to_screen(100.0, 100.0);
    click(&mut u, p);
    assert!(u.app.ui.place.is_none());
    assert!(images(&u).is_empty());
}

#[test]
fn desktop_paths_load_and_place_and_a_single_file_picker_still_works() {
    let dir = std::env::temp_dir().join(format!("figcraft-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("One.png"), dir.join("Two.png"));
    std::fs::write(&a, png(12, 8)).unwrap();
    std::fs::write(&b, png(8, 8)).unwrap();
    let mut u = Ui::new(Session::new());
    u.app.services = Box::new(Paths(vec![a.clone(), b]));
    u.app.place_dialog();
    u.frame(vec![]);
    assert_eq!(left(&u), 2);
    let p = u.app.to_screen(100.0, 100.0);
    click(&mut u, p);
    assert_eq!(images(&u).first().map(|(n, _)| n.as_str()), Some("One"));
    let mut u = Ui::new(Session::new());
    u.app.services = Box::new(OnePath(a));
    u.app.place_dialog();
    u.frame(vec![]);
    assert_eq!(left(&u), 1);
    let _ = std::fs::remove_dir_all(dir);
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    let mut out = vec![];
    shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}
