//! The workspace on the home screen and in the editor (fork): new files go to Drafts, the setup
//! card, the file listing and the breadcrumb under the file name.

use std::path::{Path, PathBuf};

use figcraft_engine::Session;
use serde_json::json;

use crate::home::Section;
use crate::import_report_tests::shown_texts;
use crate::layers_drag_tests::Ui;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-ui-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn texts(u: &mut Ui) -> Vec<String> {
    u.frame(vec![]);
    shown_texts(u.frame_output(vec![]).shapes)
}

fn add(u: &mut Ui, root: &Path, name: &str) {
    u.app.run("workspace.add", json!({"path": root.to_string_lossy(), "name": name})).expect("added");
}

#[test]
fn new_files_go_to_drafts_with_a_workspace_and_stay_untitled_without_one() {
    let mut u = Ui::new(Session::new());
    crate::menus::invoke(&mut u.app, "file.new");
    assert_eq!(u.app.session.ds().unwrap().path, None, "no workspace: an untitled tab as before");

    let root = scratch("new");
    add(&mut u, &root, "Studio");
    crate::menus::invoke(&mut u.app, "file.new");
    crate::menus::invoke(&mut u.app, "file.newBoard");
    let board = u.app.session.ds().unwrap().path.clone().expect("saved at once");
    assert!(Path::new(&board).is_file() && Path::new(&board).parent().unwrap().ends_with("Drafts"));
    assert!(board.ends_with("Untitled board.figcraft"));
    assert!(root.join("Drafts").join("Untitled.figcraft").is_file());

    // The editor says where the file is.
    u.app.ui.home = false;
    assert!(texts(&mut u).iter().any(|t| t == "Studio / Drafts"), "the breadcrumb");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_workspace_section_offers_setup_then_lists_the_places_files() {
    let mut u = Ui::new(Session::new());
    u.app.ui.home = true;
    u.app.ui.home_view.section = Section::Workspace;
    let shown = texts(&mut u);
    assert!(shown.iter().any(|t| t == "Set up your workspace"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Set up a workspace"), "the sidebar's switcher");

    let root = scratch("list");
    add(&mut u, &root, "Client work");
    u.app.run("workspace.newProject", json!({"name": "Website"}));
    u.app.run("workspace.newFile", json!({"project": "Website", "name": "Landing"}));
    u.app.run("file.close", json!({}));
    u.app.ui.home = true;
    u.app.ui.home_view.section = Section::Workspace;
    u.app.ui.home_view.workspace.place = crate::home_workspace::Place::Project("Website".into());
    u.app.ui.home_view.workspace.relist();
    let shown = texts(&mut u);
    for want in ["Client work", "Drafts", "Starred", "Projects", "Website", "Landing", "New design file"] {
        assert!(shown.iter().any(|t| t == want), "{want} in {shown:?}");
    }
    let inspect = crate::home_workspace::inspect(&u.app);
    assert_eq!(inspect["place"]["project"], "Website");
    assert_eq!(inspect["files"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_outside_any_workspace_shows_its_folder_and_an_unsaved_one_says_so() {
    let mut u = Ui::new(Session::new());
    u.app.run("file.new", json!({}));
    u.app.ui.home = false;
    assert!(texts(&mut u).iter().any(|t| t == "Not saved yet"));
    let dir = scratch("loose");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Loose.figcraft");
    u.app.run("file.save", json!({"path": path.to_string_lossy()}));
    let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
    // The label is worked out again after two seconds.
    u.advance_clock(100.0);
    assert!(texts(&mut u).iter().any(|t| t == &folder), "the folder's name");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renaming_a_workspace_file_in_the_editor_renames_it_on_disk() {
    let mut u = Ui::new(Session::new());
    let root = scratch("rename");
    add(&mut u, &root, "Studio");
    crate::menus::invoke(&mut u.app, "file.new");
    u.app.rename_file("Home screen");
    let ds = u.app.session.ds().unwrap();
    assert_eq!(ds.doc.name, "Home screen");
    let path = ds.path.clone().unwrap();
    assert!(path.ends_with("Home screen.figcraft") && Path::new(&path).is_file());
    assert!(!root.join("Drafts").join("Untitled.figcraft").exists(), "renamed, not copied");
    // Outside a workspace only the document's name changes.
    u.app.run("file.new", json!({}));
    u.app.rename_file("Loose");
    assert_eq!(u.app.session.ds().unwrap().doc.name, "Loose");
    assert_eq!(u.app.session.ds().unwrap().path, None);
    let _ = std::fs::remove_dir_all(&root);
}
