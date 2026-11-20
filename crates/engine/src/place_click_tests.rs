//! `image.place` with `click`: where a click with Place image's loaded cursor puts an image (the
//! documented flow: "Select an existing object on the canvas to replace its fill", else "Place the
//! image ... in a new layer, using its original dimensions").
use base64::Engine as _;
use figcraft_color::PaintKind;
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;

fn png(width: u32, height: u32) -> String {
    let bytes = figcraft_render::encode_png(&figcraft_render::Pixmap::new(width, height).unwrap()).unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn click(s: &mut Session, x: f64, y: f64, extra: Value) -> crate::Result<Value> {
    let mut p = json!({"data": png(40, 20), "x": x, "y": y, "click": true, "name": "Photo"});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s.execute("image.place", &p)
}

fn image_fills(s: &Session, n: NodeId) -> usize {
    s.doc().unwrap().node(n).unwrap().fills.iter().filter(|f| matches!(f.kind, PaintKind::Image(_))).count()
}

#[test]
fn a_click_on_empty_canvas_places_a_layer_at_its_size_with_its_corner_at_the_point() {
    let mut s = Session::new();
    let placed = id(&click(&mut s, 300.0, 200.0, json!({})).unwrap());
    let doc = s.doc().unwrap();
    let n = doc.node(placed).unwrap();
    assert_eq!(n.parent, Some(s.page().unwrap()));
    assert_eq!((n.width, n.height), (40.0, 20.0), "its original size");
    let b = doc.world_bounds(placed).unwrap();
    assert_eq!((b.x0, b.y0), (300.0, 200.0));
    assert_eq!(n.name, "Photo");
    assert_eq!(s.selection(), vec![placed]);
}

#[test]
fn a_click_on_a_shape_fills_it_instead_of_making_a_layer() {
    let mut s = Session::new();
    for shape in ["shape.rectangle", "shape.ellipse", "shape.star"] {
        let r = id(&s.execute(shape, &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
        let layers = s.doc().unwrap().nodes().count();
        let v = click(&mut s, 50.0, 50.0, json!({})).unwrap();
        assert_eq!(id(&v), r, "{shape}: the result names the filled layer");
        assert_eq!(s.doc().unwrap().nodes().count(), layers, "{shape}: no new layer");
        assert_eq!(image_fills(&s, r), 1, "{shape}");
        s.execute("edit.delete", &json!({"ids": [r]})).unwrap();
    }
}

#[test]
fn a_click_on_a_shape_inside_a_group_fills_that_shape() {
    let mut s = Session::new();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 80, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("object.group", &json!({"ids": [a, b]})).unwrap();
    click(&mut s, 100.0, 25.0, json!({})).unwrap();
    assert_eq!((image_fills(&s, a), image_fills(&s, b)), (0, 1));
}

#[test]
fn a_click_inside_a_frame_places_a_layer_in_it_at_the_point() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 100, "y": 100, "width": 400, "height": 300})).unwrap());
    let placed = id(&click(&mut s, 150.0, 130.0, json!({})).unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(placed).unwrap().parent, Some(f), "in the frame, not filling it");
    assert_eq!(image_fills(&s, f), 0);
    let b = doc.world_bounds(placed).unwrap();
    assert_eq!((b.x0, b.y0), (150.0, 130.0), "where the click was, on the page");
}

#[test]
fn a_drag_over_a_shape_makes_a_layer_of_the_dragged_width_keeping_the_proportion() {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap());
    let placed = id(&click(&mut s, 10.0, 10.0, json!({"width": 120})).unwrap());
    assert_ne!(placed, r);
    assert_eq!(image_fills(&s, r), 0, "a drag never fills");
    let n = s.doc().unwrap().node(placed).unwrap();
    assert_eq!((n.width, n.height), (120.0, 60.0));
}

#[test]
fn text_and_lines_are_not_filled() {
    let mut s = Session::new();
    let t = id(&s.execute("text.create", &json!({"text": "Hello there", "x": 0, "y": 0, "props": {"fontSize": 40}})).unwrap());
    let before = s.doc().unwrap().node(t).unwrap().fills.len();
    let placed = id(&click(&mut s, 5.0, 10.0, json!({})).unwrap());
    assert_ne!(placed, t);
    assert_eq!(s.doc().unwrap().node(t).unwrap().fills.len(), before);
}

#[test]
fn click_refuses_a_target_or_parent_and_junk_points() {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    let err = click(&mut s, 10.0, 10.0, json!({"target": r.0})).unwrap_err().to_string();
    assert!(err.contains("leave `target` and `parent` out"), "{err}");
    assert!(click(&mut s, 10.0, 10.0, json!({"parent": r.0})).is_err());
    let layers = s.doc().unwrap().nodes().count();
    for junk in [json!("x"), json!(f64::MAX), json!(null)] {
        let _ = s.execute("image.place", &json!({"data": png(4, 4), "x": junk, "y": 0, "click": true}));
    }
    assert!(s.doc().unwrap().nodes().count() <= layers + 3, "no more than one layer per call");
}

#[test]
fn a_click_is_one_undo_step() {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    click(&mut s, 10.0, 10.0, json!({})).unwrap();
    assert_eq!(image_fills(&s, r), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(image_fills(&s, r), 0);
}
