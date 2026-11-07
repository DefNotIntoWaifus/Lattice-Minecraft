//! What the keys and buttons do. Every action can be rebound in Settings.
//!
//! Movement keys go by position on the keyboard, so W A S D stays in the same place
//! on any layout. Keys that mean a character, like `/` for a command, go by what the
//! key types instead: on many layouts `/` isn't where it is on a US keyboard.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Sneak,
    Sprint,
    Attack,
    Use,
    PickBlock,
    Drop,
    SwapHands,
    Inventory,
    Chat,
    Command,
    PlayerList,
    Perspective,
    SmoothCamera,
    Screenshot,
    Fullscreen,
    HideHud,
    Debug,
    Advancements,
    SaveToolbar,
    LoadToolbar,
    Hotbar(u8),
}

impl Action {
    /// Name used in the settings file.
    pub fn id(self) -> String {
        match self {
            Action::Hotbar(n) => format!("hotbar_{}", n + 1),
            other => match other {
                Action::Forward => "forward",
                Action::Back => "back",
                Action::Left => "left",
                Action::Right => "right",
                Action::Jump => "jump",
                Action::Sneak => "sneak",
                Action::Sprint => "sprint",
                Action::Attack => "attack",
                Action::Use => "use",
                Action::PickBlock => "pick_block",
                Action::Drop => "drop",
                Action::SwapHands => "swap_hands",
                Action::Inventory => "inventory",
                Action::Chat => "chat",
                Action::Command => "command",
                Action::PlayerList => "player_list",
                Action::Perspective => "perspective",
                Action::SmoothCamera => "smooth_camera",
                Action::Screenshot => "screenshot",
                Action::Fullscreen => "fullscreen",
                Action::HideHud => "hide_hud",
                Action::Debug => "debug",
                Action::Advancements => "advancements",
                Action::SaveToolbar => "save_toolbar",
                Action::LoadToolbar => "load_toolbar",
                Action::Hotbar(_) => unreachable!(),
            }
            .to_string(),
        }
    }

    pub fn label(self) -> String {
        match self {
            Action::Forward => "Walk forward".into(),
            Action::Back => "Walk backward".into(),
            Action::Left => "Strafe left".into(),
            Action::Right => "Strafe right".into(),
            Action::Jump => "Jump".into(),
            Action::Sneak => "Sneak".into(),
            Action::Sprint => "Sprint".into(),
            Action::Attack => "Attack or break".into(),
            Action::Use => "Use or place".into(),
            Action::PickBlock => "Pick block".into(),
            Action::Drop => "Drop item".into(),
            Action::SwapHands => "Swap hands".into(),
            Action::Inventory => "Open inventory".into(),
            Action::Chat => "Open chat".into(),
            Action::Command => "Type a command".into(),
            Action::PlayerList => "Show players".into(),
            Action::Perspective => "Change view".into(),
            Action::SmoothCamera => "Smooth camera".into(),
            Action::Screenshot => "Screenshot".into(),
            Action::Fullscreen => "Fullscreen".into(),
            Action::HideHud => "Hide interface".into(),
            Action::Debug => "Show details".into(),
            Action::Advancements => "Advancements".into(),
            Action::SaveToolbar => "Save hotbar (with a number, in creative)".into(),
            Action::LoadToolbar => "Load hotbar (with a number, in creative)".into(),
            Action::Hotbar(n) => format!("Hotbar slot {}", n + 1),
        }
    }

    /// Grouped as the settings screen shows them.
    pub fn groups() -> Vec<(&'static str, Vec<Action>)> {
        vec![
            ("Movement", vec![Action::Forward, Action::Back, Action::Left, Action::Right, Action::Jump, Action::Sneak, Action::Sprint]),
            ("Actions", vec![Action::Attack, Action::Use, Action::PickBlock, Action::Drop, Action::SwapHands]),
            ("Screens", vec![Action::Inventory, Action::Chat, Action::Command, Action::PlayerList, Action::Advancements]),
            ("View", vec![Action::Perspective, Action::SmoothCamera, Action::Screenshot, Action::Fullscreen, Action::HideHud, Action::Debug]),
            ("Hotbar", (0..9).map(Action::Hotbar).chain([Action::SaveToolbar, Action::LoadToolbar]).collect()),
        ]
    }

    pub fn all() -> Vec<Action> {
        Self::groups().into_iter().flat_map(|(_, a)| a).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bind {
    None,
    /// By position on the keyboard.
    Key(KeyCode),
    /// By the character the key types.
    Char(char),
    Mouse(u8),
}

const KEYS: &[KeyCode] = &[
    KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ,
    KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT,
    KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ, KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2,
    KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9, KeyCode::Space, KeyCode::Tab,
    KeyCode::Enter, KeyCode::Backspace, KeyCode::CapsLock, KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight,
    KeyCode::AltLeft, KeyCode::AltRight, KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::F1, KeyCode::F2,
    KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12, KeyCode::Backquote,
    KeyCode::Minus, KeyCode::Equal, KeyCode::BracketLeft, KeyCode::BracketRight, KeyCode::Backslash, KeyCode::Semicolon, KeyCode::Quote, KeyCode::Comma,
    KeyCode::Period, KeyCode::Slash, KeyCode::Insert, KeyCode::Delete, KeyCode::Home, KeyCode::End, KeyCode::PageUp, KeyCode::PageDown, KeyCode::Numpad0,
    KeyCode::Numpad1, KeyCode::Numpad2, KeyCode::Numpad3, KeyCode::Numpad4, KeyCode::Numpad5, KeyCode::Numpad6, KeyCode::Numpad7, KeyCode::Numpad8,
    KeyCode::Numpad9, KeyCode::NumpadAdd, KeyCode::NumpadSubtract, KeyCode::NumpadMultiply, KeyCode::NumpadDivide, KeyCode::NumpadDecimal,
    KeyCode::NumpadEnter, KeyCode::IntlBackslash,
];

pub fn key_by_name(name: &str) -> Option<KeyCode> {
    KEYS.iter().copied().find(|k| format!("{k:?}") == name)
}

pub fn key_label(code: KeyCode) -> String {
    let name = format!("{code:?}");
    if let Some(l) = name.strip_prefix("Key") {
        return l.to_string();
    }
    if let Some(d) = name.strip_prefix("Digit") {
        return d.to_string();
    }
    if let Some(n) = name.strip_prefix("Numpad") {
        return format!("Numpad {n}");
    }
    match code {
        KeyCode::ShiftLeft => "Left Shift".into(),
        KeyCode::ShiftRight => "Right Shift".into(),
        KeyCode::ControlLeft => "Left Ctrl".into(),
        KeyCode::ControlRight => "Right Ctrl".into(),
        KeyCode::AltLeft => "Left Alt".into(),
        KeyCode::AltRight => "Right Alt".into(),
        KeyCode::ArrowUp => "Up".into(),
        KeyCode::ArrowDown => "Down".into(),
        KeyCode::ArrowLeft => "Left".into(),
        KeyCode::ArrowRight => "Right".into(),
        KeyCode::Backquote => "`".into(),
        KeyCode::Minus => "-".into(),
        KeyCode::Equal => "=".into(),
        KeyCode::BracketLeft => "[".into(),
        KeyCode::BracketRight => "]".into(),
        KeyCode::Backslash => "\\".into(),
        KeyCode::Semicolon => ";".into(),
        KeyCode::Quote => "'".into(),
        KeyCode::Comma => ",".into(),
        KeyCode::Period => ".".into(),
        KeyCode::Slash => "/".into(),
        _ => name,
    }
}

impl Bind {
    pub fn parse(s: &str) -> Bind {
        if s == "none" {
            return Bind::None;
        }
        if let Some(n) = s.strip_prefix("mouse:") {
            return n.parse().map_or(Bind::None, Bind::Mouse);
        }
        if let Some(c) = s.strip_prefix("char:") {
            return c.chars().next().map_or(Bind::None, Bind::Char);
        }
        s.strip_prefix("key:").and_then(key_by_name).map_or(Bind::None, Bind::Key)
    }

    pub fn to_config(&self) -> String {
        match self {
            Bind::None => "none".into(),
            Bind::Key(k) => format!("key:{k:?}"),
            Bind::Char(c) => format!("char:{c}"),
            Bind::Mouse(n) => format!("mouse:{n}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Bind::None => "Not set".into(),
            Bind::Key(k) => key_label(*k),
            Bind::Char(c) => c.to_string(),
            Bind::Mouse(0) => "Left button".into(),
            Bind::Mouse(1) => "Right button".into(),
            Bind::Mouse(2) => "Middle button".into(),
            Bind::Mouse(n) => format!("Mouse button {}", n + 1),
        }
    }
}

/// The game's default keys.
pub fn default_bind(a: Action) -> Bind {
    match a {
        Action::Forward => Bind::Key(KeyCode::KeyW),
        Action::Back => Bind::Key(KeyCode::KeyS),
        Action::Left => Bind::Key(KeyCode::KeyA),
        Action::Right => Bind::Key(KeyCode::KeyD),
        Action::Jump => Bind::Key(KeyCode::Space),
        Action::Sneak => Bind::Key(KeyCode::ShiftLeft),
        Action::Sprint => Bind::Key(KeyCode::ControlLeft),
        Action::Attack => Bind::Mouse(0),
        Action::Use => Bind::Mouse(1),
        Action::PickBlock => Bind::Mouse(2),
        Action::Drop => Bind::Key(KeyCode::KeyQ),
        Action::SwapHands => Bind::Key(KeyCode::KeyF),
        Action::Inventory => Bind::Key(KeyCode::KeyE),
        Action::Chat => Bind::Key(KeyCode::KeyT),
        Action::Command => Bind::Char('/'),
        Action::PlayerList => Bind::Key(KeyCode::Tab),
        Action::Perspective => Bind::Key(KeyCode::F5),
        Action::SmoothCamera => Bind::None,
        Action::Screenshot => Bind::Key(KeyCode::F2),
        Action::Fullscreen => Bind::Key(KeyCode::F11),
        Action::HideHud => Bind::Key(KeyCode::F1),
        Action::Debug => Bind::Key(KeyCode::F3),
        Action::Advancements => Bind::Key(KeyCode::KeyL),
        Action::SaveToolbar => Bind::Key(KeyCode::KeyC),
        Action::LoadToolbar => Bind::Key(KeyCode::KeyX),
        Action::Hotbar(n) => Bind::Key(match n {
            0 => KeyCode::Digit1,
            1 => KeyCode::Digit2,
            2 => KeyCode::Digit3,
            3 => KeyCode::Digit4,
            4 => KeyCode::Digit5,
            5 => KeyCode::Digit6,
            6 => KeyCode::Digit7,
            7 => KeyCode::Digit8,
            _ => KeyCode::Digit9,
        }),
    }
}

// While the settings wait for a key, the next press goes there instead of the game.
static CAPTURE: Mutex<(Option<Action>, Option<(Action, Bind)>)> = Mutex::new((None, None));

pub fn start_capture(a: Action) {
    CAPTURE.lock().unwrap().0 = Some(a);
}

pub fn capturing() -> Option<Action> {
    CAPTURE.lock().unwrap().0
}

/// Gives the waiting setting this key or button. Escape clears the binding.
pub fn finish_capture(bind: Bind) {
    let mut c = CAPTURE.lock().unwrap();
    if let Some(a) = c.0.take() {
        c.1 = Some((a, bind));
    }
}

/// What pressing this key binds. Escape clears; everything else goes by position,
/// except keys with no fixed position in the list, which go by what they type.
pub fn bind_from_key(code: KeyCode, typed: Option<char>) -> Bind {
    if code == KeyCode::Escape {
        return Bind::None;
    }
    if KEYS.contains(&code) {
        return Bind::Key(code);
    }
    typed.map_or(Bind::None, Bind::Char)
}

pub fn bind_from_mouse(button: winit::event::MouseButton) -> Bind {
    use winit::event::MouseButton as M;
    Bind::Mouse(match button {
        M::Left => 0,
        M::Right => 1,
        M::Middle => 2,
        M::Back => 3,
        M::Forward => 4,
        M::Other(n) => n.min(32) as u8,
    })
}

pub fn take_captured() -> Option<(Action, Bind)> {
    CAPTURE.lock().unwrap().1.take()
}

/// The current bindings, looked up quickly in both directions.
#[derive(Debug, Clone)]
pub struct Binds {
    by_action: HashMap<Action, Bind>,
}

impl Binds {
    pub fn from_config(saved: &BTreeMap<String, String>) -> Binds {
        let by_action = Action::all()
            .into_iter()
            .map(|a| {
                let b = saved.get(&a.id()).map_or_else(|| default_bind(a), |s| Bind::parse(s));
                (a, b)
            })
            .collect();
        Binds { by_action }
    }

    pub fn get(&self, a: Action) -> Bind {
        self.by_action.get(&a).cloned().unwrap_or(Bind::None)
    }

    pub fn actions_for_key(&self, code: KeyCode, typed: Option<char>) -> Vec<Action> {
        self.by_action
            .iter()
            .filter(|(_, b)| match b {
                Bind::Key(k) => *k == code,
                Bind::Char(c) => typed == Some(*c),
                _ => false,
            })
            .map(|(a, _)| *a)
            .collect()
    }

    pub fn actions_for_mouse(&self, button: u8) -> Vec<Action> {
        self.by_action.iter().filter(|(_, b)| matches!(b, Bind::Mouse(n) if *n == button)).map(|(a, _)| *a).collect()
    }

    /// Other actions on the same key, for the conflict warning.
    pub fn conflicts(&self, a: Action) -> Vec<Action> {
        let bind = self.get(a);
        if bind == Bind::None {
            return Vec::new();
        }
        Action::all().into_iter().filter(|o| *o != a && self.get(*o) == bind).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_round_trip() {
        for b in [Bind::None, Bind::Key(KeyCode::KeyW), Bind::Char('/'), Bind::Mouse(3), Bind::Key(KeyCode::F5)] {
            assert_eq!(Bind::parse(&b.to_config()), b);
        }
    }

    #[test]
    fn defaults_and_overrides() {
        let mut saved = BTreeMap::new();
        saved.insert("jump".to_string(), "key:KeyJ".to_string());
        let b = Binds::from_config(&saved);
        assert_eq!(b.get(Action::Jump), Bind::Key(KeyCode::KeyJ));
        assert_eq!(b.get(Action::Forward), Bind::Key(KeyCode::KeyW));
        assert!(b.actions_for_key(KeyCode::KeyJ, None).contains(&Action::Jump));
    }

    #[test]
    fn slash_follows_the_typed_character() {
        let b = Binds::from_config(&BTreeMap::new());
        // On a layout where `/` is Shift+7, the physical key is Digit7.
        let on_seven = b.actions_for_key(KeyCode::Digit7, Some('/'));
        assert!(on_seven.contains(&Action::Command));
    }

    #[test]
    fn every_key_name_resolves() {
        for k in KEYS {
            assert_eq!(key_by_name(&format!("{k:?}")), Some(*k));
        }
    }
}
