//! The Sessions page: what's running, how it's doing, and its log.

use super::Launcher;
use super::sessions::{Exit, LogLevel, SessionProc};
use crate::ipc::Phase;
use crate::ui::theme::*;
use crate::ui::{column, widgets};
use egui::{Color32, RichText, Sense, Vec2, vec2};

fn duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

enum RowAction {
    Select,
    Stop,
    Restart,
    Dismiss,
}

fn state_color(s: &SessionProc) -> Color32 {
    match (s.exit, s.status.as_ref().map(|st| st.phase)) {
        (Some(Exit::Crashed(_)), _) => danger(),
        (_, Some(Phase::Playing)) => accent(),
        (_, Some(Phase::Ended)) => warn(),
        _ => ink_muted(),
    }
}

/// A number with its name beneath, in a column that doesn't shift as it changes.
fn metric(ui: &mut egui::Ui, value: String, name: &str) {
    ui.allocate_ui_with_layout(vec2(84.0, 42.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_width(84.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(value).font(font_bold(SIZE_BODY)));
        ui.label(RichText::new(name).size(SIZE_SMALL).color(ink_muted()));
    });
}

fn row(app: &mut Launcher, ui: &mut egui::Ui, index: usize) -> Option<RowAction> {
    let selected = app.selected_session == Some(app.sessions[index].id);
    let face = match &app.sessions[index].spec.profile {
        crate::ipc::SpecProfile::Online { uuid, .. } => {
            let uuid = *uuid;
            let skin = app.accounts.accounts.iter().find(|a| a.uuid == uuid).and_then(|a| a.skin_url.clone());
            app.face_texture(uuid, skin.as_deref())
        }
        _ => None,
    };
    let s = &app.sessions[index];
    let mut action = None;
    egui::Frame::new().fill(if selected { raised() } else { surface() }).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            widgets::face_tile(ui, rect, face.as_ref());
            ui.add_space(4.0);
            // Buttons claim their space first; the labels get what's left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if s.running() {
                    let label = if s.stopping() {
                        "Stopping"
                    } else if s.ended() {
                        "Close"
                    } else {
                        "Stop"
                    };
                    if widgets::secondary(ui, label).clicked() {
                        action = Some(RowAction::Stop);
                    }
                } else {
                    if widgets::secondary(ui, "Dismiss").clicked() {
                        action = Some(RowAction::Dismiss);
                    }
                    if widgets::secondary(ui, "Restart").clicked() {
                        action = Some(RowAction::Restart);
                    }
                }
                if !selected && widgets::secondary(ui, "Show log").clicked() {
                    action = Some(RowAction::Select);
                }
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(format!("{} on {}", s.name(), s.spec.server_name)).font(font_bold(SIZE_BODY))).truncate());
                    ui.add(egui::Label::new(RichText::new(s.state_text()).size(SIZE_SMALL).color(state_color(s))).truncate());
                });
            });
        });
        if let (Some(st), true) = (&s.status, s.on_server()) {
            if st.fps > 0.0 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(44.0);
                    metric(ui, format!("{:.0}", st.fps), "frames/s");
                    metric(ui, format!("{:.1} ms", st.frame_ms), "frame time");
                    metric(ui, st.ping.filter(|p| *p >= 0).map_or("none".to_string(), |p| format!("{p} ms")), "ping");
                    metric(ui, s.memory_mb.map_or("unknown".to_string(), |m| format!("{m} MB")), "memory");
                    metric(ui, duration(s.started.elapsed().as_secs()), "open for");
                });
            }
            if let Some(p) = st.pos {
                let dim = st.dimension.as_deref().map_or("", |d| d.trim_start_matches("minecraft:"));
                ui.horizontal(|ui| {
                    ui.add_space(44.0);
                    widgets::muted(ui, format!("At {:.0}, {:.0}, {:.0} in {dim}", p[0], p[1], p[2]));
                });
            }
        }
    });
    action
}

fn log_view(app: &Launcher, ui: &mut egui::Ui, height: f32) {
    let Some(s) = app.selected_session.and_then(|id| app.sessions.iter().find(|s| s.id == id)) else { return };
    ui.horizontal(|ui| {
        ui.set_min_height(TARGET);
        widgets::subheading(ui, format!("Log for {}", s.name()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::secondary(ui, "Open log file").clicked() {
                let path = crate::session_log_path(&app.paths, s.name());
                if let Err(e) = webbrowser::open(&format!("file://{}", path.display())) {
                    tracing::warn!("couldn't open {}: {e}", path.display());
                }
            }
        });
    });
    egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().max_height((height - 40.0).max(80.0)).auto_shrink([false, false]).stick_to_bottom(true).id_salt(("log", s.id)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if s.log.is_empty() {
                widgets::muted(ui, "Nothing logged yet.");
            }
            for line in &s.log {
                let color = match line.level {
                    LogLevel::Error => danger(),
                    LogLevel::Warn => warn(),
                    LogLevel::Info => ink_muted(),
                };
                ui.label(RichText::new(format!("{:>6.1}s  {}", line.at, line.text.trim())).size(SIZE_SMALL).color(color));
            }
        });
    });
}

pub fn draw(app: &mut Launcher, ui: &mut egui::Ui) {
    column(ui, |ui| {
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            ui.set_min_height(TARGET);
            widgets::heading(ui, "Sessions");
            let running = app.running_count();
            if running > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, "Stop all").clicked() {
                        let ids: Vec<u64> = app.sessions.iter().filter(|s| s.running()).map(|s| s.id).collect();
                        for id in ids {
                            app.stop(id);
                        }
                    }
                });
            }
        });
        ui.add_space(10.0);

        if app.sessions.is_empty() && app.pending.is_empty() {
            ui.label("Nothing is running. Each session opens in its own window, and Lattice stays open here so you can start several and compare them.");
            ui.add_space(12.0);
            if widgets::primary(ui, "Choose where to play").clicked() {
                app.screen = super::Screen::Play;
            }
            return;
        }

        let log_height = (ui.available_height() * 0.42).clamp(160.0, 360.0);
        let list_height = (ui.available_height() - log_height - 56.0).max(120.0);
        let mut action = None;
        egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, true]).id_salt("session_list").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for p in &app.pending {
                egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        crate::ui::widgets::busy(ui);
                        ui.label(RichText::new(format!("{} on {}", p.account, p.server_name)).font(font_bold(SIZE_BODY)));
                    });
                    widgets::muted(ui, "Refreshing the Microsoft sign-in…");
                });
            }
            for i in 0..app.sessions.len() {
                if let Some(a) = row(app, ui, i) {
                    action = Some((app.sessions[i].id, a));
                }
            }
        });
        match action {
            Some((id, RowAction::Select)) => app.selected_session = Some(id),
            Some((id, RowAction::Stop)) => app.stop(id),
            Some((id, RowAction::Restart)) => app.restart(id),
            Some((id, RowAction::Dismiss)) => app.dismiss(id),
            None => {}
        }
        ui.add_space(14.0);
        log_view(app, ui, log_height);
    });
}
