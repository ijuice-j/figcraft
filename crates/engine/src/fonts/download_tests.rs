//! `font.download` without the network: this crate's tests use a store folder of their own, where
//! downloading was never turned on.

use serde_json::{Value, json};

use super::{Download, downloads, downloads_allowed, jobs, library_now, set, source};
use crate::{EngineError, Session};

#[test]
fn the_store_index_says_which_missing_families_can_be_downloaded() {
    assert_eq!(source("Lato").as_deref(), Some("Google Fonts"));
    assert_eq!(source(" lato ").as_deref(), Some("Google Fonts"), "as the document spells it");
    assert_eq!(source("FigCraft Not A Store Font"), None);

    let mut s = Session::new();
    for family in ["FigCraft Not A Store Font", "Lato"] {
        s.execute("text.create", &json!({"text": "Hamburgefonstiv", "props": {"fontFamily": family}})).unwrap();
    }
    let listed = s.execute("text.missingFonts", &json!({})).unwrap();
    let ours = |name: &str| listed["families"].as_array().unwrap().iter().find(|f| f["family"] == name).cloned();
    assert_eq!(ours("FigCraft Not A Store Font").unwrap()["source"], Value::Null);
    if !figcraft_text::has_family("Lato") {
        assert_eq!(ours("Lato").unwrap()["source"], "Google Fonts");
    }
}

#[test]
fn nothing_downloads_until_the_user_turns_downloading_on() {
    let mut s = Session::new();
    assert!(!downloads_allowed(), "off by default");
    let Some(family) = ["Lato", "Merriweather", "Bungee Shade"].into_iter().find(|f| !figcraft_text::has_family(f)) else { return };
    let refused = s.execute("font.download", &json!({"family": family})).unwrap_err();
    assert!(matches!(&refused, EngineError::Disabled(cmd, why) if cmd == "font.download" && why.contains("turned off")), "{refused}");
    assert!(!downloads().iter().any(|(f, _)| f == family), "no download started");
    assert_eq!(s.execute("font.downloads", &json!({})).unwrap()["allowed"], false);

    assert!(matches!(s.execute("font.download", &json!({})), Err(EngineError::BadParams { .. })));
    let unknown = s.execute("font.download", &json!({"family": "FigCraft Not A Store Font"})).unwrap_err();
    assert!(unknown.to_string().contains("not in the font store's index"), "{unknown}");
}

#[test]
fn a_download_reports_its_progress_then_fits_the_text_once_installed() {
    let family = "FigCraft Download Progress";
    let mut s = Session::new();
    jobs().insert(family.to_lowercase(), (family.into(), Download::Running { done: 50, total: 200 }));
    let r = s.execute("font.download", &json!({"family": family})).unwrap();
    assert_eq!(r, json!({"family": family, "state": "downloading", "done": 50, "total": 200}), "no second download");
    let listed = s.execute("font.downloads", &json!({})).unwrap();
    assert!(listed["downloads"].as_array().unwrap().contains(&json!({"family": family, "state": "downloading", "done": 50, "total": 200})));

    set(family, Download::Failed("downloading x failed".into()));
    let listed = s.execute("font.downloads", &json!({})).unwrap();
    assert!(listed["downloads"].as_array().unwrap().contains(&json!({"family": family, "state": "failed", "error": "downloading x failed"})));

    // Installed (and scanned by the download's thread): the next call fits the text, once.
    set(family, Download::Installed);
    let r = s.execute("font.download", &json!({"family": family})).unwrap();
    assert_eq!(r, json!({"family": family, "state": "installed", "refitted": []}));
    assert!(!downloads().iter().any(|(f, _)| f == family));
}

#[test]
fn the_library_lists_each_store_family_once_by_name() {
    let list = library_now();
    assert!(list.len() > 1000, "Google Fonts and Noto: {}", list.len());
    assert!(list.iter().any(|(f, src)| f == "Lato" && src == "Google Fonts"));
    let keys: Vec<String> = list.iter().map(|(f, _)| f.to_lowercase()).collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "sorted by name ignoring case, each name once");
    // The picker downloads what font.download can: the same source as the Missing fonts window.
    for (family, src) in list.iter().step_by(97) {
        assert_eq!(source(family).as_deref(), Some(src.as_str()), "{family}");
    }
}

#[test]
fn font_library_finds_families_by_part_of_their_name() {
    let mut s = Session::new();
    let r = s.execute("font.library", &json!({"query": "  MERRIWEATHER "})).unwrap();
    let names: Vec<&str> = r["families"].as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap()).collect();
    assert!(names.contains(&"Merriweather"), "{names:?}");
    assert!(names.iter().all(|n| n.to_lowercase().contains("merriweather")), "{names:?}");
    assert_eq!(r["total"].as_u64(), Some(names.len() as u64));
    let lato = s.execute("font.library", &json!({"query": "lato", "limit": 50})).unwrap();
    let lato = lato["families"].as_array().unwrap().iter().find(|f| f["family"] == "Lato").cloned().unwrap();
    assert_eq!(lato["source"], "Google Fonts");
    assert_eq!(lato["installed"], figcraft_text::has_family("Lato"));

    let few = s.execute("font.library", &json!({"limit": 3})).unwrap();
    assert_eq!(few["families"].as_array().unwrap().len(), 3);
    assert!(few["total"].as_u64().unwrap() > 3);
    let none = s.execute("font.library", &json!({"query": "FigCraft Not A Store Font"})).unwrap();
    assert_eq!(none, json!({"total": 0, "families": []}));
    for junk in [json!({"query": 5}), json!({"limit": -1}), json!({"limit": "many"}), json!({"limit": 1.5})] {
        assert!(matches!(s.execute("font.library", &junk), Err(EngineError::BadParams { .. })), "{junk}");
    }
}
