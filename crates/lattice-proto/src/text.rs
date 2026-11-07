//! Text components (chat, titles, disconnect reasons, MOTDs).
//!
//! Received as NBT in configuration/play and as JSON in status/login. Both are
//! parsed into the same tree, then flattened into styled [`Span`]s for drawing.
//! Legacy `§` formatting codes inside literal text are honoured because many
//! servers still send them.

use crate::nbt::Nbt;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    /// 0xRRGGBB
    pub color: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    /// Target of an `open_url` click event, if any.
    pub link: Option<String>,
    /// What clicking it does, whatever that is.
    pub click: Option<Click>,
    /// What's shown when the pointer's over it.
    pub hover: Option<Box<Text>>,
}

impl Style {
    fn inherit(&self, parent: &Style) -> Style {
        Style {
            color: self.color.or(parent.color),
            bold: self.bold.or(parent.bold),
            italic: self.italic.or(parent.italic),
            underlined: self.underlined.or(parent.underlined),
            strikethrough: self.strikethrough.or(parent.strikethrough),
            obfuscated: self.obfuscated.or(parent.obfuscated),
            link: self.link.clone().or_else(|| parent.link.clone()),
            click: self.click.clone().or_else(|| parent.click.clone()),
            hover: self.hover.clone().or_else(|| parent.hover.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Literal(String),
    Translate { key: String, fallback: Option<String>, with: Vec<Text> },
    Keybind(String),
    /// Score, selector, NBT and object components need world context we don't
    /// resolve on the client; servers normally resolve them before sending.
    Unresolved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub content: Content,
    pub style: Style,
    pub extra: Vec<Text>,
}

/// What clicking a bit of text asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    OpenUrl(String),
    RunCommand(String),
    SuggestCommand(String),
    Copy(String),
    ChangePage(i32),
}

impl Click {
    fn from_nbt(c: &Nbt) -> Option<Click> {
        let text = |key: &str| c.get(key).and_then(Nbt::as_str).map(str::to_owned);
        Some(match c.get("action").and_then(Nbt::as_str)? {
            "open_url" => Click::OpenUrl(text("url")?),
            "run_command" => Click::RunCommand(text("command")?),
            "suggest_command" => Click::SuggestCommand(text("command")?),
            "copy_to_clipboard" => Click::Copy(text("value")?),
            "change_page" => Click::ChangePage(c.get("page").and_then(Nbt::as_i64)? as i32),
            _ => return None,
        })
    }

    fn from_json(c: &Value) -> Option<Click> {
        let text = |keys: &[&str]| keys.iter().find_map(|k| c.get(*k).and_then(Value::as_str)).map(str::to_owned);
        Some(match c.get("action").and_then(Value::as_str)? {
            "open_url" => Click::OpenUrl(text(&["url", "value"])?),
            "run_command" => Click::RunCommand(text(&["command", "value"])?),
            "suggest_command" => Click::SuggestCommand(text(&["command", "value"])?),
            "copy_to_clipboard" => Click::Copy(text(&["value"])?),
            "change_page" => Click::ChangePage(c.get("page").and_then(Value::as_i64).or(text(&["value"]).and_then(|v| v.parse().ok()))? as i32),
            _ => return None,
        })
    }
}

/// A run of text with one style, ready to draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub color: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
    pub link: Option<String>,
    pub click: Option<Click>,
    pub hover: Option<Box<Text>>,
}

/// Looks up translation keys (e.g. from the game's `en_us.json`).
pub trait Translator {
    fn translate(&self, key: &str) -> Option<&str>;
}

/// A translator that knows nothing; keys fall back to `fallback` or themselves.
pub struct NoTranslations;
impl Translator for NoTranslations {
    fn translate(&self, _: &str) -> Option<&str> {
        None
    }
}

pub const NAMED_COLORS: [(&str, u32); 16] = [
    ("black", 0x000000),
    ("dark_blue", 0x0000AA),
    ("dark_green", 0x00AA00),
    ("dark_aqua", 0x00AAAA),
    ("dark_red", 0xAA0000),
    ("dark_purple", 0xAA00AA),
    ("gold", 0xFFAA00),
    ("gray", 0xAAAAAA),
    ("dark_gray", 0x555555),
    ("blue", 0x5555FF),
    ("green", 0x55FF55),
    ("aqua", 0x55FFFF),
    ("red", 0xFF5555),
    ("light_purple", 0xFF55FF),
    ("yellow", 0xFFFF55),
    ("white", 0xFFFFFF),
];

fn parse_color(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix('#') {
        return u32::from_str_radix(hex, 16).ok().map(|c| c & 0xFFFFFF);
    }
    NAMED_COLORS.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

impl Text {
    pub fn literal(s: impl Into<String>) -> Text {
        Text { content: Content::Literal(s.into()), style: Style::default(), extra: Vec::new() }
    }

    pub fn translate(key: impl Into<String>, with: Vec<Text>) -> Text {
        Text {
            content: Content::Translate { key: key.into(), fallback: None, with },
            style: Style::default(),
            extra: Vec::new(),
        }
    }

    // ---------- NBT ----------

    pub fn from_nbt(tag: &Nbt) -> Text {
        match tag {
            Nbt::String(s) => Text::literal(s.clone()),
            Nbt::List(items) => list_to_text(items.iter().map(Text::from_nbt).collect()),
            Nbt::Compound(entries) => {
                // Heterogeneous lists wrap each element as {"": value}.
                if entries.len() == 1 && entries[0].0.is_empty() {
                    return Text::from_nbt(&entries[0].1);
                }
                let s = |k: &str| tag.get(k).and_then(Nbt::as_str).map(str::to_owned);
                let b = |k: &str| tag.get(k).and_then(Nbt::as_bool);
                let content = if let Some(t) = s("text") {
                    Content::Literal(t)
                } else if let Some(key) = s("translate") {
                    let with = tag
                        .get("with")
                        .and_then(Nbt::as_list)
                        .map(|l| l.iter().map(arg_from_nbt).collect())
                        .unwrap_or_default();
                    Content::Translate { key, fallback: s("fallback"), with }
                } else if let Some(k) = s("keybind") {
                    Content::Keybind(k)
                } else if tag.get("score").is_some() || tag.get("selector").is_some() || tag.get("nbt").is_some() {
                    Content::Unresolved
                } else {
                    Content::Literal(String::new())
                };
                let link = tag.get("click_event").and_then(|c| {
                    (c.get("action").and_then(Nbt::as_str) == Some("open_url"))
                        .then(|| c.get("url").and_then(Nbt::as_str).map(str::to_owned))
                        .flatten()
                });
                let click = tag.get("click_event").and_then(Click::from_nbt);
                let hover = tag.get("hover_event").filter(|h| h.get("action").and_then(Nbt::as_str) == Some("show_text")).and_then(|h| h.get("value")).map(|v| Box::new(Text::from_nbt(v)));
                let style = Style {
                    color: s("color").as_deref().and_then(parse_color),
                    bold: b("bold"),
                    italic: b("italic"),
                    underlined: b("underlined"),
                    strikethrough: b("strikethrough"),
                    obfuscated: b("obfuscated"),
                    link,
                    click,
                    hover,
                };
                let extra = tag
                    .get("extra")
                    .and_then(Nbt::as_list)
                    .map(|l| l.iter().map(Text::from_nbt).collect())
                    .unwrap_or_default();
                Text { content, style, extra }
            }
            other => match other.as_f64() {
                Some(n) => Text::literal(format_number(n, other)),
                None => Text::literal(String::new()),
            },
        }
    }

    // ---------- JSON ----------

    pub fn from_json_str(s: &str) -> Result<Text, String> {
        match serde_json::from_str::<Value>(s) {
            Ok(v) => Ok(Text::from_json(&v)),
            // Some servers send a bare string that isn't valid JSON.
            Err(_) if !s.trim_start().starts_with(['{', '[', '"']) => Ok(Text::literal(s)),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn from_json(v: &Value) -> Text {
        match v {
            Value::String(s) => Text::literal(s.clone()),
            Value::Number(n) => Text::literal(n.to_string()),
            Value::Bool(b) => Text::literal(b.to_string()),
            Value::Array(items) => list_to_text(items.iter().map(Text::from_json).collect()),
            Value::Object(o) => {
                let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
                let b = |k: &str| o.get(k).and_then(Value::as_bool);
                let content = if let Some(t) = o.get("text") {
                    Content::Literal(match t {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                } else if let Some(key) = s("translate") {
                    let with = o
                        .get("with")
                        .and_then(Value::as_array)
                        .map(|l| l.iter().map(Text::from_json).collect())
                        .unwrap_or_default();
                    Content::Translate { key, fallback: s("fallback"), with }
                } else if let Some(k) = s("keybind") {
                    Content::Keybind(k)
                } else if o.contains_key("score") || o.contains_key("selector") || o.contains_key("nbt") {
                    Content::Unresolved
                } else {
                    Content::Literal(String::new())
                };
                let click = o.get("click_event").or_else(|| o.get("clickEvent"));
                let link = click.and_then(|c| {
                    (c.get("action").and_then(Value::as_str) == Some("open_url"))
                        .then(|| c.get("url").or_else(|| c.get("value")).and_then(Value::as_str).map(str::to_owned))
                        .flatten()
                });
                let click = click.and_then(Click::from_json);
                let style = Style {
                    color: s("color").as_deref().and_then(parse_color),
                    bold: b("bold"),
                    italic: b("italic"),
                    underlined: b("underlined"),
                    strikethrough: b("strikethrough"),
                    obfuscated: b("obfuscated"),
                    link,
                    click,
                    hover: None,
                };
                let extra = o
                    .get("extra")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().map(Text::from_json).collect())
                    .unwrap_or_default();
                Text { content, style, extra }
            }
            Value::Null => Text::literal(String::new()),
        }
    }

    // ---------- flattening ----------

    pub fn spans(&self, tr: &dyn Translator) -> Vec<Span> {
        let mut out = Vec::new();
        self.flatten(&Style::default(), tr, &mut out, 0);
        // Merge neighbours with identical style.
        let mut merged: Vec<Span> = Vec::with_capacity(out.len());
        for s in out {
            if s.text.is_empty() {
                continue;
            }
            if let Some(last) = merged.last_mut() {
                if same_style(last, &s) {
                    last.text.push_str(&s.text);
                    continue;
                }
            }
            merged.push(s);
        }
        merged
    }

    pub fn to_plain(&self, tr: &dyn Translator) -> String {
        self.spans(tr).into_iter().map(|s| s.text).collect()
    }

    fn flatten(&self, parent: &Style, tr: &dyn Translator, out: &mut Vec<Span>, depth: usize) {
        if depth > 64 {
            return;
        }
        let style = self.style.inherit(parent);
        match &self.content {
            Content::Literal(t) => push_legacy(t, &style, out),
            Content::Keybind(k) => push_legacy(&keybind_label(k), &style, out),
            Content::Unresolved => {}
            Content::Translate { key, fallback, with } => {
                let pattern = tr.translate(key).or(fallback.as_deref()).unwrap_or(key);
                let mut next_arg = 0usize;
                let mut literal = String::new();
                let mut chars = pattern.char_indices().peekable();
                while let Some((_, c)) = chars.next() {
                    if c != '%' {
                        literal.push(c);
                        continue;
                    }
                    // %%, %s, %n$s
                    let mut digits = String::new();
                    while let Some(&(_, d)) = chars.peek() {
                        if d.is_ascii_digit() {
                            digits.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let index = if !digits.is_empty() && chars.peek().map(|p| p.1) == Some('$') {
                        chars.next();
                        digits.parse::<usize>().ok().map(|n| n.saturating_sub(1))
                    } else if digits.is_empty() {
                        None
                    } else {
                        literal.push('%');
                        literal.push_str(&digits);
                        continue;
                    };
                    match chars.next() {
                        Some((_, 's')) | Some((_, 'd')) => {
                            push_legacy(&literal, &style, out);
                            literal.clear();
                            let i = index.unwrap_or_else(|| {
                                next_arg += 1;
                                next_arg - 1
                            });
                            if let Some(arg) = with.get(i) {
                                arg.flatten(&style, tr, out, depth + 1);
                            }
                        }
                        Some((_, '%')) => literal.push('%'),
                        Some((_, other)) => {
                            literal.push('%');
                            literal.push(other);
                        }
                        None => literal.push('%'),
                    }
                }
                push_legacy(&literal, &style, out);
            }
        }
        for e in &self.extra {
            e.flatten(&style, tr, out, depth + 1);
        }
    }
}

fn list_to_text(mut items: Vec<Text>) -> Text {
    if items.is_empty() {
        return Text::literal(String::new());
    }
    let mut first = items.remove(0);
    first.extra.extend(items);
    first
}

fn arg_from_nbt(tag: &Nbt) -> Text {
    Text::from_nbt(tag)
}

fn format_number(n: f64, tag: &Nbt) -> String {
    match tag {
        Nbt::Float(_) | Nbt::Double(_) => n.to_string(),
        _ => (n as i64).to_string(),
    }
}

fn keybind_label(key: &str) -> String {
    // Shown as the action name; our own key bindings use different names.
    key.trim_start_matches("key.").replace('.', " ")
}

fn same_style(a: &Span, b: &Span) -> bool {
    a.color == b.color
        && a.bold == b.bold
        && a.italic == b.italic
        && a.underlined == b.underlined
        && a.strikethrough == b.strikethrough
        && a.obfuscated == b.obfuscated
        && a.link == b.link
        && a.click == b.click
        && a.hover == b.hover
}

/// Appends text, interpreting legacy `§x` codes.
fn push_legacy(text: &str, base: &Style, out: &mut Vec<Span>) {
    let mk = |s: &Style| Span {
        text: String::new(),
        color: s.color,
        bold: s.bold.unwrap_or(false),
        italic: s.italic.unwrap_or(false),
        underlined: s.underlined.unwrap_or(false),
        strikethrough: s.strikethrough.unwrap_or(false),
        obfuscated: s.obfuscated.unwrap_or(false),
        link: s.link.clone(),
        click: s.click.clone(),
        hover: s.hover.clone(),
    };
    if !text.contains('§') {
        let mut span = mk(base);
        span.text.push_str(text);
        out.push(span);
        return;
    }
    let mut cur = mk(base);
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '§' {
            cur.text.push(c);
            continue;
        }
        let Some(code) = chars.next() else { break };
        let code = code.to_ascii_lowercase();
        let finished = std::mem::replace(&mut cur.text, String::new());
        if !finished.is_empty() {
            let mut done = cur.clone();
            done.text = finished;
            out.push(done);
        }
        match code {
            '0'..='9' | 'a'..='f' => {
                let idx = code.to_digit(16).unwrap() as usize;
                cur = mk(base);
                cur.color = Some(NAMED_COLORS[idx].1);
            }
            'k' => cur.obfuscated = true,
            'l' => cur.bold = true,
            'm' => cur.strikethrough = true,
            'n' => cur.underlined = true,
            'o' => cur.italic = true,
            'r' => cur = mk(base),
            _ => {}
        }
    }
    if !cur.text.is_empty() {
        out.push(cur);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct En;
    impl Translator for En {
        fn translate(&self, key: &str) -> Option<&str> {
            match key {
                "chat.type.text" => Some("<%s> %s"),
                "commands.give.success" => Some("Gave %2$s × %1$s"),
                "percent" => Some("100%%"),
                _ => None,
            }
        }
    }

    #[test]
    fn translate_sequential_and_positional() {
        let t = Text::translate("chat.type.text", vec![Text::literal("Alex"), Text::literal("hello")]);
        assert_eq!(t.to_plain(&En), "<Alex> hello");
        let t = Text::translate("commands.give.success", vec![Text::literal("Dirt"), Text::literal("64")]);
        assert_eq!(t.to_plain(&En), "Gave 64 × Dirt");
        assert_eq!(Text::translate("percent", vec![]).to_plain(&En), "100%");
        assert_eq!(Text::translate("missing.key", vec![]).to_plain(&En), "missing.key");
    }

    #[test]
    fn json_with_extra_and_colors() {
        let t = Text::from_json_str(r#"{"text":"A","color":"red","extra":[{"text":"B","bold":true},"C"]}"#).unwrap();
        let spans = t.spans(&NoTranslations);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "A");
        assert_eq!(spans[0].color, Some(0xFF5555));
        assert_eq!(spans[1].text, "B");
        assert!(spans[1].bold);
        assert_eq!(spans[2].text, "C");
        assert_eq!(spans[2].color, Some(0xFF5555));
        assert!(!spans[2].bold);
        assert_eq!(t.to_plain(&NoTranslations), "ABC");
    }

    #[test]
    fn legacy_codes() {
        let spans = Text::literal("§aGreen §lBold§r plain").spans(&NoTranslations);
        assert_eq!(spans[0].text, "Green ");
        assert_eq!(spans[0].color, Some(0x55FF55));
        assert_eq!(spans[1].text, "Bold");
        assert!(spans[1].bold);
        assert_eq!(spans[2].text, " plain");
        assert_eq!(spans[2].color, None);
    }

    #[test]
    fn nbt_heterogeneous_list() {
        let tag = Nbt::Compound(vec![
            ("translate".into(), Nbt::String("chat.type.text".into())),
            (
                "with".into(),
                Nbt::List(vec![
                    Nbt::Compound(vec![(String::new(), Nbt::String("Steve".into()))]),
                    Nbt::Compound(vec![(String::new(), Nbt::Int(5))]),
                ]),
            ),
        ]);
        assert_eq!(Text::from_nbt(&tag).to_plain(&En), "<Steve> 5");
    }

    #[test]
    fn hex_color() {
        let t = Text::from_json_str(r##"{"text":"x","color":"#5FB3A1"}"##).unwrap();
        assert_eq!(t.spans(&NoTranslations)[0].color, Some(0x5FB3A1));
    }
}
