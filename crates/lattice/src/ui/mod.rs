//! Interface shared by the launcher and game windows, plus the game window's
//! screens. Every control here is wired to something real.

pub mod block_screens;
pub mod chrome;
pub mod doll;
pub mod hud;
pub mod inventory;
pub mod items;
pub mod modal;
pub mod recipes;
pub mod session;
pub mod settings;
pub mod theme;
pub mod widgets;

use crate::app::{App, Screen};
use crate::assets::AssetState;
use egui::{CornerRadius, Frame, Margin};
use std::time::Instant;
use theme::*;

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    chrome::set_unit(ui.ctx().content_rect(), ui.ctx().pixels_per_point());
    match app.screen.clone() {
        Screen::InGame => hud::draw(app, ui),
        Screen::Settings => settings::in_game(app, ui),
        screen => {
            let frame = Frame::new().fill(ground()).inner_margin(Margin::ZERO);
            egui::CentralPanel::no_frame().frame(frame).show(ui, |ui| match screen {
                Screen::Loading => match &app.assets {
                    AssetState::Loading(l) => {
                        let p = l.progress.lock().unwrap().clone();
                        loading(ui, &app.spec.server_name, &p.step, p.fraction);
                    }
                    AssetState::Failed(e) => {
                        let e = e.clone();
                        session::files_failed(app, ui, &e);
                    }
                    AssetState::Ready(_) => {}
                },
                Screen::Connecting => session::connecting(app, ui),
                Screen::Ended { .. } => session::ended(app, ui),
                Screen::InGame | Screen::Settings => unreachable!(),
            });
        }
    }
    let toast_time = app.config.interface.toast_time;
    toasts(&mut app.toasts, ui, toast_time);
}

/// Content column used by menu screens: left-aligned, max 1040 px wide.
pub fn column(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let avail = ui.available_rect_before_wrap();
    let width = (avail.width() - 64.0).min(1040.0);
    let rect = egui::Rect::from_min_size(avail.min + egui::vec2(32.0, 0.0), egui::vec2(width, avail.height()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    add(&mut child);
    // Tell a surrounding scroll area how tall this was.
    let used = child.min_rect();
    ui.allocate_rect(egui::Rect::from_min_max(avail.min, egui::pos2(avail.right(), used.bottom())), egui::Sense::hover());
}

/// The picture behind screens with no world to show, faded where words go.
pub fn backdrop(p: &egui::Painter, rect: egui::Rect, name: &str) {
    let seed = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    widgets::lattice_field(p, rect, seed, 30.0);
    let g = ground();
    widgets::scrim(p, egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.bottom() - 220.0)), g, 110, 200);
    let lower = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 220.0), rect.right_bottom());
    widgets::scrim(p, lower, g, 200, 245);
}

fn loading(ui: &mut egui::Ui, server: &str, step: &str, fraction: Option<f32>) {
    backdrop(ui.painter(), ui.max_rect(), server);
    column(ui, |ui| {
        ui.add_space((ui.available_height() - 160.0).max(0.0));
        widgets::heading(ui, format!("Joining {server}"));
        ui.add_space(4.0);
        ui.label(step);
        ui.add_space(10.0);
        match fraction {
            Some(f) => widgets::progress(ui, f, ui.available_width().min(520.0)),
            None => {
                crate::ui::widgets::busy(ui);
            }
        }
    });
}

pub fn toasts(list: &mut Vec<Toast>, ui: &mut egui::Ui, time: f32) {
    list.retain(|t| t.at.elapsed().as_secs_f32() < time);
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let mut y = screen.bottom() - 24.0;
    for t in list.iter().rev() {
        let galley = ui.painter().layout(t.text.clone(), font(SIZE_BODY), ink(), 520.0);
        let size = galley.size() + egui::vec2(28.0, 18.0);
        let rect = egui::Rect::from_min_size(egui::pos2(screen.right() - size.x - 24.0, y - size.y), size);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("toasts")));
        painter.rect_filled(rect, CornerRadius::ZERO, raised());
        painter.rect_stroke(rect, CornerRadius::ZERO, egui::Stroke::new(if t.error { 2.0 } else { 1.0 }, if t.error { danger() } else { edge() }), egui::StrokeKind::Inside);
        painter.galley(rect.min + egui::vec2(14.0, 9.0), galley, ink());
        y = rect.top() - 8.0;
    }
}
