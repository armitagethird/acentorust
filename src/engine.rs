//! Pure state machine: for each keyboard event, decides whether to block it and what to do next.
//! No Win32 calls here — everything the engine needs from the system comes through [`Env`].

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
