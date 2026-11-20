//! Place image (Shift+Cmd+K): the images picked load onto the cursor, which carries the next one's
//! thumbnail and a badge with how many are left. A click places the next image: on a shape it
//! becomes that shape's fill, elsewhere a layer at the image's own size with its top-left corner at
//! the click, in the frame under the pointer. A drag places it at the dragged width. Place all puts
//! the rest in a row in the middle of the view as one undo step; Escape, Delete or Cancel discards
//! them. The engine decides where each one goes (`image.place` with `click`).
//!
//! Behaviour from Figma's help page "Add images and videos to designs" (the cursor's count badge,
//! a click on empty canvas at the original size, a click on a layer to fill it, Place all, Delete
//! to discard); the thumbnail on the cursor and the drag to size are FigCraft's.

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::{Color32, CornerRadius, CursorIcon, Pos2, Rect, RichText, Stroke, StrokeKind, vec2};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, trn};
use crate::services::Picked;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The picture files Place image takes.
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"];

/// The most images one Place image loads.
const MAX_LOADED: usize = 200;

/// The thumbnail's longer side on the cursor, in points.
const THUMB: f32 = 64.0;

/// Where the thumbnail sits from the pointer, in points: below right, clear of the crosshair.
const THUMB_OFFSET: egui::Vec2 = vec2(14.0, 14.0);

/// A press that moves less than this (points) before it is let go is a click, not a drag.
const DRAG_SLOP: f32 = 4.0;

/// The most of a picked file read for its thumbnail (the engine reads the whole file to place it).
const MAX_THUMB_READ: u64 = 64 * 1024 * 1024;

/// The gap between images that Place all lines up, in page units.
const PLACE_ALL_GAP: f64 = 20.0;

/// Where a loaded image comes from.
enum Source {
    Path(PathBuf),
    /// The file's contents, where the platform hands them over (the browser).
    Bytes(Vec<u8>),
}

/// One image on the cursor.
struct Item {
    /// The layer's name: the file's name without its extension.
    name: String,
    source: Source,
    /// The thumbnail, made when first shown (`Some(None)`: it couldn't be decoded) and its size
    /// in points.
    thumb: Option<Option<(egui::TextureHandle, egui::Vec2)>>,
}

impl Item {
    fn new(name: &str, source: Source) -> Self {
        let name = std::path::Path::new(name).file_stem().map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned());
        Self { name, source, thumb: None }
    }

    /// `image.place`'s parameters for this image, with `extra` (where it goes) merged in.
    fn params(&self, extra: Value) -> Value {
        let mut p = match &self.source {
            Source::Path(path) => json!({"path": path.to_string_lossy()}),
            Source::Bytes(bytes) => {
                use base64::Engine as _;
                json!({"data": base64::engine::general_purpose::STANDARD.encode(bytes)})
            }
        };
        if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
            p.insert("name".into(), Value::String(self.name.clone()));
            p.extend(extra);
        }
        p
    }

    /// The thumbnail texture and its size on the cursor, decoded the first time it's asked for.
    fn thumb(&mut self, ctx: &egui::Context) -> Option<(egui::TextureHandle, egui::Vec2)> {
        if self.thumb.is_none() {
            let decoded = match &self.source {
                Source::Bytes(bytes) => decode(bytes),
                Source::Path(path) => figcraft_engine::file_access::read_capped(path, MAX_THUMB_READ).ok().and_then(|b| decode(&b)),
            };
            self.thumb = Some(decoded.map(|pic| {
                let (w, h) = (pic.width as f32, pic.height as f32);
                let size = vec2(w, h) * (THUMB / w.max(h).max(1.0));
                let image = egui::ColorImage::from_rgba_unmultiplied([pic.width as usize, pic.height as usize], &pic.rgba);
                (ctx.load_texture("place-image-thumb", image, egui::TextureOptions::LINEAR), size)
            }));
        }
        self.thumb.clone().flatten()
    }
}

/// A picture decoded small enough for the cursor (twice its size, for sharp high-density screens).
fn decode(bytes: &[u8]) -> Option<figcraft_format::thumbnail::Thumbnail> {
    figcraft_format::thumbnail::decode_thumbnail(bytes, (THUMB * 2.0) as u32)
}

/// The images loaded on the cursor, for one document.
pub struct Loaded {
    /// The document they go into: they're dropped when another file comes to the front.
    pub uid: u64,
    items: VecDeque<Item>,
    /// Where the primary button went down on the canvas (screen), while it's held.
    press: Option<Pos2>,
}

impl Loaded {
    /// How many images are still to place.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Load the images a Place image dialog picked onto the cursor (replacing any still loaded). A file
/// that couldn't be read is said so; the others still load.
pub(crate) fn load(app: &mut FigApp, picked: Picked) {
    let Ok(uid) = app.session.ds().map(|d| d.uid) else { return };
    let all = match picked {
        Picked::Many(all) => all,
        one => vec![one],
    };
    let mut items = VecDeque::new();
    let mut unreadable = None;
    for p in all.into_iter().take(MAX_LOADED) {
        match p {
            Picked::Path(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                items.push_back(Item::new(&name, Source::Path(path)));
            }
            Picked::Contents(name, bytes) => items.push_back(Item::new(&name, Source::Bytes(bytes))),
            Picked::Unreadable(why) => {
                unreadable.get_or_insert(why);
            }
            Picked::Many(_) => {}
        }
    }
    if let Some(why) = unreadable {
        app.show_error(why);
    }
    app.ui.place = (!items.is_empty()).then_some(Loaded { uid, items, press: None });
}

/// The images loaded for the file in front, if any. Images loaded for another file are dropped, and
/// so are they in Dev Mode, which places nothing.
fn current(app: &mut FigApp) -> Option<&mut Loaded> {
    let uid = app.session.ds().ok().map(|d| d.uid);
    let dev = app.ui.right_tab == crate::RightTab::Inspect;
    if app.ui.place.as_ref().is_some_and(|l| dev || Some(l.uid) != uid || l.is_empty()) {
        app.ui.place = None;
    }
    app.ui.place.as_mut()
}

/// Whether images are loaded on the cursor for the file in front.
pub fn active(app: &mut FigApp) -> bool {
    current(app).is_some()
}

/// Escape, Delete or Cancel: the images still loaded are discarded. True when there were some.
pub fn discard(app: &mut FigApp) -> bool {
    let had = active(app);
    app.ui.place = None;
    had
}

/// The pointer on the canvas while images are loaded: a press and release places the next one
/// (a click, or a drag that sizes it). True when it took the pointer (the canvas tool doesn't get
/// it and its cursor doesn't show). Panning and zooming are handled before this.
pub fn input(app: &mut FigApp, ctx: &egui::Context, pos: Pos2, hovered: bool) -> bool {
    let canvas = app.canvas.rect;
    let Some(loaded) = current(app) else { return false };
    let (pressed, released, down, origin, at) = ctx.input(|i| {
        let p = &i.pointer;
        (
            p.button_pressed(egui::PointerButton::Primary),
            p.button_released(egui::PointerButton::Primary),
            p.primary_down(),
            p.press_origin(),
            p.interact_pos(),
        )
    });
    if pressed && hovered {
        loaded.press = origin.or(Some(pos)).filter(|p| canvas.contains(*p));
    }
    let start = loaded.press;
    if released {
        loaded.press = None;
        if let Some(start) = start {
            place_next(app, start, at.unwrap_or(pos));
        }
    } else if !down {
        // A press let go somewhere this didn't see (off the window) places nothing.
        loaded.press = None;
    }
    if hovered {
        ctx.set_cursor_icon(CursorIcon::Crosshair);
    }
    true
}

/// Place the next image for a press at `start` let go at `end` (screen): a click there, or a
/// drag from one to the other that sets its width.
fn place_next(app: &mut FigApp, start: Pos2, end: Pos2) {
    let (a, b) = (app.to_page(start), app.to_page(end));
    let extra = if start.distance(end) >= DRAG_SLOP {
        json!({"click": true, "x": a.x.min(b.x), "y": a.y.min(b.y), "width": (b.x - a.x).abs().max(1.0)})
    } else {
        json!({"click": true, "x": a.x, "y": a.y})
    };
    let Some(item) = app.ui.place.as_mut().and_then(|l| l.items.pop_front()) else { return };
    // A file that can't be placed is said so (`run` shows why) and the next one comes up.
    app.run("image.place", item.params(extra));
    if app.ui.place.as_ref().is_some_and(Loaded::is_empty) {
        app.ui.place = None;
    }
}

/// Place all: the images still loaded go in a row in the middle of the view, top-aligned
/// [`PLACE_ALL_GAP`] apart and selected, as one undo step.
pub fn place_all(app: &mut FigApp) {
    let Some(loaded) = app.ui.place.take() else { return };
    let centre = app.to_page(app.canvas.rect.center());
    let result = app.session.grouped(tr("Place all"), |s| {
        let (mut x, mut placed, mut first_error) = (0.0, Vec::new(), None);
        for item in &loaded.items {
            match s.execute("image.place", &item.params(json!({"x": centre.x + x, "y": centre.y}))) {
                Ok(v) => {
                    x += v["width"].as_f64().unwrap_or(0.0) + PLACE_ALL_GAP;
                    placed.extend(v["id"].as_u64());
                }
                Err(e) => {
                    first_error.get_or_insert(e.to_string());
                }
            }
        }
        // The row is centred on the middle of the view.
        let shift = (x - PLACE_ALL_GAP).max(0.0) / 2.0;
        for id in &placed {
            // Placed on the page, so page and parent coordinates are the same.
            let left = s.doc()?.world_bounds(figcraft_doc::NodeId(*id)).map(|b| b.x0);
            if let Some(left) = left {
                s.execute("node.set", &json!({"id": id, "x": left - shift, "y": centre.y}))?;
            }
        }
        if !placed.is_empty() {
            s.execute("select.set", &json!({"ids": placed}))?;
        }
        Ok(first_error)
    });
    match result {
        Ok(Some(why)) => app.show_error(why),
        Ok(None) => {}
        Err(e) => app.show_error(e.to_string()),
    }
}

/// The thumbnail and count that follow the pointer over the canvas, and the outline of a drag.
pub fn draw(app: &mut FigApp, painter: &egui::Painter) {
    let ctx = painter.ctx().clone();
    let canvas = app.canvas.rect;
    let Some(loaded) = current(app) else { return };
    let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()).filter(|p| canvas.contains(*p)) else { return };
    let t = Tokens::get(&ctx);
    let count = loaded.len();
    let press = loaded.press;
    let thumb = loaded.items.front_mut().and_then(|i| i.thumb(&ctx));
    // A drag: the box the image will fill, its height following the picture's proportion.
    if let Some(start) = press.filter(|s| s.distance(pointer) >= DRAG_SLOP) {
        let w = (pointer.x - start.x).abs();
        let h = thumb.as_ref().map_or((pointer.y - start.y).abs(), |(_, size)| w * size.y / size.x.max(1.0));
        let r = Rect::from_min_size(Pos2::new(start.x.min(pointer.x), start.y.min(pointer.y)), vec2(w, h));
        painter.rect_stroke(r, 0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    // The next image, white-edged and shadowed so it reads on any canvas.
    let size = thumb.as_ref().map_or(vec2(THUMB, THUMB * 0.75), |(_, s)| *s);
    let r = Rect::from_min_size(pointer + THUMB_OFFSET, size);
    painter.rect_filled(r.translate(vec2(0.0, 2.0)).expand(2.0), 4, Color32::from_black_alpha(60));
    painter.rect_filled(r.expand(2.0), 4, Color32::WHITE);
    match &thumb {
        Some((tex, _)) => {
            painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(r, 2, t.hover);
        }
    }
    // How many are left, on the thumbnail's top-left corner.
    let label = count.to_string();
    let galley = painter.layout_no_wrap(label, semibold(11.0), t.accent_on);
    let d = (galley.size().x + 10.0).max(20.0);
    let badge = Rect::from_center_size(r.left_top() + vec2(2.0, 2.0), vec2(d, 20.0));
    painter.rect(badge, CornerRadius::same(10), t.accent, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
    painter.galley(badge.center() - galley.size() / 2.0, galley, t.accent_on);
}

/// The bar at the top of the canvas while images are loaded: how many are left, how to place
/// them, Place all and Cancel.
pub fn banner(app: &mut FigApp, ctx: &egui::Context, canvas: Rect) {
    let Some(count) = current(app).map(|l| l.len()) else { return };
    let t = Tokens::get(ctx);
    let (all, cancel) = widgets::canvas_banner(ctx, egui::Id::unique("place_images_banner"), canvas, |ui| {
        ui.label(RichText::new(trn("{n} images to place", count, &[])).font(semibold(11.0)).color(t.text));
        ui.label(RichText::new(tr("Click to place, drag to size, click a shape to fill it")).font(regular(11.0)).color(t.text_secondary));
        let all = widgets::button(ui, tr("Place all"), false, true).clicked();
        let cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        (all, cancel)
    });
    if all {
        place_all(app);
    } else if cancel {
        app.ui.place = None;
    }
}
