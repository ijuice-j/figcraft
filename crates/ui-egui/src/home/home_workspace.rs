//! The home screen's workspace (fork feature): a switcher, Drafts, Starred and the projects in the
//! sidebar, and in the main area the chosen place's files as cards or a table, with buttons to make
//! a new file there. Without a workspace a setup card offers one in `Documents/FigCraft`.
//!
//! Everything it changes goes through the `workspace.*` commands ([`figcraft_engine::workspace`]);
//! this module only shows and asks. What the folders hold is read again every
//! [`RELIST_SECS`], and at once after a change, so files made, moved or deleted outside FigCraft
//! show up by themselves. The browser has no folders: there the section isn't offered.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, Id, Rect, Sense, Ui, pos2, vec2};
use figcraft_engine::workspace::{self, DRAFTS, Project, Workspace};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::home_recents::{Action, Menu, Recents, Sort, View};
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// How often the workspace's folders are listed again while in view (seconds).
const RELIST_SECS: f64 = 2.0;
/// The home section id the workspace's grid or table choice is kept under.
const VIEW_SECTION: &str = "workspace";

/// Which of the workspace's places the main area shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Place {
    #[default]
    Drafts,
    Starred,
    Project(String),
}

impl Place {
    /// The project folder name for the `workspace.*` commands (none for Starred).
    fn project(&self) -> Option<&str> {
        match self {
            Place::Drafts => Some(DRAFTS),
            Place::Project(p) => Some(p),
            Place::Starred => None,
        }
    }
}

/// What a dialog of the workspace asks.
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    NewProject(String),
    RenameProject { from: String, to: String },
    RenameFile { path: String, name: String },
    DeleteFile(String),
    DeleteProject(String),
    RenameWorkspace(String),
}

/// What the folders said at the last listing.
#[derive(Default)]
struct Listed {
    at: f64,
    /// The workspace and place it was listed for.
    key: Option<(String, Place)>,
    workspace: Option<Workspace>,
    projects: Vec<Project>,
    files: Vec<String>,
    starred: Vec<String>,
    error: Option<String>,
}

/// The workspace section's state.
#[derive(Default)]
pub struct State {
    pub place: Place,
    /// What is known about the files shown, and the selection.
    files: Recents,
    listed: Listed,
    ask: Option<Ask>,
    /// The setup card's fields.
    setup_name: String,
    setup_folder: String,
}

impl State {
    /// List the folders again on the next frame.
    pub fn relist(&mut self) {
        self.listed.at = f64::NEG_INFINITY;
    }
}

/// Where a first workspace goes: `Documents/FigCraft` in the home folder.
fn default_folder() -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    home.map(|h| h.join("Documents").join("FigCraft").to_string_lossy().into_owned()).unwrap_or_default()
}

/// Whether the workspace section is offered (the desktop app: the browser has no folders).
pub fn offered() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// List the active workspace's projects and the place's files when they are due.
fn relist(app: &mut FigApp, time: f64) {
    let prefs = &app.session.prefs;
    let state = &mut app.ui.home_view.workspace;
    let key = workspace::active(prefs).map(|w| (w.path.clone(), state.place.clone()));
    if key == state.listed.key && time - state.listed.at < RELIST_SECS {
        return;
    }
    let mut listed = Listed { at: time, key, ..Default::default() };
    if let Some(ws) = workspace::active(prefs) {
        if !ws.missing {
            match workspace::projects(&ws) {
                Ok(projects) => listed.projects = projects,
                Err(e) => listed.error = Some(e.to_string()),
            }
            listed.starred = workspace::starred_files(&ws);
            // A project that went away (renamed or deleted outside FigCraft) falls back to Drafts.
            if let Place::Project(p) = &state.place
                && !listed.projects.iter().any(|x| x.name.eq_ignore_ascii_case(p))
            {
                state.place = Place::Drafts;
            }
            listed.files = match &state.place {
                Place::Starred => listed.starred.clone(),
                place => workspace::files(&ws, place.project().unwrap_or(DRAFTS)).unwrap_or_default(),
            };
        }
        listed.workspace = Some(ws);
    }
    state.listed = listed;
}

// ---- sidebar ----

/// The workspace part of the home sidebar from `y` down to `bottom`: the switcher, Drafts,
/// Starred and the projects with a "+" for a new one. Returns where it ended.
pub fn sidebar(app: &mut FigApp, ui: &mut Ui, t: &Tokens, left: Rect, mut y: f32, bottom: f32) -> f32 {
    if !offered() {
        return y;
    }
    relist(app, ui.input(|i| i.time));
    let width = left.width() - 16.0;
    ui.painter().line_segment([pos2(left.min.x + 12.0, y), pos2(left.max.x - 12.0, y)], egui::Stroke::new(1.0, t.border));
    y += 10.0;
    let ws = app.ui.home_view.workspace.listed.workspace.clone();
    let switcher = Rect::from_min_size(pos2(left.min.x + 8.0, y), vec2(width, 32.0));
    switcher_row(app, ui, t, switcher, ws.as_ref());
    y += 38.0;
    if !ws.is_some_and(|w| !w.missing) {
        return y;
    }
    let in_ws = app.ui.home_view.section == crate::home::Section::Workspace && app.ui.home_view.file_query.trim().is_empty();
    let place = app.ui.home_view.workspace.place.clone();
    for (target, icon, label) in [(Place::Drafts, Icon::Page, tr("Drafts")), (Place::Starred, Icon::Star, tr("Starred"))] {
        let r = Rect::from_min_size(pos2(left.min.x + 8.0, y), vec2(width, 30.0));
        if row(ui, t, r, icon, label, in_ws && place == target, ("ws_place", label)).clicked() {
            go(app, target);
        }
        y += 32.0;
    }
    y += 6.0;
    let head = Rect::from_min_size(pos2(left.min.x + 8.0, y), vec2(width, 24.0));
    ui.painter().text(pos2(head.min.x + 10.0, head.center().y), Align2::LEFT_CENTER, tr("Projects"), semibold(11.0), t.text_secondary);
    let plus = Rect::from_center_size(pos2(head.max.x - 14.0, head.center().y), vec2(22.0, 22.0));
    let presp = ui.interact(plus, Id::unique("ws_new_project"), Sense::click());
    a11y::button(&presp, tr("New project"));
    if presp.hovered() {
        ui.painter().rect_filled(plus, CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), plus.shrink(4.0), Icon::Plus, t.icon);
    if presp.on_hover_text(tr("New project")).clicked() {
        app.ui.home_view.workspace.ask = Some(Ask::NewProject(String::new()));
    }
    y += 28.0;
    let projects: Vec<Project> = app.ui.home_view.workspace.listed.projects.iter().filter(|p| !p.drafts).cloned().collect();
    if projects.is_empty() {
        ui.painter().text(pos2(left.min.x + 18.0, y + 10.0), Align2::LEFT_CENTER, tr("No projects yet"), regular(11.0), t.text_tertiary);
        return y + 24.0;
    }
    // The projects scroll when they don't fit above Preferences.
    let area = Rect::from_min_max(pos2(left.min.x, y), pos2(left.max.x, bottom.max(y + 32.0)));
    let mut clicked = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::vertical().id_salt("ws_projects").auto_shrink([false, true]).show(ui, |ui| {
            // Rows a step apart like Drafts and Starred above.
            ui.spacing_mut().item_spacing.y = 0.0;
            for p in &projects {
                let (r, _) = ui.allocate_exact_size(vec2(area.width(), 32.0), Sense::hover());
                let r = Rect::from_min_size(pos2(r.min.x + 8.0, r.min.y), vec2(width, 30.0));
                let current = in_ws && place == Place::Project(p.name.clone());
                let resp = row(ui, t, r, if p.starred { Icon::Star } else { Icon::Library }, &p.name, current, ("ws_project", p.name.as_str()));
                if resp.clicked() {
                    clicked = Some(p.name.clone());
                }
            }
        });
    });
    if let Some(p) = clicked {
        go(app, Place::Project(p));
    }
    bottom
}

/// One sidebar row: an icon and a label, highlighted when `current`.
fn row(ui: &Ui, t: &Tokens, r: Rect, icon: Icon, label: &str, current: bool, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Response {
    let resp = ui.interact(r, Id::unique(salt), Sense::click());
    a11y::tab(&resp, label, current);
    if current || resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), if current { t.selected_row } else { t.hover });
    }
    icons::paint(ui.painter(), Rect::from_min_size(pos2(r.min.x + 10.0, r.center().y - 8.0), vec2(16.0, 16.0)), icon, t.icon);
    let text = widgets::elide_font(ui, label, r.width() - 46.0, regular(12.0));
    ui.painter().text(pos2(r.min.x + 36.0, r.center().y), Align2::LEFT_CENTER, text, regular(12.0), t.text);
    resp
}

/// The workspace's name with its menu: the other workspaces, New workspace, Add existing folder,
/// Rename, Show in folder and Forget.
fn switcher_row(app: &mut FigApp, ui: &mut Ui, t: &Tokens, r: Rect, ws: Option<&Workspace>) {
    let label = ws.map_or_else(|| tr("Set up a workspace").to_string(), |w| w.name.clone());
    let resp = ui.interact(r, Id::unique("ws_switcher"), Sense::click());
    a11y::button(&resp, &trf("Workspace: {name}", &[("name", &label)]));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_min_size(pos2(r.min.x + 10.0, r.center().y - 8.0), vec2(16.0, 16.0)), Icon::Home, t.icon);
    let text = widgets::elide_font(ui, &label, r.width() - 60.0, semibold(12.0));
    ui.painter().text(pos2(r.min.x + 36.0, r.center().y), Align2::LEFT_CENTER, text, semibold(12.0), t.text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(10.0, 10.0)), Icon::Chevron, t.icon_secondary);
    let all = workspace::workspaces(&app.session.prefs);
    if all.is_empty() {
        if resp.clicked() {
            go(app, Place::Drafts);
        }
        return;
    }
    let mut pick: Option<String> = None;
    let mut ask_rename = false;
    let (mut new_one, mut add_existing, mut forget, mut reveal) = (false, false, false, false);
    crate::right::menu_popup(&resp).show(|ui| {
        ui.set_min_width(220.0);
        for w in &all {
            let on = ws.is_some_and(|a| a.path == w.path);
            let name = if w.missing { trf("{name} (folder not found)", &[("name", &w.name)]) } else { w.name.clone() };
            if crate::right::menu_check(ui, on, &name, "") {
                pick = Some(w.path.clone());
                ui.close();
            }
        }
        ui.separator();
        if crate::right::menu_item(ui, tr("New workspace…"), "") {
            new_one = true;
            ui.close();
        }
        if crate::right::menu_item(ui, tr("Add existing folder…"), "") {
            add_existing = true;
            ui.close();
        }
        if ws.is_some() {
            ui.separator();
            if crate::right::menu_item(ui, tr("Rename workspace…"), "") {
                ask_rename = true;
                ui.close();
            }
            if crate::right::menu_item(ui, tr("Show in folder"), "") {
                reveal = true;
                ui.close();
            }
            if crate::right::menu_item(ui, tr("Forget workspace"), "") {
                forget = true;
                ui.close();
            }
        }
    });
    if let Some(path) = pick {
        app.run("workspace.switch", json!({"workspace": path}));
        go(app, Place::Drafts);
    }
    if new_one {
        // The setup card makes a new one.
        app.session.prefs.active_workspace = None;
        go(app, Place::Drafts);
    }
    if add_existing {
        add_folder(app);
    }
    if let (true, Some(w)) = (ask_rename, ws) {
        app.ui.home_view.workspace.ask = Some(Ask::RenameWorkspace(w.name.clone()));
    }
    if let (true, Some(w)) = (reveal, ws) {
        app.services.reveal_path(Path::new(&w.path));
    }
    if forget {
        app.run("workspace.forget", json!({}));
        go(app, Place::Drafts);
    }
}

/// Show `place` of the workspace in the main area.
fn go(app: &mut FigApp, place: Place) {
    let hv = &mut app.ui.home_view;
    hv.section = crate::home::Section::Workspace;
    hv.file_query.clear();
    hv.workspace.place = place;
    hv.workspace.relist();
}

/// Pick a folder and add it as a workspace (its folders become projects).
fn add_folder(app: &mut FigApp) {
    app.pick_folder(tr("Add existing folder"), |app, dir| {
        app.run("workspace.add", json!({"path": dir.to_string_lossy()}));
        go(app, Place::Drafts);
    });
}

// ---- main area ----

/// The workspace's main area: the setup card without a workspace, else the place's files.
pub fn section(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    relist(app, ui.input(|i| i.time));
    let ws = app.ui.home_view.workspace.listed.workspace.clone();
    match ws {
        None => setup(app, ui, t),
        Some(w) if w.missing => missing(app, ui, t, &w),
        Some(w) => files(app, ui, t, &w),
    }
}

/// "Set up your workspace": a name and a folder, filled in for a first one.
fn setup(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let state = &mut app.ui.home_view.workspace;
    if state.setup_folder.is_empty() {
        state.setup_folder = default_folder();
    }
    if state.setup_name.is_empty() {
        state.setup_name = tr("My workspace").to_string();
    }
    ui.add_space(28.0);
    ui.label(egui::RichText::new(tr("Set up your workspace")).font(semibold(18.0)).color(t.text));
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(tr(
            "A workspace is a folder for your files: new files go to its Drafts, and each folder in it is a project. Pick an empty folder, or one you already keep designs in.",
        ))
        .font(regular(12.0))
        .color(t.text_secondary),
    );
    ui.add_space(18.0);
    let mut create = false;
    let mut choose = false;
    let mut existing = false;
    egui::Grid::new("ws_setup").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
        ui.label(egui::RichText::new(tr("Name")).font(regular(12.0)).color(t.text_secondary));
        widgets::text_input(ui, Id::unique("ws_setup_name"), &mut app.ui.home_view.workspace.setup_name, 360.0, tr("My workspace"));
        ui.end_row();
        ui.label(egui::RichText::new(tr("Folder")).font(regular(12.0)).color(t.text_secondary));
        ui.horizontal(|ui| {
            widgets::text_input(ui, Id::unique("ws_setup_folder"), &mut app.ui.home_view.workspace.setup_folder, 360.0, "");
            choose = widgets::button(ui, tr("Choose…"), false, true).clicked();
        });
        ui.end_row();
    });
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        let ready = !app.ui.home_view.workspace.setup_folder.trim().is_empty();
        create = widgets::button(ui, tr("Create workspace"), true, ready).clicked() && ready;
        existing = widgets::button(ui, tr("Add existing folder…"), false, true).clicked();
    });
    if choose {
        app.pick_folder(tr("Choose a folder for the workspace"), |app, dir| {
            app.ui.home_view.workspace.setup_folder = dir.to_string_lossy().into_owned();
        });
    }
    if existing {
        add_folder(app);
    }
    if create {
        let state = &app.ui.home_view.workspace;
        let (path, name) = (state.setup_folder.trim().to_string(), state.setup_name.trim().to_string());
        if app.run("workspace.add", json!({"path": path, "name": name})).is_some() {
            go(app, Place::Drafts);
        }
    }
}

/// The active workspace's folder isn't there.
fn missing(app: &mut FigApp, ui: &mut Ui, t: &Tokens, ws: &Workspace) {
    ui.add_space(28.0);
    ui.label(egui::RichText::new(trf("{name}: folder not found", &[("name", &ws.name)])).font(semibold(16.0)).color(t.text));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(&ws.path).font(regular(12.0)).color(t.text_secondary));
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if widgets::button(ui, tr("Forget workspace"), false, true).clicked() {
            app.run("workspace.forget", json!({}));
            go(app, Place::Drafts);
        }
        if widgets::button(ui, tr("Add existing folder…"), false, true).clicked() {
            add_folder(app);
        }
    });
}

/// The place's heading with New design file, New board and the project's menu, then its files.
fn files(app: &mut FigApp, ui: &mut Ui, t: &Tokens, ws: &Workspace) {
    let place = app.ui.home_view.workspace.place.clone();
    let title = match &place {
        Place::Drafts => tr("Drafts").to_string(),
        Place::Starred => tr("Starred").to_string(),
        Place::Project(p) => p.clone(),
    };
    ui.add_space(24.0);
    let mut new_kind: Option<&str> = None;
    let mut project_menu: Option<&str> = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&title).font(semibold(18.0)).color(t.text));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(&ws.name).font(regular(12.0)).color(t.text_secondary));
        if place.project().is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Place::Project(_) = &place {
                    let more = icons::button(ui, Icon::Dots, 28.0, false, tr("Project"));
                    crate::right::menu_popup(&more).show(|ui| {
                        for (key, label) in [
                            ("rename", tr("Rename project…")),
                            ("star", tr("Star project")),
                            ("reveal", tr("Show in folder")),
                            ("delete", tr("Delete project…")),
                        ] {
                            let label = if key == "star" && starred_project(app, &place) { tr("Unstar project") } else { label };
                            if crate::right::menu_item(ui, label, "") {
                                project_menu = Some(key);
                                ui.close();
                            }
                        }
                    });
                }
                if widgets::button(ui, tr("New board"), false, true).clicked() {
                    new_kind = Some("board");
                }
                if widgets::button(ui, tr("New design file"), true, true).clicked() {
                    new_kind = Some("design");
                }
            });
        }
    });
    if let Some(e) = app.ui.home_view.workspace.listed.error.clone() {
        widgets::label(ui, &e, t.danger);
    }
    let project_names: Vec<String> = app.ui.home_view.workspace.listed.projects.iter().map(|p| p.name.clone()).collect();
    let paths = app.ui.home_view.workspace.listed.files.clone();
    let starred = app.ui.home_view.workspace.listed.starred.clone();
    let reduce_motion = app.reduce_motion();
    let (view, sort) = (View::of_section(&app.session.prefs, VIEW_SECTION), Sort::of(&app.session.prefs));
    let heading = match place {
        Place::Starred => tr("Starred files"),
        _ => tr("Files"),
    };
    let action = if paths.is_empty() {
        ui.add_space(28.0);
        let empty = match place {
            Place::Starred => tr("Star files from their right-click menu to keep them here."),
            _ => tr("No files here yet. Make one with New design file, or move files here from another project."),
        };
        widgets::label(ui, empty, t.text_tertiary);
        None
    } else {
        let menu = Menu::Workspace { projects: &project_names, starred: &starred };
        crate::home_recents::listing(ui, t, &mut app.ui.home_view.workspace.files, &paths, heading, view, sort, menu, reduce_motion)
    };
    if let Some(kind) = new_kind {
        let project = place.project().unwrap_or(DRAFTS).to_string();
        if app.run("workspace.newFile", json!({"project": project, "kind": kind})).is_some() {
            app.ui.home = false;
        }
        app.ui.home_view.workspace.relist();
    }
    if let (Some(key), Place::Project(p)) = (project_menu, &place) {
        match key {
            "rename" => app.ui.home_view.workspace.ask = Some(Ask::RenameProject { from: p.clone(), to: p.clone() }),
            "star" => {
                let on = !starred_project(app, &place);
                app.run("workspace.star", json!({"project": p, "starred": on}));
                app.ui.home_view.workspace.relist();
            }
            "reveal" => {
                if let Some(dir) = app.ui.home_view.workspace.listed.projects.iter().find(|x| &x.name == p).map(|x| x.path.clone()) {
                    app.services.reveal_path(Path::new(&dir));
                }
            }
            _ => app.ui.home_view.workspace.ask = Some(Ask::DeleteProject(p.clone())),
        }
    }
    act(app, action);
}

fn starred_project(app: &FigApp, place: &Place) -> bool {
    let Place::Project(p) = place else { return false };
    app.ui.home_view.workspace.listed.projects.iter().any(|x| &x.name == p && x.starred)
}

/// Do what was asked on a file of the listing.
fn act(app: &mut FigApp, action: Option<Action>) {
    let Some(action) = action else { return };
    match action {
        Action::Open(path) => {
            if app.run_user("file.open", json!({"path": path})).is_some() {
                app.ui.home = false;
            }
        }
        Action::Reveal(path) => app.services.reveal_path(Path::new(&path)),
        Action::Sort(sort) => {
            sort.store(&mut app.session.prefs);
            app.services.save_prefs(&app.session.prefs);
        }
        Action::View(view) => {
            view.store_for(&mut app.session.prefs, VIEW_SECTION);
            app.services.save_prefs(&app.session.prefs);
        }
        Action::Rename(path) => {
            let name = Path::new(&path).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            app.ui.home_view.workspace.ask = Some(Ask::RenameFile { path, name });
        }
        Action::Move(path, project) => {
            app.run("workspace.moveFile", json!({"file": path, "project": project}));
        }
        Action::Star(path, on) => {
            app.run("workspace.star", json!({"file": path, "starred": on}));
        }
        Action::Delete(path) => app.ui.home_view.workspace.ask = Some(Ask::DeleteFile(path)),
        Action::Remove(_) => {}
    }
    app.ui.home_view.workspace.relist();
}

// ---- dialogs ----

/// The workspace's dialog, if one is asking.
pub fn dialogs(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut ask) = app.ui.home_view.workspace.ask.take() else { return };
    let t = Tokens::get(ctx);
    let title = match &ask {
        Ask::NewProject(_) => tr("New project"),
        Ask::RenameProject { .. } => tr("Rename project"),
        Ask::RenameFile { .. } => tr("Rename file"),
        Ask::DeleteFile(_) => tr("Delete file"),
        Ask::DeleteProject(_) => tr("Delete project"),
        Ask::RenameWorkspace(_) => tr("Rename workspace"),
    };
    let mut open = true;
    let mut done: Option<(&str, serde_json::Value)> = None;
    let mut cancel = false;
    crate::dialogs::window(title, ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(360.0);
        let (text, confirm, danger) = match &mut ask {
            Ask::NewProject(name) => (Some(name), tr("Create"), false),
            Ask::RenameProject { to, .. } => (Some(to), tr("Rename"), false),
            Ask::RenameFile { name, .. } => (Some(name), tr("Rename"), false),
            Ask::RenameWorkspace(name) => (Some(name), tr("Rename"), false),
            Ask::DeleteFile(_) | Ask::DeleteProject(_) => (None, tr("Move to Recycle Bin"), true),
        };
        let mut enter = false;
        let ok = match text {
            Some(text) => {
                let field = widgets::text_input(ui, Id::unique("ws_dialog_text"), text, 336.0, tr("Name"));
                if !field.has_focus() && !field.lost_focus() {
                    field.request_focus();
                }
                enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                !text.trim().is_empty()
            }
            None => {
                let what = match &ask {
                    Ask::DeleteFile(p) => trf(
                        "Move “{name}” to the Recycle Bin? You can restore it from there.",
                        &[("name", &Path::new(p).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())],
                    ),
                    Ask::DeleteProject(p) => {
                        trf("Move the project “{name}” and all its files to the Recycle Bin? You can restore them from there.", &[("name", p)])
                    }
                    _ => String::new(),
                };
                ui.add(egui::Label::new(egui::RichText::new(what).font(regular(12.0)).color(t.text)).wrap());
                true
            }
        };
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if (widgets::button(ui, confirm, !danger, ok).clicked() || enter) && ok {
                done = Some(match &ask {
                    Ask::NewProject(name) => ("workspace.newProject", json!({"name": name.trim()})),
                    Ask::RenameProject { from, to } => ("workspace.renameProject", json!({"project": from, "name": to.trim()})),
                    Ask::RenameFile { path, name } => ("workspace.renameFile", json!({"file": path, "name": name.trim()})),
                    Ask::RenameWorkspace(name) => ("workspace.rename", json!({"name": name.trim()})),
                    Ask::DeleteFile(path) => ("workspace.deleteFile", json!({"file": path})),
                    Ask::DeleteProject(project) => ("workspace.deleteProject", json!({"project": project})),
                });
            }
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    if let Some((id, params)) = done {
        let result = app.run(id, params);
        let state = &mut app.ui.home_view.workspace;
        state.relist();
        if let Some(r) = result {
            match id {
                "workspace.newProject" | "workspace.renameProject" => {
                    if let Some(name) = r["project"]["name"].as_str() {
                        go(app, Place::Project(name.to_string()));
                    }
                }
                "workspace.deleteProject" => go(app, Place::Drafts),
                _ => {}
            }
        } else {
            // Refused (a taken name, a file that's open): the dialog stays for another try.
            app.ui.home_view.workspace.ask = Some(ask);
        }
        return;
    }
    if open && !cancel && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.home_view.workspace.ask = Some(ask);
    }
}

/// For `ui.inspect`: the workspace section's place, files and dialog.
pub fn inspect(app: &FigApp) -> serde_json::Value {
    let state = &app.ui.home_view.workspace;
    let place = match &state.place {
        Place::Drafts => json!("drafts"),
        Place::Starred => json!("starred"),
        Place::Project(p) => json!({"project": p}),
    };
    json!({
        "place": place,
        "workspace": state.listed.workspace.as_ref().map(|w| w.path.clone()),
        "files": state.listed.files,
        "dialog": state.ask.as_ref().map(|a| format!("{a:?}")),
    })
}
