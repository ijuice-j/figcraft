//! Remove background against a stand-in remover and a stand-in Cloudflare API on this computer:
//! nothing here reaches the internet.

// The remover is reached over the network, which the browser build doesn't have.
#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use figcraft_color::PaintKind;
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::cloudflare::{self, InstallError, Step};
use super::{Connection, net};
use crate::{EngineError, Session};

/// A request the stand-in server got.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

type Reply = (u16, &'static str, Vec<u8>);

/// A server on 127.0.0.1 that answers each request with `reply` and records it.
fn serve(reply: impl Fn(&Seen) -> Reply + Send + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let mut headers = Vec::new();
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.trim().split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            let req = Seen { method, path, headers, body };
            let (status, kind, out) = reply(&req);
            log.lock().unwrap().push(req);
            let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&out);
        }
    });
    (address, seen)
}

/// A `w`×`h` PNG: opaque everywhere, or only in a centred square (`cut`: a cut-out).
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

const KEY: &str = "test-key-123";

/// A remover that checks the key and answers with a cut-out the size of `w`×`h`.
fn remover(w: u32, h: u32) -> (Connection, Arc<Mutex<Vec<Seen>>>) {
    let (address, seen) = serve(move |r| {
        if r.method != "POST" || r.path != "/remove-background" {
            return (404, "text/plain", b"Not found".to_vec());
        }
        if r.header("x-access-key") != Some(KEY) {
            return (401, "text/plain", b"Wrong or missing access key".to_vec());
        }
        (200, "image/png", png(w, h, true))
    });
    (Connection::new(&address, KEY).unwrap(), seen)
}

/// A session with one rectangle filled with an opaque 40×30 photo.
fn photo() -> (Session, NodeId, String) {
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(png(40, 30, false));
    let v = s.execute("image.place", &json!({"data": data, "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
    let id = NodeId(v["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    (s, id, v["image"].as_str().unwrap().to_string())
}

fn fill_image(s: &Session, id: NodeId) -> (String, Option<String>) {
    let n = s.doc().unwrap().node(id).unwrap();
    n.fills.iter().find_map(|f| if let PaintKind::Image(img) = &f.kind { Some((img.image.clone(), img.original.clone())) } else { None }).unwrap()
}

#[test]
fn addresses_must_be_https_and_keys_present() {
    assert!(Connection::new("https://figcraft-bg.someone.workers.dev/", "k").is_ok_and(|c| c.address == "https://figcraft-bg.someone.workers.dev"));
    assert!(Connection::new("http://figcraft-bg.someone.workers.dev", "k").is_err(), "plain http only on this computer");
    assert!(Connection::new("http://127.0.0.1:8080", "k").is_ok());
    assert!(Connection::new("https://x.workers.dev", "  ").is_err());
    assert!(Connection::new("https://x.workers.dev", "a b").is_err());
    assert!(Connection::new("https://x .workers.dev", "k").is_err());
    let shown = format!("{:?}", Connection::new("https://x.workers.dev", "secret-key").unwrap());
    assert!(!shown.contains("secret-key"), "the key is never printed: {shown}");
}

#[test]
fn waiting_puts_the_cutout_in_and_undo_and_restore_bring_the_photo_back() {
    let (mut s, id, photo_key) = photo();
    let (c, seen) = remover(40, 30);
    s.set_remove_bg(Some(c));
    let v = s.execute("image.removeBackground", &json!({"wait": true})).unwrap();
    assert_eq!(v["state"], "done");
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(40), Some(30)), "the cut-out keeps the photo's size");
    let (now, original) = fill_image(&s, id);
    assert_ne!(now, photo_key);
    assert_eq!(original.as_deref(), Some(photo_key.as_str()), "the photo is kept as the original");
    // The Worker got the photo's own bytes, with the key, and nothing went to the journal.
    let req = seen.lock().unwrap()[0].clone();
    assert_eq!(req.header("x-access-key"), Some(KEY));
    assert_eq!(req.header("content-type"), Some("image/png"));
    assert_eq!(&*req.body, &**s.doc().unwrap().images.get(&photo_key).unwrap().bytes);
    assert!(!format!("{:?}", s.journal).contains(KEY), "the key isn't journalled");
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_image(&s, id), (photo_key.clone(), None));
    s.execute("edit.redo", &json!({})).unwrap();
    // Restore original, and the photo's data is still in the file.
    let r = s.execute("image.restoreOriginal", &json!({})).unwrap();
    assert_eq!(r["image"], photo_key.as_str());
    assert_eq!(fill_image(&s, id), (photo_key, None));
    assert!(s.execute("image.restoreOriginal", &json!({})).is_err(), "nothing left to restore");
}

#[test]
fn removing_twice_keeps_the_first_original_and_it_stays_in_use() {
    let (mut s, id, photo_key) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    s.execute("image.removeBackground", &json!({"wait": true})).unwrap();
    s.execute("image.removeBackground", &json!({"wait": true})).unwrap();
    assert_eq!(fill_image(&s, id).1.as_deref(), Some(photo_key.as_str()));
    assert!(figcraft_doc::library::used_images(s.doc().unwrap()).contains(&photo_key), "saving keeps the original");
    // Replacing the picture drops the original: it belonged to the old one.
    let data = base64::engine::general_purpose::STANDARD.encode(png(8, 8, false));
    s.execute("image.replace", &json!({"data": data})).unwrap();
    assert_eq!(fill_image(&s, id).1, None);
}

#[test]
fn without_waiting_the_job_finishes_later_and_is_applied_by_its_number() {
    let (mut s, id, photo_key) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    let v = s.execute("image.removeBackground", &json!({})).unwrap();
    assert_eq!(v["state"], "working");
    let job = v["job"].as_u64().unwrap();
    assert_eq!(s.remove_bg_working().len(), 1);
    assert_eq!(fill_image(&s, id).0, photo_key, "nothing changes yet");
    // Asking again while it works doesn't start another.
    assert_eq!(s.execute("image.removeBackground", &json!({})).unwrap()["job"], job);
    let start = Instant::now();
    while s.remove_bg_ready().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(20), "the job finishes");
        std::thread::sleep(Duration::from_millis(20));
    }
    let undo_before = s.ds().unwrap().history.undo.len();
    let done = s.execute("image.removeBackground", &json!({"job": job})).unwrap();
    assert_eq!(done["state"], "done");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_before + 1, "one undo step, when it lands");
    assert!(s.remove_bg_working().is_empty() && s.remove_bg_ready().is_empty());
    assert!(s.execute("image.removeBackground", &json!({"job": job})).is_err(), "a job lands once");
}

#[test]
fn cancel_drops_the_answer() {
    let (mut s, id, photo_key) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    let job = s.execute("image.removeBackground", &json!({})).unwrap()["job"].as_u64().unwrap();
    assert!(s.remove_bg_cancel(job));
    std::thread::sleep(Duration::from_millis(300));
    assert!(s.remove_bg_ready().is_empty());
    assert_eq!(fill_image(&s, id).0, photo_key);
}

#[test]
fn not_connected_asks_the_app_for_setup_and_changes_nothing() {
    let (mut s, _, _) = photo();
    let e = s.execute("image.removeBackground", &json!({})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)), "{e}");
    assert!(s.ui_requests.iter().any(|r| r.kind == "image.removeBackground" && r.data["setup"] == true));
}

#[test]
fn refusals_and_failures_leave_the_file_alone() {
    let (mut s, id, photo_key) = photo();
    // A wrong key.
    let (good, _) = remover(40, 30);
    s.set_remove_bg(Some(Connection { key: "wrong".into(), ..good.clone() }));
    let e = s.execute("image.removeBackground", &json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("didn't accept FigCraft's access key"), "{e}");
    // Something that isn't a PNG.
    let (address, _) = serve(|_| (200, "text/html", b"<html>hello</html>".to_vec()));
    s.set_remove_bg(Some(Connection::new(&address, KEY).unwrap()));
    let e = s.execute("image.removeBackground", &json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("isn't an image"), "{e}");
    // A cut-out with nothing left in it.
    let (address, _) = serve(|_| {
        // A new pixmap is fully transparent.
        let px = figcraft_render::Pixmap::new(40, 30).unwrap();
        (200, "image/png", figcraft_render::encode_png(&px).unwrap())
    });
    s.set_remove_bg(Some(Connection::new(&address, KEY).unwrap()));
    let e = s.execute("image.removeBackground", &json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("no clear subject"), "{e}");
    // The monthly limit, as Cloudflare words it inside the Worker's error.
    let (address, _) = serve(|_| (502, "text/plain", b"Background removal failed: ERROR 9422: limit reached".to_vec()));
    s.set_remove_bg(Some(Connection::new(&address, KEY).unwrap()));
    let e = s.execute("image.removeBackground", &json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("this month's free images"), "{e}");
    // Nobody listening.
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    s.set_remove_bg(Some(Connection::new(&format!("http://{free}"), KEY).unwrap()));
    let e = s.execute("image.removeBackground", &json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("couldn't reach"), "{e}");
    assert_eq!(fill_image(&s, id), (photo_key, None));
    assert!(s.ds().unwrap().history.undo.iter().all(|h| h.label != "Remove background"));
}

#[test]
fn a_layer_deleted_or_changed_while_waiting_is_left_alone() {
    let (mut s, id, _) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    let job = s.execute("image.removeBackground", &json!({})).unwrap()["job"].as_u64().unwrap();
    s.execute("edit.delete", &json!({"ids": [id]})).unwrap();
    let start = Instant::now();
    while s.remove_bg_ready().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(20));
    }
    let e = s.execute("image.removeBackground", &json!({"job": job})).unwrap_err().to_string();
    assert!(e.contains("deleted"), "{e}");

    let (mut s, id, _) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    let job = s.execute("image.removeBackground", &json!({})).unwrap()["job"].as_u64().unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png(8, 8, false));
    s.execute("image.replace", &json!({"ids": [id], "data": data})).unwrap();
    let start = Instant::now();
    while s.remove_bg_ready().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(20));
    }
    let e = s.execute("image.removeBackground", &json!({"job": job})).unwrap_err().to_string();
    assert!(e.contains("image changed"), "{e}");
}

#[test]
fn only_one_layer_with_an_image_and_not_text() {
    let (mut s, _, _) = photo();
    s.set_remove_bg(Some(remover(40, 30).0));
    let r = s.execute("shape.rectangle", &json!({"x": 300, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
    assert!(s.execute("image.removeBackground", &json!({})).is_err(), "no image");
    let t = s.execute("text.create", &json!({"x": 0, "y": 300, "text": "Hi"})).unwrap();
    s.execute("select.set", &json!({"ids": [t["id"]]})).unwrap();
    assert!(s.execute("image.removeBackground", &json!({})).is_err(), "text");
    s.execute("select.all", &json!({})).unwrap();
    assert!(s.execute("image.removeBackground", &json!({})).is_err(), "one layer at a time");
    assert!(s.execute("image.removeBackground", &json!({"job": "x"})).is_err());
    assert!(s.execute("image.removeBackground", &json!({"job": 999})).is_err());
}

#[test]
fn the_test_picture_is_a_png() {
    let t = net::test_image();
    assert!(t.starts_with(&[0x89, b'P', b'N', b'G']) && t.len() < 20_000);
    let (c, _) = remover(160, 120);
    assert!(net::test(&c).is_ok());
}

/// A stand-in Cloudflare API: `existing` says whether `figcraft-bg` is there already and
/// `subdomain` whether the account has a workers.dev name.
fn cloudflare_api(existing: bool, subdomain: bool, token_ok: bool) -> (String, Arc<Mutex<Vec<Seen>>>) {
    serve(move |r| {
        let ok = |result: Value| (200, "application/json", json!({"success": true, "errors": [], "result": result}).to_string().into_bytes());
        let not_found =
            || (404, "application/json", json!({"success": false, "errors": [{"code": 10007, "message": "not found"}]}).to_string().into_bytes());
        if !token_ok || r.header("authorization") != Some("Bearer cf-token") {
            return (
                401,
                "application/json",
                json!({"success": false, "errors": [{"code": 10001, "message": "Unable to authenticate request"}]}).to_string().into_bytes(),
            );
        }
        match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/accounts?per_page=50") => ok(json!([{"id": "abc123", "name": "Ada's Account"}])),
            ("GET", "/accounts/abc123/workers/subdomain") if subdomain => ok(json!({"subdomain": "ada"})),
            ("GET", "/accounts/abc123/workers/subdomain") => not_found(),
            ("PUT", "/accounts/abc123/workers/subdomain") => {
                let asked: Value = serde_json::from_slice(&r.body).unwrap();
                ok(json!({"subdomain": asked["subdomain"]}))
            }
            ("GET", "/accounts/abc123/workers/scripts/figcraft-bg/settings") if existing => ok(json!({"bindings": []})),
            ("GET", "/accounts/abc123/workers/scripts/figcraft-bg/settings") => not_found(),
            ("PUT", "/accounts/abc123/workers/scripts/figcraft-bg") => ok(json!({"id": "figcraft-bg"})),
            ("POST", "/accounts/abc123/workers/scripts/figcraft-bg/subdomain") => ok(json!({"enabled": true})),
            _ => not_found(),
        }
    })
}

#[test]
fn install_uploads_the_worker_with_images_and_a_fresh_key() {
    let (api, seen) = cloudflare_api(false, true, true);
    let mut steps = Vec::new();
    let done = cloudflare::install(&api, " cf-token\n", false, &mut |s| steps.push(s)).unwrap();
    assert_eq!(steps, [Step::Checking, Step::Installing, Step::Address]);
    assert_eq!(done.account, "Ada's Account");
    assert_eq!(done.connection.address, "https://figcraft-bg.ada.workers.dev");
    assert_eq!(done.connection.key.len(), 24);
    let seen = seen.lock().unwrap();
    let upload = seen.iter().find(|r| r.method == "PUT" && r.path.ends_with("/scripts/figcraft-bg")).unwrap();
    let body = String::from_utf8_lossy(&upload.body);
    assert!(upload.header("content-type").unwrap().starts_with("multipart/form-data; boundary="));
    let metadata = body.lines().find(|l| l.starts_with('{')).unwrap();
    let metadata: Value = serde_json::from_str(metadata).unwrap();
    assert_eq!(metadata["main_module"], "worker.js");
    assert_eq!(metadata["bindings"][0], json!({"type": "images", "name": "IMAGES"}));
    assert_eq!(metadata["bindings"][1], json!({"type": "secret_text", "name": "ACCESS_KEY", "text": done.connection.key}));
    assert!(body.contains("segment: \"foreground\"") && body.contains("export default"), "the Worker's code is uploaded");
    let enable = seen.iter().find(|r| r.path.ends_with("/scripts/figcraft-bg/subdomain")).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&enable.body).unwrap()["enabled"], true);
    // A second install makes a different key.
    drop(seen);
    let (api, _) = cloudflare_api(false, true, true);
    assert_ne!(cloudflare::install(&api, "cf-token", false, &mut |_| {}).unwrap().connection.key, done.connection.key);
}

#[test]
fn install_asks_before_replacing_and_names_a_new_workers_dev() {
    let (api, seen) = cloudflare_api(true, false, true);
    let e = cloudflare::install(&api, "cf-token", false, &mut |_| {}).unwrap_err();
    assert_eq!(e, InstallError::Exists { account: "Ada's Account".into() });
    assert!(!seen.lock().unwrap().iter().any(|r| r.method == "PUT" && r.path.ends_with("figcraft-bg")), "nothing replaced without asking");
    let done = cloudflare::install(&api, "cf-token", true, &mut |_| {}).unwrap();
    let host = done.connection.address.strip_prefix("https://figcraft-bg.figcraft-").unwrap();
    assert!(host.ends_with(".workers.dev") && host.len() == 8 + ".workers.dev".len(), "{host}");
}

#[test]
fn install_refuses_a_bad_key_without_touching_the_account() {
    let (api, seen) = cloudflare_api(false, true, false);
    assert_eq!(cloudflare::install(&api, "cf-token", false, &mut |_| {}).unwrap_err(), InstallError::BadKey);
    assert_eq!(seen.lock().unwrap().len(), 1, "stopped at the first request");
    assert_eq!(cloudflare::install(&api, "", false, &mut |_| {}).unwrap_err(), InstallError::BadKey);
    assert_eq!(cloudflare::install(&api, "has space", false, &mut |_| {}).unwrap_err(), InstallError::BadKey);
    let (forbidden, _) =
        serve(|_| (403, "application/json", br#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#.to_vec()));
    assert_eq!(cloudflare::install(&forbidden, "cf-token", false, &mut |_| {}).unwrap_err(), InstallError::NoPermission);
    let (none, _) = serve(|_| (200, "application/json", br#"{"success":true,"result":[]}"#.to_vec()));
    assert_eq!(cloudflare::install(&none, "cf-token", false, &mut |_| {}).unwrap_err(), InstallError::NoAccount);
    let (junk, _) = serve(|_| (200, "text/html", b"<html>".to_vec()));
    assert!(cloudflare::install(&junk, "cf-token", false, &mut |_| {}).is_err());
}

#[test]
fn settle_waits_for_a_new_address_and_a_replaced_key_but_not_for_ever() {
    let (good, _) = remover(160, 120);
    assert!(cloudflare::settle(&good, Duration::from_secs(1)).is_ok());
    // A key refusal is waited out for a while (a replaced Worker answers with its old key for a
    // few seconds), then given up.
    let wrong = Connection { key: "nope".into(), ..good };
    let start = Instant::now();
    assert!(cloudflare::settle(&wrong, Duration::from_secs(600)).unwrap_err().contains("access key"));
    let took = start.elapsed();
    assert!(took >= cloudflare::KEY_GRACE - Duration::from_secs(6) && took < cloudflare::KEY_GRACE + Duration::from_secs(10), "{took:?}");
}

#[test]
fn the_upload_is_well_formed_multipart() {
    let body = String::from_utf8(cloudflare::upload_for_tests("B0UND", "k")).unwrap();
    assert!(body.starts_with("--B0UND\r\n") && body.ends_with("--B0UND--\r\n"));
    assert_eq!(body.matches("--B0UND\r\n").count(), 2);
    assert!(cloudflare::new_access_key().unwrap().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}
