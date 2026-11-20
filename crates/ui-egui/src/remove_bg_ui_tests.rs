//! Remove background in the interface: Preferences ▸ AI tools, the right-click menu, the card that
//! offers to connect a remover, and the cut-out landing in its layer. A stand-in remover on this
//! computer answers; nothing reaches the internet.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use base64::Engine as _;
use egui::Pos2;
use figcraft_color::PaintKind;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::remove_bg::Connection;
use serde_json::json;

use crate::dialogs::PrefsGroup;
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::present_tests::id;

/// A `w`×`h` PNG, opaque, or a cut-out (a centred square on transparency).
fn png(w: u32, h: u32, cut: bool) -> Vec<u8> {
    let mut px = figcraft_render::Pixmap::new(w, h).unwrap();
    for (i, p) in px.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let inside = x > w / 4 && x < 3 * w / 4 && y > h / 4 && y < 3 * h / 4;
        p.copy_from_slice(if !cut {
            &[30, 90, 160, 255]
        } else if inside {
            &[200, 80, 40, 255]
        } else {
            &[0, 0, 0, 0]
        });
    }
    figcraft_render::encode_png(&px).unwrap()
}

/// A remover on 127.0.0.1 that answers every request with a 40×30 cut-out.
fn remover() -> Connection {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            let out = png(40, 30, true);
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).as_bytes(),
            );
            let _ = stream.write_all(&out);
        }
    });
    Connection::new(&address, "test-key").unwrap()
}

/// A window with one layer filled with a 40×30 photo, selected.
fn photo() -> (Ui, NodeId) {
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(png(40, 30, false));
    let v = s.execute("image.place", &json!({"data": data, "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
    let image = id(&v);
    s.execute("select.set", &json!({"ids": [image]})).unwrap();
    (Ui::new(s), image)
}

fn menu_on(u: &mut Ui, n: NodeId) -> Vec<egui::Shape> {
    u.app.run("select.set", json!({"ids": [n]}));
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![n]));
    u.frame(vec![]);
    flat(&u.frame_output(vec![]).shapes)
}

fn click(u: &mut Ui, shapes: &[egui::Shape], text: &str) {
    let at = text_center(shapes, text);
    u.button(at, true);
    u.button(at, false);
}

fn image_of(u: &Ui, n: NodeId) -> (String, Option<String>) {
    let node = u.app.session.doc().unwrap().node(n).unwrap();
    node.fills.iter().find_map(|f| if let PaintKind::Image(img) = &f.kind { Some((img.image.clone(), img.original.clone())) } else { None }).unwrap()
}

#[test]
fn preferences_is_a_page_of_groups_with_ai_tools() {
    let (mut u, _) = photo();
    u.app.ui.preferences = true;
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    for group in ["General", "Appearance", "Canvas", "Online sharing", "AI tools"] {
        assert!(has_text(&shapes, group), "{group}");
    }
    assert!(has_text(&shapes, "Your name"), "General is shown first");
    click(&mut u, &shapes, "AI tools");
    assert_eq!(u.app.ui.prefs_group, PrefsGroup::AiTools);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Remove background") && has_text(&shapes, "Not connected"));
    assert!(has_text(&shapes, "What happens when you connect"), "what connecting does is explained before it's done");
    assert!(has_text(&shapes, "Connect Cloudflare") && has_text(&shapes, "Set up by hand"));
    click(&mut u, &shapes, "Connect Cloudflare");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Create a free Cloudflare account"));
    assert!(has_text(&shapes, "Make a Cloudflare key for FigCraft"));
    assert!(has_text(&shapes, "Paste the key here"));
}

#[test]
fn connected_shows_the_remover_and_disconnect_forgets_it() {
    let (mut u, _) = photo();
    let c = remover();
    u.app.session.prefs.remove_bg_address = c.address.clone();
    crate::remove_bg_ui::connect_for_tests(&mut u.app, c.clone());
    u.app.ui.preferences = true;
    u.app.ui.prefs_group = PrefsGroup::AiTools;
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Connected") && has_text(&shapes, &c.address));
    assert!(!has_text(&shapes, "test-key"), "the key is hidden until Show");
    click(&mut u, &shapes, "Test");
    let start = Instant::now();
    while u.app.ui.ai.status.is_none() {
        assert!(start.elapsed() < Duration::from_secs(20), "the test finishes");
        std::thread::sleep(Duration::from_millis(20));
        u.frame(vec![]);
    }
    assert_eq!(u.app.ui.ai.status.as_ref().map(|s| s.0), Some(true), "{:?}", u.app.ui.ai.status);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, &shapes, "Disconnect");
    assert!(u.app.session.remove_bg().is_none());
    assert!(u.app.session.prefs.remove_bg_address.is_empty());
}

#[test]
fn without_a_remover_the_menu_item_offers_to_set_one_up() {
    let (mut u, image) = photo();
    let shapes = menu_on(&mut u, image);
    assert!(has_text(&shapes, "Remove background"));
    assert!(!has_text(&shapes, "Restore original image"), "nothing to restore yet");
    click(&mut u, &shapes, "Remove background");
    assert!(u.app.ui.remove_bg_setup, "the card that offers to connect");
    assert!(u.app.ui.last_error.is_none(), "no error toast on top of the card");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Connect Cloudflare to remove backgrounds"));
    click(&mut u, &shapes, "Set it up in Preferences");
    assert!(u.app.ui.preferences && u.app.ui.prefs_group == PrefsGroup::AiTools);
    assert!(!u.app.ui.remove_bg_setup);
}

#[test]
fn the_cutout_lands_in_the_layer_and_restore_brings_the_photo_back() {
    let (mut u, image) = photo();
    let (photo_key, _) = image_of(&u, image);
    crate::remove_bg_ui::connect_for_tests(&mut u.app, remover());
    let shapes = menu_on(&mut u, image);
    click(&mut u, &shapes, "Remove background");
    let start = Instant::now();
    while image_of(&u, image).0 == photo_key {
        assert!(start.elapsed() < Duration::from_secs(20), "the cut-out lands");
        std::thread::sleep(Duration::from_millis(20));
        u.frame(vec![]);
    }
    assert_eq!(image_of(&u, image).1.as_deref(), Some(photo_key.as_str()), "the photo is kept");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    let shapes = menu_on(&mut u, image);
    assert!(has_text(&shapes, "Restore original image"));
    click(&mut u, &shapes, "Restore original image");
    assert_eq!(image_of(&u, image), (photo_key, None));
}

#[test]
fn text_and_shapes_without_images_have_no_remove_background() {
    let (mut u, _) = photo();
    let r = id(&u.app.run("shape.rectangle", json!({"x": 400, "y": 0, "width": 50, "height": 50})).unwrap());
    assert!(!has_text(&menu_on(&mut u, r), "Remove background"));
    let t = id(&u.app.run("text.create", json!({"x": 400, "y": 200, "text": "Hello"})).unwrap());
    assert!(!has_text(&menu_on(&mut u, t), "Remove background"));
}
