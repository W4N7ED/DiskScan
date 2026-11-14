//! Fenêtre principale : état, panneau de paramètres, barre d'état, boutons de fenêtre.

use crate::icons;
use crate::theme::{self, regular, semibold, Palette};
use crate::widgets::{self as w, Button, Kind};
use eframe::egui::{self, pos2, vec2, Align, Frame, Layout, Margin, Rect, Sense, Ui, Vec2};
use serde::{Deserialize, Serialize};

const SHORTCUTS: &str = "F5 Lancer · Échap Arrêter / désélectionner · Clic droit : actions";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub dark_mode: bool,
    pub win_size: Option<[f32; 2]>,
    pub win_pos: Option<[f32; 2]>,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { dark_mode: false, win_size: None, win_pos: None, maximized: false }
    }
}

pub struct DiskScanApp {
    s: Settings,
    frames: u32,
    toggle_theme: bool,
    #[cfg(debug_assertions)]
    script: Option<crate::demo::Script>,
}

impl DiskScanApp {
    pub fn new(cc: &eframe::CreationContext<'_>, _cli_target: Option<String>, _autostart: bool) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::setup_fonts(&cc.egui_ctx);
        let stored: Option<Settings> = cc.storage.and_then(|st| eframe::get_value(st, eframe::APP_KEY));
        let first_run = stored.is_none();
        let mut s = stored.unwrap_or_default();
        if first_run {
            // Premier lancement : on suit le thème clair / sombre de Windows.
            s.dark_mode = crate::win::system_prefers_dark().unwrap_or(false);
        }
        restore_window(&cc.egui_ctx, &s);
        theme::apply(&cc.egui_ctx, s.dark_mode);
        Self {
            s,
            frames: 0,
            toggle_theme: false,
            #[cfg(debug_assertions)]
            script: crate::demo::Script::from_env(),
        }
    }

    /// Mémorise taille, position et état « agrandi » pour le prochain lancement.
    fn track_window(&mut self, ctx: &egui::Context) {
        self.frames = self.frames.saturating_add(1);
        if self.frames < 10 {
            return; // la restauration de la fenêtre n'est pas encore appliquée
        }
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.minimized == Some(true) {
            return;
        }
        let maximized = vp.maximized.unwrap_or(false);
        self.s.maximized = maximized;
        if !maximized && vp.fullscreen != Some(true) {
            if let Some(r) = vp.inner_rect.filter(|r| r.width() >= 200.0 && r.height() >= 200.0) {
                self.s.win_size = Some([r.width(), r.height()]);
            }
            if let Some(r) = vp.outer_rect {
                self.s.win_pos = Some([r.left(), r.top()]);
            }
        }
    }

    fn status_bar(&self, ctx: &egui::Context, p: &'static Palette) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(Frame::none().fill(p.sidebar).inner_margin(Margin::symmetric(12.0, 0.0)))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let cy = rect.center().y;
                icons::paint(ui, &icons::INFO, w::icon_rect(rect.left(), cy, 14.0), p.accent_text);
                let sg = w::galley(ui, SHORTCUTS, regular(12.0), p.text3);
                let sw = sg.size().x;
                w::text_at(ui.painter(), sg, rect.right() - sw, cy);
                let g = w::galley_trunc(ui, "Prêt.", regular(12.0), p.text, rect.width() - sw - 38.0);
                w::text_at(ui.painter(), g, rect.left() + 22.0, cy);
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context, p: &'static Palette) {
        egui::SidePanel::left("sidebar")
            .exact_width(300.0)
            .resizable(false)
            .frame(Frame::none().fill(p.sidebar))
            .show(ctx, |ui| {
                egui::TopBottomPanel::top("sidebar_head")
                    .frame(Frame::none().fill(p.sidebar).inner_margin(Margin { left: 16.0, right: 12.0, top: 12.0, bottom: 12.0 }))
                    .show_inside(ui, |ui| self.sidebar_head(ui, p));
            });
    }

    fn sidebar_head(&mut self, ui: &mut Ui, p: &'static Palette) {
        // L'en-tête sert aussi de poignée pour déplacer la fenêtre.
        window_drag(ui, ui.max_rect().expand2(vec2(16.0, 12.0)), "drag_sidebar");
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            w::paint_logo(ui.painter(), r, p.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                w::label(ui, "DiskScan", semibold(15.0), p.text);
                w::label(ui, "Analyseur d'espace disque", regular(11.0), p.text3);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (icon, tip) = if self.s.dark_mode {
                    (&icons::SUN, "Passer au thème clair")
                } else {
                    (&icons::MOON, "Passer au thème sombre")
                };
                let r = Button::new(Kind::Secondary, "").icon(icon).icon_size(16.0).size(32.0, 32.0).show(ui, p);
                if r.on_hover_text(tip).clicked() {
                    self.toggle_theme = true;
                }
            });
        });
    }
}

impl eframe::App for DiskScanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.track_window(ctx);
        let p = Palette::get(self.s.dark_mode);
        self.status_bar(ctx, p);
        self.sidebar(ctx, p);
        title_bar(ctx, p);
        egui::CentralPanel::default()
            .frame(Frame::none().fill(p.bg).inner_margin(Margin { left: 16.0, right: 16.0, top: 4.0, bottom: 16.0 }))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().rect(r.shrink(0.5), 8.0, p.surface, w::stroke(1.0, p.border));
                w::paint_empty_logo(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(56.0)), p);
            });
        resize_handles(ctx);
        #[cfg(debug_assertions)]
        if self.script.as_ref().is_some_and(|s| s.pending()) {
            crate::demo::keep_alive(ctx);
        }
        if std::mem::take(&mut self.toggle_theme) {
            self.s.dark_mode = !self.s.dark_mode;
            theme::apply(ctx, self.s.dark_mode);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
    }

    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(script) = &mut self.script {
            script.inject(ctx, raw_input);
            if script.pending() {
                crate::demo::keep_alive(ctx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fenêtre sans barre de titre : déplacement, boutons, redimensionnement
// ---------------------------------------------------------------------------

/// Remet la fenêtre comme à la dernière fermeture : taille, position (si l'écran est
/// toujours branché), puis agrandissement.
fn restore_window(ctx: &egui::Context, s: &Settings) {
    use egui::ViewportCommand as Cmd;
    if let Some([w, h]) = s.win_size {
        ctx.send_viewport_cmd(Cmd::InnerSize(vec2(w.max(960.0), h.max(560.0))));
    }
    if let Some([x, y]) = s.win_pos {
        let ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
        if crate::win::point_on_screen(((x + 120.0) * ppp) as i32, ((y + 16.0) * ppp) as i32) {
            ctx.send_viewport_cmd(Cmd::OuterPosition(pos2(x, y)));
        }
    }
    if s.maximized {
        ctx.send_viewport_cmd(Cmd::Maximized(true));
    }
}

const TITLE_H: f32 = 32.0;
const CAPTION_W: f32 = 46.0;

fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Zone qui déplace la fenêtre (glisser) ou l'agrandit (double-clic).
/// À appeler avant les widgets qu'elle contient, pour qu'ils restent cliquables.
fn window_drag(ui: &Ui, rect: Rect, id: &str) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    if resp.double_clicked() {
        let maximized = is_maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Bandeau du haut, de la couleur du fond : uniquement réduire / agrandir / fermer.
fn title_bar(ctx: &egui::Context, p: &'static Palette) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(TITLE_H)
        .show_separator_line(false)
        .frame(Frame::none().fill(p.bg))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            window_drag(ui, rect, "drag_title");
            let maximized = is_maximized(ctx);
            let mut x = rect.right();
            for kind in [Caption::Close, Caption::Maximize, Caption::Minimize] {
                x -= CAPTION_W;
                let r = Rect::from_min_size(pos2(x, rect.top()), vec2(CAPTION_W, TITLE_H));
                if caption_button(ui, p, r, kind, maximized).clicked() {
                    ctx.send_viewport_cmd(match kind {
                        Caption::Minimize => egui::ViewportCommand::Minimized(true),
                        Caption::Maximize => egui::ViewportCommand::Maximized(!maximized),
                        Caption::Close => egui::ViewportCommand::Close,
                    });
                }
            }
        });
}

fn caption_button(ui: &Ui, p: &Palette, rect: Rect, kind: Caption, maximized: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("caption", kind as u8)), Sense::click());
    let down = resp.is_pointer_button_down_on();
    let hot = resp.hovered() || down;
    let (bg, fg) = match (kind, hot) {
        (_, false) => (p.bg, p.text2),
        (Caption::Close, true) => {
            let red = egui::Color32::from_rgb(0xC4, 0x2B, 0x1C);
            (if down { w::lerp_color(red, p.bg, 0.25) } else { red }, egui::Color32::WHITE)
        }
        (_, true) => (w::lerp_color(p.bg, p.text, if down { 0.14 } else { 0.08 }), p.text),
    };
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, 0.0, bg);
    }
    let c = pos2(rect.center().x.round(), rect.center().y.round());
    let line = w::stroke(1.0, fg);
    match kind {
        Caption::Minimize => {
            painter.hline(c.x - 5.0..=c.x + 5.0, c.y + 0.5, line);
        }
        Caption::Maximize if maximized => {
            let back = Rect::from_min_size(pos2(c.x - 2.5, c.y - 5.5), Vec2::splat(8.0));
            let front = Rect::from_min_size(pos2(c.x - 5.5, c.y - 2.5), Vec2::splat(8.0));
            painter.rect_stroke(back, 1.5, line);
            painter.rect(front, 1.5, bg, line);
        }
        Caption::Maximize => {
            painter.rect_stroke(Rect::from_center_size(pos2(c.x + 0.5, c.y + 0.5), Vec2::splat(10.0)), 1.5, line);
        }
        Caption::Close => {
            let line = w::stroke(1.1, fg);
            painter.line_segment([pos2(c.x - 4.5, c.y - 4.5), pos2(c.x + 5.5, c.y + 5.5)], line);
            painter.line_segment([pos2(c.x + 5.5, c.y - 4.5), pos2(c.x - 4.5, c.y + 5.5)], line);
        }
    }
    resp.on_hover_text(match kind {
        Caption::Minimize => "Réduire",
        Caption::Maximize if maximized => "Restaurer",
        Caption::Maximize => "Agrandir",
        Caption::Close => "Fermer",
    })
}

/// Bords et coins de la fenêtre : redimensionnement (la bordure Windows a disparu avec la barre).
fn resize_handles(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if is_maximized(ctx) {
        return;
    }
    const E: f32 = 4.0;
    const K: f32 = 8.0;
    let s = ctx.screen_rect();
    let (l, r, t, b) = (s.left(), s.right(), s.top(), s.bottom());
    let zones = [
        (Rect::from_min_max(pos2(l + K, t), pos2(r - K, t + E)), D::North, C::ResizeNorth),
        (Rect::from_min_max(pos2(l + K, b - E), pos2(r - K, b)), D::South, C::ResizeSouth),
        (Rect::from_min_max(pos2(l, t + K), pos2(l + E, b - K)), D::West, C::ResizeWest),
        (Rect::from_min_max(pos2(r - E, t + K), pos2(r, b - K)), D::East, C::ResizeEast),
        (Rect::from_min_max(pos2(l, t), pos2(l + K, t + K)), D::NorthWest, C::ResizeNorthWest),
        (Rect::from_min_max(pos2(r - K, t), pos2(r, t + K)), D::NorthEast, C::ResizeNorthEast),
        (Rect::from_min_max(pos2(l, b - K), pos2(l + K, b)), D::SouthWest, C::ResizeSouthWest),
        (Rect::from_min_max(pos2(r - K, b - K), pos2(r, b)), D::SouthEast, C::ResizeSouthEast),
    ];
    // Appelé en fin d'image : le curseur de redimensionnement l'emporte sur celui des widgets.
    let (pos, pressed, held) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
    let Some(pos) = pos else { return };
    if let Some((_, dir, cursor)) = zones.into_iter().find(|z| z.0.contains(pos)) {
        if pressed {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
        if pressed || !held {
            ctx.set_cursor_icon(cursor);
        }
    }
}

/// Icône de la fenêtre.
pub fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: crate::icon::logo_rgba(64),
        width: 64,
        height: 64,
    }
}
