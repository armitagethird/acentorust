//! Pure state machine: for each keyboard event, decides whether to block it and what to do next.
//! No Win32 calls here — everything the engine needs from the system comes through [`Env`].

use crate::accents::Letter;

/// How long an unconfirmed session waits before showing the popup (and becoming a real accent
/// session). Shorter than this and a fast `letter, Space` rollover is treated as normal typing.
pub const CONFIRM_DELAY_MS: u32 = 200;

/// A physical key as seen by the low-level keyboard hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawKey {
    pub vk: u16,
    pub scan: u16,
    pub extended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    KeyDown(RawKey),
    KeyUp(RawKey),
    /// The confirmation timer started by [`Action::Arm`] fired.
    Timer,
}

/// System queries: answered by Win32 in production and by fakes in tests.
pub trait Env {
    /// Whether `vk` is physically down (state before the event being handled).
    fn is_down(&self, vk: u16) -> bool;
    fn caps_lock_on(&self) -> bool;
    /// Whether the foreground window is a fullscreen app (game, video player).
    fn foreground_fullscreen(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start the confirmation timer (`CONFIRM_DELAY_MS`).
    Arm,
    /// Show or refresh the popup with `variants`, highlighting `index`.
    Show {
        variants: &'static [char],
        index: usize,
    },
    /// End the session: hide the popup, cancel the timer.
    Hide,
    /// End the session, then send Backspace + this char (replaces the letter already typed).
    Commit(char),
    /// End the session, then send the swallowed Space, then press this key (if any).
    Replay(Option<RawKey>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// `true` = swallow the event; `false` = let it reach the application.
    pub block: bool,
    pub action: Option<Action>,
}

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;

const PASS: Outcome = Outcome {
    block: false,
    action: None,
};
const BLOCK: Outcome = Outcome {
    block: true,
    action: None,
};

fn pass(action: Action) -> Outcome {
    Outcome {
        block: false,
        action: Some(action),
    }
}

fn block(action: Action) -> Outcome {
    Outcome {
        block: true,
        action: Some(action),
    }
}

fn is_shift(vk: u16) -> bool {
    matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT)
}

/// Ctrl/Alt/Win mean a shortcut (or AltGr), never an accent.
fn modifier_down(env: &impl Env) -> bool {
    [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|vk| env.is_down(vk))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    /// An accent letter is physically held; it was typed normally.
    Held { letter: Letter, upper: bool },
    /// Space was pressed while holding the letter. Until `confirmed` (popup shown or user
    /// navigated) the session may still turn out to be fast typing.
    Active {
        letter: Letter,
        upper: bool,
        index: usize,
        confirmed: bool,
    },
}

#[derive(Debug, Default)]
pub struct Engine {
    state: State,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decides what to do with one keyboard event. `env` is queried lazily, only when needed.
    pub fn handle(&mut self, event: Event, env: &impl Env) -> Outcome {
        match self.state {
            State::Idle => self.idle(event, env),
            State::Held { letter, upper } => self.held(letter, upper, event, env),
            State::Active {
                letter,
                upper,
                index,
                confirmed,
            } => self.active(letter, upper, index, confirmed, event),
        }
    }

    fn idle(&mut self, event: Event, env: &impl Env) -> Outcome {
        if let Event::KeyDown(key) = event
            && let Some(letter) = Letter::from_vk(key.vk)
            && !modifier_down(env)
        {
            let upper = env.is_down(VK_SHIFT) != env.caps_lock_on();
            self.state = State::Held { letter, upper };
        }
        PASS
    }

    fn held(&mut self, letter: Letter, upper: bool, event: Event, env: &impl Env) -> Outcome {
        match event {
            Event::KeyDown(key) if Letter::from_vk(key.vk) == Some(letter) => PASS,
            Event::KeyDown(key) if key.vk == VK_SPACE && can_activate(letter, env) => {
                self.state = State::Active {
                    letter,
                    upper,
                    index: 0,
                    confirmed: false,
                };
                block(Action::Arm)
            }
            Event::KeyDown(key) if is_shift(key.vk) => PASS,
            Event::KeyDown(_) => {
                self.state = State::Idle;
                self.idle(event, env)
            }
            Event::KeyUp(key) if Letter::from_vk(key.vk) == Some(letter) => {
                self.state = State::Idle;
                PASS
            }
            Event::KeyUp(_) | Event::Timer => PASS,
        }
    }

    fn active(
        &mut self,
        letter: Letter,
        upper: bool,
        index: usize,
        confirmed: bool,
        event: Event,
    ) -> Outcome {
        let variants = letter.variants(upper);
        let count = variants.len();
        match event {
            Event::KeyDown(key) => match key.vk {
                VK_SPACE | VK_RIGHT => self.navigate(letter, upper, (index + 1) % count),
                VK_LEFT => self.navigate(letter, upper, (index + count - 1) % count),
                vk if Letter::from_vk(vk) == Some(letter) => BLOCK,
                vk if is_shift(vk) => PASS,
                VK_ESCAPE if confirmed => {
                    self.state = State::Held { letter, upper };
                    block(Action::Hide)
                }
                _ if confirmed => {
                    self.state = State::Idle;
                    pass(Action::Hide)
                }
                _ => {
                    self.state = State::Idle;
                    block(Action::Replay(Some(key)))
                }
            },
            Event::KeyUp(key) if Letter::from_vk(key.vk) == Some(letter) => {
                self.state = State::Idle;
                if confirmed {
                    pass(Action::Commit(variants[index]))
                } else {
                    pass(Action::Replay(None))
                }
            }
            Event::KeyUp(key) if matches!(key.vk, VK_SPACE | VK_LEFT | VK_RIGHT) => BLOCK,
            Event::KeyUp(_) => PASS,
            Event::Timer if !confirmed => {
                self.state = State::Active {
                    letter,
                    upper,
                    index,
                    confirmed: true,
                };
                pass(Action::Show { variants, index })
            }
            Event::Timer => PASS,
        }
    }

    fn navigate(&mut self, letter: Letter, upper: bool, index: usize) -> Outcome {
        self.state = State::Active {
            letter,
            upper,
            index,
            confirmed: true,
        };
        block(Action::Show {
            variants: letter.variants(upper),
            index,
        })
    }
}

/// Space starts a session only for a real, physically held letter, outside fullscreen apps
/// (games use "hold A + Space"). Cheapest checks first: `foreground_fullscreen` does syscalls.
fn can_activate(letter: Letter, env: &impl Env) -> bool {
    !modifier_down(env) && env.is_down(letter.vk()) && !env.foreground_fullscreen()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 0x41;
    const B: u16 = 0x42;
    const D: u16 = 0x44;
    const E: u16 = 0x45;
    const I: u16 = 0x49;

    const A_LOWER: &[char] = &['á', 'ã', 'â', 'à'];
    const A_UPPER: &[char] = &['Á', 'Ã', 'Â', 'À'];

    fn key(vk: u16) -> RawKey {
        RawKey {
            vk,
            scan: 0,
            extended: false,
        }
    }

    #[derive(Default)]
    struct FakeEnv {
        down: Vec<u16>,
        caps: bool,
        fullscreen: bool,
    }

    impl Env for FakeEnv {
        fn is_down(&self, vk: u16) -> bool {
            self.down.contains(&vk)
        }
        fn caps_lock_on(&self) -> bool {
            self.caps
        }
        fn foreground_fullscreen(&self) -> bool {
            self.fullscreen
        }
    }

    /// Drives the engine like the real hook: a key that reaches the system (not blocked) becomes
    /// "physically down" for later queries, just like GetAsyncKeyState.
    #[derive(Default)]
    struct Harness {
        engine: Engine,
        env: FakeEnv,
    }

    impl Harness {
        fn down(&mut self, vk: u16) -> Outcome {
            let outcome = self.engine.handle(Event::KeyDown(key(vk)), &self.env);
            if !outcome.block && !self.env.down.contains(&vk) {
                self.env.down.push(vk);
            }
            outcome
        }

        fn up(&mut self, vk: u16) -> Outcome {
            let outcome = self.engine.handle(Event::KeyUp(key(vk)), &self.env);
            self.env.down.retain(|&held| held != vk);
            outcome
        }

        fn timer(&mut self) -> Outcome {
            self.engine.handle(Event::Timer, &self.env)
        }

        /// Hold `letter`, press Space and let the confirmation timer fire.
        fn open(&mut self, letter: u16) {
            self.down(letter);
            self.down(VK_SPACE);
            self.timer();
        }
    }

    fn passed() -> Outcome {
        Outcome {
            block: false,
            action: None,
        }
    }
    fn blocked() -> Outcome {
        Outcome {
            block: true,
            action: None,
        }
    }
    fn passed_with(action: Action) -> Outcome {
        Outcome {
            block: false,
            action: Some(action),
        }
    }
    fn blocked_with(action: Action) -> Outcome {
        Outcome {
            block: true,
            action: Some(action),
        }
    }
    fn show(variants: &'static [char], index: usize) -> Action {
        Action::Show { variants, index }
    }

    #[test]
    fn plain_typing_passes_everything() {
        let mut h = Harness::default();
        for vk in [B, A, VK_SPACE] {
            assert_eq!(h.down(vk), passed());
            assert_eq!(h.up(vk), passed());
        }
    }

    #[test]
    fn letter_then_space_starts_provisional_session() {
        let mut h = Harness::default();
        assert_eq!(h.down(A), passed());
        assert_eq!(h.down(VK_SPACE), blocked_with(Action::Arm));
    }

    #[test]
    fn fast_rollover_release_replays_space() {
        // "casa " typed fast: a↓ Space↓ a↑ Space↑ before the popup is confirmed.
        let mut h = Harness::default();
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(h.up(A), passed_with(Action::Replay(None)));
        assert_eq!(h.up(VK_SPACE), passed());
    }

    #[test]
    fn fast_rollover_next_key_replays_space_then_key() {
        let mut h = Harness::default();
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(h.down(D), blocked_with(Action::Replay(Some(key(D)))));
        assert_eq!(h.up(A), passed());
    }

    #[test]
    fn escape_before_confirmation_is_a_rollover_key() {
        let mut h = Harness::default();
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(
            h.down(VK_ESCAPE),
            blocked_with(Action::Replay(Some(key(VK_ESCAPE))))
        );
    }

    #[test]
    fn timer_confirms_and_shows_first_variant() {
        let mut h = Harness::default();
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(h.timer(), passed_with(show(A_LOWER, 0)));
    }

    #[test]
    fn release_after_confirmation_commits_selection() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.up(VK_SPACE), blocked());
        assert_eq!(h.down(VK_SPACE), blocked_with(show(A_LOWER, 1)));
        assert_eq!(h.up(A), passed_with(Action::Commit('ã')));
    }

    #[test]
    fn space_and_right_advance_and_wrap() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.down(VK_RIGHT), blocked_with(show(A_LOWER, 1)));
        assert_eq!(h.down(VK_SPACE), blocked_with(show(A_LOWER, 2)));
        assert_eq!(h.down(VK_SPACE), blocked_with(show(A_LOWER, 3)));
        assert_eq!(h.down(VK_SPACE), blocked_with(show(A_LOWER, 0)));
    }

    #[test]
    fn left_goes_back_and_wraps() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.down(VK_LEFT), blocked_with(show(A_LOWER, 3)));
        assert_eq!(h.up(VK_LEFT), blocked());
        assert_eq!(h.down(VK_LEFT), blocked_with(show(A_LOWER, 2)));
    }

    #[test]
    fn second_space_confirms_before_timer() {
        let mut h = Harness::default();
        h.down(A);
        h.down(VK_SPACE);
        h.up(VK_SPACE);
        assert_eq!(h.down(VK_SPACE), blocked_with(show(A_LOWER, 1)));
        assert_eq!(h.timer(), passed()); // late timer is a no-op
        assert_eq!(h.up(A), passed_with(Action::Commit('ã')));
    }

    #[test]
    fn escape_closes_keeps_letter_and_space_reopens() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.down(VK_ESCAPE), blocked_with(Action::Hide));
        assert_eq!(h.up(VK_ESCAPE), passed());
        assert_eq!(h.down(VK_SPACE), blocked_with(Action::Arm));
    }

    #[test]
    fn escape_then_release_leaves_text_untouched() {
        let mut h = Harness::default();
        h.open(A);
        h.down(VK_ESCAPE);
        assert_eq!(h.up(A), passed());
    }

    #[test]
    fn other_key_after_confirmation_cancels_and_passes() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.down(D), passed_with(Action::Hide));
        assert_eq!(h.up(A), passed());
    }

    #[test]
    fn letter_autorepeat_passes_while_held_and_is_swallowed_while_active() {
        let mut h = Harness::default();
        h.down(A);
        assert_eq!(h.down(A), passed());
        h.down(VK_SPACE);
        assert_eq!(h.down(A), blocked());
    }

    #[test]
    fn long_hold_keeps_repeats_and_replaces_only_last_letter() {
        // Held past the autorepeat delay: "aaa" already typed, a single Commit (one Backspace).
        let mut h = Harness::default();
        h.down(A);
        h.down(A);
        h.down(A);
        h.open(A);
        assert_eq!(h.up(A), passed_with(Action::Commit('á')));
    }

    #[test]
    fn shift_selects_uppercase() {
        let mut h = Harness::default();
        h.down(VK_SHIFT);
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(h.timer(), passed_with(show(A_UPPER, 0)));
        assert_eq!(h.up(A), passed_with(Action::Commit('Á')));
    }

    #[test]
    fn caps_lock_selects_uppercase_and_shift_inverts_it() {
        let mut h = Harness::default();
        h.env.caps = true;
        h.down(A);
        h.down(VK_SPACE);
        assert_eq!(h.timer(), passed_with(show(A_UPPER, 0)));
        h.up(A);
        h.up(VK_SPACE);

        h.down(VK_SHIFT);
        h.open(A);
        assert_eq!(h.up(A), passed_with(Action::Commit('á')));
    }

    #[test]
    fn letter_with_ctrl_alt_or_win_is_ignored() {
        for modifier in [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN] {
            let mut h = Harness::default();
            h.down(modifier);
            h.down(A);
            assert_eq!(h.down(VK_SPACE), passed(), "modifier {modifier:#x}");
        }
    }

    #[test]
    fn ctrl_space_while_holding_letter_passes() {
        let mut h = Harness::default();
        h.down(A);
        assert_eq!(h.down(VK_CONTROL), passed());
        assert_eq!(h.down(VK_SPACE), passed());
    }

    #[test]
    fn fullscreen_foreground_never_activates() {
        let mut h = Harness::default();
        h.env.fullscreen = true;
        h.down(A);
        assert_eq!(h.down(VK_SPACE), passed());
        assert_eq!(h.up(A), passed());
    }

    #[test]
    fn space_does_not_activate_when_letter_is_not_physically_down() {
        // Key-up lost (e.g. secure desktop switch): the engine thinks A is held, the system disagrees.
        let mut h = Harness::default();
        h.down(A);
        h.env.down.clear();
        assert_eq!(h.down(VK_SPACE), passed());
    }

    #[test]
    fn pressing_another_letter_switches_the_held_letter() {
        let mut h = Harness::default();
        h.down(A);
        h.down(E);
        h.down(VK_SPACE);
        assert_eq!(h.timer(), passed_with(show(&['é', 'ê'], 0)));
    }

    #[test]
    fn non_letter_key_ends_the_hold() {
        let mut h = Harness::default();
        h.down(A);
        h.down(B);
        assert_eq!(h.down(VK_SPACE), passed());
    }

    #[test]
    fn shift_during_session_changes_nothing() {
        let mut h = Harness::default();
        h.open(A);
        assert_eq!(h.down(VK_LSHIFT), passed());
        assert_eq!(h.up(A), passed_with(Action::Commit('á')));
    }

    #[test]
    fn single_variant_letter_stays_on_its_only_option() {
        let mut h = Harness::default();
        h.open(I);
        assert_eq!(h.down(VK_SPACE), blocked_with(show(&['í'], 0)));
        assert_eq!(h.up(I), passed_with(Action::Commit('í')));
    }

    #[test]
    fn timer_outside_a_session_is_a_no_op() {
        let mut h = Harness::default();
        assert_eq!(h.timer(), passed());
        h.down(A);
        assert_eq!(h.timer(), passed());
    }
}
