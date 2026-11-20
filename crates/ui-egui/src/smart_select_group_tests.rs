//! Smart-selection reorder rings for layers inside a plain group, reached the way a user does:
//! a click selects the group, a double-click enters it, Shift-click adds a sibling. The rings show
//! over the selection and dragging one onto the other swaps the two layers.
use crate::layers_drag_tests::Ui;
use egui::{Event, Modifiers, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

fn grouped_row() -> (Ui, NodeId, Vec<NodeId>) {
    let mut s = Session::new();
    let ids: Vec<NodeId> = (0..3)
        .map(|i| {
            NodeId(s.execute("shape.rectangle", &json!({"x": 100 + i * 80, "y": 100, "width": 50, "height": 60})).unwrap()["id"].as_u64().unwrap())
        })
        .collect();
    let group = NodeId(s.execute("object.group", &json!({"ids": ids})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    (Ui::new(s), group, ids)
}

fn centre(u: &Ui, id: NodeId) -> Pos2 {
    let b = u.app.session.doc().unwrap().world_bounds(id).unwrap();
    u.app.to_screen((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
}

fn x(u: &Ui, id: NodeId) -> f64 {
    u.app.session.doc().unwrap().world_bounds(id).unwrap().x0
}

fn click(u: &mut Ui, p: Pos2, modifiers: Modifiers) {
    for pressed in [true, false] {
        u.frame_with(
            vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers }],
            modifiers,
        );
    }
}

/// Reorder rings painted this frame: white dots with a pink edge, or solid pink when hot.
fn rings(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn walk(s: &egui::Shape, n: &mut usize) {
        match s {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, n)),
            egui::Shape::Circle(c)
                if c.fill == crate::handles::PINK || (c.fill == egui::Color32::WHITE && c.stroke.color == crate::handles::PINK) =>
            {
                *n += 1
            }
            _ => {}
        }
    }
    let mut n = 0;
    shapes.iter().for_each(|c| walk(&c.shape, &mut n));
    n
}

/// Click the group, double-click into it, then Shift-click a sibling: the two children are selected.
fn select_two_children(u: &mut Ui, group: NodeId, ids: &[NodeId]) {
    let a = centre(u, ids[0]);
    click(u, a, Modifiers::NONE);
    assert_eq!(u.app.session.selection(), vec![group], "a click selects the group");
    click(u, a, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), vec![ids[0]], "a double-click enters it");
    // Past the double-click interval, so the next press is a click of its own.
    for _ in 0..40 {
        u.frame(vec![]);
    }
    let b = centre(u, ids[1]);
    click(u, b, Modifiers::SHIFT);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), vec![ids[0], ids[1]], "Shift-click adds the sibling, not the group");
}

#[test]
fn two_children_of_a_plain_group_show_reorder_rings() {
    let (mut u, group, ids) = grouped_row();
    select_two_children(&mut u, group, &ids);
    let over = centre(&u, ids[1]) + egui::vec2(10.0, 10.0);
    u.frame(vec![Event::PointerMoved(over)]);
    let out = u.frame_output(vec![Event::PointerMoved(over)]);
    assert_eq!(rings(&out.shapes), 2, "one ring per selected child");
}

#[test]
fn dragging_a_ring_inside_a_group_swaps_the_two_children() {
    let (mut u, group, ids) = grouped_row();
    select_two_children(&mut u, group, &ids);
    let (a, b) = (centre(&u, ids[0]), centre(&u, ids[1]));
    let (xa, xb) = (x(&u, ids[0]), x(&u, ids[1]));
    let before = u.app.session.undo_depth().0;
    u.frame(vec![Event::PointerMoved(a)]);
    u.button(a, true);
    for i in 1..=6 {
        u.frame(vec![Event::PointerMoved(a + (b - a) * i as f32 / 6.0)]);
    }
    u.button(b, false);
    u.frame(vec![]);
    assert_eq!((x(&u, ids[0]), x(&u, ids[1])), (xb, xa), "the two children trade places");
    assert_eq!(x(&u, ids[2]), 260.0, "the third child, not selected, stays put");
    assert_eq!(u.app.session.selection(), vec![ids[0], ids[1]], "the selection is kept");
    assert_eq!(u.app.session.doc().unwrap().node(ids[0]).unwrap().parent, Some(group), "still in the group");
    assert_eq!(u.app.session.undo_depth().0, before + 1, "one undo step");
    u.app.run("edit.undo", json!({}));
    assert_eq!((x(&u, ids[0]), x(&u, ids[1])), (xa, xb));
}
