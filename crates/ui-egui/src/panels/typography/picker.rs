use super::{Align2, CornerRadius, Icon, Id, Rect, Sense, Tokens, Ui, icons, pos2, regular, semibold, tr, vec2, widgets};

// ---- Font picker ----

const RECENT_KEY: &str = "figcraft_recent_fonts";

const ROW: f32 = 26.0;

/// The font list's tallest (it scrolls past that).
const LIST_HEIGHT: f32 = 380.0;

/// Return the persisted list of recently used font families.
fn recent_families(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|d| d.get_persisted::<Vec<String>>(Id::unique(RECENT_KEY))).unwrap_or_default()
}

/// Remember a font family at the front of the recent list, removing an earlier
/// case-insensitive match and keeping at most eight entries.
pub(super) fn remember_family(ctx: &egui::Context, family: &str) {
    ctx.data_mut(|d| {
        let v = d.get_persisted_mut_or_default::<Vec<String>>(Id::unique(RECENT_KEY));
        v.retain(|f| !f.eq_ignore_ascii_case(family));
        v.insert(0, family.to_string());
        v.truncate(8);
    });
}

enum PickRow {
    Header(String),
    /// An installed family.
    Family(String),
    /// A family the font store can download (Google Fonts, Noto) that isn't installed.
    Library(String),
}

impl PickRow {
    /// The family a pick of this row sets, if it's a family.
    fn family(&self) -> Option<&str> {
        match self {
            PickRow::Family(f) | PickRow::Library(f) => Some(f),
            PickRow::Header(_) => None,
        }
    }
}

/// The font store's families matching `ql` (lower case; empty: all) that `installed` (lower-case
/// names) doesn't have, under a header per source in the order the sources first come. Empty
/// while the store's list is still being read (the menu repaints until it's there) and in the
/// browser.
fn library_rows(ctx: &egui::Context, ql: &str, installed: &std::collections::HashSet<String>) -> Vec<PickRow> {
    let Some(library) = figcraft_engine::fonts::library() else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return Vec::new();
    };
    let mut groups: Vec<(&str, Vec<PickRow>)> = Vec::new();
    for (family, source) in library {
        let key = family.to_lowercase();
        if (!ql.is_empty() && !key.contains(ql)) || installed.contains(&key) {
            continue;
        }
        let row = PickRow::Library(family.clone());
        match groups.iter_mut().find(|(s, _)| *s == source.as_str()) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((source, vec![row])),
        }
    }
    groups.into_iter().flat_map(|(source, rows)| std::iter::once(PickRow::Header(source.to_string())).chain(rows)).collect()
}

/// What the family list did this frame.
#[derive(Default)]
pub(crate) struct FontPick {
    /// The family clicked, or the one Enter picked.
    pub picked: Option<String>,
    /// The family Up and Down are on (Enter picks it).
    pub highlighted: Option<String>,
    /// The family the pointer is over (the caller previews it on the canvas).
    pub hovered: Option<String>,
    /// `picked` is a font store family that isn't installed: the caller downloads it
    /// (`missing_fonts::fetch`), and its text is drawn in a fallback font until then.
    pub download: bool,
}

/// The most the font menu is tall (its search field and the "Apply variable" row above the list
/// included); the list scrolls past that.
const MENU_HEIGHT: f32 = LIST_HEIGHT + 40.0;

/// What the menu's header worked out for its list.
#[derive(Default)]
struct Header {
    rows: Vec<PickRow>,
    /// The row Up and Down are on.
    hi: Option<usize>,
    /// The rows Up and Down moved this frame (the list scrolls to the row they are on).
    step: i64,
    /// Enter picked this family (true: from the font store).
    entered: Option<(String, bool)>,
}

/// The searchable family menu under `anchor`: click a family, or step through it with Up and Down
/// and press Enter. Hovering a family previews it on the canvas (closing without picking restores
/// the original). `before_search` draws the rows above the search field and returns true when one
/// of them handled the menu (it closed it). None while the menu is closed.
///
/// With `library`, the font store's families that aren't installed (Google Fonts, Noto) follow
/// the installed ones under a header per source. They draw in the interface font (there is no
/// face to preview them in) and aren't previewed on the canvas; picking one sets
/// [`FontPick::download`].
///
/// The search field and those rows stay in place; only the list scrolls. Headers ("Recent",
/// "All fonts") are as tall as family rows, so the list is uniform and virtualised.
pub(crate) fn font_picker(
    ui: &Ui,
    anchor: &egui::Response,
    current: Option<&str>,
    width: f32,
    library: bool,
    before_search: impl FnOnce(&mut Ui) -> bool,
) -> Option<FontPick> {
    let t = Tokens::get(ui.ctx());
    let just_opened = anchor.clicked();
    let hi_id = Id::unique("font_highlight");
    let q_id = Id::unique("font_query");
    craft_ui::combo::Menu::new(ui, anchor)
        .height(MENU_HEIGHT)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .style(crate::right::menu_style)
        .show_rows_with_header(
            |ui| {
                // The menu is at least `width` wide, even under a narrower field.
                ui.set_min_width(width);
                if before_search(ui) {
                    return Header::default();
                }
                let mut q: String = ui.data(|d| d.get_temp(q_id)).unwrap_or_default();
                if just_opened {
                    q.clear();
                    ui.data_mut(|d| d.remove::<(String, usize)>(hi_id));
                }
                // Taken before the search field sees them (it would move its cursor).
                let step = ui.input_mut(|i| {
                    i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
                        - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64
                });
                let search =
                    ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Search fonts")).desired_width(ui.available_width()).font(regular(11.5)));
                crate::widgets::select_all_on_focus(ui, &search, &q);
                if just_opened {
                    search.request_focus();
                }
                ui.data_mut(|d| d.insert_temp(q_id, q.clone()));
                let ql = q.trim().to_lowercase();
                let all = figcraft_text::family_names();
                let matches: Vec<String> = all.iter().filter(|f| ql.is_empty() || f.to_lowercase().contains(&ql)).cloned().collect();
                let store = if library {
                    let installed: std::collections::HashSet<String> = all.iter().map(|f| f.to_lowercase()).collect();
                    library_rows(ui.ctx(), &ql, &installed)
                } else {
                    Vec::new()
                };
                let mut rows: Vec<PickRow> = Vec::new();
                if ql.is_empty() {
                    let recent: Vec<String> =
                        recent_families(ui.ctx()).into_iter().filter(|f| all.iter().any(|a| a.eq_ignore_ascii_case(f))).collect();
                    if !recent.is_empty() {
                        rows.push(PickRow::Header(tr("Recent").to_string()));
                        rows.extend(recent.into_iter().map(PickRow::Family));
                    }
                }
                if matches.is_empty() && store.is_empty() {
                    widgets::label(ui, tr("No fonts match"), t.text_tertiary);
                    return Header::default();
                }
                // The installed fonts get a header when another list comes before or after them.
                if !matches.is_empty() && (!rows.is_empty() || !store.is_empty()) {
                    let label = if store.is_empty() { tr("All fonts") } else { tr("On this computer") };
                    rows.push(PickRow::Header(label.to_string()));
                }
                rows.extend(matches.into_iter().map(PickRow::Family));
                rows.extend(store);
                let family = |i: usize| rows.get(i).and_then(PickRow::family).map(str::to_string);
                // A new search starts again; the first step starts from the current family.
                let mut hi = ui.data(|d| d.get_temp::<(String, usize)>(hi_id)).filter(|(hq, _)| *hq == q).map(|(_, i)| i);
                if step != 0 {
                    let next = |i: usize| if step > 0 { i.checked_add(1) } else { i.checked_sub(1) };
                    let mut from =
                        hi.or_else(|| rows.iter().rposition(|r| r.family().is_some_and(|f| current.is_some_and(|c| c.eq_ignore_ascii_case(f)))));
                    for _ in 0..step.unsigned_abs() {
                        let mut at = match from {
                            Some(i) => next(i),
                            None if step > 0 => Some(0),
                            None => rows.len().checked_sub(1),
                        };
                        // Past the headers; at either end it stays where it was.
                        while let Some(i) = at.filter(|i| *i < rows.len() && family(*i).is_none()) {
                            at = next(i);
                        }
                        if let Some(i) = at.filter(|i| family(*i).is_some()) {
                            from = Some(i);
                            hi = Some(i);
                        }
                    }
                }
                match hi {
                    Some(i) => ui.data_mut(|d| {
                        d.insert_temp(hi_id, (q.clone(), i));
                    }),
                    None => ui.data_mut(|d| d.remove::<(String, usize)>(hi_id)),
                }
                let mut entered = None;
                if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let at = hi.filter(|i| family(*i).is_some()).or_else(|| rows.iter().position(|r| r.family().is_some()));
                    entered = at.and_then(|i| rows.get(i)).and_then(|r| Some((r.family()?.to_string(), matches!(r, PickRow::Library(_)))));
                    ui.close();
                }
                Header { rows, hi, step, entered }
            },
            ROW,
            |h| h.rows.len(),
            |ui, range, h| {
                if let Some((f, download)) = h.entered {
                    return FontPick { picked: Some(f), highlighted: None, hovered: None, download };
                }
                if let (Some(i), true) = (h.hi, h.step != 0) {
                    // Scrolled just enough to show the row Up or Down moved to; it may be out of
                    // the range laid out, so its rect comes from the list's top.
                    let stride = ROW + ui.spacing().item_spacing.y;
                    let top = ui.max_rect().top() - range.start as f32 * stride + i as f32 * stride;
                    ui.scroll_to_rect(Rect::from_min_size(pos2(ui.max_rect().left(), top), vec2(ui.max_rect().width(), ROW)), None);
                }
                let mut budget = 4;
                let mut picked = None;
                let mut download = false;
                let mut hovered_family = None;
                let hi = h.hi;
                for (i, r) in h.rows.iter().enumerate().skip(range.start).take(range.len()) {
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW), if r.family().is_some() { Sense::click() } else { Sense::hover() });
                    match r {
                        PickRow::Header(label) => {
                            ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, semibold(10.5), t.text_secondary);
                        }
                        PickRow::Library(f) => {
                            // Not installed: no face to preview it in, so its name in the
                            // interface font, and Download on the row the pointer or keys are on.
                            let hovered = resp.hovered() || hi == Some(i);
                            if hovered {
                                ui.painter().rect_filled(rect, CornerRadius::same(5), t.accent);
                            }
                            let col = if hovered { t.accent_on } else { t.text };
                            if current.is_some_and(|c| c.eq_ignore_ascii_case(f)) {
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(pos2(rect.min.x + 10.0, rect.center().y), vec2(12.0, 12.0)),
                                    Icon::Check,
                                    col,
                                );
                            }
                            ui.painter().text(pos2(rect.min.x + 22.0, rect.center().y), Align2::LEFT_CENTER, f, regular(12.0), col);
                            if hovered {
                                ui.painter().text(pos2(rect.max.x - 8.0, rect.center().y), Align2::RIGHT_CENTER, tr("Download"), regular(10.5), col);
                            }
                            if resp.clicked() {
                                picked = Some(f.clone());
                                download = true;
                                ui.close();
                            }
                        }
                        PickRow::Family(f) => {
                            let hovered = resp.hovered() || hi == Some(i);
                            if hovered {
                                ui.painter().rect_filled(rect, CornerRadius::same(5), t.accent);
                            }
                            let col = if hovered { t.accent_on } else { t.text };
                            if current.is_some_and(|c| c.eq_ignore_ascii_case(f)) {
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(pos2(rect.min.x + 10.0, rect.center().y), vec2(12.0, 12.0)),
                                    Icon::Check,
                                    col,
                                );
                            }
                            let text_min = pos2(rect.min.x + 22.0, rect.min.y);
                            match crate::font_preview::family(ui.ctx(), f, 13.0, &mut budget) {
                                Some(tex) => {
                                    crate::font_preview::paint(
                                        ui.painter(),
                                        &tex,
                                        pos2(text_min.x, rect.center().y),
                                        rect.max.x - text_min.x - 6.0,
                                        ROW,
                                        col,
                                    );
                                }
                                None => {
                                    ui.painter().text(pos2(text_min.x, rect.center().y), Align2::LEFT_CENTER, f, regular(12.0), col);
                                }
                            }
                            if resp.clicked() {
                                picked = Some(f.clone());
                                ui.close();
                            }
                            if resp.hovered() {
                                hovered_family = Some(f.clone());
                            }
                        }
                    }
                }
                let highlighted = hi.and_then(|i| match h.rows.get(i) {
                    Some(PickRow::Family(f)) => Some(f.clone()),
                    _ => None,
                });
                FontPick { highlighted, picked, hovered: hovered_family, download }
            },
        )
}
