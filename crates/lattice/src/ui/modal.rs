//! Screens the client opens itself: writing on a sign, reading and writing books.

use super::theme::*;
use super::widgets;
use crate::app::App;
use crate::game::Modal;
use egui::{CornerRadius, Key, Rect, RichText};
use lattice_proto::packets::play;
use lattice_proto::Span;

/// A book's page, in the game's own text units.
const PAGE_WIDTH: f32 = 114.0;
const PAGE_LINES: usize = 14;
const PAGE_CHARS: usize = 1024;
const MAX_PAGES: usize = 100;
const TITLE_CHARS: usize = 15;

/// What a screen wants done once it's finished drawing.
enum After {
    None,
    Save,
    Sign,
    Discard,
}

pub fn draw(app: &mut App, ui: &mut egui::Ui, screen: Rect) {
    let Some(assets) = (match &app.assets {
        crate::assets::AssetState::Ready(a) => Some(a.clone()),
        _ => None,
    }) else {
        return;
    };
    let font_data = &assets.bodies.font;
    let measure = |s: &str| font_data.width(&[Span { text: s.to_string(), ..Default::default() }]);
    let tr: &dyn lattice_proto::Translator = &assets.lang;
    let item_texture = app.item_texture(ui.ctx());
    let app_server_name = app.spec.server_name.clone();
    let Some(session) = app.session.as_mut() else { return };
    let session_advancements: Vec<crate::game::Advancement> = session.advancements.values().cloned().collect();
    let session_stats = session.stats.clone();
    let Some(modal) = session.modal.as_mut() else { return };
    let ctx = ui.ctx().clone();
    let shade = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("modal_shade")));
    shade.rect_filled(screen, CornerRadius::ZERO, widgets_backing());
    let mut after = After::None;
    let mut chosen_action = None;
    let mut dialog_values: Vec<crate::game::DialogInput> = Vec::new();
    let mut open_link: Option<String> = None;
    let mut book_click: Option<lattice_proto::text::Click> = None;
    let server_links = session.server_links.clone();
    let server_name = app_server_name.clone();
    let advancements_view: Vec<crate::game::Advancement> = if matches!(modal, Modal::Advancements { .. }) { session_advancements.clone() } else { Vec::new() };
    let stats_view = session_stats;
    let tex = item_texture;

    let width = match modal {
        Modal::Sign { .. } => 340.0,
        Modal::Advancements { .. } | Modal::Stats { .. } => (screen.width() - 120.0).clamp(420.0, 900.0),
        _ => 440.0,
    };
    egui::Area::new(egui::Id::unique("modal")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_CENTER).fixed_pos(screen.center()).show(&ctx, |ui| {
        crate::ui::chrome::window(ui, 24, |ui| {
            ui.set_width(width);
            match modal {
                // These draw themselves, in ui/block_screens.rs.
                Modal::Block(_) | Modal::GameMode { .. } | Modal::GameRules { .. } | Modal::Credits { .. } | Modal::Teleport => {}
                Modal::Link { url } => {
                    widgets::heading(ui, "Open this link?");
                    ui.add_space(6.0);
                    widgets::muted(ui, "A server sent it, so only open it if you trust where it goes.");
                    ui.add_space(10.0);
                    ui.label(RichText::new(url.as_str()).color(ink()).font(font(SIZE_BODY)));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if widgets::primary(ui, "Open it").clicked() {
                            chosen_action = Some(crate::game::DialogAction::Url(url.clone()));
                        }
                        if widgets::secondary(ui, "Copy it").clicked() {
                            ui.ctx().copy_text(url.clone());
                            after = After::Discard;
                        }
                        if widgets::secondary(ui, "Cancel").clicked() {
                            after = After::Discard;
                        }
                    });
                }
                Modal::Advancements { tab, offset } => {
                    advancements_page(ui, session_view(&advancements_view), tab, offset, width, screen, &assets, tex, tr, &mut after);
                }
                Modal::Stats { tab, search } => {
                    stats_page(ui, &stats_view, tab, search, width, screen, tr, &mut after);
                }
                Modal::Links => {
                    widgets::heading(ui, "Server links");
                    widgets::muted(ui, format!("Links {} offers. Lattice asks before opening them.", server_name));
                    ui.add_space(12.0);
                    for link in &server_links {
                        let label = match (&link.label, link.known) {
                            (Some(t), _) => t.to_plain(tr),
                            (None, Some(k)) => known_link_name(k).map_or(link.url.clone(), |key| lattice_proto::Text::translate(key, Vec::new()).to_plain(tr)),
                            (None, None) => link.url.clone(),
                        };
                        if widgets::menu_button(ui, &label, false, width).clicked() {
                            open_link = Some(link.url.clone());
                        }
                    }
                    if server_links.is_empty() {
                        ui.label("This server hasn't shared any links.");
                    }
                    ui.add_space(8.0);
                    if widgets::menu_button(ui, "Done", true, width).clicked() {
                        after = After::Discard;
                    }
                }
                Modal::Dialog { title, body, inputs, buttons, .. } => {
                    if let Some(t) = title {
                        let mut heading = t.spans(tr);
                        heading.iter_mut().for_each(|s| s.bold = true);
                        ui.label(widgets::spans_job(&heading, SIZE_TITLE, ink(), width));
                        ui.add_space(10.0);
                    }
                    for line in body.iter() {
                        ui.label(widgets::spans_job(&line.spans(tr), SIZE_BODY, ink(), width));
                        ui.add_space(6.0);
                    }
                    for input in inputs.iter_mut() {
                        dialog_input(ui, input, tr, width);
                        ui.add_space(8.0);
                    }
                    ui.add_space(8.0);
                    for b in buttons.iter() {
                        let label = b.label.to_plain(tr);
                        let r = widgets::secondary(ui, &label);
                        let r = match &b.tooltip {
                            Some(t) => r.on_hover_text(t.to_plain(tr)),
                            None => r,
                        };
                        if r.clicked() {
                            chosen_action = Some(b.action.clone());
                            dialog_values = inputs.clone();
                        }
                        ui.add_space(4.0);
                    }
                }
                Modal::Sign { front, lines, limit, focus, .. } => {
                    widgets::heading(ui, if *front { "Front of the sign" } else { "Back of the sign" });
                    ui.add_space(14.0);
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    let down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                    let up = ui.input(|i| i.key_pressed(Key::ArrowUp));
                    let mut go_to = None;
                    for (i, line) in lines.iter_mut().enumerate() {
                        let before = line.clone();
                        let r = ui.add(egui::TextEdit::singleline(line).desired_width(width).margin(egui::vec2(10.0, 7.0)).font(font(SIZE_BODY)).align(egui::Align2::CENTER_CENTER));
                        crate::dev::note_widget(&format!("sign line {}", i + 1), r.rect);
                        if measure(line) > *limit {
                            *line = before;
                        }
                        if *focus == Some(i) {
                            r.request_focus();
                            *focus = None;
                        }
                        let active = r.has_focus() || r.lost_focus();
                        if active && (enter || down) {
                            go_to = Some((i + 1) % 4);
                        } else if active && up {
                            go_to = Some((i + 3) % 4);
                        }
                        ui.add_space(6.0);
                    }
                    if go_to.is_some() {
                        *focus = go_to;
                    }
                    ui.add_space(8.0);
                    if widgets::primary(ui, "Done").clicked() {
                        after = After::Save;
                    }
                }
                Modal::Book { title, author, pages, page } => {
                    if let Some(title) = title {
                        widgets::heading(ui, title.clone());
                    }
                    if let Some(author) = author {
                        widgets::muted(ui, format!("by {author}"));
                    }
                    ui.add_space(12.0);
                    let total = pages.len().max(1);
                    *page = (*page).min(total - 1);
                    let spans = pages.get(*page).map(|t| t.spans(tr)).unwrap_or_default();
                    egui::Frame::new().fill(ground()).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                        ui.set_min_height(300.0);
                        ui.set_width(width - 28.0);
                        // Words in a book can be pressed: they turn pages, run commands and open links.
                        let galley = ui.fonts_mut(|f| f.layout_job(widgets::spans_job(&spans, SIZE_BODY, ink(), width - 28.0)));
                        let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
                        crate::dev::note_widget("book page", rect);
                        let at = resp.interact_pointer_pos().filter(|_| resp.clicked());
                        let under = resp.hover_pos().or(at).map(|q| galley.cursor_from_pos(q - rect.min).index.0);
                        let span = under.and_then(|i| {
                            let mut passed = 0;
                            spans.iter().find(|s| {
                                passed += s.text.chars().count();
                                i < passed
                            })
                        });
                        if span.is_some_and(|s| s.click.is_some()) {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            book_click = span.and_then(|s| s.click.clone());
                        }
                        ui.painter().galley(rect.min, galley, ink());
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if widgets::secondary(ui, "Previous").clicked() && *page > 0 {
                            *page -= 1;
                        }
                        ui.label(RichText::new(format!("Page {} of {total}", *page + 1)).color(ink_muted()).font(font(SIZE_SMALL)));
                        if widgets::secondary(ui, "Next").clicked() && *page + 1 < total {
                            *page += 1;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::primary(ui, "Done").clicked() {
                                after = After::Discard;
                            }
                        });
                    });
                }
                Modal::WriteBook { pages, page, title, signing, .. } => {
                    if *signing {
                        widgets::heading(ui, "Sign this book");
                        widgets::muted(ui, "Once signed, nobody can change it, and that includes you.");
                        ui.add_space(12.0);
                        let r = widgets::field(ui, "Title", title, "", width);
                        crate::dev::note_widget("book title", r.rect);
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if title.chars().count() > TITLE_CHARS {
                            *title = title.chars().take(TITLE_CHARS).collect();
                        }
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            let ok = !title.trim().is_empty();
                            if widgets::primary_enabled(ui, "Sign and close", ok).clicked() && ok {
                                after = After::Sign;
                            }
                            if widgets::secondary(ui, "Back to writing").clicked() {
                                *signing = false;
                            }
                        });
                    } else {
                        if pages.is_empty() {
                            pages.push(String::new());
                        }
                        *page = (*page).min(pages.len() - 1);
                        widgets::heading(ui, "Writing");
                        ui.add_space(10.0);
                        let text = &mut pages[*page];
                        let before = text.clone();
                        let r = ui.add(egui::TextEdit::multiline(text).desired_width(width).desired_rows(PAGE_LINES).margin(egui::vec2(12.0, 10.0)).font(font(SIZE_BODY)));
                        crate::dev::note_widget("book page", r.rect);
                        if ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if text.chars().count() > PAGE_CHARS || page_lines(text, &measure) > PAGE_LINES {
                            *text = before;
                        }
                        ui.add_space(6.0);
                        widgets::muted(ui, format!("{} of {PAGE_LINES} lines, {} of {PAGE_CHARS} letters", page_lines(text, &measure), text.chars().count()));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if widgets::secondary(ui, "Previous").clicked() && *page > 0 {
                                *page -= 1;
                            }
                            ui.label(RichText::new(format!("Page {} of {}", *page + 1, pages.len())).color(ink_muted()).font(font(SIZE_SMALL)));
                            if widgets::secondary(ui, "Next").clicked() {
                                if *page + 1 == pages.len() && pages.len() < MAX_PAGES {
                                    pages.push(String::new());
                                }
                                if *page + 1 < pages.len() {
                                    *page += 1;
                                }
                            }
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if widgets::primary(ui, "Done").clicked() {
                                after = After::Save;
                            }
                            let any = pages.iter().any(|p| !p.trim().is_empty());
                            if widgets::primary_enabled(ui, "Sign", any).clicked() && any {
                                *signing = true;
                            }
                            if widgets::secondary(ui, "Discard changes").clicked() {
                                after = After::Discard;
                            }
                        });
                    }
                }
            }
        });
    });
    if let Some(url) = open_link {
        if let Some(s) = app.session.as_mut() {
            s.modal = Some(Modal::Link { url });
        }
        return;
    }
    if let Some(click) = book_click {
        use lattice_proto::text::Click;
        match click {
            Click::ChangePage(page) => {
                if let Some(Some(Modal::Book { page: now, .. })) = app.session.as_mut().map(|s| s.modal.as_mut()) {
                    *now = (page.max(1) - 1) as usize;
                }
            }
            Click::OpenUrl(url) => {
                if let (Some(s), true) = (app.session.as_mut(), url.starts_with("http://") || url.starts_with("https://")) {
                    s.modal = Some(Modal::Link { url });
                }
            }
            Click::RunCommand(c) => run_dialog_action(app, ui, crate::game::DialogAction::Command(c), &[]),
            Click::SuggestCommand(c) => run_dialog_action(app, ui, crate::game::DialogAction::Suggest(c), &[]),
            Click::Copy(c) => ui.ctx().copy_text(c),
        }
        return;
    }
    if let Some(action) = chosen_action {
        run_dialog_action(app, ui, action, &dialog_values);
        return;
    }
    match after {
        After::None => {}
        After::Save => app.close_modal(true),
        After::Discard => app.close_modal(false),
        After::Sign => {
            if let Some(s) = app.session.as_mut() {
                if let Some(Modal::WriteBook { slot, pages, title, .. }) = s.modal.as_mut() {
                    trim_pages(pages);
                    s.conn.send(play::edit_book(*slot, pages, Some(title.trim())));
                }
            }
            app.close_modal(false);
        }
    }
}

/// The game's translation key for a kind of server link.
fn known_link_name(kind: i32) -> Option<&'static str> {
    ["known_server_link.report_bug", "known_server_link.community_guidelines", "known_server_link.support", "known_server_link.status", "known_server_link.feedback", "known_server_link.community", "known_server_link.website", "known_server_link.forums", "known_server_link.news", "known_server_link.announcements"]
        .get(kind as usize)
        .copied()
}

/// One value a server's dialog asks for.
fn dialog_input(ui: &mut egui::Ui, input: &mut crate::game::DialogInput, tr: &dyn lattice_proto::Translator, width: f32) {
    use crate::game::InputControl as C;
    let label = input.label.to_plain(tr);
    match &mut input.control {
        C::Toggle { value, .. } => {
            widgets::switch_line(ui, value, &label);
        }
        control => {
            if input.label_visible {
                ui.label(RichText::new(&label).font(font(SIZE_SMALL)).color(ink()));
            }
            match control {
                C::Text { value, max, multiline } => {
                    let edit = if *multiline { egui::TextEdit::multiline(value).desired_rows(4) } else { egui::TextEdit::singleline(value) };
                    let r = ui.add(edit.char_limit(*max).desired_width(width).margin(egui::vec2(10.0, 8.0)).font(font(SIZE_BODY)));
                    crate::dev::note_widget(&format!("input:{}", input.key), r.rect);
                    widgets::focus_ring(ui, &r);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &label));
                }
                C::Options { entries, index } => {
                    let current = entries.get(*index).map_or(String::new(), |e| e.1.to_plain(tr));
                    egui::ComboBox::from_id_salt(&input.key).selected_text(current).width(width - 8.0).show_ui(ui, |ui| {
                        for (i, e) in entries.iter().enumerate() {
                            ui.selectable_value(index, i, e.1.to_plain(tr));
                        }
                    });
                }
                C::Number { value, start, end, step } => {
                    ui.spacing_mut().slider_width = (width - 96.0).max(100.0);
                    widgets::slider_look(ui);
                    let mut slider = egui::Slider::new(value, *start..=*end);
                    if let Some(s) = step {
                        let decimals = if *s >= 1.0 { 0 } else if *s >= 0.1 { 1 } else { 2 };
                        slider = slider.step_by(*s as f64).fixed_decimals(decimals);
                    }
                    let r = ui.add(slider);
                    crate::dev::note_widget(&format!("input:{}", input.key), r.rect);
                    widgets::focus_ring(ui, &r);
                }
                C::Toggle { .. } => {}
            }
        }
    }
}

fn run_dialog_action(app: &mut App, ui: &egui::Ui, action: crate::game::DialogAction, values: &[crate::game::DialogInput]) {
    use crate::game::DialogAction as A;
    match action {
        A::Close => {}
        A::Command(c) => {
            if let Some(s) = app.session.as_mut() {
                // What a server's own button sends is the server's business, and nothing it
                // can't already do. It's still sent as the player's own command.
                let line = if c.starts_with('/') { c } else { format!("/{c}") };
                if let Err(e) = s.send_chat(&line) {
                    tracing::warn!("couldn't send a command from a server's screen: {e}");
                }
            }
        }
        A::Suggest(c) => {
            app.close_modal(false);
            app.open_chat_with(c);
            return;
        }
        A::Url(url) => {
            // Only ever a web link, and the browser's the one to open it.
            if url.starts_with("http://") || url.starts_with("https://") {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        A::Custom(id, payload) => {
            if let Some(s) = app.session.as_mut() {
                s.conn.send(play::custom_click_action(&id, payload.as_ref()));
            }
        }
        A::Template(template) => {
            let line = crate::game::fill_template(&template, values);
            return run_dialog_action(app, ui, A::Command(line), &[]);
        }
        A::CustomAll(id, additions) => {
            use lattice_proto::Nbt;
            let mut all = match additions {
                Some(Nbt::Compound(list)) => list,
                _ => Vec::new(),
            };
            for v in values {
                all.push((v.key.clone(), v.control.as_nbt()));
            }
            if let Some(s) = app.session.as_mut() {
                s.conn.send(play::custom_click_action(&id, Some(&Nbt::Compound(all))));
            }
        }
        A::Copy(text) => ui.ctx().copy_text(text),
        A::Show(dialog) => {
            if let (Some(s), Some(next)) = (app.session.as_mut(), crate::game::parse_dialog(&dialog)) {
                s.modal = Some(next);
            }
            return;
        }
    }
    app.close_modal(false);
}

/// Opens the tree of advancements on the first page that has anything to show.
pub fn open_advancements(session: &mut crate::game::Session) {
    let tab = root_tabs_of(session.advancements.values()).into_iter().next().unwrap_or_default();
    if !tab.is_empty() {
        session.conn.send(play::seen_advancements(Some(&tab)));
    }
    session.modal = Some(Modal::Advancements { tab, offset: (0.0, 0.0) });
    session.screen_opened = true;
}

/// Opens the statistics, asking the server for the latest.
pub fn open_links(session: &mut crate::game::Session) {
    session.modal = Some(Modal::Links);
    session.screen_opened = true;
}

pub fn open_stats(session: &mut crate::game::Session) {
    session.conn.send(play::client_command(1));
    session.modal = Some(Modal::Stats { tab: 0, search: String::new() });
    session.screen_opened = true;
}

fn root_tabs_of<'a>(list: impl Iterator<Item = &'a crate::game::Advancement>) -> Vec<String> {
    let mut roots: Vec<String> = list.filter(|a| a.entry.parent.is_none() && a.entry.display.is_some()).map(|a| a.entry.id.clone()).collect();
    roots.sort();
    roots
}

fn session_view(list: &[crate::game::Advancement]) -> &[crate::game::Advancement] {
    list
}

/// Which root an advancement's chain leads up to.
fn root_of<'a>(id: &'a str, by_id: &'a std::collections::HashMap<&str, &crate::game::Advancement>) -> &'a str {
    let mut at = id;
    for _ in 0..64 {
        match by_id.get(at).and_then(|a| a.entry.parent.as_deref()) {
            Some(p) => at = p,
            None => break,
        }
    }
    at
}

#[allow(clippy::too_many_arguments)]
fn advancements_page(ui: &mut egui::Ui, list: &[crate::game::Advancement], tab: &mut String, offset: &mut (f32, f32), width: f32, screen: Rect, assets: &crate::assets::GameAssets, tex: egui::TextureId, tr: &dyn lattice_proto::Translator, after: &mut After) {
    use egui::{Pos2, Sense, Stroke, Vec2};
    let by_id: std::collections::HashMap<&str, &crate::game::Advancement> = list.iter().map(|a| (a.entry.id.as_str(), a)).collect();
    let roots = root_tabs_of(list.iter());
    widgets::heading(ui, "Advancements");
    ui.add_space(8.0);
    if roots.is_empty() {
        widgets::muted(ui, "The server hasn't sent any.");
        ui.add_space(10.0);
        if widgets::primary(ui, "Done").clicked() {
            *after = After::Discard;
        }
        return;
    }
    if !roots.contains(tab) {
        *tab = roots[0].clone();
    }
    ui.horizontal_wrapped(|ui| {
        for r in &roots {
            let Some(a) = by_id.get(r.as_str()) else { continue };
            let title = a.entry.display.as_ref().map_or(r.clone(), |d| d.title.to_plain(tr));
            let on = r == tab;
            let text = RichText::new(&title).color(if on { ink() } else { ink_muted() }).font(font(SIZE_BODY));
            let l = ui.add(egui::Label::new(text).sense(Sense::click()));
            crate::dev::note_widget(&format!("tab:{title}"), l.rect);
            if l.clicked() {
                *tab = r.clone();
                *offset = (0.0, 0.0);
            }
            if on {
                ui.painter().line_segment([l.rect.left_bottom() + Vec2::new(0.0, 2.0), l.rect.right_bottom() + Vec2::new(0.0, 2.0)], Stroke::new(2.0, accent()));
            }
            ui.add_space(10.0);
        }
    });
    ui.add_space(8.0);
    let height = (screen.height() - 290.0).clamp(200.0, 600.0);
    let (area, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::drag());
    let p = ui.painter().with_clip_rect(area);
    p.rect_filled(area, CornerRadius::ZERO, ground());
    const STEP: f32 = 56.0;
    const ICON: f32 = 40.0;
    let shown: Vec<&crate::game::Advancement> = list.iter().filter(|a| a.entry.display.is_some() && root_of(&a.entry.id, &by_id) == tab.as_str()).collect();
    let (mut max_x, mut max_y) = (0.0f32, 0.0f32);
    for a in &shown {
        max_x = max_x.max(a.entry.x * STEP + ICON);
        max_y = max_y.max(a.entry.y * STEP + ICON);
    }
    // Drag to look around, within the page.
    if resp.dragged() {
        offset.0 += resp.drag_delta().x;
        offset.1 += resp.drag_delta().y;
    }
    offset.0 = offset.0.clamp((area.width() - max_x - 32.0).min(0.0), 0.0);
    offset.1 = offset.1.clamp((area.height() - max_y - 32.0).min(0.0), 0.0);
    let at = |a: &crate::game::Advancement| area.min + Vec2::new(16.0 + a.entry.x * STEP + offset.0, 16.0 + a.entry.y * STEP + offset.1);
    // Lines first, so the icons sit on top: out of the parent, along, and into the child.
    for a in &shown {
        let Some(parent) = a.entry.parent.as_deref().and_then(|x| by_id.get(x)).filter(|x| x.entry.display.is_some()) else { continue };
        let (from, to) = (at(parent) + Vec2::splat(ICON / 2.0), at(a) + Vec2::splat(ICON / 2.0));
        let mid = (from.x + to.x) / 2.0;
        let color = if a.complete() { accent() } else { edge() };
        for (u, v) in [(from, Pos2::new(mid, from.y)), (Pos2::new(mid, from.y), Pos2::new(mid, to.y)), (Pos2::new(mid, to.y), to)] {
            p.line_segment([u, v], Stroke::new(2.0, color));
        }
    }
    let pointer = ui.ctx().pointer_latest_pos().filter(|q| area.contains(*q));
    let mut tip = None;
    for a in &shown {
        let Some(d) = &a.entry.display else { continue };
        let r = Rect::from_min_size(at(a), Vec2::splat(ICON));
        let done = a.complete();
        let fill = if done { accent_dim() } else { raised() };
        // A challenge is a little wider than a task, a goal rounder.
        let corners = match d.kind {
            2 => CornerRadius::same(14),
            1 => CornerRadius::same(2),
            _ => CornerRadius::ZERO,
        };
        p.rect_filled(r, corners, fill);
        let edge = if d.kind == 1 { warn() } else if done { accent() } else { edge() };
        p.rect_stroke(r, corners, Stroke::new(if d.kind == 1 { 2.0 } else { 1.0 }, edge), egui::StrokeKind::Inside);
        super::items::draw_stack(&p, r.shrink(5.0), tex, assets, &d.icon, 0.0);
        crate::dev::note_widget(&format!("advancement:{}", a.entry.id), r);
        if pointer.is_some_and(|q| r.contains(q)) {
            let (got, of) = a.progress();
            let mut lines = vec![(d.title.to_plain(tr), ink()), (d.description.to_plain(tr), ink_muted())];
            if of > 1 && !done {
                lines.push((format!("{got} of {of} done"), ink_muted()));
            }
            if done {
                lines.push(("Done".to_string(), accent()));
            }
            tip = Some(lines);
        }
    }
    if let (Some(lines), Some(q)) = (tip, pointer) {
        let tip_painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("advancement_tip")));
        super::items::tooltip(&tip_painter, q, screen, &lines);
    }
    ui.add_space(10.0);
    let (total, complete) = (shown.len(), shown.iter().filter(|a| a.complete()).count());
    ui.horizontal(|ui| {
        if widgets::primary(ui, "Done").clicked() {
            *after = After::Discard;
        }
        ui.add_space(8.0);
        widgets::muted(ui, format!("{complete} of {total} on this page"));
    });
}

/// A statistic's number the way it's meant to be read.
fn format_stat(name: &str, value: i32) -> String {
    if name.ends_with("_one_cm") {
        let m = value as f64 / 100.0;
        return if m >= 1000.0 { format!("{:.2} km", m / 1000.0) } else { format!("{m:.1} m") };
    }
    if name.starts_with("damage_") {
        return format!("{:.1}", value as f64 / 10.0);
    }
    if name.ends_with("_time") || name.starts_with("time_since") {
        let s = value as f64 / 20.0;
        return if s >= 3600.0 {
            format!("{:.1} h", s / 3600.0)
        } else if s >= 60.0 {
            format!("{:.1} min", s / 60.0)
        } else {
            format!("{s:.0} s")
        };
    }
    value.to_string()
}

#[allow(clippy::too_many_arguments)]
fn stats_page(ui: &mut egui::Ui, stats: &std::collections::HashMap<(i32, i32), i32>, tab: &mut usize, search: &mut String, width: f32, screen: Rect, tr: &dyn lattice_proto::Translator, after: &mut After) {
    let game = lattice_data::game();
    let kinds = game.registry("stat_type");
    widgets::heading(ui, "Statistics");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        for (i, label) in ["General", "Items and blocks", "Mobs"].iter().enumerate() {
            let on = *tab == i;
            let l = ui.add(egui::Label::new(RichText::new(*label).color(if on { ink() } else { ink_muted() }).font(font(SIZE_BODY))).sense(egui::Sense::click()));
            crate::dev::note_widget(&format!("tab:{label}"), l.rect);
            if l.clicked() {
                *tab = i;
            }
            if on {
                ui.painter().line_segment([l.rect.left_bottom() + egui::vec2(0.0, 2.0), l.rect.right_bottom() + egui::vec2(0.0, 2.0)], egui::Stroke::new(2.0, accent()));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(8.0);
    ui.add(egui::TextEdit::singleline(search).hint_text(RichText::new("Search").color(ink_muted())).desired_width(240.0).font(font(SIZE_BODY)));
    ui.add_space(6.0);
    let needle = search.to_lowercase();
    let height = (screen.height() - 340.0).clamp(160.0, 600.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(width - 16.0);
        let kind_of = |i: i32| kinds.get(i as usize).map_or("", |s| s.as_str());
        let row = |ui: &mut egui::Ui, name: String, value: String| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).color(ink()).font(font(SIZE_BODY)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(value).color(ink_muted()).font(font(SIZE_BODY)));
                });
            });
        };
        match *tab {
            0 => {
                let names = game.registry("custom_stat");
                let mut rows: Vec<(String, String)> = stats
                    .iter()
                    .filter(|((k, _), _)| kind_of(*k) == "custom")
                    .filter_map(|((_, id), v)| {
                        let n = names.get(*id as usize)?;
                        let key = format!("stat.minecraft.{n}");
                        Some((tr.translate(&key).map_or(n.clone(), str::to_string), format_stat(n, *v)))
                    })
                    .filter(|(n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
                    .collect();
                rows.sort();
                if rows.is_empty() {
                    widgets::muted(ui, "Nothing yet.");
                }
                for (n, v) in rows {
                    row(ui, n, v);
                }
            }
            1 => {
                // Mined blocks count by block, the rest by item, so they're shown by name.
                let items = game.registry("item");
                let blocks = game.registry("block");
                let mut by_name: std::collections::BTreeMap<String, Vec<(&str, i32)>> = Default::default();
                for ((k, id), v) in stats {
                    let (kind, name) = match kind_of(*k) {
                        "mined" => ("mined", blocks.get(*id as usize)),
                        "crafted" => ("made", items.get(*id as usize)),
                        "used" => ("used", items.get(*id as usize)),
                        "broken" => ("worn out", items.get(*id as usize)),
                        "picked_up" => ("picked up", items.get(*id as usize)),
                        "dropped" => ("dropped", items.get(*id as usize)),
                        _ => continue,
                    };
                    let Some(n) = name else { continue };
                    let shown = tr.translate(&format!("item.minecraft.{n}")).or(tr.translate(&format!("block.minecraft.{n}"))).map_or(n.clone(), str::to_string);
                    by_name.entry(shown).or_default().push((kind, *v));
                }
                let mut any = false;
                for (n, mut list) in by_name {
                    if !needle.is_empty() && !n.to_lowercase().contains(&needle) {
                        continue;
                    }
                    any = true;
                    list.sort();
                    let text = list.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ");
                    row(ui, n, text);
                }
                if !any {
                    widgets::muted(ui, "Nothing yet.");
                }
            }
            _ => {
                let mobs = game.registry("entity_type");
                let mut by_name: std::collections::BTreeMap<String, (i32, i32)> = Default::default();
                for ((k, id), v) in stats {
                    let Some(n) = mobs.get(*id as usize) else { continue };
                    let shown = tr.translate(&format!("entity.minecraft.{n}")).map_or(n.clone(), str::to_string);
                    match kind_of(*k) {
                        "killed" => by_name.entry(shown).or_default().0 = *v,
                        "killed_by" => by_name.entry(shown).or_default().1 = *v,
                        _ => {}
                    }
                }
                let mut any = false;
                for (n, (killed, by)) in by_name {
                    if !needle.is_empty() && !n.to_lowercase().contains(&needle) {
                        continue;
                    }
                    any = true;
                    row(ui, n, format!("killed {killed}, killed you {by}"));
                }
                if !any {
                    widgets::muted(ui, "Nothing yet.");
                }
            }
        }
    });
    ui.add_space(10.0);
    if widgets::primary(ui, "Done").clicked() {
        *after = After::Discard;
    }
}

fn widgets_backing() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(ground().r(), ground().g(), ground().b(), 170)
}

/// Empty pages at the end aren't kept.
pub fn trim_pages(pages: &mut Vec<String>) {
    while pages.last().is_some_and(|p| p.is_empty()) {
        pages.pop();
    }
}

/// How many lines a page takes in the game's own text, which is what readers will see.
pub fn page_lines(text: &str, measure: &dyn Fn(&str) -> f32) -> usize {
    let mut lines = 0;
    for paragraph in text.split('\n') {
        lines += 1;
        let mut used = 0.0;
        let mut word = String::new();
        let flush = |word: &mut String, used: &mut f32, lines: &mut usize| {
            let w = measure(word);
            if *used + w > PAGE_WIDTH && *used > 0.0 {
                *lines += 1;
                *used = 0.0;
            }
            let mut w = w;
            // A word longer than the page runs on to the next line.
            while w > PAGE_WIDTH {
                *lines += 1;
                w -= PAGE_WIDTH;
            }
            *used += w;
            word.clear();
        };
        for c in paragraph.chars() {
            word.push(c);
            if c == ' ' {
                flush(&mut word, &mut used, &mut lines);
            }
        }
        flush(&mut word, &mut used, &mut lines);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines_like_the_page_does() {
        // Every letter is 6 wide, so 19 fit on a line.
        let m = |s: &str| s.chars().count() as f32 * 6.0;
        assert_eq!(page_lines("", &m), 1);
        assert_eq!(page_lines("one\ntwo", &m), 2);
        assert_eq!(page_lines(&"a".repeat(19), &m), 1);
        assert_eq!(page_lines(&"a".repeat(20), &m), 2);
        // The second word doesn't fit after the first and moves down whole.
        assert_eq!(page_lines(&format!("{} {}", "a".repeat(15), "b".repeat(15)), &m), 2);
    }

    #[test]
    fn drops_only_the_empty_pages_at_the_end() {
        let mut p = vec!["a".to_string(), String::new(), "b".to_string(), String::new(), String::new()];
        trim_pages(&mut p);
        assert_eq!(p, vec!["a", "", "b"]);
    }
}
