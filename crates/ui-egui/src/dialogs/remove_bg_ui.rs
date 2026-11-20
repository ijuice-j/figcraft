//! Remove background in the interface: Preferences ▸ AI tools (connect a Cloudflare account, test,
//! disconnect), the card that offers to connect when there's no remover yet, the canvas banner
//! while a background is being removed, and putting finished cut-outs in their layers.
//!
//! The engine does the work ([`figcraft_engine::remove_bg`]); this module keeps the remover's
//! access key apart from the preferences through [`Services`](crate::Services) and runs the
//! Cloudflare install and the tests on threads of their own, so the window never waits.

use egui::{Color32, CornerRadius, Id, RichText, Sense, Stroke, Ui, vec2};
use figcraft_engine::EngineError;
use figcraft_engine::remove_bg::{Connection, SECRET_NAME};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// Cloudflare's sign-up page.
const SIGN_UP: &str = "https://dash.cloudflare.com/sign-up";
/// Where Cloudflare API keys ("API tokens") are made.
const API_TOKENS: &str = "https://dash.cloudflare.com/profile/api-tokens";

/// Which part of the AI tools card shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Connected, or the Connect button.
    #[default]
    Summary,
    /// The steps to connect a Cloudflare account.
    Connect,
    /// Typing a remover's address and key.
    Manual,
}

/// Where a connect or test running on its thread is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Checking,
    Installing,
    Address,
    Testing,
}

/// What the connect and test threads report.
#[cfg(not(target_arch = "wasm32"))]
enum Msg {
    Phase(Phase),
    Installed(Result<figcraft_engine::remove_bg::cloudflare::Installed, figcraft_engine::remove_bg::cloudflare::InstallError>),
    Tested(Result<std::time::Duration, String>),
}

/// Preferences ▸ AI tools' state.
#[derive(Default)]
pub struct AiState {
    pub mode: Mode,
    /// The Cloudflare API key being pasted. Never saved; cleared once it was used.
    pub token: String,
    pub manual_address: String,
    pub manual_key: String,
    /// "What happens when you connect" is closed (it starts open).
    pub explain_closed: bool,
    pub show_key: bool,
    pub error: Option<String>,
    /// The account that already has a remover: asking whether to replace it.
    pub replace_account: Option<String>,
    /// The last test: worked, and what to say.
    pub status: Option<(bool, String)>,
    /// A connect or test under way.
    pub phase: Option<Phase>,
    #[cfg(not(target_arch = "wasm32"))]
    rx: Option<std::sync::mpsc::Receiver<Msg>>,
}

/// Set the remover the preferences name, with its key from the app's secrets (at start).
pub fn load(app: &mut FigApp) {
    let address = app.session.prefs.remove_bg_address.clone();
    if address.is_empty() {
        return;
    }
    if let Some(key) = app.services.secret(SECRET_NAME)
        && let Ok(c) = Connection::new(&address, &key)
    {
        app.session.set_remove_bg(Some(c));
    }
}

/// What to tell the user when Remove background didn't work: the reason, as a sentence.
pub fn message(e: &EngineError) -> String {
    let text = match e {
        EngineError::Disabled(_, why) => why.clone(),
        EngineError::Other(why) => why.clone(),
        other => other.to_string(),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

/// Put finished cut-outs in their layers, and take what the connect and test threads say. Every
/// frame, before anything draws.
pub fn poll(app: &mut FigApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    take_messages(app);
    for job in app.session.remove_bg_ready() {
        match app.session.execute("image.removeBackground", &json!({"job": job})) {
            Ok(_) => app.toast(tr("Background removed. Undo brings the original back.")),
            Err(e) => app.show_error(message(&e)),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let threads = app.ui.ai.rx.is_some();
    #[cfg(target_arch = "wasm32")]
    let threads = false;
    // The threads don't wake the window: look again shortly while one runs.
    if threads || !app.session.remove_bg_working().is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
}

/// "Removing background" over the canvas while the open file waits for a cut-out, with Cancel.
pub fn banner(app: &mut FigApp, ctx: &egui::Context, canvas: egui::Rect) {
    let uid = app.session.ds().ok().map(|d| d.uid);
    let jobs: Vec<u64> = app.session.remove_bg_working().into_iter().filter(|w| Some(w.doc) == uid).map(|w| w.job).collect();
    if jobs.is_empty() {
        return;
    }
    let t = Tokens::get(ctx);
    let cancel = widgets::canvas_banner(ctx, Id::unique("remove_bg_banner"), canvas, |ui| {
        ui.add(egui::Spinner::new().size(12.0).color(t.text_secondary));
        ui.label(RichText::new(tr("Removing background…")).font(regular(11.0)).color(t.text));
        widgets::button(ui, tr("Cancel"), false, true).clicked()
    });
    if cancel {
        for job in jobs {
            app.session.remove_bg_cancel(job);
        }
        app.toast(tr("Stopped. The image wasn't changed."));
    }
}

/// The card Remove background opens when no remover is connected.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    if !app.ui.remove_bg_setup {
        return;
    }
    let t = Tokens::get(ctx);
    let (mut open, mut go, mut later) = (true, false, false);
    super::window(tr("Remove background"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(340.0);
        if cfg!(target_arch = "wasm32") {
            wrapped(ui, tr("Remove background works in the FigCraft desktop app for now."), t.text_secondary);
            ui.add_space(12.0);
            later = widgets::button(ui, tr("OK"), true, true).clicked();
            return;
        }
        ui.label(RichText::new(tr("Connect Cloudflare to remove backgrounds")).font(semibold(13.0)).color(t.text));
        ui.add_space(4.0);
        wrapped(
            ui,
            tr("FigCraft removes backgrounds with your own free Cloudflare account. It's free for up to 5,000 images a month and takes about two minutes to set up, once."),
            t.text_secondary,
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            go = widgets::button(ui, tr("Set it up in Preferences"), true, true).clicked();
            later = widgets::button(ui, tr("Not now"), false, true).clicked();
        });
    });
    if go {
        app.ui.preferences = true;
        app.ui.prefs_group = crate::dialogs::PrefsGroup::AiTools;
        if app.session.remove_bg().is_none() && app.ui.ai.mode == Mode::Summary {
            app.ui.ai.mode = Mode::Connect;
        }
    }
    if go || later || !open {
        app.ui.remove_bg_setup = false;
    }
}

fn wrapped(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(regular(11.0)).color(color)).wrap());
}

fn heading(ui: &mut Ui, text: &str, t: &Tokens) {
    ui.label(RichText::new(text).font(semibold(11.0)).color(t.text));
}

/// A field typed into in place that shows dots (a key).
fn secret_input(ui: &mut Ui, id: Id, text: &mut String, width: f32, hint: &str, shown: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, widgets::FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let r = widgets::field_editor(ui, rect, id, text, hint, !shown);
    if r.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
    }
    crate::a11y::text(&r, hint, if shown { text.as_str() } else { "" });
    r
}

/// Preferences ▸ AI tools.
pub fn section(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let connected = app.session.remove_bg().cloned();
    egui::Frame::NONE.stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Remove background")).font(semibold(13.0)).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, fill, color) = if connected.is_some() {
                    (tr("Connected"), Color32::from_rgb(0xDC, 0xFC, 0xE7), Color32::from_rgb(0x16, 0x65, 0x34))
                } else {
                    (tr("Not connected"), t.hover, t.text_secondary)
                };
                egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                    ui.label(RichText::new(text).font(semibold(10.0)).color(color));
                });
            });
        });
        ui.add_space(2.0);
        wrapped(
            ui,
            tr("Cuts the subject out of an image and makes the background transparent, at the image's full size. It runs on your own free Cloudflare account, so your images go from FigCraft to your account and straight back, through no one else's server."),
            t.text_secondary,
        );
        ui.add_space(10.0);
        if cfg!(target_arch = "wasm32") {
            wrapped(ui, tr("Remove background works in the FigCraft desktop app for now."), t.text);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        desktop(app, ui, &t, connected);
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn desktop(app: &mut FigApp, ui: &mut Ui, t: &Tokens, connected: Option<Connection>) {
    if let Some(phase) = app.ui.ai.phase {
        progress(ui, t, phase, app.ui.ai.mode == Mode::Manual || connected.is_some() && app.ui.ai.status.is_some());
        ui.add_space(8.0);
    }
    // In the steps, questions and errors show by the button that led to them.
    if app.ui.ai.mode == Mode::Summary {
        notices(app, ui, t);
    }
    match (app.ui.ai.mode, connected) {
        (Mode::Connect, _) => connect_steps(app, ui, t),
        (Mode::Manual, _) => manual(app, ui, t),
        (Mode::Summary, Some(c)) => connected_view(app, ui, t, &c),
        (Mode::Summary, None) => {
            explanation(app, ui, t);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, tr("Connect Cloudflare"), true, app.ui.ai.phase.is_none()).clicked() {
                    app.ui.ai.mode = Mode::Connect;
                    app.ui.ai.error = None;
                    // Read already: the steps get the room (it opens again with a click).
                    app.ui.ai.explain_closed = true;
                }
                if widgets::button(ui, tr("Set up by hand"), false, app.ui.ai.phase.is_none())
                    .on_hover_text(tr("For a remover you installed yourself, or the address and key from another computer"))
                    .clicked()
                {
                    app.ui.ai.mode = Mode::Manual;
                    app.ui.ai.error = None;
                }
            });
        }
    }
}

/// The question whether to replace an existing remover, and the last error.
#[cfg(not(target_arch = "wasm32"))]
fn notices(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    if let Some(account) = app.ui.ai.replace_account.clone() {
        replace_question(app, ui, t, &account);
        ui.add_space(8.0);
    }
    if let Some(e) = app.ui.ai.error.clone() {
        banner_text(ui, &e, Color32::from_rgb(0xFE, 0xE4, 0xE2), Color32::from_rgb(0xB4, 0x23, 0x18));
        ui.add_space(8.0);
    }
}

/// "What happens when you connect": what FigCraft will do in the user's account.
#[cfg(not(target_arch = "wasm32"))]
fn explanation(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let open = !app.ui.ai.explain_closed;
    if widgets::disclosure(ui, tr("What happens when you connect"), open) {
        app.ui.ai.explain_closed = open;
    }
    if !open {
        return;
    }
    egui::Frame::NONE.fill(t.hover).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for line in [
            tr("FigCraft adds one small program, named figcraft-bg, to your Cloudflare account. When you remove a background, FigCraft sends the image to it, it cuts out the subject with Cloudflare's background remover, and sends the result straight back."),
            tr("FigCraft makes a private access key for it, so only your FigCraft can use it. The key is kept in your computer's password store."),
            tr("To install it, you give FigCraft a Cloudflare key once. FigCraft uses it for the install and then forgets it. Nothing else in your account is changed."),
            tr("It's free for up to 5,000 images a month. If you use them all, it stops until the next month. Cloudflare doesn't charge you on the free plan."),
        ] {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").font(regular(11.0)).color(t.text_secondary));
                wrapped(ui, line, t.text);
            });
            ui.add_space(4.0);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn connect_steps(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    explanation(app, ui, t);
    ui.add_space(10.0);
    let busy = app.ui.ai.phase.is_some();
    let number = |ui: &mut Ui, n: &str| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(5), t.text);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n, semibold(10.0), t.panel);
    };
    ui.horizontal_top(|ui| {
        number(ui, "1");
        ui.vertical(|ui| {
            heading(ui, tr("Create a free Cloudflare account"), t);
            wrapped(ui, tr("Skip this if you already have one."), t.text_secondary);
            if widgets::button(ui, tr("Open Cloudflare sign-up"), false, true).clicked() {
                app.services.open_url(SIGN_UP);
            }
        });
    });
    ui.add_space(10.0);
    ui.horizontal_top(|ui| {
        number(ui, "2");
        ui.vertical(|ui| {
            heading(ui, tr("Make a Cloudflare key for FigCraft"), t);
            wrapped(
                ui,
                tr("On the API Tokens page, choose Create Token, then Use template next to “Edit Cloudflare Workers”. Choose Continue to summary, then Create Token, and copy the key it shows."),
                t.text_secondary,
            );
            if widgets::button(ui, tr("Open the API Tokens page"), false, true).clicked() {
                app.services.open_url(API_TOKENS);
            }
        });
    });
    ui.add_space(10.0);
    let mut go = false;
    ui.horizontal_top(|ui| {
        number(ui, "3");
        ui.vertical(|ui| {
            heading(ui, tr("Paste the key here"), t);
            let width = (ui.available_width() - 4.0).clamp(160.0, 360.0);
            let r = secret_input(ui, Id::unique("remove_bg_token"), &mut app.ui.ai.token, width, tr("Cloudflare key"), false);
            go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            wrapped(
                ui,
                tr("FigCraft uses this key once to install the remover, then forgets it. You can delete it in Cloudflare afterwards."),
                t.text_secondary,
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                go |= widgets::button(ui, tr("Connect"), true, !busy && !app.ui.ai.token.trim().is_empty()).clicked();
                if widgets::button(ui, tr("Cancel"), false, !busy).clicked() {
                    app.ui.ai.mode = Mode::Summary;
                    app.ui.ai.token.clear();
                    app.ui.ai.error = None;
                }
            });
        });
    });
    ui.add_space(8.0);
    notices(app, ui, t);
    if go && !busy && !app.ui.ai.token.trim().is_empty() {
        install(app, false);
    }
}

/// The account has a remover already: replace it?
#[cfg(not(target_arch = "wasm32"))]
fn replace_question(app: &mut FigApp, ui: &mut Ui, t: &Tokens, account: &str) {
    egui::Frame::NONE.fill(t.hover).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let text = trf(
            "{account} already has a background remover named figcraft-bg, perhaps from FigCraft on another computer. Replace it? FigCraft on other computers will need to connect again.",
            &[("account", &account)],
        );
        wrapped(ui, &text, t.text);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Replace it"), true, true).clicked() {
                app.ui.ai.replace_account = None;
                install(app, true);
            }
            if widgets::button(ui, tr("Cancel"), false, true).clicked() {
                app.ui.ai.replace_account = None;
                app.ui.ai.token.clear();
                app.ui.ai.mode = Mode::Summary;
            }
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn manual(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let busy = app.ui.ai.phase.is_some();
    wrapped(
        ui,
        tr(
            "Type the address and access key of a remover that's already installed. On another computer, Copy address and key gives both on one line: paste it into Address.",
        ),
        t.text_secondary,
    );
    ui.add_space(6.0);
    let width = (ui.available_width() - 4.0).clamp(160.0, 380.0);
    heading(ui, tr("Address"), t);
    let r =
        widgets::text_input(ui, Id::unique("remove_bg_address"), &mut app.ui.ai.manual_address, width, "https://figcraft-bg.your-name.workers.dev");
    crate::a11y::text(&r, tr("Address"), &app.ui.ai.manual_address);
    // The line Copy address and key made: the address, a space, the key.
    if let Some((address, key)) = app.ui.ai.manual_address.trim().split_once(char::is_whitespace).map(|(a, k)| (a.to_string(), k.trim().to_string()))
    {
        app.ui.ai.manual_address = address;
        app.ui.ai.manual_key = key;
    }
    ui.add_space(4.0);
    heading(ui, tr("Access key"), t);
    secret_input(ui, Id::unique("remove_bg_manual_key"), &mut app.ui.ai.manual_key, width, tr("Access key"), false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if widgets::button(ui, tr("Save and test"), true, !busy).clicked() {
            match Connection::new(&app.ui.ai.manual_address, &app.ui.ai.manual_key) {
                Ok(c) => {
                    save(app, c.clone(), String::new());
                    app.ui.ai.manual_key.clear();
                    app.ui.ai.mode = Mode::Summary;
                    test(app, c);
                }
                Err(e) => app.ui.ai.error = Some(format!("{e}.")),
            }
        }
        if widgets::button(ui, tr("Cancel"), false, !busy).clicked() {
            app.ui.ai.mode = Mode::Summary;
            app.ui.ai.manual_key.clear();
            app.ui.ai.error = None;
        }
    });
    ui.add_space(8.0);
    notices(app, ui, t);
}

#[cfg(not(target_arch = "wasm32"))]
fn connected_view(app: &mut FigApp, ui: &mut Ui, t: &Tokens, c: &Connection) {
    let account = app.session.prefs.remove_bg_account.clone();
    if !account.is_empty() {
        wrapped(ui, &trf("Installed in the Cloudflare account “{account}”.", &[("account", &account)]), t.text);
        ui.add_space(6.0);
    }
    heading(ui, tr("Remover address"), t);
    ui.add(egui::Label::new(RichText::new(&c.address).font(regular(11.0)).color(t.text)).selectable(true).wrap());
    ui.add_space(4.0);
    heading(ui, tr("Access key"), t);
    ui.horizontal(|ui| {
        let shown = if app.ui.ai.show_key { c.key.clone() } else { "•".repeat(c.key.chars().count().min(24)) };
        ui.add(egui::Label::new(RichText::new(shown).font(regular(11.0)).color(t.text)).selectable(app.ui.ai.show_key));
        if widgets::button(ui, if app.ui.ai.show_key { tr("Hide") } else { tr("Show") }, false, true).clicked() {
            app.ui.ai.show_key = !app.ui.ai.show_key;
        }
    });
    wrapped(ui, tr("Kept in your computer's password store, not in FigCraft's settings file."), t.text_secondary);
    ui.add_space(8.0);
    if let Some((ok, text)) = app.ui.ai.status.clone() {
        if ok {
            banner_text(ui, &text, Color32::from_rgb(0xDC, 0xFC, 0xE7), Color32::from_rgb(0x16, 0x65, 0x34));
        } else {
            banner_text(ui, &text, Color32::from_rgb(0xFE, 0xE4, 0xE2), Color32::from_rgb(0xB4, 0x23, 0x18));
        }
        ui.add_space(8.0);
    }
    let busy = app.ui.ai.phase.is_some();
    ui.horizontal_wrapped(|ui| {
        if widgets::button(ui, tr("Test"), false, !busy).clicked() {
            test(app, c.clone());
        }
        if widgets::button(ui, tr("Copy address and key"), false, true)
            .on_hover_text(tr("To set up FigCraft on another computer: Set up by hand, then paste"))
            .clicked()
        {
            app.services.clipboard_set_text(&format!("{} {}", c.address, c.key));
            app.toast(tr("Address and key copied"));
        }
        if widgets::button(ui, tr("Connect again"), false, !busy)
            .on_hover_text(tr("Install a fresh copy of the remover with a new access key"))
            .clicked()
        {
            app.ui.ai.mode = Mode::Connect;
            app.ui.ai.error = None;
            app.ui.ai.explain_closed = true;
        }
        if widgets::button(ui, tr("Disconnect"), false, !busy).clicked() {
            disconnect(app);
        }
    });
    ui.add_space(8.0);
    wrapped(
        ui,
        tr("To use it, right-click an image on the canvas and choose Remove background, or use Remove background in the image's Fill settings."),
        t.text_secondary,
    );
}

/// The steps of a connect (or a test alone), done ones ticked.
#[cfg(not(target_arch = "wasm32"))]
fn progress(ui: &mut Ui, t: &Tokens, phase: Phase, test_only: bool) {
    let steps: &[(Phase, &str)] = if test_only {
        &[(Phase::Testing, tr("Testing with a sample picture…"))]
    } else {
        &[
            (Phase::Checking, tr("Checking the key and finding your account")),
            (Phase::Installing, tr("Installing the background remover")),
            (Phase::Address, tr("Switching on its address")),
            (Phase::Testing, tr("Testing with a sample picture. The first time can take a minute or two while Cloudflare switches the address on.")),
        ]
    };
    let at = steps.iter().position(|(p, _)| *p == phase).unwrap_or(0);
    for (i, (_, text)) in steps.iter().enumerate() {
        ui.horizontal_top(|ui| {
            if i < at {
                ui.label(RichText::new("✓").font(semibold(11.0)).color(Color32::from_rgb(0x16, 0x65, 0x34)));
            } else if i == at {
                ui.add(egui::Spinner::new().size(11.0).color(t.text_secondary));
            } else {
                ui.label(RichText::new("·").font(semibold(11.0)).color(t.text_tertiary));
            }
            wrapped(ui, text, if i <= at { t.text } else { t.text_tertiary });
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn banner_text(ui: &mut Ui, text: &str, fill: Color32, color: Color32) {
    egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).font(regular(11.0)).color(color)).wrap());
    });
}

/// Keep `c` as the remover: its key in the app's secrets, its address in the preferences.
#[cfg(not(target_arch = "wasm32"))]
fn save(app: &mut FigApp, c: Connection, account: String) {
    let kept = app.services.set_secret(SECRET_NAME, Some(&c.key));
    app.session.prefs.remove_bg_address = c.address.clone();
    app.session.prefs.remove_bg_account = account;
    app.services.save_prefs(&app.session.prefs);
    app.session.set_remove_bg(Some(c));
    app.ui.ai.error = kept.err().map(|e| {
        trf(
            "Connected for now, but FigCraft couldn't save the access key on this computer ({error}). You'll need to connect again after FigCraft restarts.",
            &[("error", &e)],
        )
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn disconnect(app: &mut FigApp) {
    let _ = app.services.set_secret(SECRET_NAME, None);
    app.session.prefs.remove_bg_address.clear();
    app.session.prefs.remove_bg_account.clear();
    app.services.save_prefs(&app.session.prefs);
    app.session.set_remove_bg(None);
    app.ui.ai = AiState { explain_closed: app.ui.ai.explain_closed, ..AiState::default() };
    app.toast(tr("Disconnected. The remover is still in your Cloudflare account."));
}

/// Install the remover with the pasted key, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn install(app: &mut FigApp, replace: bool) {
    use figcraft_engine::remove_bg::cloudflare;
    let token = app.ui.ai.token.trim().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    app.ui.ai.error = None;
    app.ui.ai.status = None;
    app.ui.ai.phase = Some(Phase::Checking);
    let spawned = std::thread::Builder::new().name("figcraft-remove-bg-install".into()).spawn(move || {
        let installed = cloudflare::install(cloudflare::API, &token, replace, &mut |step| {
            let _ = tx.send(Msg::Phase(match step {
                cloudflare::Step::Checking => Phase::Checking,
                cloudflare::Step::Installing => Phase::Installing,
                cloudflare::Step::Address => Phase::Address,
            }));
        });
        let connection = installed.as_ref().ok().map(|i| i.connection.clone());
        let _ = tx.send(Msg::Installed(installed));
        if let Some(c) = connection {
            let _ = tx.send(Msg::Phase(Phase::Testing));
            let _ = tx.send(Msg::Tested(cloudflare::settle(&c, cloudflare::SETTLE)));
        }
    });
    match spawned {
        Ok(_) => app.ui.ai.rx = Some(rx),
        Err(e) => {
            app.ui.ai.phase = None;
            app.ui.ai.error = Some(format!("FigCraft couldn't start connecting ({e})."));
        }
    }
}

/// Test the remover `c` on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn test(app: &mut FigApp, c: Connection) {
    let (tx, rx) = std::sync::mpsc::channel();
    app.ui.ai.status = None;
    app.ui.ai.phase = Some(Phase::Testing);
    let spawned = std::thread::Builder::new().name("figcraft-remove-bg-test".into()).spawn(move || {
        let _ = tx.send(Msg::Tested(figcraft_engine::remove_bg::net::test(&c)));
    });
    match spawned {
        Ok(_) => app.ui.ai.rx = Some(rx),
        Err(e) => {
            app.ui.ai.phase = None;
            app.ui.ai.status = Some((false, format!("FigCraft couldn't start the test ({e}).")));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn take_messages(app: &mut FigApp) {
    use figcraft_engine::remove_bg::cloudflare::InstallError;
    loop {
        let Some(rx) = app.ui.ai.rx.as_ref() else { return };
        let msg = match rx.try_recv() {
            Ok(msg) => msg,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.ui.ai.rx = None;
                app.ui.ai.phase = None;
                return;
            }
        };
        match msg {
            Msg::Phase(p) => app.ui.ai.phase = Some(p),
            Msg::Installed(Ok(done)) => {
                app.ui.ai.token.clear();
                app.ui.ai.mode = Mode::Summary;
                app.ui.ai.show_key = false;
                save(app, done.connection, done.account);
            }
            Msg::Installed(Err(InstallError::Exists { account })) => {
                app.ui.ai.phase = None;
                app.ui.ai.rx = None;
                app.ui.ai.replace_account = Some(account);
            }
            Msg::Installed(Err(e)) => {
                app.ui.ai.phase = None;
                app.ui.ai.rx = None;
                app.ui.ai.error = Some(e.to_string());
            }
            Msg::Tested(result) => {
                app.ui.ai.phase = None;
                app.ui.ai.rx = None;
                app.ui.ai.status = Some(match result {
                    Ok(took) => (
                        true,
                        trf(
                            "Working. A sample picture came back without its background in {seconds} seconds.",
                            &[("seconds", &format!("{:.1}", took.as_secs_f64()))],
                        ),
                    ),
                    Err(e) => (false, e),
                });
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn connect_for_tests(app: &mut FigApp, c: Connection) {
    app.session.set_remove_bg(Some(c));
}
