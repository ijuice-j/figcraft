//! Preferences: one window, laid out as a settings page. Groups on the left (General, Appearance,
//! Canvas, Online sharing, AI tools), the chosen group's settings on the right.

use egui::{Align, Context, CornerRadius, Layout, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{self, tr, tr_noop, trf};
use crate::theme::{Tokens, regular, semibold};

/// A group of settings: a row on the left of Preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrefsGroup {
    #[default]
    General,
    Appearance,
    Canvas,
    OnlineSharing,
    AiTools,
}

impl PrefsGroup {
    pub const ALL: [PrefsGroup; 5] = [Self::General, Self::Appearance, Self::Canvas, Self::OnlineSharing, Self::AiTools];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => tr("General"),
            Self::Appearance => tr("Appearance"),
            Self::Canvas => tr("Canvas"),
            Self::OnlineSharing => tr("Online sharing"),
            Self::AiTools => tr("AI tools"),
        }
    }

    /// One line under the group's title.
    fn summary(self) -> &'static str {
        match self {
            Self::General => tr("Your name, language and how files are kept."),
            Self::Appearance => tr("Theme, interface size and motion."),
            Self::Canvas => tr("How tools, zoom and nudging behave on the canvas."),
            Self::OnlineSharing => tr("Where files shared online are kept."),
            Self::AiTools => tr("Features that send an image to a service you connect. Nothing is sent until you set one up."),
        }
    }
}

/// Width of the groups column.
const NAV: f32 = 168.0;

pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.preferences {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let width = (screen.width() - 64.0).clamp(380.0, 720.0);
    let height = (screen.height() - 160.0).clamp(260.0, 500.0);
    let mut open = true;
    super::window(tr("Preferences"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(width);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(NAV, height), Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(vec2(NAV, height));
                nav(app, ui, &t);
            });
            let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
            ui.painter().line_segment([line.center_top(), line.center_bottom()], Stroke::new(1.0, t.border));
            ui.add_space(12.0);
            let body = (width - NAV - 26.0).max(200.0);
            ui.allocate_ui_with_layout(vec2(body, height), Layout::top_down(Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("preferences_page").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(body - 14.0);
                    let group = app.ui.prefs_group;
                    ui.label(RichText::new(group.label()).font(semibold(15.0)).color(t.text));
                    ui.add(egui::Label::new(RichText::new(group.summary()).font(regular(11.0)).color(t.text_secondary)).wrap());
                    ui.add_space(12.0);
                    match group {
                        PrefsGroup::General => general(app, ui, &t),
                        PrefsGroup::Appearance => appearance(app, ui, &t),
                        PrefsGroup::Canvas => canvas(app, ui),
                        PrefsGroup::OnlineSharing => online_sharing(app, ui, &t),
                        PrefsGroup::AiTools => crate::remove_bg_ui::section(app, ui),
                    }
                });
            });
        });
    });
    if !open {
        app.ui.preferences = false;
    }
}

/// The groups, one row each.
fn nav(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    for group in PrefsGroup::ALL {
        let label = group.label();
        let current = app.ui.prefs_group == group;
        let (r, resp) = ui.allocate_exact_size(vec2(NAV - 8.0, 30.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, current, label));
        if current || resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), if current { t.selected_row } else { t.hover });
        }
        let font = if current { semibold(12.0) } else { regular(12.0) };
        ui.painter().text(pos2(r.min.x + 10.0, r.center().y), egui::Align2::LEFT_CENTER, label, font, t.text);
        if resp.clicked() {
            app.ui.prefs_group = group;
        }
        ui.add_space(2.0);
    }
}

fn general(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let prefs = app.session.prefs.clone();
    ui.horizontal(|ui| {
        ui.label(tr("Your name"));
        crate::live::name_field(app, ui, egui::Id::unique("pref_user_name"), 160.0);
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tr("Language"));
        // Each language by its own name; System follows the OS (the browser on the web).
        let langs = || {
            [("system", tr("System"))].into_iter().chain(i18n::languages().iter().filter(|l| crate::cjk::can_show(l.code)).map(|l| (l.code, l.name)))
        };
        let on = |code: &str| app.session.prefs.language.eq_ignore_ascii_case(code);
        let selected = langs().find(|(code, _)| on(code)).map_or(tr("System"), |(_, name)| name);
        let resp = crate::widgets::dropdown(ui, selected, 220.0, None);
        let mut pick = None;
        crate::right::menu_popup(&resp).show(|ui| {
            for (code, name) in langs() {
                if crate::right::menu_check(ui, on(code), name, "") {
                    pick = Some(code);
                    ui.close();
                }
            }
        });
        if let Some(code) = pick {
            app.run("pref.language", json!({"language": code}));
            ui.ctx().request_repaint();
        }
    });
    if let Some(lang) = i18n::current().filter(|l| l.draft) {
        let note = trf("{language}: draft translation, waiting for a native speaker's review.", &[("language", &lang.name)]);
        crate::widgets::label(ui, &note, t.text_secondary);
    }
    ui.add_space(8.0);
    let mut toggles = vec![];
    // The browser has no files to save to.
    if !cfg!(target_arch = "wasm32") && crate::widgets::checkbox(ui, prefs.auto_save, tr("Save files automatically")) {
        toggles.push("pref.autoSave");
    }
    if !cfg!(target_arch = "wasm32") {
        if crate::widgets::checkbox(ui, prefs.start_on_home, tr("Start on the Home screen")) {
            toggles.push("pref.startOnHome");
        }
        let note = if prefs.start_on_home {
            tr("FigCraft opens on the Home screen, with your recent files.")
        } else {
            tr("FigCraft opens the files you had open when it last closed, or the Home screen if there were none.")
        };
        crate::widgets::label(ui, note, t.text_secondary);
    }
    for c in toggles {
        app.run(c, json!({}));
    }
    // Where the app registers itself as the opener of invite links (Windows).
    if let Some(on) = app.session.invite_links_on() {
        if crate::widgets::checkbox(ui, on, tr("Open invite links in FigCraft")) {
            app.run("collab.registerLinks", json!({"on": !on}));
        }
        crate::widgets::label(ui, tr("figcraft:// links from a browser, chat or email open this app and join the session."), t.text_secondary);
    }
}

fn appearance(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.label(tr("Theme"));
        for (l, c) in [(tr_noop("Light"), "view.themeLight"), (tr_noop("Dark"), "view.themeDark"), (tr_noop("System"), "view.themeSystem")] {
            if ui.selectable_label(app.session.prefs.theme.eq_ignore_ascii_case(l), tr(l)).clicked() {
                app.run(c, json!({}));
            }
        }
    });
    ui.add_space(8.0);
    super::interface_size(app, ui, app.session.prefs.ui_scale);
    ui.add_space(8.0);
    if crate::widgets::checkbox(ui, app.session.prefs.reduce_motion, tr("Reduce motion")) {
        app.run("pref.reduceMotion", json!({}));
    }
    if app.services.reduce_motion() {
        crate::widgets::label(ui, tr("Your system asks for reduced motion: animations stay off."), t.text_secondary);
    }
}

fn canvas(app: &mut FigApp, ui: &mut Ui) {
    let prefs = app.session.prefs.clone();
    let mut toggles = vec![];
    for (label, val, cmd) in [
        (tr("Keep tool selected after use"), prefs.keep_tool, "pref.keepTool"),
        (tr("Highlight layers on hover"), prefs.highlight_on_hover, "pref.highlightHover"),
        (tr("Rename duplicated layers"), prefs.rename_duplicates, "pref.renameDuplicates"),
        (tr("Show dimensions on objects"), prefs.show_dimensions, "pref.showDimensions"),
        (tr("Use number keys for opacity"), prefs.number_keys_opacity, "pref.numberKeysOpacity"),
        (tr("Scroll wheel zoom"), prefs.scroll_wheel_zoom, "pref.scrollWheelZoom"),
        (tr("Invert zoom direction"), prefs.invert_zoom, "pref.invertZoom"),
    ] {
        if crate::widgets::checkbox(ui, val, label) {
            toggles.push(cmd);
        }
    }
    for c in toggles {
        app.run(c, json!({}));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(tr("Nudge amount"));
        let mut small = prefs.nudge_small;
        let mut big = prefs.nudge_big;
        let a = ui.add(egui::DragValue::new(&mut small).range(0.1..=100.0));
        let b = ui.add(egui::DragValue::new(&mut big).range(1.0..=1000.0));
        if a.changed() || b.changed() {
            app.run("pref.nudge", json!({"small": small, "big": big}));
        }
    });
    ui.add_space(8.0);
    crate::board_bar::grid_settings(app, ui);
}

fn online_sharing(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let prefs = app.session.prefs.clone();
    ui.horizontal(|ui| {
        ui.label(tr("Server for online sharing"));
        // Where Share ▸ Online keeps files and join codes are asked; no key here
        // (preferences are saved as text).
        let name = tr("Server for online sharing");
        if let Some(url) = crate::widgets::text_field_named(ui, egui::Id::unique("pref_publish_server"), &prefs.publish_server, 220.0, name) {
            app.run("pref.publishServer", json!({"url": url}));
        }
        let usual = prefs.publish_server == figcraft_engine::live::DEFAULT_SERVER;
        if crate::widgets::button(ui, tr("Reset"), false, !usual).on_hover_text(tr("Back to the FigCraft server")).clicked() {
            app.run("pref.publishServer", json!({}));
        }
    });
    if prefs.publish_server.is_empty() {
        crate::widgets::label(ui, tr("Online sharing is off: files are shared on the local network only."), t.text_secondary);
    }
}
