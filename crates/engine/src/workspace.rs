//! Workspaces (fork feature): your files organised as Workspace ▸ Projects ▸ Files, kept as real
//! folders on disk so they can be synced, backed up and browsed outside FigCraft.
//!
//! A workspace is a folder you choose. Its `Drafts` folder holds new files, and every other folder
//! in it is a project (folders whose names start with a dot are skipped). A project's files are the
//! `.figcraft` files directly in its folder. `.figcraft-workspace.json` in the workspace folder
//! keeps its name and what you starred, so both travel with the folder.
//!
//! The app remembers the workspaces it knows and which one is active with the preferences
//! ([`Prefs::workspaces`](crate::Prefs), [`Prefs::active_workspace`](crate::Prefs)). Every change
//! goes through [`file_access`], so automation roots confine it like any other file work. Moving or
//! renaming a file or a project updates the tabs that have its files open, the recent files and
//! the stars. Deleting moves to the system's Recycle Bin or Trash. The browser has no folders:
//! there every command that touches one fails.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::file_access::{self, Access};
use crate::params::{boolean, get, req_string, string};
use crate::{EngineError, Prefs, Result, Session, bad, spec};

/// The folder of a workspace that holds new files.
pub const DRAFTS: &str = "Drafts";
/// The file in a workspace folder that keeps its name and stars.
pub const META: &str = ".figcraft-workspace.json";
/// The extension of the files a project lists.
const EXT: &str = "figcraft";
/// The most workspaces the app remembers.
pub const MAX_WORKSPACES: usize = 32;
/// The most stars a workspace keeps.
const MAX_STARS: usize = 500;
/// The most projects and files a listing returns.
const MAX_LISTED: usize = 5000;
/// The longest name (in characters) a workspace, project or file can be given.
const MAX_NAME: usize = 120;

/// What `.figcraft-workspace.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Meta {
    format: u32,
    name: String,
    /// Paths relative to the workspace folder, with `/`: `Drafts/Logo.figcraft`, `Website`.
    starred: Vec<String>,
}

/// A workspace as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Workspace {
    /// Its folder, as the preferences have it.
    pub path: String,
    pub name: String,
    /// Starred files and projects, relative to the folder with `/`.
    pub starred: Vec<String>,
    /// The folder isn't there (moved, renamed, on a drive that isn't connected).
    pub missing: bool,
}

/// A project (or Drafts) of a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Project {
    pub name: String,
    pub path: String,
    /// How many files it lists.
    pub files: usize,
    pub starred: bool,
    pub drafts: bool,
}

/// Where a file sits in a workspace (see [`locate`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Location {
    /// The workspace's folder and name.
    pub workspace: String,
    #[serde(rename = "workspaceName")]
    pub workspace_name: String,
    /// The project's folder name (`Drafts` for drafts).
    pub project: String,
    pub drafts: bool,
}

// ---- paths ----

/// Whether `a` and `b` name the same place, comparing their components as written (not resolving
/// links: this runs every frame), without regard to case on Windows and macOS.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s }
            })
            .collect()
    };
    norm(a) == norm(b)
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `name` as a file or folder name: trimmed, at most [`MAX_NAME`] characters, without the
/// characters file systems refuse, and never empty, `.` or `..`.
fn clean_name(cmd: &str, name: &str) -> Result<String> {
    let name: String = crate::cmd::file::safe_file_name(name.trim()).chars().take(MAX_NAME).collect();
    let name = name.trim().trim_end_matches('.').to_string();
    if name.is_empty() || name.starts_with('.') {
        return Err(bad(cmd, "give it a name that doesn't start with a dot"));
    }
    Ok(name)
}

/// The first of `stem.ext`, `stem 2.ext`, `stem 3.ext`… (or folder names without `ext`) that
/// nothing in `dir` has.
fn unique(dir: &Path, stem: &str, ext: Option<&str>) -> PathBuf {
    let make = |n: u32| {
        let base = if n < 2 { stem.to_string() } else { format!("{stem} {n}") };
        dir.join(match ext {
            Some(e) => format!("{base}.{e}"),
            None => base,
        })
    };
    (1..10_000).map(make).find(|p| !file_access::exists(p) && file_access::link_metadata(p).is_err()).unwrap_or_else(|| make(10_000))
}

/// The path of `rel` (`Drafts/Logo.figcraft`) in workspace folder `root`.
fn join_rel(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").fold(root.to_path_buf(), |p, c| p.join(c))
}

/// `path` relative to workspace folder `root`, with `/`, when it lies in it.
fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let root_n = root.components().count();
    let parts: Vec<_> = path.components().collect();
    let (head, rest) = (parts.get(..root_n)?, parts.get(root_n..)?);
    if rest.is_empty() || !same_path(&head.iter().collect::<PathBuf>(), root) {
        return None;
    }
    Some(rest.iter().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
}

// ---- reading ----

fn read_meta(root: &Path) -> Meta {
    file_access::read_capped(root.join(META), 1 << 20).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_meta(root: &Path, meta: &Meta) -> Result<()> {
    let mut meta = meta.clone();
    meta.format = 1;
    meta.starred.truncate(MAX_STARS);
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| EngineError::Other(e.to_string()))?;
    file_access::write(root.join(META), &bytes)
}

/// The workspace at folder `path`: its name (the folder's when none was given) and stars.
pub fn load(path: &str) -> Workspace {
    let root = Path::new(path);
    let missing = !file_access::is_dir(root);
    let meta = if missing { Meta::default() } else { read_meta(root) };
    let folder = root.file_name().map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned());
    let name = if meta.name.trim().is_empty() { folder } else { meta.name.clone() };
    Workspace { path: path.to_string(), name, starred: meta.starred, missing }
}

/// The workspaces the app knows, in the order they were added.
pub fn workspaces(prefs: &Prefs) -> Vec<Workspace> {
    prefs.workspaces.iter().map(|p| load(p)).collect()
}

/// The workspace the home screen shows and new files go to.
pub fn active(prefs: &Prefs) -> Option<Workspace> {
    let path = prefs.active_workspace.as_ref().filter(|a| prefs.workspaces.iter().any(|w| same_path(Path::new(w), Path::new(a))))?;
    Some(load(path))
}

/// The projects of `ws`: Drafts first (made when it is missing), then the others by name.
pub fn projects(ws: &Workspace) -> Result<Vec<Project>> {
    let root = Path::new(&ws.path);
    if ws.missing {
        return Err(EngineError::Other(format!("{}: the workspace's folder isn't there", ws.path)));
    }
    let drafts = root.join(DRAFTS);
    if !file_access::is_dir(&drafts) {
        file_access::create_dir_all(&drafts)?;
    }
    let mut out = Vec::new();
    for dir in file_access::read_dir(root)?.into_iter().filter(|p| file_access::is_dir(p)).take(MAX_LISTED) {
        let Some(name) = dir.file_name().map(|f| f.to_string_lossy().into_owned()) else { continue };
        if name.starts_with('.') {
            continue;
        }
        let is_drafts = name.eq_ignore_ascii_case(DRAFTS);
        let files = files_in(&dir).map(|f| f.len()).unwrap_or(0);
        out.push(Project { starred: ws.starred.iter().any(|s| s.eq_ignore_ascii_case(&name)), path: path_str(&dir), name, files, drafts: is_drafts });
    }
    out.sort_by(|a, b| b.drafts.cmp(&a.drafts).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// The `.figcraft` files directly in folder `dir`.
fn files_in(dir: &Path) -> Result<Vec<String>> {
    let mut files: Vec<String> = file_access::read_dir(dir)?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT)))
        .filter(|p| file_access::metadata(p).is_ok_and(|m| m.is_file()))
        .take(MAX_LISTED)
        .map(|p| path_str(&p))
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    Ok(files)
}

/// The project of `ws` named `name` (any case; `Drafts` for drafts).
fn project_dir(cmd: &str, ws: &Workspace, name: &str) -> Result<PathBuf> {
    let found = projects(ws)?.into_iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
    found.map(|p| PathBuf::from(p.path)).ok_or_else(|| bad(cmd, format!("{name} isn't a project of {}", ws.name)))
}

/// The files of project `name` in `ws`.
pub fn files(ws: &Workspace, name: &str) -> Result<Vec<String>> {
    files_in(&project_dir("workspace.files", ws, name)?)
}

/// The starred files of `ws` that are there.
pub fn starred_files(ws: &Workspace) -> Vec<String> {
    let root = Path::new(&ws.path);
    ws.starred.iter().filter(|s| s.contains('/')).map(|s| join_rel(root, s)).filter(|p| file_access::exists(p)).map(|p| path_str(&p)).collect()
}

/// Where `file` sits: directly in a project folder (or Drafts) of a workspace the app knows.
pub fn locate(prefs: &Prefs, file: &str) -> Option<Location> {
    let project_dir = Path::new(file).parent()?;
    let root = project_dir.parent()?;
    let ws = prefs.workspaces.iter().find(|w| same_path(Path::new(w), root))?;
    let project = project_dir.file_name()?.to_string_lossy().into_owned();
    if project.starts_with('.') {
        return None;
    }
    let loaded = load(ws);
    Some(Location { workspace: ws.clone(), workspace_name: loaded.name, drafts: project.eq_ignore_ascii_case(DRAFTS), project })
}

/// Whether `file` is in a workspace the app knows (it saves itself: see the app's auto-save).
pub fn in_workspace(prefs: &Prefs, file: &str) -> bool {
    let Some(root) = Path::new(file).parent().and_then(Path::parent) else { return false };
    prefs.workspaces.iter().any(|w| same_path(Path::new(w), root))
}

// ---- changes ----

/// After `from` moved to `to` (a file, or a folder and everything in it): the tabs, the recent
/// files and the workspace's stars follow.
fn moved(s: &mut Session, root: &Path, from: &Path, to: &Path) {
    let follow = |p: &str| -> Option<String> {
        let p = Path::new(p);
        if same_path(p, from) {
            return Some(path_str(to));
        }
        let rest = rel_of(from, p)?;
        Some(path_str(&join_rel(to, &rest)))
    };
    for ds in &mut s.docs {
        if let Some(new) = ds.path.as_deref().and_then(follow) {
            ds.path = Some(new);
            ds.note_on_disk();
        }
    }
    for r in &mut s.prefs.recent_files {
        if let Some(new) = follow(r) {
            *r = new;
        }
    }
    s.prefs_unsaved = true;
    let mut meta = read_meta(root);
    let (Some(old_rel), Some(new_rel)) = (rel_of(root, from), rel_of(root, to)) else { return };
    let mut changed = false;
    for star in &mut meta.starred {
        let starred = Path::new(star.as_str()).to_path_buf();
        if same_path(&starred, Path::new(&old_rel)) {
            *star = new_rel.clone();
            changed = true;
        } else if let Some(rest) = rel_of(Path::new(&old_rel), &starred) {
            *star = format!("{new_rel}/{rest}");
            changed = true;
        }
    }
    if changed && let Err(e) = write_meta(root, &meta) {
        log::warn!("workspace stars not updated: {e}");
    }
}

/// After `gone` (a file or a folder) was deleted: forgotten by the recent files and the stars.
fn deleted(s: &mut Session, root: &Path, gone: &Path) {
    let inside = |p: &str| same_path(Path::new(p), gone) || rel_of(gone, Path::new(p)).is_some();
    s.prefs.recent_files.retain(|r| !inside(r));
    s.prefs_unsaved = true;
    let mut meta = read_meta(root);
    let before = meta.starred.len();
    meta.starred.retain(|star| !inside(&path_str(&join_rel(root, star))));
    if meta.starred.len() != before
        && let Err(e) = write_meta(root, &meta)
    {
        log::warn!("workspace stars not updated: {e}");
    }
}

/// The workspace a command acts on: `path` when given (one the app knows), else the active one.
fn target(cmd: &str, s: &Session, p: &Value) -> Result<Workspace> {
    match string(p, "workspace") {
        Some(w) => {
            let known = s.prefs.workspaces.iter().find(|k| same_path(Path::new(k), Path::new(&w)));
            known.map(|k| load(k)).ok_or_else(|| bad(cmd, format!("{w} isn't a workspace FigCraft knows (workspace.add)")))
        }
        None => active(&s.prefs).ok_or_else(|| bad(cmd, "no workspace yet: add one with workspace.add")),
    }
}

/// The workspace file `file` belongs to (a `.figcraft` file in one of its projects).
fn file_target(cmd: &str, s: &Session, file: &str) -> Result<(Workspace, Location)> {
    let loc = locate(&s.prefs, file).ok_or_else(|| bad(cmd, format!("{file} isn't in a project of a workspace")))?;
    if !Path::new(file).extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT)) {
        return Err(bad(cmd, format!("{file} isn't a .figcraft file")));
    }
    if !file_access::exists(file) {
        return Err(bad(cmd, format!("{file} isn't there any more")));
    }
    Ok((load(&loc.workspace), loc))
}

/// Close the tabs that have `path` (or a file in folder `path`) open, so it can go; refused, with
/// nothing closed, while one of them has unsaved changes.
fn close_tabs(cmd: &str, s: &mut Session, path: &Path, what: &str) -> Result<()> {
    let open = |d: &crate::DocState| d.path.as_deref().is_some_and(|p| same_path(Path::new(p), path) || rel_of(path, Path::new(p)).is_some());
    if s.docs.iter().any(|d| open(d) && d.is_dirty()) {
        return Err(bad(cmd, format!("{what} has unsaved changes in a tab: save or close it first")));
    }
    while let Some(i) = s.docs.iter().rposition(open) {
        s.execute("file.close", &json!({"index": i, "force": true}))?;
    }
    Ok(())
}

fn workspace_json(ws: &Workspace, active: bool) -> Value {
    json!({"path": ws.path, "name": ws.name, "missing": ws.missing, "active": active})
}

fn list(s: &Session) -> Value {
    let act = active(&s.prefs).map(|a| a.path);
    let all: Vec<Value> = workspaces(&s.prefs).iter().map(|w| workspace_json(w, act.as_deref() == Some(w.path.as_str()))).collect();
    json!({"active": act, "workspaces": all})
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.add";
    let path = req_string(CMD, p, "path")?;
    let path = path.trim();
    if path.is_empty() || !Path::new(path).is_absolute() {
        return Err(bad(CMD, "`path` must be a full folder path"));
    }
    if s.prefs.workspaces.iter().any(|w| same_path(Path::new(w), Path::new(path))) {
        s.prefs.active_workspace = Some(path.to_string());
        s.prefs_unsaved = true;
        return Ok(json!({"workspace": workspace_json(&load(path), true), "existed": true}));
    }
    if s.prefs.workspaces.len() >= MAX_WORKSPACES {
        return Err(bad(CMD, format!("FigCraft keeps at most {MAX_WORKSPACES} workspaces: forget one first")));
    }
    let root = Path::new(path);
    if file_access::exists(root) && !file_access::is_dir(root) {
        return Err(bad(CMD, format!("{path} is a file, not a folder")));
    }
    file_access::create_dir_all(root.join(DRAFTS))?;
    let mut meta = read_meta(root);
    if let Some(name) = string(p, "name").filter(|n| !n.trim().is_empty()) {
        meta.name = name.trim().chars().take(MAX_NAME).collect();
    }
    if !file_access::exists(root.join(META)) || string(p, "name").is_some() {
        write_meta(root, &meta)?;
    }
    s.prefs.workspaces.push(path.to_string());
    s.prefs.active_workspace = Some(path.to_string());
    s.prefs_unsaved = true;
    Ok(json!({"workspace": workspace_json(&load(path), true), "existed": false}))
}

fn forget(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.forget";
    let ws = target(CMD, s, p)?;
    s.prefs.workspaces.retain(|w| !same_path(Path::new(w), Path::new(&ws.path)));
    if s.prefs.active_workspace.as_deref().is_some_and(|a| same_path(Path::new(a), Path::new(&ws.path))) {
        s.prefs.active_workspace = s.prefs.workspaces.first().cloned();
    }
    s.prefs_unsaved = true;
    Ok(list(s))
}

fn switch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.switch";
    if get(p, "workspace").is_none() {
        return Err(bad(CMD, "`workspace` (a workspace's folder) is required"));
    }
    let ws = target(CMD, s, p)?;
    s.prefs.active_workspace = Some(ws.path);
    s.prefs_unsaved = true;
    Ok(list(s))
}

fn rename_workspace(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.rename";
    let name = req_string(CMD, p, "name")?;
    let name: String = name.trim().chars().take(MAX_NAME).collect();
    if name.is_empty() {
        return Err(bad(CMD, "`name` can't be empty"));
    }
    let ws = target(CMD, s, p)?;
    let root = Path::new(&ws.path);
    let mut meta = read_meta(root);
    meta.name = name;
    write_meta(root, &meta)?;
    Ok(json!({"workspace": workspace_json(&load(&ws.path), active(&s.prefs).is_some_and(|a| a.path == ws.path))}))
}

fn projects_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ws = target("workspace.projects", s, p)?;
    Ok(json!({"workspace": ws.path, "projects": projects(&ws)?}))
}

fn file_json(ws: &Workspace, path: &str) -> Value {
    let root = Path::new(&ws.path);
    let starred = rel_of(root, Path::new(path)).is_some_and(|r| ws.starred.iter().any(|s| same_path(Path::new(s), Path::new(&r))));
    let name = Path::new(path).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    json!({"path": path, "name": name, "starred": starred})
}

fn files_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.files";
    let ws = target(CMD, s, p)?;
    let list = if boolean(p, "starred") == Some(true) {
        starred_files(&ws)
    } else {
        files_in(&project_dir(CMD, &ws, &string(p, "project").unwrap_or_else(|| DRAFTS.into()))?)?
    };
    Ok(json!({"files": list.iter().map(|f| file_json(&ws, f)).collect::<Vec<_>>()}))
}

fn new_project(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.newProject";
    let ws = target(CMD, s, p)?;
    let name = clean_name(CMD, &req_string(CMD, p, "name")?)?;
    if name.eq_ignore_ascii_case(DRAFTS) {
        return Err(bad(CMD, "Drafts is already part of every workspace"));
    }
    projects(&ws)?;
    let dir = unique(Path::new(&ws.path), &name, None);
    file_access::create_dir_all(&dir)?;
    let name = dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(name);
    Ok(json!({"project": {"name": name, "path": path_str(&dir)}}))
}

fn rename_project(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.renameProject";
    let ws = target(CMD, s, p)?;
    let from = project_dir(CMD, &ws, &req_string(CMD, p, "project")?)?;
    if from.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(DRAFTS)) {
        return Err(bad(CMD, "Drafts can't be renamed"));
    }
    let name = clean_name(CMD, &req_string(CMD, p, "name")?)?;
    if name.eq_ignore_ascii_case(DRAFTS) {
        return Err(bad(CMD, "Drafts is already part of every workspace"));
    }
    let root = PathBuf::from(&ws.path);
    let to = root.join(&name);
    if same_path(&from, &to) {
        return Ok(json!({"project": {"name": name, "path": path_str(&from)}}));
    }
    // A change of case only: through a free name, since some file systems call both the same.
    if from.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy()) {
        let step = unique(&root, &format!(".rename-{name}"), None);
        file_access::rename(&from, &step)?;
        file_access::rename(&step, &to)?;
    } else {
        file_access::rename(&from, &to)?;
    }
    moved(s, &root, &from, &to);
    Ok(json!({"project": {"name": name, "path": path_str(&to)}}))
}

fn delete_project(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.deleteProject";
    let ws = target(CMD, s, p)?;
    let dir = project_dir(CMD, &ws, &req_string(CMD, p, "project")?)?;
    if dir.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(DRAFTS)) {
        return Err(bad(CMD, "Drafts can't be deleted"));
    }
    close_tabs(CMD, s, &dir, "A file of this project")?;
    file_access::trash(&dir)?;
    deleted(s, Path::new(&ws.path), &dir);
    Ok(json!({"deleted": path_str(&dir)}))
}

fn new_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.newFile";
    let ws = target(CMD, s, p)?;
    let dir = project_dir(CMD, &ws, &string(p, "project").unwrap_or_else(|| DRAFTS.into()))?;
    let board = match string(p, "kind").as_deref() {
        None | Some("design") => false,
        Some("board") => true,
        Some(k) => return Err(bad(CMD, format!("`kind` must be design or board, not {k}"))),
    };
    let default = if board { "Untitled board" } else { "Untitled" };
    let name = match string(p, "name").filter(|n| !n.trim().is_empty()) {
        Some(n) => clean_name(CMD, &n)?,
        None => default.to_string(),
    };
    let path = unique(&dir, &name, Some(EXT));
    let stem = path.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or(name);
    s.execute(if board { "file.newBoard" } else { "file.new" }, &json!({"name": stem}))?;
    let index = s.active;
    if let Err(e) = s.execute("file.save", &json!({"path": path_str(&path)})) {
        // Not saved: the new tab goes again, so no unsaved stray is left behind.
        let _ = s.execute("file.close", &json!({"index": index, "force": true}));
        return Err(e);
    }
    s.prefs_unsaved = true;
    Ok(json!({"index": s.active, "path": path_str(&path), "name": stem}))
}

fn move_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.moveFile";
    let file = req_string(CMD, p, "file")?;
    let (ws, loc) = file_target(CMD, s, &file)?;
    let dir = project_dir(CMD, &ws, &req_string(CMD, p, "project")?)?;
    let from = PathBuf::from(&file);
    if from.parent().is_some_and(|d| same_path(d, &dir)) {
        return Ok(json!({"path": file, "moved": false}));
    }
    let stem = from.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let to = unique(&dir, &stem, Some(EXT));
    file_access::rename(&from, &to)?;
    moved(s, Path::new(&loc.workspace), &from, &to);
    Ok(json!({"path": path_str(&to), "moved": true}))
}

fn rename_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.renameFile";
    let file = req_string(CMD, p, "file")?;
    let (_, loc) = file_target(CMD, s, &file)?;
    let name = clean_name(CMD, &req_string(CMD, p, "name")?)?;
    let from = PathBuf::from(&file);
    let dir = from.parent().map(Path::to_path_buf).unwrap_or_default();
    let to = dir.join(format!("{name}.{EXT}"));
    if !same_path(&from, &to) {
        if from.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy()) {
            let step = unique(&dir, &format!(".rename-{name}"), Some(EXT));
            file_access::rename(&from, &step)?;
            file_access::rename(&step, &to)?;
        } else {
            file_access::rename(&from, &to)?;
        }
        moved(s, Path::new(&loc.workspace), &from, &to);
    }
    // An open tab shows the new name too (its document's name, saved with it).
    let to_s = path_str(&to);
    for ds in s.docs.iter_mut().filter(|d| d.path.as_deref() == Some(to_s.as_str())) {
        if ds.doc.name != name {
            ds.doc_mut().name = name.clone();
        }
    }
    Ok(json!({"path": to_s, "name": name}))
}

fn delete_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.deleteFile";
    let file = req_string(CMD, p, "file")?;
    let (_, loc) = file_target(CMD, s, &file)?;
    close_tabs(CMD, s, Path::new(&file), "This file")?;
    file_access::trash(&file)?;
    deleted(s, Path::new(&loc.workspace), Path::new(&file));
    Ok(json!({"deleted": file}))
}

fn star(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "workspace.star";
    let starred = boolean(p, "starred").unwrap_or(true);
    let (root, rel) = match (string(p, "file"), string(p, "project")) {
        (Some(file), None) => {
            let (ws, _) = file_target(CMD, s, &file)?;
            let root = PathBuf::from(&ws.path);
            let rel = rel_of(&root, Path::new(&file)).ok_or_else(|| bad(CMD, "not in the workspace"))?;
            (root, rel)
        }
        (None, Some(project)) => {
            let ws = target(CMD, s, p)?;
            let dir = project_dir(CMD, &ws, &project)?;
            let root = PathBuf::from(&ws.path);
            let rel = rel_of(&root, &dir).ok_or_else(|| bad(CMD, "not in the workspace"))?;
            (root, rel)
        }
        _ => return Err(bad(CMD, "give `file` or `project`")),
    };
    let mut meta = read_meta(&root);
    meta.starred.retain(|x| !same_path(Path::new(x), Path::new(&rel)));
    if starred {
        if meta.starred.len() >= MAX_STARS {
            return Err(bad(CMD, format!("a workspace keeps at most {MAX_STARS} stars")));
        }
        meta.starred.insert(0, rel.clone());
    }
    write_meta(&root, &meta)?;
    Ok(json!({"starred": starred, "path": rel}))
}

fn locate_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let file = match string(p, "path") {
        Some(f) => f,
        None => s.ds()?.path.clone().ok_or_else(|| bad("workspace.locate", "the file in front isn't saved yet"))?,
    };
    Ok(json!({"location": locate(&s.prefs, &file)}))
}

/// The workspace commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "workspace.list",
            "Workspaces",
            "{}  the workspaces FigCraft knows (folders) and the active one → {active, workspaces: [{path, name, missing, active}]}",
            false,
            |s, _| Ok(list(s))
        ),
        spec!(
            "workspace.add",
            "Add workspace",
            "{path (a folder: made when missing, an existing one is adopted with its folders as projects), name? (default: the folder's name)}  adds it with a Drafts folder and makes it active → {workspace, existed}",
            false,
            add
        ),
        spec!(
            "workspace.forget",
            "Forget workspace",
            "{workspace? (default the active one)}  FigCraft stops listing it; its folder and files stay → the list",
            false,
            forget
        ),
        spec!("workspace.switch", "Switch workspace", "{workspace (its folder)}  makes it the active one → the list", false, switch),
        spec!(
            "workspace.rename",
            "Rename workspace",
            "{name, workspace?}  the name shown (kept in its folder's .figcraft-workspace.json; the folder keeps its name)",
            false,
            rename_workspace
        ),
        spec!(
            "workspace.projects",
            "Workspace projects",
            "{workspace?}  Drafts, then the projects (its folders) by name → {workspace, projects: [{name, path, files, starred, drafts}]}",
            false,
            projects_cmd
        ),
        spec!(
            "workspace.files",
            "Workspace files",
            "{project? (default Drafts) | starred: true, workspace?}  the .figcraft files in a project, or the starred files → {files: [{path, name, starred}]}",
            false,
            files_cmd
        ),
        spec!(
            "workspace.newProject",
            "New project",
            "{name, workspace?}  a project (a folder; a taken name gets a number) → {project: {name, path}}",
            false,
            new_project
        ),
        spec!(
            "workspace.renameProject",
            "Rename project",
            "{project, name, workspace?}  renames its folder; open tabs, recent files and stars follow → {project: {name, path}}",
            false,
            rename_project
        ),
        spec!(
            "workspace.deleteProject",
            "Delete project",
            "{project, workspace?}  moves its folder and files to the Recycle Bin / Trash (not Drafts); tabs that have its files open close, unless one has unsaved changes (then nothing happens) → {deleted}",
            false,
            delete_project
        ),
        spec!(
            "workspace.newFile",
            "New file in workspace",
            "{project? (default Drafts), kind? (design|board), name? (default Untitled; a taken name gets a number), workspace?}  a new file saved in the project at once, opened in a tab → {index, path, name}",
            false,
            new_file
        ),
        spec!(
            "workspace.moveFile",
            "Move file to project",
            "{file (its path), project}  moves it into another project of its workspace (a taken name gets a number); an open tab, recent files and stars follow → {path, moved}",
            false,
            move_file
        ),
        spec!(
            "workspace.renameFile",
            "Rename workspace file",
            "{file, name}  renames it on disk and, when it's open, its document → {path, name}",
            false,
            rename_file
        ),
        spec!(
            "workspace.deleteFile",
            "Delete workspace file",
            "{file}  moves it to the Recycle Bin / Trash; a tab that has it open closes, unless it has unsaved changes (then nothing happens) → {deleted}",
            false,
            delete_file
        ),
        spec!(
            "workspace.star",
            "Star",
            "{file | project, starred? (default true), workspace? (for a project)}  stars or unstars it (kept in the workspace folder) → {starred, path}",
            false,
            star
        ),
        spec!(
            "workspace.locate",
            "Locate in workspace",
            "{path? (default the file in front)}  where a file sits → {location: {workspace, workspaceName, project, drafts} | null}",
            false,
            locate_cmd
        ),
    ]
}

/// Paths for tests and callers that hold a workspace.
pub fn drafts_dir(ws: &Workspace) -> PathBuf {
    Path::new(&ws.path).join(DRAFTS)
}

/// Whether a workspace's folder may be written now (automation roots).
pub fn writable(ws: &Workspace) -> bool {
    file_access::allowed(&ws.path, Access::Write)
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
