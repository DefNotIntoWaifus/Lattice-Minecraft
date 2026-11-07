//! Screen reader support, the same for the launcher and game windows. egui builds
//! the accessibility tree; this passes requests from assistive software to it.

use egui_winit::accesskit_winit;

/// What the windowing loop hears from outside it.
pub enum UserEvent {
    Accessibility(accesskit_winit::Event),
    /// Someone launched the app again while the launcher was open.
    Raise,
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        UserEvent::Accessibility(e)
    }
}

pub fn handle(state: &mut egui_winit::State, ctx: &egui::Context, event: UserEvent) {
    let UserEvent::Accessibility(event) = event else { return };
    match event.window_event {
        accesskit_winit::WindowEvent::InitialTreeRequested => {
            ctx.enable_accesskit();
            ctx.request_repaint();
        }
        accesskit_winit::WindowEvent::ActionRequested(request) => {
            state.on_accesskit_action_request(request);
            ctx.request_repaint();
        }
        accesskit_winit::WindowEvent::AccessibilityDeactivated => ctx.disable_accesskit(),
    }
}

/// Development aid: writes what a screen reader would be told, one line per
/// thing it can name, so labels can be checked without one.
pub fn dump(update: &egui::accesskit::TreeUpdate, path: &std::path::Path) {
    let mut out = String::new();
    for (_, node) in &update.nodes {
        if let Some(label) = node.label() {
            out.push_str(&format!("{:?}: {label}", node.role()));
            if let Some(v) = node.value() {
                out.push_str(&format!(" = {v}"));
            }
            if let Some(n) = node.numeric_value() {
                out.push_str(&format!(" = {n}"));
            }
            out.push('\n');
        }
    }
    let _ = std::fs::write(path, out);
}
