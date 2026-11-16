//! Software-Runner für den Browser ohne WebGL (Hardwarebeschleunigung aus, kein SwiftShader-Ersatz mehr).
//!
//! Gleiche App wie mit eframe (`App::frame_ui`), aber ohne eframe-Painter: Eingaben aus DOM-Ereignissen
//! (Zeiger, Touch mit Druck, Rad/Pinch, Tastatur, Texteingabe/IME über verstecktes `<input>`, Kopieren/Einfügen,
//! Ablegen von Dateien, Größe und `devicePixelRatio`, Fokus), egui tesselieren, mit [`crate::softraster`] auf der
//! CPU rastern und nur die geänderten Rechtecke per `putImageData` auf ein 2D-Canvas kopieren.
//! Neu gezeichnet wird nur bei Bedarf (Eingaben, `request_repaint`/`request_repaint_after` von egui).
//!
//! Ereignis-, Eingabe- und Text-Agent-Code ist aus eframe 0.37 übernommen und angepasst (`events.rs`,
//! `input.rs`, `text_agent.rs`, `glyphs.rs`, `dropped_file.rs`; MIT oder Apache-2.0).

mod dropped_file;
mod events;
mod glyphs;
mod input;
mod text_agent;

use crate::softraster::SoftPainter;
use egui::mutex::Mutex;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use text_agent::TextAgent;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, Node};

use input::{button_from_mouse_event, modifiers_from_kb_event, modifiers_from_mouse_event, modifiers_from_wheel_event, pos_from_mouse_event, primary_touch_pos, push_touches, text_from_keyboard_event, translate_key};

/// Größenänderungen protokollieren?
const DEBUG_RESIZE: bool = false;

/// Schlüssel in `localStorage`: Hinweis „Ohne Grafikbeschleunigung“ weggeklickt.
const HINT_KEY: &str = "owndraw.softwareHint";

static PANICKED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------- Hilfen (wie eframe::web)

pub(crate) fn string_from_js_value(value: &JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:#?}"))
}

pub(crate) fn focused_element(root: &Node) -> Option<web_sys::Element> {
    if let Some(document) = root.dyn_ref::<Document>() {
        document.active_element()
    } else if let Some(shadow) = root.dyn_ref::<web_sys::ShadowRoot>() {
        shadow.active_element()
    } else {
        None
    }
}

pub(crate) fn has_focus<T: JsCast>(element: &T) -> bool {
    fn try_has_focus<T: JsCast>(element: &T) -> Option<bool> {
        let element = element.dyn_ref::<web_sys::Element>()?;
        let root = element.get_root_node();
        let focused_element = focused_element(&root)?;
        Some(element == &focused_element)
    }
    try_has_focus(element).unwrap_or(false)
}

pub(crate) fn focus_without_scroll(element: &web_sys::HtmlElement) -> Result<(), JsValue> {
    let options = web_sys::FocusOptions::new();
    options.set_prevent_scroll(true);
    element.focus_with_options(&options)
}

/// Zeit in Sekunden (monoton).
pub fn now_sec() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now() / 1000.0).unwrap_or(0.0)
}

/// `window.devicePixelRatio` (inkl. Browser-Zoom).
pub fn native_pixels_per_point() -> f32 {
    let ppp = web_sys::window().map(|w| w.device_pixel_ratio() as f32).unwrap_or(1.0);
    if ppp > 0.0 && ppp.is_finite() {
        ppp
    } else {
        1.0
    }
}

pub fn system_theme() -> Option<egui::Theme> {
    let window = web_sys::window()?;
    if does_prefer_color_scheme(&window, egui::Theme::Dark) == Some(true) {
        Some(egui::Theme::Dark)
    } else if does_prefer_color_scheme(&window, egui::Theme::Light) == Some(true) {
        Some(egui::Theme::Light)
    } else {
        None
    }
}

fn does_prefer_color_scheme(window: &web_sys::Window, theme: egui::Theme) -> Option<bool> {
    Some(prefers_color_scheme(window, theme).ok()??.matches())
}

fn prefers_color_scheme(window: &web_sys::Window, theme: egui::Theme) -> Result<Option<web_sys::MediaQueryList>, JsValue> {
    let theme = match theme {
        egui::Theme::Dark => "dark",
        egui::Theme::Light => "light",
    };
    window.match_media(&format!("(prefers-color-scheme: {theme})"))
}

/// Canvas in Client-Koordinaten (ohne Innenabstand).
fn canvas_content_rect(canvas: &HtmlCanvasElement) -> egui::Rect {
    let b = canvas.get_bounding_client_rect();
    let mut rect = egui::Rect::from_min_max(egui::pos2(b.left() as f32, b.top() as f32), egui::pos2(b.right() as f32, b.bottom() as f32));
    if let Some(Ok(Some(style))) = web_sys::window().map(|w| w.get_computed_style(canvas)) {
        let get = |name: &str| -> Option<f32> { style.get_property_value(name).ok()?.trim_end_matches("px").parse::<f32>().ok() };
        rect.min.x += get("padding-left").unwrap_or_default();
        rect.min.y += get("padding-top").unwrap_or_default();
        rect.max.x -= get("padding-right").unwrap_or_default();
        rect.max.y -= get("padding-bottom").unwrap_or_default();
    }
    rect
}

fn canvas_size_in_points(canvas: &HtmlCanvasElement, ctx: &egui::Context) -> egui::Vec2 {
    let ppp = ctx.zoom_factor() * native_pixels_per_point();
    egui::vec2(canvas.width() as f32 / ppp, canvas.height() as f32 / ppp)
}

fn set_cursor_icon(canvas: &HtmlCanvasElement, cursor: egui::CursorIcon) {
    use egui::CursorIcon as C;
    let name = match cursor {
        C::Alias => "alias",
        C::AllScroll => "all-scroll",
        C::Cell => "cell",
        C::ContextMenu => "context-menu",
        C::Copy => "copy",
        C::Crosshair => "crosshair",
        C::Default => "default",
        C::Grab => "grab",
        C::Grabbing => "grabbing",
        C::Help => "help",
        C::Move => "move",
        C::NoDrop => "no-drop",
        C::None => "none",
        C::NotAllowed => "not-allowed",
        C::PointingHand => "pointer",
        C::Progress => "progress",
        C::ResizeHorizontal => "ew-resize",
        C::ResizeNeSw => "nesw-resize",
        C::ResizeNwSe => "nwse-resize",
        C::ResizeVertical => "ns-resize",
        C::ResizeEast => "e-resize",
        C::ResizeSouthEast => "se-resize",
        C::ResizeSouth => "s-resize",
        C::ResizeSouthWest => "sw-resize",
        C::ResizeWest => "w-resize",
        C::ResizeNorthWest => "nw-resize",
        C::ResizeNorth => "n-resize",
        C::ResizeNorthEast => "ne-resize",
        C::ResizeColumn => "col-resize",
        C::ResizeRow => "row-resize",
        C::Text => "text",
        C::VerticalText => "vertical-text",
        C::Wait => "wait",
        C::ZoomIn => "zoom-in",
        C::ZoomOut => "zoom-out",
    };
    let _ = canvas.style().set_property("cursor", name);
}

fn secure_window() -> Option<web_sys::Window> {
    let w = web_sys::window()?;
    if !w.is_secure_context() {
        log::warn!("Zwischenablage nur in sicherem Kontext (HTTPS/localhost) verfügbar");
        return None;
    }
    Some(w)
}

fn set_clipboard_text(s: &str) {
    let Some(w) = secure_window() else { return };
    let p = w.navigator().clipboard().write_text(s);
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(p).await {
            log::warn!("Kopieren fehlgeschlagen: {}", string_from_js_value(&e));
        }
    });
}

fn set_clipboard_image(image: &egui::ColorImage) {
    let Some(w) = secure_window() else { return };
    let raw: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let Some(img) = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, raw) else { return };
    let mut png = Vec::new();
    if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
        return;
    }
    let item = (|| -> Result<web_sys::ClipboardItem, JsValue> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(&png[..]));
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type("image/png");
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)?;
        let items = js_sys::Object::new();
        js_sys::Reflect::set(&items, &JsValue::from_str("image/png"), &blob)?;
        web_sys::ClipboardItem::new_with_record_from_str_to_blob_promise(&items)
    })();
    let Ok(item) = item else { return };
    let p = w.navigator().clipboard().write(&js_sys::Array::of1(&item));
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(p).await {
            log::warn!("Bild kopieren fehlgeschlagen: {}", string_from_js_value(&e));
        }
    });
}

fn open_url(url: &str, new_tab: bool) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target(url, if new_tab { "_blank" } else { "_self" });
    }
}

// ---------------------------------------------------------------- Eingaben, Neuzeichnen

/// Zwischen zwei Frames gesammelte Eingaben.
#[derive(Default)]
pub(crate) struct WebInput {
    pub primary_touch: Option<egui::TouchId>,
    pub accumulated_scale: f32,
    pub accumulated_rotation: f32,
    pub modifiers: egui::Modifiers,
    pub raw: egui::RawInput,
}

impl WebInput {
    pub fn new_frame(&mut self, canvas_size: egui::Vec2) -> egui::RawInput {
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Default::default(), canvas_size)), time: Some(now_sec()), ..self.raw.take() };
        let vp = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        vp.native_pixels_per_point = Some(native_pixels_per_point());
        vp.occluded = Some(web_sys::window().and_then(|w| w.document()).is_some_and(|d| d.hidden()));
        raw
    }
    pub fn set_focus(&mut self, focused: bool) {
        if self.raw.focused == focused {
            return;
        }
        self.modifiers = egui::Modifiers::default();
        self.raw.focused = focused;
        self.raw.events.push(egui::Event::WindowFocused(focused));
        self.primary_touch = None;
    }
    pub fn set_modifiers(&mut self, modifiers: egui::Modifiers) {
        if self.modifiers != modifiers {
            self.modifiers = modifiers;
            self.raw.events.push(egui::Event::ModifiersChanged(modifiers));
        }
    }
}

/// Zeitpunkt des nächsten Neuzeichnens.
pub(crate) struct NeedRepaint(Mutex<f64>);

impl NeedRepaint {
    pub fn clear(&self) {
        *self.0.lock() = f64::INFINITY;
    }
    pub fn repaint_after(&self, secs: f64) {
        let mut t = self.0.lock();
        *t = t.min(now_sec() + secs);
    }
    pub fn repaint(&self) {
        let mut t = self.0.lock();
        *t = t.min(now_sec());
    }
    pub fn repaint_asap(&self) {
        *self.0.lock() = f64::NEG_INFINITY;
    }
    pub fn needs_repaint(&self) -> bool {
        *self.0.lock() <= now_sec()
    }
}

/// Wie `eframe::WebOptions` (immer Weitergabe stoppen und Standardaktion verhindern).
pub(crate) struct Opts {
    pub should_stop_propagation: fn(&egui::Event) -> bool,
    pub should_prevent_default: fn(&egui::Event) -> bool,
}

/// Messwerte für Tests (`window.owndrawSoftStats`).
#[derive(Default)]
struct Stats {
    frames: u32,
    logic_s: f64,
    paint_s: f64,
    pixels: f64,
    obj: Option<js_sys::Object>,
}

// ---------------------------------------------------------------- Runner

pub(crate) struct Runner {
    pub web_options: Opts,
    canvas: HtmlCanvasElement,
    ctx2d: CanvasRenderingContext2d,
    egui_ctx: egui::Context,
    app: crate::app::App,
    pub input: WebInput,
    pub needs_repaint: Arc<NeedRepaint>,
    pub text_agent: TextAgent,
    painter: SoftPainter,
    textures_delta: egui::TexturesDelta,
    clipped_primitives: Option<Vec<egui::ClippedPrimitive>>,
    stats: Stats,
}

impl Runner {
    pub fn egui_ctx(&self) -> &egui::Context {
        &self.egui_ctx
    }
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
    /// Canvas wurde geleert (Größe gesetzt): beim nächsten Malen alles neu.
    pub fn invalidate(&mut self) {
        self.painter.invalidate();
    }
    pub fn has_outstanding_paint_data(&self) -> bool {
        self.clipped_primitives.is_some()
    }
    pub fn auto_save_if_needed(&mut self) {}
    pub fn save(&mut self) {}

    pub fn has_focus(&self) -> bool {
        if web_sys::window().and_then(|w| w.document()).is_none_or(|d| d.hidden()) {
            return false;
        }
        has_focus(&self.canvas) || self.text_agent.has_focus()
    }

    pub fn update_focus(&mut self) {
        let f = self.has_focus();
        if self.input.raw.focused != f {
            self.input.set_focus(f);
            self.egui_ctx.request_repaint();
        }
    }

    /// App-Logik eines Frames (ohne zu malen).
    pub fn logic(&mut self) {
        let t0 = now_sec();
        self.update_focus();
        let size = canvas_size_in_points(&self.canvas, &self.egui_ctx);
        let raw = self.input.new_frame(size);
        let app = &mut self.app;
        let out = self.egui_ctx.run_ui(raw, |ui| app.frame_ui(ui));
        // Fenster-Befehle (Titel, Schließen, …) gibt es im Browser nicht; den Titel setzt die App selbst.
        self.handle_platform_output(out.platform_output);
        self.textures_delta.append(out.textures_delta);
        self.clipped_primitives = Some(self.egui_ctx.tessellate(out.shapes, out.pixels_per_point));
        self.stats.logic_s += now_sec() - t0;
    }

    /// Malt das Ergebnis des letzten [`Self::logic`].
    pub fn paint(&mut self) {
        let Some(prims) = self.clipped_primitives.take() else { return };
        let t0 = now_sec();
        let (w, h) = (self.canvas.width() as usize, self.canvas.height() as usize);
        if w == 0 || h == 0 {
            return;
        }
        let ppp = self.egui_ctx.pixels_per_point();
        let clear = self.egui_ctx.global_style().visuals.panel_fill;
        let (damage, info) = self.painter.paint(w, h, clear, &prims, ppp, &mut self.textures_delta);
        let px = self.painter.pixels();
        let mut tmp: Vec<u8> = Vec::new();
        for r in &damage {
            let (rw, rh) = (r.width() as usize, r.height() as usize);
            tmp.clear();
            tmp.reserve(rw * rh * 4);
            for y in r.y0 as usize..r.y1 as usize {
                let o = y * w + r.x0 as usize;
                tmp.extend_from_slice(px[o..o + rw].as_flattened());
            }
            match web_sys::ImageData::new_with_u8_clamped_array_and_sh(wasm_bindgen::Clamped(&tmp), rw as u32, rh as u32) {
                Ok(img) => {
                    let _ = self.ctx2d.put_image_data(&img, r.x0 as f64, r.y0 as f64);
                }
                Err(e) => log::error!("putImageData: {}", string_from_js_value(&e)),
            }
        }
        self.stats.frames += 1;
        self.stats.paint_s += now_sec() - t0;
        self.stats.pixels += info.pixels as f64;
        self.publish_stats(info.primitives);
    }

    pub fn report_frame_time(&mut self, _secs: f64) {}

    fn publish_stats(&mut self, prims: usize) {
        let Some(o) = &self.stats.obj else { return };
        let set = |k: &str, v: f64| {
            let _ = js_sys::Reflect::set(o, &JsValue::from_str(k), &JsValue::from_f64(v));
        };
        set("frames", self.stats.frames as f64);
        set("logicMs", self.stats.logic_s * 1000.0);
        set("paintMs", self.stats.paint_s * 1000.0);
        set("pixels", self.stats.pixels);
        set("primitives", prims as f64);
    }

    fn handle_platform_output(&self, out: egui::PlatformOutput) {
        for c in out.commands {
            match c {
                egui::OutputCommand::CopyText(t) => set_clipboard_text(&t),
                egui::OutputCommand::CopyImage(i) => set_clipboard_image(&i),
                egui::OutputCommand::OpenUrl(u) => open_url(&u.url, u.new_tab),
                _ => {}
            }
        }
        set_cursor_icon(&self.canvas, out.cursor_icon);
        if self.has_focus() {
            if let Some(ime) = &out.ime {
                if ime.should_interrupt_composition {
                    self.text_agent.interrupt_ime_composition();
                }
                self.text_agent.focus();
            } else {
                self.text_agent.blur();
                focus_without_scroll(&self.canvas).ok();
            }
        }
        if let Err(e) = self.text_agent.update(out.ime, &self.canvas, self.egui_ctx.zoom_factor()) {
            log::error!("Text-Agent: {}", string_from_js_value(&e));
        }
    }
}

// ---------------------------------------------------------------- SoftWeb (wie eframe::WebRunner)

struct TargetEvent {
    target: web_sys::EventTarget,
    event_name: String,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

struct AnimationFrameRequest {
    id: i32,
    timeout: bool,
    _closure: Closure<dyn FnMut() -> Result<(), JsValue>>,
}

impl AnimationFrameRequest {
    fn cancel(&self, w: &web_sys::Window) {
        if self.timeout {
            w.clear_timeout_with_handle(self.id);
        } else {
            w.cancel_animation_frame(self.id).ok();
        }
    }
}

/// Hält den [`Runner`], die Ereignis-Abos und die Frame-Planung (günstig zu klonen).
#[derive(Clone)]
pub(crate) struct SoftWeb {
    runner: Rc<RefCell<Option<Runner>>>,
    events: Rc<RefCell<Vec<TargetEvent>>>,
    frame: Rc<RefCell<Option<AnimationFrameRequest>>>,
    resize_observer: Rc<RefCell<Option<events::ResizeObserverContext>>>,
}

impl SoftWeb {
    fn unsubscribe_from_all_events(&self) {
        for e in std::mem::take(&mut *self.events.borrow_mut()) {
            let _ = e.target.remove_event_listener_with_callback(&e.event_name, e.closure.as_ref().unchecked_ref());
        }
        self.resize_observer.replace(None);
    }

    /// `None` nach einem Absturz (dann keine Ereignisse mehr verarbeiten).
    pub(crate) fn try_lock(&self) -> Option<std::cell::RefMut<'_, Runner>> {
        if PANICKED.load(Ordering::Relaxed) {
            self.unsubscribe_from_all_events();
            return None;
        }
        let lock = self.runner.try_borrow_mut().ok()?;
        std::cell::RefMut::filter_map(lock, |r| r.as_mut()).ok()
    }

    pub fn add_event_listener<E: JsCast>(&self, target: &web_sys::EventTarget, event_name: &'static str, mut closure: impl FnMut(E, &mut Runner) + 'static) -> Result<(), JsValue> {
        let options = web_sys::AddEventListenerOptions::default();
        self.add_event_listener_ex(target, event_name, &options, move |e, r, _| closure(e, r))
    }

    pub fn add_event_listener_ex<E: JsCast>(&self, target: &web_sys::EventTarget, event_name: &'static str, options: &web_sys::AddEventListenerOptions, mut closure: impl FnMut(E, &mut Runner, &Self) + 'static) -> Result<(), JsValue> {
        let web = self.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
            if let Some(mut r) = web.try_lock() {
                closure(event.unchecked_into::<E>(), &mut r, &web);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        target.add_event_listener_with_callback_and_add_event_listener_options(event_name, closure.as_ref().unchecked_ref(), options)?;
        self.events.borrow_mut().push(TargetEvent { target: target.clone(), event_name: event_name.to_owned(), closure });
        Ok(())
    }

    /// Plant einen Frame (höchstens einen gleichzeitig); im verborgenen Tab per `setTimeout`.
    pub(crate) fn request_animation_frame(&self) -> Result<(), JsValue> {
        if self.frame.borrow().is_some() {
            return Ok(());
        }
        let w = web_sys::window().ok_or("kein window")?;
        let closure = Closure::once({
            let web = self.clone();
            move || {
                let _ = web.frame.take();
                events::paint_and_schedule(&web)
            }
        });
        let hidden = w.document().is_some_and(|d| d.hidden());
        let req = if hidden {
            let id = w.set_timeout_with_callback_and_timeout_and_arguments_0(closure.as_ref().unchecked_ref(), 10)?;
            AnimationFrameRequest { id, timeout: true, _closure: closure }
        } else {
            let id = w.request_animation_frame(closure.as_ref().unchecked_ref())?;
            AnimationFrameRequest { id, timeout: false, _closure: closure }
        };
        self.frame.borrow_mut().replace(req);
        Ok(())
    }

    pub(crate) fn reschedule_frame(&self) -> Result<(), JsValue> {
        if let (Some(f), Some(w)) = (self.frame.borrow_mut().take(), web_sys::window()) {
            f.cancel(&w);
        }
        self.request_animation_frame()
    }
}

/// Absturz: im Browser melden (Konsole und Fehlerseite über `window.owndrawFehler`, siehe `index.html`).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        PANICKED.store(true, Ordering::Relaxed);
        let msg = format!("Interner Fehler: {info}");
        web_sys::console::error_1(&JsValue::from_str(&msg));
        if let Some(w) = web_sys::window() {
            if let Ok(f) = js_sys::Reflect::get(&w, &JsValue::from_str("owndrawFehler")) {
                if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                    let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&msg));
                }
            }
        }
    }));
}

/// 2D-Kontext des Canvas; hat der Canvas schon einen anderen Kontext (WebGL-Versuch), wird er durch eine
/// frische Kopie ersetzt.
fn context_2d(canvas: HtmlCanvasElement) -> Result<(HtmlCanvasElement, CanvasRenderingContext2d), JsValue> {
    let get = |c: &HtmlCanvasElement| -> Option<CanvasRenderingContext2d> {
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &JsValue::from_str("alpha"), &JsValue::FALSE);
        c.get_context_with_context_options("2d", &opts).ok().flatten()?.dyn_into().ok()
    };
    if let Some(ctx) = get(&canvas) {
        return Ok((canvas, ctx));
    }
    let fresh: HtmlCanvasElement = canvas.clone_node()?.dyn_into().map_err(|_| JsValue::from_str("Canvas nicht kopierbar"))?;
    canvas.replace_with_with_node_1(&fresh)?;
    let ctx = get(&fresh).ok_or_else(|| JsValue::from_str("Canvas 2D wird von diesem Browser nicht unterstützt"))?;
    Ok((fresh, ctx))
}

/// Startet die App ohne WebGL im Canvas. `make_app` baut die App mit dem egui-Kontext.
pub async fn start(canvas: HtmlCanvasElement, make_app: impl FnOnce(&egui::Context) -> crate::app::App) -> Result<(), JsValue> {
    install_panic_hook();
    let (canvas, ctx2d) = context_2d(canvas)?;
    canvas.set_tab_index(0);
    canvas.style().set_property("outline", "none")?;

    let web = SoftWeb { runner: Rc::new(RefCell::new(None)), events: Default::default(), frame: Default::default(), resize_observer: Default::default() };
    let text_agent = TextAgent::attach(&web, &canvas)?;

    let egui_ctx = egui::Context::default();
    egui_ctx.add_glyph_rasterizer(glyphs::glyph_rasterizer());
    let ua = web_sys::window().and_then(|w| w.navigator().user_agent().ok()).unwrap_or_default();
    egui_ctx.set_os(egui::os::OperatingSystem::from_user_agent(&ua));
    egui_ctx.options_mut(|o| {
        // wie eframe: Browser-Zoom gilt, Zoom-Tastenkürzel macht der Browser
        o.zoom_with_keyboard = false;
        o.zoom_factor = 1.0;
    });
    egui_ctx.input_mut(|i| {
        i.raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_pixels_per_point());
        i.pixels_per_point = native_pixels_per_point();
    });
    let app = make_app(&egui_ctx);

    let needs_repaint = Arc::new(NeedRepaint(Mutex::new(f64::NEG_INFINITY)));
    {
        let nr = needs_repaint.clone();
        egui_ctx.set_request_repaint_callback(move |info| nr.repaint_after(info.delay.as_secs_f64()));
    }
    let stats_obj = js_sys::Object::new();
    if let Some(w) = web_sys::window() {
        let _ = js_sys::Reflect::set(&w, &JsValue::from_str("owndrawSoftStats"), &stats_obj);
    }
    let mut input = WebInput::default();
    input.raw.max_texture_side = Some(8192);
    input.raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_pixels_per_point());
    input.raw.system_theme = system_theme();
    let runner = Runner {
        web_options: Opts { should_stop_propagation: |_| true, should_prevent_default: |_| true },
        canvas: canvas.clone(),
        ctx2d,
        egui_ctx,
        app,
        input,
        needs_repaint,
        text_agent,
        painter: SoftPainter::new(),
        textures_delta: Default::default(),
        clipped_primitives: None,
        stats: Stats { obj: Some(stats_obj), ..Stats::default() },
    };
    web.runner.replace(Some(runner));

    let ro = events::ResizeObserverContext::new(&web)?;
    ro.observe(&canvas);
    web.resize_observer.replace(Some(ro));
    events::install_event_handlers(&web)?;
    show_hint();
    log::info!("OwnDraw läuft ohne WebGL (Software-Darstellung)");
    Ok(())
}

/// Dezenter Hinweis unten (wegklickbar, bleibt dann weg).
fn show_hint() {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    if storage.as_ref().and_then(|s| s.get_item(HINT_KEY).ok().flatten()).is_some() {
        return;
    }
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let Some(body) = doc.body() else { return };
    let Ok(div) = doc.create_element("div") else { return };
    div.set_id("owndraw-software-hinweis");
    let _ = div.set_attribute("role", "status");
    let _ = div.set_attribute(
        "style",
        "position:fixed;left:50%;bottom:12px;transform:translateX(-50%);z-index:10;display:flex;align-items:center;gap:10px;\
         max-width:calc(100% - 32px);box-sizing:border-box;padding:8px 8px 8px 14px;border-radius:10px;\
         background:rgba(48,49,56,.92);color:#e9eaee;font:13px/1.35 system-ui,-apple-system,'Segoe UI',Ubuntu,sans-serif;\
         box-shadow:0 2px 8px rgba(0,0,0,.25);user-select:none;-webkit-user-select:none;",
    );
    let Ok(text) = doc.create_element("span") else { return };
    text.set_text_content(Some("Ohne Grafikbeschleunigung – Zeichnen kann langsamer sein. Tipp: Hardwarebeschleunigung im Browser aktivieren."));
    let Ok(btn) = doc.create_element("button") else { return };
    btn.set_text_content(Some("×"));
    let _ = btn.set_attribute("aria-label", "Hinweis schließen");
    let _ = btn.set_attribute("title", "Schließen");
    let _ = btn.set_attribute("style", "flex:none;border:0;background:transparent;color:inherit;font-size:20px;line-height:1;cursor:pointer;padding:2px 6px;border-radius:6px;");
    let d2 = div.clone();
    let close = Closure::<dyn FnMut()>::new(move || {
        d2.remove();
        if let Some(s) = &storage {
            let _ = s.set_item(HINT_KEY, "aus");
        }
    });
    let _ = btn.add_event_listener_with_callback("click", close.as_ref().unchecked_ref());
    close.forget();
    let _ = div.append_child(&text);
    let _ = div.append_child(&btn);
    let _ = body.append_child(&div);
}
