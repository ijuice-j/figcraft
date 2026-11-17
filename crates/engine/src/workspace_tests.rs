//! Workspaces on real folders in the temp directory.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{DRAFTS, META, in_workspace, locate, same_path};
use crate::{EngineError, Session};

/// A fresh folder for one test (gone first if a run before left it).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A session with a workspace added at `root`.
fn with_workspace(root: &Path, name: Option<&str>) -> Session {
    let mut session = Session::new();
    let mut p = json!({"path": s(root)});
    if let Some(n) = name {
        p["name"] = json!(n);
    }
    session.execute("workspace.add", &p).unwrap();
    session
}

fn project_names(session: &mut Session) -> Vec<String> {
    let r = session.execute("workspace.projects", &json!({})).unwrap();
    r["projects"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn file_names(session: &mut Session, project: &str) -> Vec<String> {
    let r = session.execute("workspace.files", &json!({"project": project})).unwrap();
    r["files"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn bad(r: Result<Value, EngineError>, needle: &str) {
    match r {
        Err(EngineError::BadParams { msg, .. }) => assert!(msg.contains(needle), "{msg} should say {needle}"),
        other => panic!("expected a refusal saying {needle}, got {other:?}"),
    }
}

#[test]
fn adding_a_workspace_makes_its_folder_drafts_and_name_and_makes_it_active() {
    let root = scratch("add");
    let mut session = with_workspace(&root, Some("Studio"));
    assert!(root.join(DRAFTS).is_dir() && root.join(META).is_file());
    let list = session.execute("workspace.list", &json!({})).unwrap();
    assert_eq!(list["active"], s(&root));
    assert_eq!(list["workspaces"][0]["name"], "Studio");
    assert_eq!(list["workspaces"][0]["active"], true);

    // A second one becomes active; switching back and adding a known one again changes nothing else.
    let other = scratch("add-other");
    session.execute("workspace.add", &json!({"path": s(&other)})).unwrap();
    assert_eq!(
        session.execute("workspace.list", &json!({})).unwrap()["workspaces"][1]["name"],
        other.file_name().unwrap().to_string_lossy().as_ref()
    );
    session.execute("workspace.switch", &json!({"workspace": s(&root)})).unwrap();
    assert_eq!(session.prefs.active_workspace.as_deref(), Some(s(&root).as_str()));
    let again = session.execute("workspace.add", &json!({"path": s(&other)})).unwrap();
    assert_eq!(again["existed"], true);
    assert_eq!(session.prefs.workspaces.len(), 2);

    // Forgetting leaves the folder; the other one becomes active.
    session.execute("workspace.forget", &json!({"workspace": s(&other)})).unwrap();
    assert!(other.join(DRAFTS).is_dir());
    assert_eq!(session.prefs.workspaces, vec![s(&root)]);
    session.execute("workspace.forget", &json!({})).unwrap();
    assert!(session.prefs.workspaces.is_empty() && session.prefs.active_workspace.is_none());
    bad(session.execute("workspace.projects", &json!({})), "no workspace yet");

    bad(session.execute("workspace.add", &json!({"path": "relative/folder"})), "full folder path");
    std::fs::write(other.join("plain.txt"), b"x").unwrap();
    bad(session.execute("workspace.add", &json!({"path": s(&other.join("plain.txt"))})), "is a file");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn new_files_are_saved_in_drafts_at_once_with_numbered_names() {
    let root = scratch("new");
    let mut session = with_workspace(&root, None);
    let first = session.execute("workspace.newFile", &json!({})).unwrap();
    let second = session.execute("workspace.newFile", &json!({})).unwrap();
    let board = session.execute("workspace.newFile", &json!({"kind": "board"})).unwrap();
    assert_eq!(first["name"], "Untitled");
    assert_eq!(second["name"], "Untitled 2");
    assert_eq!(board["name"], "Untitled board");
    for r in [&first, &second, &board] {
        assert!(Path::new(r["path"].as_str().unwrap()).is_file(), "saved: {r}");
    }
    // Open in a tab with its path, not dirty, named like its file, and remembered.
    let ds = session.ds().unwrap();
    assert_eq!(ds.path.as_deref(), board["path"].as_str());
    assert!(!ds.is_dirty());
    assert_eq!(ds.doc.name, "Untitled board");
    assert_eq!(session.prefs.recent_files.first().map(String::as_str), board["path"].as_str());
    assert_eq!(
        file_names(&mut session, "Drafts"),
        ["Untitled 2", "Untitled board", "Untitled"].map(String::from).to_vec().into_iter().fold(Vec::new(), |mut v, n| {
            v.push(n);
            v.sort_by_key(|n: &String| format!("{}.figcraft", n).to_lowercase());
            v
        })
    );

    let loc = locate(&session.prefs, first["path"].as_str().unwrap()).unwrap();
    assert!(loc.drafts && loc.project == DRAFTS && same_path(Path::new(&loc.workspace), &root));
    assert!(in_workspace(&session.prefs, first["path"].as_str().unwrap()));
    assert!(!in_workspace(&session.prefs, &s(&root.join("loose.figcraft"))));
    assert_eq!(session.execute("workspace.locate", &json!({})).unwrap()["location"]["project"], DRAFTS);

    bad(session.execute("workspace.newFile", &json!({"kind": "slides"})), "design or board");
    bad(session.execute("workspace.newFile", &json!({"project": "Nope"})), "isn't a project");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn projects_are_folders_listed_after_drafts_with_clean_unique_names() {
    let root = scratch("projects");
    let mut session = with_workspace(&root, None);
    session.execute("workspace.newProject", &json!({"name": "website"})).unwrap();
    session.execute("workspace.newProject", &json!({"name": "App"})).unwrap();
    let dup = session.execute("workspace.newProject", &json!({"name": "App"})).unwrap();
    assert_eq!(dup["project"]["name"], "App 2");
    let odd = session.execute("workspace.newProject", &json!({"name": "  a/b:c*  "})).unwrap();
    assert_eq!(odd["project"]["name"], "a_b_c_");
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    assert_eq!(project_names(&mut session), ["Drafts", "a_b_c_", "App", "App 2", "website"]);

    bad(session.execute("workspace.newProject", &json!({"name": "drafts"})), "already part");
    bad(session.execute("workspace.newProject", &json!({"name": ".git"})), "dot");
    bad(session.execute("workspace.newProject", &json!({"name": "   "})), "dot");

    // A file made in a project is counted and listed there.
    session.execute("workspace.newFile", &json!({"project": "website", "name": "Home"})).unwrap();
    let projects = session.execute("workspace.projects", &json!({})).unwrap();
    let website = projects["projects"].as_array().unwrap().iter().find(|p| p["name"] == "website").cloned().unwrap();
    assert_eq!(website["files"], 1);
    assert_eq!(file_names(&mut session, "WEBSITE"), ["Home"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn moving_and_renaming_keep_open_tabs_recent_files_and_stars() {
    let root = scratch("move");
    let mut session = with_workspace(&root, None);
    session.execute("workspace.newProject", &json!({"name": "Client"})).unwrap();
    let made = session.execute("workspace.newFile", &json!({"name": "Logo"})).unwrap();
    let path = made["path"].as_str().unwrap().to_string();
    session.execute("workspace.star", &json!({"file": path})).unwrap();

    let moved = session.execute("workspace.moveFile", &json!({"file": path, "project": "Client"})).unwrap();
    let new_path = moved["path"].as_str().unwrap().to_string();
    assert!(!Path::new(&path).exists() && Path::new(&new_path).is_file());
    assert_eq!(session.ds().unwrap().path.as_deref(), Some(new_path.as_str()), "the open tab follows");
    assert_eq!(session.prefs.recent_files.first(), Some(&new_path));
    let starred = session.execute("workspace.files", &json!({"starred": true})).unwrap();
    assert_eq!(starred["files"][0]["path"], new_path.as_str(), "the star follows");
    assert_eq!(starred["files"][0]["starred"], true);

    // Renaming the file renames the open document too.
    let renamed = session.execute("workspace.renameFile", &json!({"file": new_path, "name": "Brand mark"})).unwrap();
    let renamed_path = renamed["path"].as_str().unwrap().to_string();
    assert!(renamed_path.ends_with("Brand mark.figcraft") && Path::new(&renamed_path).is_file());
    assert_eq!(session.ds().unwrap().doc.name, "Brand mark");
    assert_eq!(session.ds().unwrap().path.as_deref(), Some(renamed_path.as_str()));
    // Only the case changes: still renamed.
    let cased = session.execute("workspace.renameFile", &json!({"file": renamed_path, "name": "brand mark"})).unwrap();
    assert!(cased["path"].as_str().unwrap().ends_with("brand mark.figcraft"));

    // Renaming the project carries its open file, recent files and stars along.
    session.execute("workspace.star", &json!({"project": "Client"})).unwrap();
    let r = session.execute("workspace.renameProject", &json!({"project": "Client", "name": "Acme"})).unwrap();
    assert_eq!(r["project"]["name"], "Acme");
    let in_acme = root.join("Acme").join("brand mark.figcraft");
    assert!(same_path(Path::new(session.ds().unwrap().path.as_deref().unwrap()), &in_acme));
    assert!(session.prefs.recent_files.iter().any(|f| same_path(Path::new(f), &in_acme)));
    let projects = session.execute("workspace.projects", &json!({})).unwrap();
    assert!(projects["projects"].as_array().unwrap().iter().any(|p| p["name"] == "Acme" && p["starred"] == true));
    assert_eq!(session.execute("workspace.files", &json!({"starred": true})).unwrap()["files"].as_array().unwrap().len(), 1);

    bad(session.execute("workspace.renameProject", &json!({"project": "Drafts", "name": "X"})), "can't be renamed");
    bad(session.execute("workspace.moveFile", &json!({"file": s(&root.join("Acme").join("gone.figcraft")), "project": "Drafts"})), "isn't there");
    bad(session.execute("workspace.moveFile", &json!({"file": s(&root.join("loose.figcraft")), "project": "Acme"})), "isn't in a project");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deleting_is_refused_while_open_and_forgets_recent_files_and_stars() {
    let root = scratch("delete");
    let mut session = with_workspace(&root, None);
    session.execute("workspace.newProject", &json!({"name": "Old"})).unwrap();
    let made = session.execute("workspace.newFile", &json!({"project": "Old", "name": "Draft"})).unwrap();
    let path = made["path"].as_str().unwrap().to_string();
    session.execute("workspace.star", &json!({"file": path})).unwrap();

    bad(session.execute("workspace.deleteFile", &json!({"file": path})), "close it first");
    bad(session.execute("workspace.deleteProject", &json!({"project": "Old"})), "close it first");
    session.execute("file.close", &json!({})).unwrap();
    session.execute("workspace.deleteFile", &json!({"file": path})).unwrap();
    assert!(!Path::new(&path).exists());
    assert!(!session.prefs.recent_files.contains(&path));
    assert!(session.execute("workspace.files", &json!({"starred": true})).unwrap()["files"].as_array().unwrap().is_empty());

    session.execute("workspace.deleteProject", &json!({"project": "Old"})).unwrap();
    assert!(!root.join("Old").exists());
    bad(session.execute("workspace.deleteProject", &json!({"project": "Drafts"})), "can't be deleted");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_existing_folder_is_adopted_with_its_folders_as_projects() {
    let root = scratch("adopt");
    std::fs::create_dir_all(root.join("Mobile")).unwrap();
    let mut seed = Session::new();
    seed.execute("file.new", &json!({"name": "Splash"})).unwrap();
    seed.execute("file.save", &json!({"path": s(&root.join("Mobile").join("Splash.figcraft"))})).unwrap();
    std::fs::write(root.join("Mobile").join("notes.txt"), b"not a design").unwrap();

    let mut session = with_workspace(&root, None);
    assert_eq!(project_names(&mut session), ["Drafts", "Mobile"]);
    assert_eq!(file_names(&mut session, "Mobile"), ["Splash"], "only .figcraft files");
    // A renamed workspace keeps its folder's name.
    session.execute("workspace.rename", &json!({"name": "Phone apps"})).unwrap();
    assert_eq!(session.execute("workspace.list", &json!({})).unwrap()["workspaces"][0]["name"], "Phone apps");
    assert!(root.is_dir());
    let _ = std::fs::remove_dir_all(&root);
}
