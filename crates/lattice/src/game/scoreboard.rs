//! Objectives, scores and teams, as the server keeps them up to date.

use lattice_proto::packets::play::{NumberFormat, TeamParams};
use lattice_proto::{Span, Text, Translator};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Objective {
    pub display: Text,
    pub format: Option<NumberFormat>,
}

#[derive(Debug, Clone)]
pub struct Score {
    pub value: i32,
    pub display: Option<Text>,
    pub format: Option<NumberFormat>,
}

#[derive(Debug, Clone)]
pub struct Team {
    pub params: TeamParams,
    pub players: Vec<String>,
}

/// Named colours teams use, in the game's numbering.
const TEAM_COLORS: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xFFAA00, 0xAAAAAA, 0x555555, 0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

/// Places an objective can be shown.
pub const SLOT_LIST: i32 = 0;
pub const SLOT_SIDEBAR: i32 = 1;
pub const SLOT_BELOW_NAME: i32 = 2;

#[derive(Default)]
pub struct Scoreboard {
    pub objectives: HashMap<String, Objective>,
    pub slots: HashMap<i32, String>,
    /// Owner to objective to score.
    pub scores: HashMap<String, HashMap<String, Score>>,
    pub teams: HashMap<String, Team>,
    player_team: HashMap<String, String>,
    /// The colour of the team the player is on, set each frame for the sidebar.
    pub own_team_color: Option<i32>,
}

impl Scoreboard {
    pub fn set_objective(&mut self, name: String, method: i32, details: Option<(Text, i32, Option<NumberFormat>)>) {
        match method {
            1 => {
                self.objectives.remove(&name);
                self.slots.retain(|_, v| *v != name);
                for s in self.scores.values_mut() {
                    s.remove(&name);
                }
            }
            _ => {
                if let Some((display, _, format)) = details {
                    self.objectives.insert(name, Objective { display, format });
                }
            }
        }
    }

    pub fn set_display(&mut self, slot: i32, name: String) {
        if name.is_empty() {
            self.slots.remove(&slot);
        } else {
            self.slots.insert(slot, name);
        }
    }

    pub fn set_score(&mut self, owner: String, objective: String, value: i32, display: Option<Text>, format: Option<NumberFormat>) {
        self.scores.entry(owner).or_default().insert(objective, Score { value, display, format });
    }

    pub fn reset_score(&mut self, owner: &str, objective: Option<&str>) {
        match objective {
            Some(o) => {
                if let Some(s) = self.scores.get_mut(owner) {
                    s.remove(o);
                }
            }
            None => {
                self.scores.remove(owner);
            }
        }
    }

    pub fn set_team(&mut self, name: String, method: i32, params: Option<TeamParams>, players: Vec<String>) {
        match method {
            0 => {
                if let Some(params) = params {
                    for p in &players {
                        self.player_team.insert(p.clone(), name.clone());
                    }
                    self.teams.insert(name, Team { params, players });
                }
            }
            1 => {
                if let Some(t) = self.teams.remove(&name) {
                    for p in t.players {
                        self.player_team.remove(&p);
                    }
                }
            }
            2 => {
                if let (Some(t), Some(params)) = (self.teams.get_mut(&name), params) {
                    t.params = params;
                }
            }
            3 => {
                for p in &players {
                    self.player_team.insert(p.clone(), name.clone());
                }
                if let Some(t) = self.teams.get_mut(&name) {
                    t.players.extend(players);
                }
            }
            _ => {
                for p in &players {
                    self.player_team.remove(p);
                }
                if let Some(t) = self.teams.get_mut(&name) {
                    t.players.retain(|x| !players.contains(x));
                }
            }
        }
    }

    pub fn set_own_name(&mut self, name: &str) {
        self.own_team_color = self.team_of(name).and_then(|t| t.params.color);
    }

    pub fn team_of(&self, player: &str) -> Option<&Team> {
        self.teams.get(self.player_team.get(player)?)
    }

    /// A name with its team's prefix, colour and suffix around it.
    pub fn decorated(&self, name: &str, tr: &dyn Translator) -> Vec<Span> {
        let plain = Span { text: name.to_string(), ..Default::default() };
        let Some(team) = self.team_of(name) else { return vec![plain] };
        let mut out = team.params.prefix.spans(tr);
        let color = team.params.color.and_then(|c| TEAM_COLORS.get(c as usize)).copied();
        out.push(Span { color, ..plain });
        out.extend(team.params.suffix.spans(tr));
        out
    }

    /// Whether a player's name tag shows to this player, by the teams' rules.
    pub fn name_visible(&self, player: &str, me: &str) -> bool {
        let Some(team) = self.team_of(player) else { return true };
        let mine = self.team_of(me);
        let same = mine.is_some_and(|m| std::ptr::eq(m, team));
        match team.params.name_visibility {
            1 => false,
            2 => mine.is_none() || same,
            3 => mine.is_none() || !same,
            _ => true,
        }
    }

    /// The name of a player's team, which sorts the player list.
    pub fn team_name_of(&self, player: &str) -> &str {
        self.player_team.get(player).map_or("", |s| s.as_str())
    }

    /// Slots 3 to 18 are sidebars just for one team colour; the player's own colour wins.
    fn sidebar_objective(&self) -> Option<&String> {
        self.own_team_color.and_then(|c| self.slots.get(&(3 + c))).or_else(|| self.slots.get(&SLOT_SIDEBAR))
    }

    /// Text for a score, as its format says.
    pub fn format_score(&self, score: &Score, objective: &Objective, tr: &dyn Translator) -> Option<Vec<Span>> {
        match score.format.as_ref().or(objective.format.as_ref()) {
            Some(NumberFormat::Blank) => None,
            Some(NumberFormat::Fixed(t)) => Some(t.spans(tr)),
            _ => Some(vec![Span { text: score.value.to_string(), color: Some(0xFF5555), ..Default::default() }]),
        }
    }

    /// The line under a player's name: their score and the objective's name.
    pub fn below_name(&self, player: &str, tr: &dyn Translator) -> Option<Vec<Span>> {
        let name = self.slots.get(&SLOT_BELOW_NAME)?;
        let objective = self.objectives.get(name)?;
        let score = self.scores.get(player)?.get(name)?;
        let mut line = self.format_score(score, objective, tr).unwrap_or_default();
        for s in &mut line {
            if score.format.is_none() && objective.format.is_none() {
                s.color = None;
            }
        }
        line.push(Span { text: " ".into(), ..Default::default() });
        line.extend(objective.display.spans(tr));
        Some(line)
    }

    /// What the player list shows beside a name.
    pub fn list_score(&self, player: &str, tr: &dyn Translator) -> Option<Vec<Span>> {
        let name = self.slots.get(&SLOT_LIST)?;
        let objective = self.objectives.get(name)?;
        let score = self.scores.get(player)?.get(name)?;
        self.format_score(score, objective, tr)
    }

    /// The sidebar's title and rows, highest first: name, score text.
    pub fn sidebar(&self, tr: &dyn Translator) -> Option<(Vec<Span>, Vec<(Vec<Span>, Option<Vec<Span>>)>)> {
        let name = self.sidebar_objective()?;
        let objective = self.objectives.get(name)?;
        let mut rows: Vec<(&String, &Score)> = self.scores.iter().filter_map(|(owner, s)| s.get(name).map(|v| (owner, v))).filter(|(o, _)| !o.starts_with('#')).collect();
        rows.sort_by(|a, b| b.1.value.cmp(&a.1.value).then(a.0.cmp(b.0)));
        let out = rows
            .into_iter()
            .take(15)
            .map(|(owner, score)| {
                let label = match &score.display {
                    Some(d) => d.spans(tr),
                    None => self.decorated(owner, tr),
                };
                (label, self.format_score(score, objective, tr))
            })
            .collect();
        Some((objective.display.spans(tr), out))
    }
}
