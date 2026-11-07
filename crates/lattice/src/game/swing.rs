//! An arm swinging. Holding the button asks for a swing every tick, so a swing
//! only starts over once it's past halfway, and carries on from where it was:
//! this is what makes a held arm keep moving instead of twitching at the start.

/// How long a swing takes unless something changes it, in ticks.
pub const NORMAL_TICKS: u32 = 6;

#[derive(Debug, Clone, Default)]
pub struct Swing {
    /// How long the current swing lasts; `None` when the arm's at rest.
    current: Option<(u32, bool)>,
    ticks: u32,
    before: f32,
    now: f32,
}

impl Swing {
    /// Starts a swing unless one's still in its first half.
    pub fn start(&mut self, off_hand: bool, duration: u32) {
        if let Some((length, _)) = self.current {
            if self.ticks <= length / 2 && self.ticks > 0 {
                return;
            }
        }
        let next = Some((duration, off_hand));
        // Going on with the same swing keeps its place, so the arm doesn't jump back.
        if self.current != next {
            self.before = 0.0;
            self.now = 0.0;
        }
        self.current = next;
        self.ticks = 0;
    }

    pub fn tick(&mut self) {
        self.before = self.now;
        let Some((length, _)) = self.current else {
            self.now = 0.0;
            return;
        };
        if length > 0 {
            self.now = (self.ticks as f32 / length as f32).min(1.0);
        }
        self.ticks += 1;
        if self.ticks > length {
            self.current = None;
            self.now = 0.0;
        }
    }

    /// How far through its motion the arm is, from 0 to 1, between two ticks.
    /// Moving forward over a restart rather than back keeps the motion smooth.
    pub fn progress(&self, partial: f32) -> f32 {
        let to = if self.now < self.before { self.now + 1.0 } else { self.now };
        self.before + (to - self.before) * partial
    }

    pub fn active(&self) -> bool {
        self.current.is_some()
    }

    pub fn off_hand(&self) -> bool {
        self.current.is_some_and(|(_, off)| off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One game tick of holding the button: asked first, then the arm moves.
    fn held(s: &mut Swing, ticks: usize) -> Vec<f32> {
        (0..ticks)
            .map(|_| {
                s.start(false, NORMAL_TICKS);
                s.tick();
                s.progress(1.0)
            })
            .collect()
    }

    #[test]
    fn holding_the_button_keeps_the_arm_moving() {
        let mut s = Swing::default();
        let seen = held(&mut s, 40);
        let highest = seen.iter().cloned().fold(0.0, f32::max);
        assert!(highest >= 0.5, "the arm should get past halfway, got {highest}: {seen:?}");
        // It keeps coming back round rather than stopping.
        let later = &seen[20..];
        assert!(later.iter().cloned().fold(1.0, f32::min) < 0.2 && later.iter().cloned().fold(0.0, f32::max) >= 0.5, "{later:?}");
    }

    #[test]
    fn starting_over_never_jumps_backwards_between_ticks() {
        let mut s = Swing::default();
        for _ in 0..30 {
            s.start(false, NORMAL_TICKS);
            s.tick();
            let (a, b) = (s.progress(0.0), s.progress(1.0));
            assert!(b >= a, "from {a} to {b} within one tick");
        }
    }

    #[test]
    fn a_single_swing_takes_its_whole_length_and_ends() {
        let mut s = Swing::default();
        s.start(false, NORMAL_TICKS);
        let mut ticks = 0;
        while s.active() {
            s.tick();
            ticks += 1;
            assert!(ticks < 20);
        }
        assert_eq!(ticks, NORMAL_TICKS as usize + 1);
        s.tick();
        assert_eq!(s.progress(1.0), 0.0);
    }

    #[test]
    fn a_new_click_early_in_a_swing_doesnt_restart_it() {
        let mut s = Swing::default();
        s.start(false, NORMAL_TICKS);
        s.tick();
        s.tick();
        let before = s.progress(1.0);
        s.start(false, NORMAL_TICKS);
        s.tick();
        assert!(s.progress(1.0) > before);
    }
}
