//! The game window's screens around play: connecting and session ended.

use super::theme::*;
use super::{column, widgets};
use crate::app::{App, Screen};
use crate::game::Phase;
use egui::RichText;
use lattice_net::Stage;

pub fn stage_text(stage: Stage) -> &'static str {
    match stage {
        Stage::Resolving => "Looking up the address…",
        Stage::Connecting => "Connecting…",
        Stage::Authenticating => "Confirming your account with Mojang…",
        Stage::LoggingIn => "Logging in…",
        Stage::Configuring => "Receiving the server's game data…",
    }
}

pub fn connecting(app: &mut App, ui: &mut egui::Ui) {
    let stage = match app.session.as_ref().map(|s| &s.phase) {
        Some(Phase::Connecting(s)) => stage_text(*s),
        Some(Phase::LoadingTerrain) | Some(Phase::Playing) => "Loading terrain…",
        None => "",
    };
    let name = app.spec.server_name.clone();
    let coc = app.session.as_ref().and_then(|s| s.code_of_conduct.clone());
    let mut cancel = false;
    super::backdrop(ui.painter(), ui.max_rect(), &name);
    column(ui, |ui| {
        let reserve = if coc.is_some() { 480.0 } else { 170.0 };
        ui.add_space((ui.available_height() - reserve).max(16.0));
        widgets::heading(ui, format!("Joining {name}"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            crate::ui::widgets::busy(ui);
            ui.label(stage);
        });
        if let Some(text) = &coc {
            ui.add_space(14.0);
            egui::Frame::new().fill(surface()).stroke(egui::Stroke::new(1.0, edge())).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
                ui.set_width(ui.available_width().min(640.0));
                widgets::subheading(ui, "This server's code of conduct");
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    ui.label(text);
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::primary(ui, "Accept and join").clicked() {
                        if let Some(s) = &mut app.session {
                            s.accept_code_of_conduct();
                        }
                    }
                    if widgets::secondary(ui, "Decline").clicked() {
                        cancel = true;
                    }
                });
            });
        }
        ui.add_space(12.0);
        if coc.is_none() && widgets::secondary(ui, "Cancel").clicked() {
            cancel = true;
        }
    });
    if cancel {
        app.close_now();
    }
}

pub fn ended(app: &mut App, ui: &mut egui::Ui) {
    let Screen::Ended { title, reason, detail } = app.screen.clone() else { return };
    let job = reason.as_ref().map(|r| widgets::spans_job(&r.spans(app.lang()), SIZE_BODY, ink(), 640.0));
    let mut close = false;
    let mut rejoin = false;
    let name = app.spec.server_name.clone();
    super::backdrop(ui.painter(), ui.max_rect(), &name);
    column(ui, |ui| {
        ui.add_space((ui.available_height() - 300.0).max(16.0));
        widgets::heading(ui, &title);
        ui.add_space(8.0);
        if let Some(job) = job {
            ui.scope(|ui| {
                ui.set_max_width(680.0);
                ui.label(egui::WidgetText::LayoutJob(job.into()));
            });
        }
        if let Some(d) = &detail {
            ui.label(RichText::new(d).color(ink()));
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if widgets::primary(ui, "Rejoin").clicked() {
                rejoin = true;
            }
            if widgets::secondary(ui, "Close window").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        ui.add_space(24.0);
    });
    if close {
        app.close_now();
    }
    if rejoin {
        app.rejoin();
    }
}

pub fn files_failed(app: &mut App, ui: &mut egui::Ui, error: &str) {
    column(ui, |ui| {
        ui.add_space(ui.available_height() * 0.3);
        widgets::heading(ui, "Couldn't download the game files");
        ui.add_space(6.0);
        widgets::error_text(ui, error);
        ui.add_space(6.0);
        ui.label(RichText::new("Lattice needs the official game resources from Mojang once. Check your connection and start this session again.").color(ink_muted()));
        ui.add_space(14.0);
        if widgets::secondary(ui, "Close window").clicked() {
            app.close_now();
        }
    });
}
