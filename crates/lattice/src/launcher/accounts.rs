//! Accounts: Microsoft sign-in (device code) and the offline name.

use super::Launcher as App;
use crate::ui::theme::*;
use crate::ui::{column, widgets};
use egui::{CornerRadius, RichText, Vec2};
use lattice_auth::Step;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        column(ui, |ui| {
            ui.add_space(28.0);
            widgets::heading(ui, "Accounts");
            ui.add_space(4.0);
            ui.label(RichText::new("Online-mode servers check your account with Mojang. Offline-mode servers only need a name.").color(ink_muted()));
            ui.add_space(20.0);

            if app.sign_in.is_some() {
                sign_in_panel(app, ui);
                ui.add_space(24.0);
            }

            account_list(app, ui);
            ui.add_space(14.0);
            if app.sign_in.is_none() {
                microsoft(app, ui);
            }
            ui.add_space(32.0);
            offline(app, ui);
            ui.add_space(32.0);
        });
    });
}

fn account_list(app: &mut App, ui: &mut egui::Ui) {
    let accounts = app.accounts.accounts.clone();
    if accounts.is_empty() {
        return;
    }
    ui.label(RichText::new("Microsoft accounts").font(font_bold(SIZE_LEAD)));
    ui.add_space(6.0);
    let mut use_id = None;
    let mut remove_id = None;
    for a in &accounts {
        let active = app.accounts.active == Some(a.uuid);
        let face = app.face_texture(a.uuid, a.skin_url.as_deref());
        egui::Frame::new().fill(if active { raised() } else { surface() }).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                widgets::face_tile(ui, rect, face.as_ref());
                ui.vertical(|ui| {
                    ui.label(RichText::new(&a.name).font(font_bold(SIZE_BODY)));
                    widgets::muted(ui, if active { "In use for new sessions" } else { "Signed in" });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, "Sign out").clicked() {
                        remove_id = Some(a.uuid);
                    }
                    if !active && widgets::secondary(ui, "Use this account").clicked() {
                        use_id = Some(a.uuid);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
    if app.accounts.active.is_some() && widgets::secondary(ui, "Use an offline name instead").clicked() {
        app.accounts.active = None;
        app.accounts.save(&app.paths.config);
    }
    if let Some(id) = use_id {
        app.accounts.active = Some(id);
        app.accounts.save(&app.paths.config);
    }
    if let Some(id) = remove_id {
        let name = app.accounts.accounts.iter().find(|a| a.uuid == id).map(|a| a.name.clone()).unwrap_or_default();
        app.accounts.remove(id);
        app.mc_sessions.remove(&id);
        app.accounts.save(&app.paths.config);
        app.toast(format!("Signed out {name}. Its saved sign-in was deleted."), false);
    }
}

fn microsoft(app: &mut App, ui: &mut egui::Ui) {
    let has_id = !app.config.client_id().trim().is_empty();
    if has_id {
        if widgets::primary(ui, "Sign in with Microsoft").clicked() {
            app.start_sign_in();
        }
        return;
    }
    egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Microsoft sign-in needs an application ID").font(font_bold(SIZE_BODY)));
        ui.add_space(4.0);
        ui.label(
            "Mojang only lets approved applications sign players in. Register a free app in Azure (personal Microsoft accounts, \
             public client flows on), request Minecraft API access for its ID, then paste the ID here.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.link("Register an app in Azure").clicked() {
                let _ = webbrowser::open("https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade");
            }
            ui.add_space(12.0);
            if ui.link("Request Minecraft API access").clicked() {
                let _ = webbrowser::open("https://aka.ms/mce-reviewappid");
            }
        });
        ui.add_space(10.0);
        let id_key = egui::Id::unique("client_id_input");
        let mut value = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<String>(id_key).clone());
        widgets::field(ui, "Application (client) ID", &mut value, "00000000-0000-0000-0000-000000000000", 380.0);
        ui.ctx().memory_mut(|m| m.data.insert_temp(id_key, value.clone()));
        ui.add_space(6.0);
        let valid = uuid::Uuid::parse_str(value.trim()).is_ok();
        if !value.trim().is_empty() && !valid {
            widgets::error_text(ui, "That doesn't look like an application ID. It has the form 8-4-4-4-12 hexadecimal digits.");
        }
        if widgets::primary_enabled(ui, "Save ID", valid).clicked() {
            app.config.auth.client_id = value.trim().to_string();
            app.save_config();
            app.toast("Saved the application ID.", false);
        }
    });
}

fn sign_in_panel(app: &mut App, ui: &mut egui::Ui) {
    let Some(flow) = &app.sign_in else { return };
    let code = flow.code.clone();
    let step = flow.step;
    let error = flow.error.clone();
    let mut cancel = false;
    let mut retry = false;
    egui::Frame::new().fill(surface()).inner_margin(egui::Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Sign in with Microsoft").font(font_bold(SIZE_LEAD)));
        ui.add_space(10.0);
        if let Some(e) = &error {
            widgets::error_text(ui, e);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::primary(ui, "Try again").clicked() {
                    retry = true;
                }
                if widgets::secondary(ui, "Close").clicked() {
                    cancel = true;
                }
            });
            return;
        }
        match (&code, step) {
            (None, _) => {
                ui.horizontal(|ui| {
                    crate::ui::widgets::busy(ui);
                    ui.label("Asking Microsoft for a sign-in code…");
                });
            }
            (Some((c, started)), None) => {
                ui.horizontal(|ui| {
                    step_number(ui, "1");
                    ui.label(format!("Open {} on any device.", c.verification_uri.trim_start_matches("https://")));
                    if widgets::secondary(ui, "Open in browser").clicked() {
                        let _ = webbrowser::open(&c.verification_uri);
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    step_number(ui, "2");
                    ui.label("Enter this code:");
                    ui.add_space(8.0);
                    ui.label(RichText::new(&c.user_code).font(font_bold(SIZE_DISPLAY)).color(accent()));
                    ui.add_space(8.0);
                    if widgets::secondary(ui, "Copy code").clicked() {
                        ui.ctx().copy_text(c.user_code.clone());
                    }
                });
                let left = c.expires_in.saturating_sub(started.elapsed().as_secs());
                ui.add_space(6.0);
                widgets::muted(ui, format!("The code expires in {}:{:02}. Waiting for you to approve…", left / 60, left % 60));
            }
            (Some(_), Some(s)) => {
                let steps = [
                    (Step::XboxLive, "Signing in to Xbox Live"),
                    (Step::Xsts, "Getting Xbox authorisation"),
                    (Step::Minecraft, "Signing in to Minecraft services"),
                    (Step::Ownership, "Checking game ownership"),
                    (Step::Profile, "Loading your profile"),
                ];
                let current = steps.iter().position(|(x, _)| *x == s).unwrap_or(0);
                for (i, (_, label)) in steps.iter().enumerate() {
                    let (color, mark) = if i < current {
                        (ink_muted(), "Done")
                    } else if i == current {
                        (ink(), "Now")
                    } else {
                        (ink_muted(), "")
                    };
                    ui.horizontal(|ui| {
                        ui.add_sized([48.0, 20.0], egui::Label::new(RichText::new(mark).size(SIZE_SMALL).color(accent())));
                        ui.label(RichText::new(*label).color(color));
                    });
                }
            }
        }
        ui.add_space(12.0);
        if widgets::secondary(ui, "Cancel").clicked() {
            cancel = true;
        }
    });
    if cancel {
        if let Some(f) = app.sign_in.take() {
            f.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    if retry {
        app.sign_in = None;
        app.start_sign_in();
    }
}

fn step_number(ui: &mut egui::Ui, n: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(12), raised());
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n, font_bold(SIZE_SMALL), ink());
}

fn offline(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Offline names").font(font_bold(SIZE_LEAD)));
    ui.add_space(4.0);
    widgets::muted(ui, "For offline-mode servers, such as a local test server. Online-mode servers refuse them. Keep several to test with different players.");
    ui.add_space(8.0);
    let current = app.config.offline_name();
    let using_offline = app.accounts.active.is_none();
    let mut choose = None;
    let mut remove = None;
    for n in app.config.all_offline_names() {
        let active = using_offline && n.eq_ignore_ascii_case(&current);
        egui::Frame::new().fill(if active { raised() } else { surface() }).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&n).font(font_bold(SIZE_BODY)));
                if active {
                    widgets::muted(ui, "In use for new sessions");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !n.eq_ignore_ascii_case(&current) && widgets::secondary(ui, "Remove").clicked() {
                        remove = Some(n.clone());
                    }
                    if !active && widgets::secondary(ui, "Use this name").clicked() {
                        choose = Some(n.clone());
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
    if let Some(n) = choose {
        // Keep the name we're switching away from in the list.
        if !app.config.player.offline_names.iter().any(|x| x.eq_ignore_ascii_case(&current)) {
            app.config.player.offline_names.push(current.clone());
        }
        app.config.player.offline_name = n;
        app.accounts.active = None;
        app.accounts.save(&app.paths.config);
        app.save_config();
    }
    if let Some(n) = remove {
        app.config.player.offline_names.retain(|x| !x.eq_ignore_ascii_case(&n));
        app.save_config();
    }
    ui.add_space(6.0);
    let id = egui::Id::unique("new_offline_name");
    let mut value = ui.ctx().memory_mut(|m| m.data.get_temp_mut_or_default::<String>(id).clone());
    let mut add = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let r = widgets::field(ui, "Add a name", &mut value, "3 to 16 letters, digits or _", 260.0);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                add = true;
            }
        });
        ui.vertical(|ui| {
            ui.add_space(24.0);
            let valid = crate::config::valid_player_name(value.trim());
            if widgets::secondary_enabled(ui, "Add name", valid).clicked() {
                add = true;
            }
            add &= valid;
        });
    });
    let trimmed = value.trim().to_string();
    if !trimmed.is_empty() && !crate::config::valid_player_name(&trimmed) {
        widgets::error_text(ui, "Use 3 to 16 letters, digits or underscores.");
    }
    if add {
        if !app.config.all_offline_names().iter().any(|x| x.eq_ignore_ascii_case(&trimmed)) {
            app.config.player.offline_names.push(trimmed);
            app.save_config();
        }
        value.clear();
    }
    ui.ctx().memory_mut(|m| m.data.insert_temp(id, value));
}
