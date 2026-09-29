# AcentoRust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tiny Windows tray app, written in Rust, that clones PowerToys Quick Accent for Portuguese: hold a letter, press Space, pick the accented character.

**Architecture:** A pure, fully unit-tested state machine (`engine`) decides for every physical key event whether to block it and which single action to run. Thin Win32 layers (`hook`, `popup`, `input`, `tray`, `autostart`) feed it events and execute its actions. Everything runs on one thread. The low-level hook callback only decides block/pass and queues work, and the message loop executes that work.

**Tech Stack:** Rust 1.98 (edition 2024, MSVC), `windows-sys 0.61.2` (raw Win32 bindings), `anyhow 1`.

**Spec:** `docs/superpowers/specs/2026-09-29-acentorust-design.md`, which executors must read before starting.

## Global Constraints

- Windows 10/11 only. Single crate `acentorust` with a lib (`src/lib.rs`) and a bin (`src/main.rs`).
- Dependencies are exactly `windows-sys = 0.61.2` (features already listed in `Cargo.toml`) and `anyhow = 1.0.104`. Do not add crates. If a Win32 item fails to resolve, check whether its feature is missing from `Cargo.toml` and add it there. The registry source is at `E:\Dev\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\windows-sys-0.61.2\src\Windows\Win32\`.
- `windows-sys` 0.61 facts:
  - Handles (`HWND`, `HICON`, `HFONT`, `HGDIOBJ`, …) are `*mut c_void`. Use `ptr::null_mut()` or `.is_null()`.
  - `BOOL` is `i32`, and `0` means failure.
  - Some structs have no `Default`: `PAINTSTRUCT`, `MSG`, `NOTIFYICONDATAW`, `WNDCLASSW`, `ICONINFO`. For those, use `std::mem::zeroed()` with a `// SAFETY:` comment, or write every field out.
  - The `w!("…")` macro is `windows_sys::w` and yields a NUL-terminated `PCWSTR`.
- Lints are enforced by `Cargo.toml`:
  - `unsafe_op_in_unsafe_fn = deny` and `clippy::undocumented_unsafe_blocks = deny`: every `unsafe { }` block gets a `// SAFETY:` comment stating the invariant.
  - `clippy::unwrap_used = deny`, except in tests. `expect("reason")` is allowed only for invariants that cannot fail.
- Prefer safe `extern "system" fn` callbacks (window/hook procs) with `unsafe` blocks inside over `unsafe extern "system" fn`.
- Rust code, identifiers and comments are in English. Comments only explain *why*. User-facing strings are Portuguese (`"Iniciar com o Windows"`, `"Sair"`, error text).
- Privacy: never log, store or put into error messages any key code, character or text typed by the user.
- Done means all of these are clean: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Commits: conventional style in Portuguese (`feat: ...`). **Never** add a `Co-Authored-By` trailer or any mention of Claude/Anthropic. The git identity is already configured.
- Timing constant: `CONFIRM_DELAY_MS = 200` (already in `src/engine.rs`).
- Accent order (PT-BR frequency):
  - A: á ã â à
  - E: é ê
  - I: í
  - O: ó õ ô
  - U: ú
  - C: ç
  - Uppercase mirrors lowercase.
- Popup visuals, in DIPs scaled by monitor DPI:
  - Layout: padding 6, cells 44×52, gap 2, selected-cell radius 6, font Segoe UI semibold 24 px, 24 DIP below the top of the work area.
  - Colors: background `#18181B`, border `#3F3F46`, accent `#2F6FDB`, text `#E4E4E7`, selected text `#FAFAFA`.

## Review Focus

1. **Exe path with spaces and accents.** The user's desktop is `C:\Users\romer\OneDrive\Área de Trabalho`. The autostart value must be the quoted UTF-16 path. Pinned by `run_command_quotes_path_with_spaces_and_accents` (Task 4).
2. **Holding the letter past the Windows autorepeat delay before Space.** Repeats stay in the text and only the last letter is replaced (one Backspace). Pinned by `long_hold_keeps_repeats_and_replaces_only_last_letter` (Task 2).
3. **Second instance started (double-click, autostart plus manual start).** It must exit silently while the first keeps working. Pinned by the process-count check in Task 7.
4. **Clicking the bar, or the bar appearing, must never take focus from the app being typed in.** Pinned by `WS_EX_NOACTIVATE`/`MA_NOACTIVATE` in Task 3 and the manual check in Task 7.
5. **Explorer restarts (crash, update).** The tray icon must come back. Pinned by the `TaskbarCreated` handling in Tasks 5 and 6 and the manual check in Task 7.

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/accents.rs` | `Letter` enum, VK mapping, accent table | 1 |
| `src/engine.rs` | `Engine` state machine. Shared types (`RawKey`, `Event`, `Env`, `Action`, `Outcome`, `CONFIRM_DELAY_MS`) **already exist, do not change them** | 2 |
| `src/popup.rs` | Accent bar window (create/show/hide/paint) | 3 |
| `src/input.rs` | `SendInput`: commit and replay | 4 |
| `src/autostart.rs` | HKCU Run key | 4 |
| `src/tray.rs` | Tray icon, menu, `TaskbarCreated` | 5 |
| `src/hook.rs` (bin) | `WH_KEYBOARD_LL` install and translation, `Win32Env` | 6 |
| `src/main.rs` (bin) | Single instance, DPI, main window, message loop, action dispatch | 6 |

Execution waves. Tasks inside a wave are independent because they touch disjoint files:
- **Wave 1:** A = Tasks 1–2, B = Task 3, C = Tasks 4–5.
- **Wave 2:** Task 6, after wave 1 is merged.
- **Task 7:** the orchestrator.

---

### Task 1: Accent table (`src/accents.rs`)

**Files:**
- Modify: `src/accents.rs` (currently only a module doc line)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum Letter { A, C, E, I, O, U }` deriving `Clone, Copy, Debug, PartialEq, Eq`
  - `pub fn Letter::from_vk(vk: u16) -> Option<Letter>`
  - `pub fn Letter::vk(self) -> u16`
  - `pub fn Letter::variants(self, upper: bool) -> &'static [char]`

- [ ] **Step 1: Write the failing tests** by appending this to `src/accents.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Letter; 6] = [Letter::A, Letter::C, Letter::E, Letter::I, Letter::O, Letter::U];

    #[test]
    fn from_vk_maps_only_accentable_letters() {
        assert_eq!(Letter::from_vk(0x41), Some(Letter::A));
        assert_eq!(Letter::from_vk(0x43), Some(Letter::C));
        assert_eq!(Letter::from_vk(0x55), Some(Letter::U));
        assert_eq!(Letter::from_vk(0x42), None); // B
        assert_eq!(Letter::from_vk(0x20), None); // Space
        assert_eq!(Letter::from_vk(0x61), None); // VK_NUMPAD1, not a lowercase 'a'
        assert_eq!(Letter::from_vk(0x141), None); // outside the u8 range
    }

    #[test]
    fn vk_round_trips() {
        for letter in ALL {
            assert_eq!(Letter::from_vk(letter.vk()), Some(letter));
        }
    }

    #[test]
    fn portuguese_variants_in_frequency_order() {
        assert_eq!(Letter::A.variants(false), ['á', 'ã', 'â', 'à']);
        assert_eq!(Letter::E.variants(false), ['é', 'ê']);
        assert_eq!(Letter::I.variants(false), ['í']);
        assert_eq!(Letter::O.variants(false), ['ó', 'õ', 'ô']);
        assert_eq!(Letter::U.variants(false), ['ú']);
        assert_eq!(Letter::C.variants(false), ['ç']);
    }

    #[test]
    fn uppercase_variants_mirror_lowercase() {
        for letter in ALL {
            let lower = letter.variants(false);
            let upper = letter.variants(true);
            assert_eq!(lower.len(), upper.len());
            for (lc, uc) in lower.iter().zip(upper) {
                assert_eq!(lc.to_uppercase().collect::<Vec<_>>(), vec![*uc]);
            }
        }
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --lib accents`
Expected: compile error, because `Letter` is not defined.

- [ ] **Step 3: Implement.** Insert this above the tests module:

```rust
/// A letter that has accented variants in Portuguese.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Letter {
    A,
    C,
    E,
    I,
    O,
    U,
}

impl Letter {
    /// Maps a Windows virtual-key code (`'A'` = 0x41 …) to a letter with accents.
    pub fn from_vk(vk: u16) -> Option<Self> {
        match u8::try_from(vk).ok()? {
            b'A' => Some(Self::A),
            b'C' => Some(Self::C),
            b'E' => Some(Self::E),
            b'I' => Some(Self::I),
            b'O' => Some(Self::O),
            b'U' => Some(Self::U),
            _ => None,
        }
    }

    /// The virtual-key code of this letter.
    pub fn vk(self) -> u16 {
        u16::from(match self {
            Self::A => b'A',
            Self::C => b'C',
            Self::E => b'E',
            Self::I => b'I',
            Self::O => b'O',
            Self::U => b'U',
        })
    }

    /// Accented variants in display order: most frequent in Portuguese first, so the common
    /// ones need the fewest Space presses.
    pub fn variants(self, upper: bool) -> &'static [char] {
        match (self, upper) {
            (Self::A, false) => &['á', 'ã', 'â', 'à'],
            (Self::A, true) => &['Á', 'Ã', 'Â', 'À'],
            (Self::C, false) => &['ç'],
            (Self::C, true) => &['Ç'],
            (Self::E, false) => &['é', 'ê'],
            (Self::E, true) => &['É', 'Ê'],
            (Self::I, false) => &['í'],
            (Self::I, true) => &['Í'],
            (Self::O, false) => &['ó', 'õ', 'ô'],
            (Self::O, true) => &['Ó', 'Õ', 'Ô'],
            (Self::U, false) => &['ú'],
            (Self::U, true) => &['Ú'],
        }
    }
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib accents`
Expected: 4 passed.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`. Expected: clean.

```bash
git add src/accents.rs
git commit -m "feat: tabela de acentos PT-BR"
```

---

### Task 2: State machine (`src/engine.rs`)

**Files:**
- Modify: `src/engine.rs`. Append below the existing shared types and do **not** change them.

**Interfaces:**
- Consumes (Task 1): `Letter::from_vk`, `Letter::vk`, `Letter::variants`.
- Consumes (already in file): `RawKey { vk, scan, extended }`, `Event::{KeyDown(RawKey), KeyUp(RawKey), Timer}`, `trait Env { is_down(&self, u16) -> bool; caps_lock_on(&self) -> bool; foreground_fullscreen(&self) -> bool }`, `Action::{Arm, Show { variants, index }, Hide, Commit(char), Replay(Option<RawKey>)}`, `Outcome { block, action }`.
- Produces:
  - `pub struct Engine` deriving `Debug, Default`
  - `pub fn Engine::new() -> Engine`
  - `pub fn Engine::handle(&mut self, event: Event, env: &impl Env) -> Outcome`

**Transition table**, the contract this task implements. "Mods" means Ctrl, Alt, LWin or RWin is down.

| State | Event | Condition | Next state | Block | Action |
|---|---|---|---|---|---|
| Idle | KeyDown(k) | k is an accent letter L, no mods | Held{L, upper = Shift down XOR Caps on} | no | – |
| Idle | anything else | – | Idle | no | – |
| Held{L} | KeyDown(L) | autorepeat | Held | no | – |
| Held{L} | KeyDown(Space) | no mods, L physically down, foreground not fullscreen | Active{L, index 0, unconfirmed} | **yes** | Arm |
| Held{L} | KeyDown(Shift/LShift/RShift) | – | Held | no | – |
| Held{L} | KeyDown(other, incl. Space failing the guards) | – | re-evaluate as Idle (may become Held{other letter}) | no | – |
| Held{L} | KeyUp(L) | – | Idle | no | – |
| Held | KeyUp(other) / Timer | – | Held | no | – |
| Active | KeyDown(Space or Right) | – | index+1 mod n, confirmed | yes | Show |
| Active | KeyDown(Left) | – | index−1 mod n, confirmed | yes | Show |
| Active{L} | KeyDown(L) | autorepeat | same | yes | – |
| Active | KeyDown(Shift/LShift/RShift) | – | same | no | – |
| Active{L} | KeyDown(Esc) | confirmed | Held{L} | yes | Hide |
| Active | KeyDown(other) | confirmed | Idle | no | Hide |
| Active | KeyDown(other, incl. Esc) | unconfirmed | Idle | **yes** | Replay(Some(key)) |
| Active{L} | KeyUp(L) | confirmed | Idle | no | Commit(variants[index]) |
| Active{L} | KeyUp(L) | unconfirmed | Idle | no | Replay(None) |
| Active | KeyUp(Space/Left/Right) | – | same | yes | – |
| Active | KeyUp(other) | – | same | no | – |
| Active | Timer | unconfirmed | confirmed | no | Show |
| Active | Timer | confirmed | same | no | – |
| Idle/Held | Timer | – | same | no | – |

- [ ] **Step 1: Write the failing tests** by appending this to `src/engine.rs`:

```rust
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
        RawKey { vk, scan: 0, extended: false }
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
        Outcome { block: false, action: None }
    }
    fn blocked() -> Outcome {
        Outcome { block: true, action: None }
    }
    fn passed_with(action: Action) -> Outcome {
        Outcome { block: false, action: Some(action) }
    }
    fn blocked_with(action: Action) -> Outcome {
        Outcome { block: true, action: Some(action) }
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
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --lib engine`
Expected: compile errors, because `Engine` and `VK_SPACE` are not defined.

- [ ] **Step 3: Implement.** Insert this between the shared types and the tests module:

```rust
use crate::accents::Letter;

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

const PASS: Outcome = Outcome { block: false, action: None };
const BLOCK: Outcome = Outcome { block: true, action: None };

fn pass(action: Action) -> Outcome {
    Outcome { block: false, action: Some(action) }
}

fn block(action: Action) -> Outcome {
    Outcome { block: true, action: Some(action) }
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
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib engine`
Expected: 26 passed. If a test fails, fix the implementation, never the test. The tests encode the transition table.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`. Expected: clean.

```bash
git add src/engine.rs
git commit -m "feat: máquina de estados do acento rápido"
```

---

### Task 3: Accent bar window (`src/popup.rs`)

**Files:**
- Modify: `src/popup.rs` (currently only a module doc line)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `pub struct Popup` (owns the window and implements `Drop`)
  - `pub fn Popup::create() -> anyhow::Result<Popup>`
  - `pub fn Popup::show(&self, variants: &'static [char], index: usize)`
  - `pub fn Popup::hide(&self)`

**Behavior:**
- The window is `WS_POPUP` with `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`. It never takes focus: it is shown with `SWP_NOACTIVATE`, and `WM_MOUSEACTIVATE` returns `MA_NOACTIVATE`.
- `show` places the bar at the top-center of the work area of the foreground window's monitor (primary monitor if none), `TOP_MARGIN` DIP below the top, scaled by that monitor's DPI (`GetDpiForMonitor`, falling back to 96).
- Painting is double-buffered. The popup is the only one in the process, so painting reads thread-local `Cell`s and `RefCell` is never needed.
- The work does not include animation, by design.

- [ ] **Step 1: Write failing tests for the pure layout math.** Append to `src/popup.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_size_at_96_dpi() {
        assert_eq!(window_size(4, 96), (194, 64));
        assert_eq!(window_size(1, 96), (56, 64));
    }

    #[test]
    fn window_size_scales_with_dpi() {
        assert_eq!(window_size(4, 144), (291, 96));
    }

    #[test]
    fn cells_are_laid_out_left_to_right() {
        let first = cell_rect(0, 96);
        let second = cell_rect(1, 96);
        assert_eq!((first.left, first.top, first.right, first.bottom), (6, 6, 50, 58));
        assert_eq!((second.left, second.top, second.right, second.bottom), (52, 6, 96, 58));
    }

    #[test]
    fn last_cell_ends_one_padding_before_the_edge() {
        let (width, _) = window_size(4, 144);
        assert_eq!(cell_rect(3, 144).right + px(PAD, 144), width);
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --lib popup`
Expected: compile errors, because `window_size`, `cell_rect`, `px` and `PAD` are not defined.

- [ ] **Step 3: Implement.** Insert this above the tests module:

```rust
use std::{cell::Cell, io, ptr};

use anyhow::{Result, bail};
use windows_sys::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                DwmSetWindowAttribute,
            },
            Gdi::{
                BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
                CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
                DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
                DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_SEMIBOLD, FillRect,
                GetMonitorInfoW, GetStockObject, HFONT, InvalidateRect, MONITOR_DEFAULTTOPRIMARY,
                MONITORINFO, MonitorFromWindow, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
                RoundRect, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow,
                HWND_TOPMOST, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, RegisterClassW, SW_HIDE,
                SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WM_DPICHANGED,
                WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    w,
};

// Sizes in DIPs (1:1 at 96 DPI), scaled to the monitor's DPI.
const PAD: i32 = 6;
const CELL_W: i32 = 44;
const CELL_H: i32 = 52;
const GAP: i32 = 2;
const RADIUS: i32 = 6;
const FONT_PX: i32 = 24;
const TOP_MARGIN: i32 = 24;

const BACKGROUND: COLORREF = rgb(0x18, 0x18, 0x1B);
const BORDER: COLORREF = rgb(0x3F, 0x3F, 0x46);
const ACCENT: COLORREF = rgb(0x2F, 0x6F, 0xDB);
const TEXT: COLORREF = rgb(0xE4, 0xE4, 0xE7);
const TEXT_SELECTED: COLORREF = rgb(0xFA, 0xFA, 0xFA);

const CLASS_NAME: windows_sys::core::PCWSTR = w!("AcentoRustPopup");

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

#[derive(Clone, Copy)]
struct Content {
    variants: &'static [char],
    index: usize,
    dpi: u32,
}

thread_local! {
    static CONTENT: Cell<Content> = const {
        Cell::new(Content { variants: &[], index: 0, dpi: 96 })
    };
    /// Font cached for one DPI: (dpi, font). Recreated when the DPI changes.
    static FONT: Cell<(u32, HFONT)> = const { Cell::new((0, ptr::null_mut())) };
}

/// The accent bar window.
pub struct Popup {
    hwnd: HWND,
}

impl Popup {
    /// Creates the (hidden) bar window.
    pub fn create() -> Result<Self> {
        // SAFETY: GetModuleHandleW(null) is this executable; the class struct and the static
        // strings outlive the calls.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: ptr::null_mut(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: CLASS_NAME,
            };
            if RegisterClassW(&class) == 0 {
                bail!("RegisterClassW (barra de acentos): {}", io::Error::last_os_error());
            }
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("AcentoRust"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            bail!("CreateWindowExW (barra de acentos): {}", io::Error::last_os_error());
        }
        round_corners(hwnd);
        Ok(Self { hwnd })
    }

    /// Shows (or refreshes) the bar with `variants`, highlighting `index`, at the top-center of
    /// the foreground window's monitor. Never activates the bar.
    pub fn show(&self, variants: &'static [char], index: usize) {
        let (work, dpi) = foreground_monitor();
        let (width, height) = window_size(variants.len(), dpi);
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + px(TOP_MARGIN, dpi);
        CONTENT.set(Content { variants, index, dpi });
        // SAFETY: self.hwnd is our live bar window.
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.hwnd, ptr::null(), 0);
        }
    }

    pub fn hide(&self) {
        // SAFETY: self.hwnd is our live bar window.
        unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        let (_, font) = FONT.replace((0, ptr::null_mut()));
        // SAFETY: we own the window and the cached font; neither is used after this.
        unsafe {
            DestroyWindow(self.hwnd);
            if !font.is_null() {
                DeleteObject(font);
            }
        }
    }
}

/// DIPs → physical pixels at `dpi`, rounded to nearest.
fn px(dip: i32, dpi: u32) -> i32 {
    (dip * dpi as i32 + 48) / 96
}

/// Bar size in pixels for `count` cells.
fn window_size(count: usize, dpi: u32) -> (i32, i32) {
    let count = count as i32;
    let width = 2 * PAD + count * CELL_W + (count - 1).max(0) * GAP;
    (px(width, dpi), px(2 * PAD + CELL_H, dpi))
}

/// Cell `index` in window coordinates.
fn cell_rect(index: usize, dpi: u32) -> RECT {
    let left = PAD + index as i32 * (CELL_W + GAP);
    RECT {
        left: px(left, dpi),
        top: px(PAD, dpi),
        right: px(left + CELL_W, dpi),
        bottom: px(PAD + CELL_H, dpi),
    }
}

/// Work area and DPI of the monitor showing the foreground window (primary if none).
fn foreground_monitor() -> (RECT, u32) {
    // SAFETY: MONITORINFO has cbSize set; the out-params are valid locals. A null foreground
    // window falls back to the primary monitor.
    unsafe {
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) != 0 {
            dpi_x = 96;
        }
        (info.rcWork, dpi_x)
    }
}

/// Windows 11 rounded corners and a subtle border. Windows 10 rejects both attributes; the
/// square bar is fine there, so the results are intentionally ignored.
fn round_corners(hwnd: HWND) {
    let corner = DWMWCP_ROUND;
    let border = BORDER;
    // SAFETY: pointers to live locals, with their exact sizes.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            ptr::from_ref(&corner).cast(),
            size_of_val(&corner) as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            ptr::from_ref(&border).cast(),
            size_of_val(&border) as u32,
        );
    }
}

/// Segoe UI semibold at `FONT_PX`, cached per DPI.
fn font(dpi: u32) -> HFONT {
    let (cached_dpi, cached) = FONT.get();
    if cached_dpi == dpi && !cached.is_null() {
        return cached;
    }
    // SAFETY: plain font creation. The old font is deleted only when it is not selected into any
    // DC: paint() always restores the previous font before returning.
    unsafe {
        let font = CreateFontW(
            -px(FONT_PX, dpi),
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(DEFAULT_PITCH | FF_DONTCARE),
            w!("Segoe UI"),
        );
        if !cached.is_null() {
            DeleteObject(cached);
        }
        FONT.set((dpi, font));
        font
    }
}

fn paint(hwnd: HWND) {
    let content = CONTENT.get();
    let (width, height) = window_size(content.variants.len(), content.dpi);
    // SAFETY: BeginPaint/EndPaint are paired; every GDI object created here is deselected and
    // deleted before returning. All-zero is a valid PAINTSTRUCT.
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mem = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let old_bitmap = SelectObject(mem, bitmap);

        let background = CreateSolidBrush(BACKGROUND);
        let full = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        FillRect(mem, &full, background);
        DeleteObject(background);

        let accent = CreateSolidBrush(ACCENT);
        let old_brush = SelectObject(mem, accent);
        let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));
        let cell = cell_rect(content.index, content.dpi);
        let corner = px(2 * RADIUS, content.dpi);
        // NULL_PEN leaves the right/bottom edge unpainted, hence the +1.
        RoundRect(mem, cell.left, cell.top, cell.right + 1, cell.bottom + 1, corner, corner);
        SelectObject(mem, old_pen);
        SelectObject(mem, old_brush);
        DeleteObject(accent);

        let old_font = SelectObject(mem, font(content.dpi));
        SetBkMode(mem, TRANSPARENT as i32);
        for (i, ch) in content.variants.iter().enumerate() {
            SetTextColor(mem, if i == content.index { TEXT_SELECTED } else { TEXT });
            let mut utf16 = [0u16; 2];
            let text = ch.encode_utf16(&mut utf16);
            let mut rect = cell_rect(i, content.dpi);
            DrawTextW(
                mem,
                text.as_ptr(),
                text.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        SelectObject(mem, old_font);

        BitBlt(hdc, 0, 0, width, height, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bitmap);
        DeleteObject(bitmap);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        // paint() covers every pixel; skipping the erase avoids flicker.
        WM_ERASEBKGND => 1,
        // Clicking the bar must not steal focus from the app being typed in.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        // show() recomputes size and position for the target monitor's DPI every time.
        WM_DPICHANGED => 0,
        // SAFETY: forwarding the unmodified arguments to the default window procedure.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
```

If `Some(wndproc)` does not coerce to `WNDPROC` (`Option<unsafe extern "system" fn …>`), write `Some(wndproc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT)`.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib popup`
Expected: 4 passed.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`. Expected: clean. Fix any cast lint the compiler raises without changing behavior.

```bash
git add src/popup.rs
git commit -m "feat: barra de acentos (janela GDI sem foco, DPI-aware)"
```

---

### Task 4: Keyboard injection and autostart (`src/input.rs`, `src/autostart.rs`)

**Files:**
- Modify: `src/input.rs` and `src/autostart.rs` (each currently only a module doc line)

**Interfaces:**
- Consumes: `crate::engine::RawKey { vk: u16, scan: u16, extended: bool }` (already exists).
- Produces:
  - `pub fn input::commit(ch: char) -> anyhow::Result<()>`: Backspace, then `ch` as Unicode.
  - `pub fn input::replay(then: Option<RawKey>) -> anyhow::Result<()>`: Space down/up, then `then` key-down only.
  - `pub fn autostart::is_enabled() -> bool`
  - `pub fn autostart::set(enabled: bool) -> anyhow::Result<()>`

- [ ] **Step 1: Write failing tests for `input`.** Append to `src/input.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const A_TILDE: u16 = 0x00E3; // 'ã'

    fn ki(input: &INPUT) -> KEYBDINPUT {
        assert_eq!(input.r#type, INPUT_KEYBOARD);
        // SAFETY: every INPUT built by this module is a keyboard input.
        unsafe { input.Anonymous.ki }
    }

    #[test]
    fn commit_sends_backspace_then_unicode_char() {
        let keys: Vec<KEYBDINPUT> = commit_inputs('ã').iter().map(ki).collect();
        assert_eq!(keys.len(), 4);
        assert_eq!((keys[0].wVk, keys[0].dwFlags), (VK_BACK, 0));
        assert_eq!((keys[1].wVk, keys[1].dwFlags), (VK_BACK, KEYEVENTF_KEYUP));
        assert_eq!((keys[2].wVk, keys[2].wScan, keys[2].dwFlags), (0, A_TILDE, KEYEVENTF_UNICODE));
        assert_eq!(
            (keys[3].wVk, keys[3].wScan, keys[3].dwFlags),
            (0, A_TILDE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)
        );
    }

    #[test]
    fn replay_sends_space_then_only_the_key_down() {
        let right = RawKey { vk: 0x27, scan: 0x4D, extended: true };
        let keys: Vec<KEYBDINPUT> = replay_inputs(Some(right)).iter().map(ki).collect();
        assert_eq!(keys.len(), 3);
        assert_eq!((keys[0].wVk, keys[0].dwFlags), (VK_SPACE, 0));
        assert_eq!((keys[1].wVk, keys[1].dwFlags), (VK_SPACE, KEYEVENTF_KEYUP));
        assert_eq!((keys[2].wVk, keys[2].wScan, keys[2].dwFlags), (0x27, 0x4D, KEYEVENTF_EXTENDEDKEY));
    }

    #[test]
    fn replay_without_key_sends_only_space() {
        assert_eq!(replay_inputs(None).len(), 2);
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --lib input`
Expected: compile errors, because `commit_inputs` and `replay_inputs` are not defined.

- [ ] **Step 3: Implement `input`.** Insert this above the tests module:

```rust
use anyhow::{Result, bail};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VK_BACK,
    VK_SPACE,
};

use crate::engine::RawKey;

/// Deletes the letter just typed and types `ch` in its place.
pub fn commit(ch: char) -> Result<()> {
    send(&commit_inputs(ch))
}

/// Types the Space swallowed by a false start, then presses `then`. Only its key-down is sent:
/// the physical key-up arrives on its own.
pub fn replay(then: Option<RawKey>) -> Result<()> {
    send(&replay_inputs(then))
}

fn commit_inputs(ch: char) -> Vec<INPUT> {
    let back = virtual_key(VK_BACK);
    let mut inputs = vec![key(back, 0), key(back, KEYEVENTF_KEYUP)];
    let mut utf16 = [0u16; 2];
    for &unit in ch.encode_utf16(&mut utf16).iter() {
        inputs.push(unicode(unit, 0));
        inputs.push(unicode(unit, KEYEVENTF_KEYUP));
    }
    inputs
}

fn replay_inputs(then: Option<RawKey>) -> Vec<INPUT> {
    let space = virtual_key(VK_SPACE);
    let mut inputs = vec![key(space, 0), key(space, KEYEVENTF_KEYUP)];
    inputs.extend(then.map(|k| key(k, 0)));
    inputs
}

/// Scan codes included: some apps (games, remote desktop) read them instead of the VK.
fn virtual_key(vk: u16) -> RawKey {
    // SAFETY: plain lookup, no pointers.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
    RawKey {
        vk,
        scan: u16::try_from(scan).unwrap_or(0),
        extended: false,
    }
}

fn key(k: RawKey, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    let extended = if k.extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    keyboard(KEYBDINPUT {
        wVk: k.vk,
        wScan: k.scan,
        dwFlags: flags | extended,
        time: 0,
        dwExtraInfo: 0,
    })
}

fn unicode(unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    keyboard(KEYBDINPUT {
        wVk: 0,
        wScan: unit,
        dwFlags: KEYEVENTF_UNICODE | flags,
        time: 0,
        dwExtraInfo: 0,
    })
}

fn keyboard(ki: KEYBDINPUT) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let count = u32::try_from(inputs.len())?;
    let size = i32::try_from(size_of::<INPUT>())?;
    // SAFETY: `inputs` is a valid slice of initialized INPUTs and `size` is their exact size.
    let sent = unsafe { SendInput(count, inputs.as_ptr(), size) };
    // Never include the characters in the message (privacy).
    if sent != count {
        bail!(
            "SendInput inseriu {sent} de {count} eventos: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}
```

- [ ] **Step 4: Run the `input` tests and confirm they pass**

Run: `cargo test --lib input`
Expected: 3 passed.

- [ ] **Step 5: Write the failing test for `autostart`.** Append to `src/autostart.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_quotes_path_with_spaces_and_accents() {
        let exe = Path::new(r"C:\Users\romer\OneDrive\Área de Trabalho\acentorust.exe");
        let expected: Vec<u16> =
            "\"C:\\Users\\romer\\OneDrive\\Área de Trabalho\\acentorust.exe\"\0"
                .encode_utf16()
                .collect();
        assert_eq!(run_command(exe), expected);
    }
}
```

The registry functions are deliberately not unit-tested, because they would write the real HKCU Run key of the developer's machine. Task 7 covers them manually.

- [ ] **Step 6: Run the test and confirm it fails**

Run: `cargo test --lib autostart`
Expected: compile error, because `run_command` is not defined.

- [ ] **Step 7: Implement `autostart`.** Insert this above the tests module:

```rust
use std::{os::windows::ffi::OsStrExt, path::Path, ptr};

use anyhow::{Context, Result, bail};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    },
    core::PCWSTR,
    w,
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("AcentoRust");

/// Whether AcentoRust is registered to start with Windows.
pub fn is_enabled() -> bool {
    // SAFETY: static NUL-terminated strings; null type/data/size pointers query existence only.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    status == ERROR_SUCCESS
}

/// Registers (with the current exe path) or unregisters AcentoRust at Windows startup.
pub fn set(enabled: bool) -> Result<()> {
    if enabled { register() } else { unregister() }
}

fn register() -> Result<()> {
    let exe = std::env::current_exe().context("caminho do executável")?;
    let command = run_command(&exe);
    let bytes = u32::try_from(command.len() * size_of::<u16>())?;
    // SAFETY: `command` is a NUL-terminated UTF-16 buffer of exactly `bytes` bytes.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            REG_SZ,
            command.as_ptr().cast(),
            bytes,
        )
    };
    if status != ERROR_SUCCESS {
        bail!("não consegui ativar o início com o Windows (erro {status}) para {}", exe.display());
    }
    Ok(())
}

fn unregister() -> Result<()> {
    // SAFETY: static NUL-terminated strings.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) };
    match status {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        _ => bail!("não consegui desativar o início com o Windows (erro {status})"),
    }
}

/// `"<exe>"` as NUL-terminated UTF-16. Quoted so that paths with spaces run correctly.
fn run_command(exe: &Path) -> Vec<u16> {
    let quote = u16::from(b'"');
    let mut command = vec![quote];
    command.extend(exe.as_os_str().encode_wide());
    command.extend([quote, 0]);
    command
}
```

- [ ] **Step 8: Run the tests and confirm they pass**

Run: `cargo test --lib`
Expected: all `input` and `autostart` tests pass.

- [ ] **Step 9: Lint and commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`. Expected: clean.

```bash
git add src/input.rs src/autostart.rs
git commit -m "feat: injeção de teclas (SendInput) e início com o Windows"
```

---

### Task 5: Tray icon (`src/tray.rs`)

**Files:**
- Modify: `src/tray.rs` (currently only a module doc line)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `pub const WM_TRAY: u32 = WM_APP + 1`
  - `pub enum MenuCommand { ToggleAutostart, Quit }`
  - `pub struct Tray` (implements `Drop`: removes the icon, destroys the HICON)
  - `pub fn Tray::add(hwnd: HWND) -> anyhow::Result<Tray>`
  - `pub fn Tray::register(&self) -> anyhow::Result<()>`: idempotent. Call it again on `TaskbarCreated`.
  - `pub fn Tray::menu(&self, autostart_on: bool) -> Option<MenuCommand>`
  - `pub fn is_menu_request(lparam: LPARAM) -> bool`
  - `pub fn taskbar_created_message() -> u32`

- [ ] **Step 1: Write the failing tests** by appending this to `src/tray.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_MOUSEMOVE};

    fn callback(event: u32) -> LPARAM {
        // NOTIFYICON_VERSION_4: LOWORD = event, HIWORD = icon id.
        ((ICON_ID << 16) | event) as LPARAM
    }

    #[test]
    fn right_click_left_click_and_keyboard_open_the_menu() {
        assert!(is_menu_request(callback(WM_CONTEXTMENU)));
        assert!(is_menu_request(callback(NIN_SELECT)));
        assert!(is_menu_request(callback(NIN_SELECT | NINF_KEY)));
    }

    #[test]
    fn mouse_noise_does_not_open_the_menu() {
        // Left click already arrives as NIN_SELECT; also handling WM_LBUTTONUP would open it twice.
        assert!(!is_menu_request(callback(WM_MOUSEMOVE)));
        assert!(!is_menu_request(callback(WM_LBUTTONUP)));
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --lib tray`
Expected: compile errors, because `is_menu_request` and `ICON_ID` are not defined.

- [ ] **Step 3: Implement.** Insert this above the tests module:

```rust
use std::{io, ptr};

use anyhow::{Result, bail};
use windows_sys::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::{
            ANTIALIASED_QUALITY, BLACK_BRUSH, CLIP_DEFAULT_PRECIS, CreateBitmap,
            CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
            DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
            DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_SEMIBOLD, FillRect, GetDC,
            GetStockObject, NULL_PEN, OUT_DEFAULT_PRECIS, ReleaseDC, RoundRect, SelectObject,
            SetBkMode, SetTextColor, TRANSPARENT, WHITE_BRUSH,
        },
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
                NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu,
                GetCursorPos, GetSystemMetrics, HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR,
                MF_STRING, MF_UNCHECKED, PostMessageW, RegisterWindowMessageW, SM_CXSMICON,
                SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_NULL,
            },
        },
    },
    w,
};

/// Message the tray icon sends to its owner window.
pub const WM_TRAY: u32 = WM_APP + 1;

const ICON_ID: u32 = 1;
const ID_AUTOSTART: usize = 1;
const ID_QUIT: usize = 2;

const ACCENT: COLORREF = rgb(0x2F, 0x6F, 0xDB);
const GLYPH: COLORREF = rgb(0xFA, 0xFA, 0xFA);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    ToggleAutostart,
    Quit,
}

/// The tray icon. Removed from the tray on drop.
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    /// Draws the icon and adds it to the tray. Callbacks arrive at `hwnd` as [`WM_TRAY`].
    pub fn add(hwnd: HWND) -> Result<Self> {
        let tray = Self {
            hwnd,
            icon: draw_icon()?,
        };
        tray.register()?;
        Ok(tray)
    }

    /// (Re)adds the icon. Idempotent, so it is safe on every `TaskbarCreated`: that message also
    /// arrives when the icon still exists (e.g. taskbar DPI change).
    pub fn register(&self) -> Result<()> {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon;
        for (dst, src) in data.szTip.iter_mut().zip("AcentoRust".encode_utf16()) {
            *dst = src;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: `data` is fully initialized and outlives the calls.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                bail!("não consegui adicionar o ícone na bandeja");
            }
            if Shell_NotifyIconW(NIM_SETVERSION, &data) == 0 {
                bail!("não consegui configurar o ícone da bandeja");
            }
        }
        Ok(())
    }

    /// Shows the context menu at the cursor and returns the chosen command, if any.
    pub fn menu(&self, autostart_on: bool) -> Option<MenuCommand> {
        let check = if autostart_on { MF_CHECKED } else { MF_UNCHECKED };
        // SAFETY: the menu is created, used and destroyed inside this block; strings are
        // NUL-terminated statics; `cursor` is a valid out-param.
        let command = unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            AppendMenuW(menu, MF_STRING | check, ID_AUTOSTART, w!("Iniciar com o Windows"));
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Sair"));
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor);
            // Documented quirk: without this the menu does not close when clicking elsewhere.
            SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                cursor.x,
                cursor.y,
                0,
                self.hwnd,
                ptr::null(),
            );
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            command
        };
        match usize::try_from(command).ok()? {
            ID_AUTOSTART => Some(MenuCommand::ToggleAutostart),
            ID_QUIT => Some(MenuCommand::Quit),
            _ => None,
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        // SAFETY: NOTIFYICONDATAW is plain data (integers, arrays, handles); all-zero is valid.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = ICON_ID;
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data();
        // SAFETY: removes our own icon and frees the icon handle we created.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            DestroyIcon(self.icon);
        }
    }
}

/// Whether a [`WM_TRAY`] callback (NOTIFYICON_VERSION_4 `lparam`) asks for the menu.
pub fn is_menu_request(lparam: LPARAM) -> bool {
    let event = (lparam & 0xFFFF) as u32;
    event == WM_CONTEXTMENU || event == NIN_SELECT || event == (NIN_SELECT | NINF_KEY)
}

/// Id of the `TaskbarCreated` message, broadcast when Explorer (re)starts.
pub fn taskbar_created_message() -> u32 {
    // SAFETY: static NUL-terminated string.
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}

/// Rounded accent square with a white "á", at the small-icon size.
fn draw_icon() -> Result<HICON> {
    // SAFETY: every GDI object created here is deselected and deleted before returning;
    // CreateIconIndirect copies the bitmaps, so deleting them afterwards is correct.
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON);
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let color = CreateCompatibleBitmap(screen, size, size);
        ReleaseDC(ptr::null_mut(), screen);
        let mask = CreateBitmap(size, size, 1, 1, ptr::null());
        let full = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        let corner = size / 2;

        // Mask: 1 = transparent, 0 = opaque (the rounded square).
        let old_bitmap = SelectObject(dc, mask);
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        FillRect(dc, &full, GetStockObject(WHITE_BRUSH));
        let old_brush = SelectObject(dc, GetStockObject(BLACK_BRUSH));
        RoundRect(dc, 0, 0, size + 1, size + 1, corner, corner);

        // Color: black outside the square (neutral under the mask), accent inside, white glyph.
        SelectObject(dc, color);
        FillRect(dc, &full, GetStockObject(BLACK_BRUSH));
        let accent = CreateSolidBrush(ACCENT);
        SelectObject(dc, accent);
        RoundRect(dc, 0, 0, size + 1, size + 1, corner, corner);
        let font = CreateFontW(
            -(size * 3 / 4),
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(ANTIALIASED_QUALITY),
            u32::from(DEFAULT_PITCH | FF_DONTCARE),
            w!("Segoe UI"),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, GLYPH);
        let glyph = [0x00E1u16]; // 'á'
        let mut rect = full;
        DrawTextW(
            dc,
            glyph.as_ptr(),
            1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        SelectObject(dc, old_font);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_bitmap);
        DeleteObject(font);
        DeleteObject(accent);
        DeleteDC(dc);

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        if icon.is_null() {
            bail!("CreateIconIndirect: {}", io::Error::last_os_error());
        }
        Ok(icon)
    }
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib tray`
Expected: 2 passed.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`. Expected: clean.

```bash
git add src/tray.rs
git commit -m "feat: ícone na bandeja com menu (autostart, sair)"
```

---

### Task 6: Hook and application wiring (`src/hook.rs`, `src/main.rs`)

**Prerequisite:** Tasks 1–5 are merged into `main`.

**Files:**
- Create: `src/hook.rs`, a binary-crate module declared by `main.rs` with `mod hook;`
- Modify: `src/main.rs`, replacing the placeholder

**Interfaces:**
- Consumes:
  - `acentorust::engine::{Engine, Event, Action, RawKey, Env, CONFIRM_DELAY_MS}`
  - `acentorust::popup::Popup` (`create`, `show`, `hide`)
  - `acentorust::input::{commit, replay}`
  - `acentorust::autostart::{is_enabled, set}`
  - `acentorust::tray::{Tray, MenuCommand, WM_TRAY, is_menu_request, taskbar_created_message}`
- Produces:
  - The runnable `acentorust.exe`.
  - `hook::install(handler: fn(Event) -> bool) -> anyhow::Result<hook::Hook>`
  - `hook::Win32Env` (implements `Env`)

**Threading rules.** Put them as a comment above the `thread_local!` block in `main.rs`.
- Everything runs on the main thread. The LL hook callback runs inside whatever message-pumping call the thread is in, including `GetMessageW` and the tray menu's modal loop.
- `ENGINE` and `PENDING` are mutably borrowed only around pure Rust work plus non-messaging queries (`GetAsyncKeyState`, `GetWindowRect`, …). So the hook never finds them borrowed. If it ever does, the key passes (`try_borrow_mut`).
- `UI` is only shared-borrowed while running. It is taken once, after the loop ends, so the tray icon is removed explicitly. Thread-local destructors don't reliably run on the main thread at exit.

- [ ] **Step 1: Write failing tests for event translation.** Create `src/hook.rs` with only this content for now:

```rust
//! Low-level keyboard hook: turns raw key events into engine events and blocks what the handler
//! says to block. Runs inside the installing thread's message loop, so it must return fast:
//! Windows silently removes slow low-level hooks.

#[cfg(test)]
mod tests {
    use super::*;

    fn info(vk: u32, flags: u32) -> KBDLLHOOKSTRUCT {
        KBDLLHOOKSTRUCT {
            vkCode: vk,
            scanCode: 0x1E,
            flags,
            time: 0,
            dwExtraInfo: 0,
        }
    }

    #[test]
    fn down_and_up_messages_map_to_events() {
        let a = RawKey { vk: 0x41, scan: 0x1E, extended: false };
        assert_eq!(to_event(WM_KEYDOWN as WPARAM, &info(0x41, 0)), Some(Event::KeyDown(a)));
        assert_eq!(to_event(WM_SYSKEYDOWN as WPARAM, &info(0x41, 0)), Some(Event::KeyDown(a)));
        assert_eq!(to_event(WM_KEYUP as WPARAM, &info(0x41, 0)), Some(Event::KeyUp(a)));
        assert_eq!(to_event(WM_SYSKEYUP as WPARAM, &info(0x41, 0)), Some(Event::KeyUp(a)));
    }

    #[test]
    fn extended_flag_is_preserved() {
        let right = RawKey { vk: 0x27, scan: 0x1E, extended: true };
        assert_eq!(
            to_event(WM_KEYDOWN as WPARAM, &info(0x27, LLKHF_EXTENDED)),
            Some(Event::KeyDown(right))
        );
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(to_event(0x0200, &info(0x41, 0)), None);
    }
}
```

Replace `src/main.rs` with the full content from Step 3 before running the tests. `hook.rs` must be declared in `main.rs` for the tests to compile.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test --bin acentorust`
Expected: compile errors, because `to_event` and the imports are missing.

- [ ] **Step 3: Implement `hook.rs`.** Insert this between the doc comment and the tests:

```rust
use std::{cell::Cell, io, ptr};

use acentorust::engine::{Env, Event, RawKey};
use anyhow::{Result, bail};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, VK_CAPITAL},
        WindowsAndMessaging::{
            CallNextHookEx, GetClassNameW, GetForegroundWindow, GetWindowRect, HC_ACTION, HHOOK,
            IsZoomed, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, SetWindowsHookExW,
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
            WM_SYSKEYUP,
        },
    },
};

/// Decides, for each physical key event, whether to block it.
pub type Handler = fn(Event) -> bool;

thread_local! {
    static HANDLER: Cell<Option<Handler>> = const { Cell::new(None) };
}

/// The installed hook; uninstalled on drop.
pub struct Hook(HHOOK);

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: self.0 is the hook we installed; it is removed exactly once.
        unsafe { UnhookWindowsHookEx(self.0) };
        HANDLER.set(None);
    }
}

/// Installs the hook on the current thread, which must run a message loop.
pub fn install(handler: Handler) -> Result<Hook> {
    HANDLER.set(Some(handler));
    // SAFETY: hook_proc matches HOOKPROC; LL hooks take this module's handle and thread id 0.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(ptr::null()),
            0,
        )
    };
    if hook.is_null() {
        HANDLER.set(None);
        bail!("SetWindowsHookExW(WH_KEYBOARD_LL): {}", io::Error::last_os_error());
    }
    Ok(Hook(hook))
}

extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a valid KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // Injected input (ours included) is never touched: no feedback loops.
        let physical = info.flags & LLKHF_INJECTED == 0;
        if physical
            && let Some(event) = to_event(wparam, info)
            && HANDLER.get().is_some_and(|handle| handle(event))
        {
            return 1;
        }
    }
    // SAFETY: forwarding the unmodified arguments to the next hook in the chain.
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn to_event(wparam: WPARAM, info: &KBDLLHOOKSTRUCT) -> Option<Event> {
    let key = RawKey {
        vk: u16::try_from(info.vkCode).ok()?,
        scan: u16::try_from(info.scanCode).ok()?,
        extended: info.flags & LLKHF_EXTENDED != 0,
    };
    match u32::try_from(wparam).ok()? {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(Event::KeyDown(key)),
        WM_KEYUP | WM_SYSKEYUP => Some(Event::KeyUp(key)),
        _ => None,
    }
}

/// [`Env`] answered by Win32. Only non-messaging APIs, so it is safe inside the hook.
pub struct Win32Env;

impl Env for Win32Env {
    fn is_down(&self, vk: u16) -> bool {
        // SAFETY: plain query; the high bit (negative i16) means "down".
        unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
    }

    fn caps_lock_on(&self) -> bool {
        // SAFETY: plain query; the low bit means "toggled on".
        unsafe { GetKeyState(i32::from(VK_CAPITAL)) & 1 != 0 }
    }

    fn foreground_fullscreen(&self) -> bool {
        foreground_is_fullscreen()
    }
}

/// A foreground window that covers its whole monitor, is not maximized and is not the desktop
/// is a fullscreen app (game, video). Maximized windows are excluded so that an auto-hidden
/// taskbar doesn't disable the feature.
fn foreground_is_fullscreen() -> bool {
    // SAFETY: plain queries on a window handle (a stale handle only makes them fail); the
    // out-params are valid locals and MONITORINFO has cbSize set.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsZoomed(hwnd) != 0 || is_desktop(hwnd) {
            return false;
        }
        let mut window = RECT::default();
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetWindowRect(hwnd, &mut window) == 0 || GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
        let screen = info.rcMonitor;
        window.left <= screen.left
            && window.top <= screen.top
            && window.right >= screen.right
            && window.bottom >= screen.bottom
    }
}

/// Desktop windows cover the monitor too, and renaming a desktop icon is typing.
fn is_desktop(hwnd: HWND) -> bool {
    let mut class = [0u16; 16];
    // SAFETY: the buffer pointer and length match.
    let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    let name = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
    matches!(name.as_str(), "Progman" | "WorkerW")
}
```

- [ ] **Step 4: Implement `main.rs`.** Replace the whole file:

```rust
#![windows_subsystem = "windows"]

mod hook;

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    io, ptr,
};

use acentorust::{
    autostart,
    engine::{Action, CONFIRM_DELAY_MS, Engine, Event},
    input,
    popup::Popup,
    tray::{self, MenuCommand, Tray, WM_TRAY},
};
use anyhow::{Result, bail};
use windows_sys::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
        },
        System::{
            Diagnostics::Debug::OutputDebugStringW, LibraryLoader::GetModuleHandleW,
            Threading::CreateMutexW,
        },
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer,
                MB_ICONERROR, MB_OK, MSG, MessageBoxW, PostMessageW, PostQuitMessage,
                RegisterClassW, SetTimer, WM_APP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
                WS_OVERLAPPED,
            },
        },
    },
    w,
};

/// Posted by the hook: run queued actions outside the hook callback.
const WM_APP_FLUSH: u32 = WM_APP + 2;
const CONFIRM_TIMER: usize = 1;

struct Ui {
    popup: Popup,
    tray: Tray,
}

// Threading rules (all on the main thread):
// - The LL hook callback runs inside any message-pumping call, including GetMessageW and the
//   tray menu's modal loop.
// - ENGINE and PENDING are mutably borrowed only around pure Rust work plus non-messaging
//   queries, so the hook never finds them borrowed. If it ever does, the key passes.
// - UI is only shared-borrowed while running. It is taken once after the loop, so the tray
//   icon is removed explicitly (main-thread TLS destructors may not run at exit).
thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
    static PENDING: RefCell<VecDeque<Action>> = RefCell::new(VecDeque::with_capacity(16));
    static MAIN_WINDOW: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn main() {
    if let Err(error) = run() {
        show_error(&error);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    // SAFETY: plain call. It fails only if awareness was already set, which is harmless.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let hwnd = create_main_window()?;
    MAIN_WINDOW.set(hwnd);
    TASKBAR_CREATED.set(tray::taskbar_created_message());
    let ui = Ui {
        popup: Popup::create()?,
        tray: Tray::add(hwnd)?,
    };
    UI.set(Some(ui));

    // Keep the Run entry pointing at this exe in case it was moved.
    if autostart::is_enabled()
        && let Err(error) = autostart::set(true)
    {
        debug_log(&format!("autostart: {error:#}"));
    }

    let hook = hook::install(on_key)?;
    message_loop();
    drop(hook);
    drop(UI.take());
    Ok(())
}

fn message_loop() {
    // SAFETY: all-zero is a valid MSG; standard loop. GetMessageW returns 0 on WM_QUIT and -1
    // on error, and both end the loop.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    }
}

/// Hook handler: runs inside the hook callback, so it only updates the engine and queues work.
fn on_key(event: Event) -> bool {
    let outcome = ENGINE.with(|engine| {
        engine
            .try_borrow_mut()
            .ok()
            .map(|mut engine| engine.handle(event, &hook::Win32Env))
    });
    // A busy engine can only mean unexpected re-entrancy: let the key through untouched.
    let Some(outcome) = outcome else {
        return false;
    };
    if let Some(action) = outcome.action {
        enqueue(action);
    }
    outcome.block
}

fn enqueue(action: Action) {
    let queued = PENDING.with(|pending| {
        pending
            .try_borrow_mut()
            .map(|mut pending| pending.push_back(action))
            .is_ok()
    });
    if !queued {
        debug_log("fila de ações ocupada; ação descartada");
        return;
    }
    // SAFETY: posting to our own live window.
    if unsafe { PostMessageW(MAIN_WINDOW.get(), WM_APP_FLUSH, 0, 0) } == 0 {
        debug_log(&format!("PostMessageW: {}", io::Error::last_os_error()));
    }
}

fn flush() {
    while let Some(action) = PENDING.with_borrow_mut(VecDeque::pop_front) {
        UI.with_borrow(|ui| {
            if let Some(ui) = ui {
                execute(ui, action);
            }
        });
    }
}

fn execute(ui: &Ui, action: Action) {
    match action {
        Action::Arm => {
            // SAFETY: timer on our own window; re-arming replaces a pending one.
            let armed = unsafe { SetTimer(MAIN_WINDOW.get(), CONFIRM_TIMER, CONFIRM_DELAY_MS, None) };
            if armed == 0 {
                debug_log(&format!("SetTimer: {}", io::Error::last_os_error()));
            }
        }
        Action::Show { variants, index } => ui.popup.show(variants, index),
        Action::Hide => end_session(ui),
        Action::Commit(ch) => {
            end_session(ui);
            report("commit", input::commit(ch));
        }
        Action::Replay(key) => {
            end_session(ui);
            report("replay", input::replay(key));
        }
    }
}

fn end_session(ui: &Ui) {
    // SAFETY: killing a timer that may not exist is harmless.
    unsafe { KillTimer(MAIN_WINDOW.get(), CONFIRM_TIMER) };
    ui.popup.hide();
}

fn on_timer() {
    // SAFETY: our own window and timer id.
    unsafe { KillTimer(MAIN_WINDOW.get(), CONFIRM_TIMER) };
    let action = ENGINE.with(|engine| {
        engine
            .try_borrow_mut()
            .ok()
            .and_then(|mut engine| engine.handle(Event::Timer, &hook::Win32Env).action)
    });
    if let Some(action) = action {
        enqueue(action);
    }
}

fn on_tray_menu() {
    let command = UI.with_borrow(|ui| {
        ui.as_ref()
            .and_then(|ui| ui.tray.menu(autostart::is_enabled()))
    });
    match command {
        Some(MenuCommand::ToggleAutostart) => {
            if let Err(error) = autostart::set(!autostart::is_enabled()) {
                show_error(&error);
            }
        }
        // SAFETY: plain call; ends the message loop.
        Some(MenuCommand::Quit) => unsafe { PostQuitMessage(0) },
        None => {}
    }
}

fn on_taskbar_created() {
    UI.with_borrow(|ui| {
        if let Some(ui) = ui
            && let Err(error) = ui.tray.register()
        {
            debug_log(&format!("bandeja: {error:#}"));
        }
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_FLUSH => flush(),
        WM_TIMER if wparam == CONFIRM_TIMER => on_timer(),
        WM_TRAY if tray::is_menu_request(lparam) => on_tray_menu(),
        _ if msg != 0 && msg == TASKBAR_CREATED.get() => on_taskbar_created(),
        // SAFETY: forwarding the unmodified arguments to the default window procedure.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

fn create_main_window() -> Result<HWND> {
    let class_name = w!("AcentoRustMain");
    // SAFETY: the class struct and static strings outlive the calls.
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name,
        };
        if RegisterClassW(&class) == 0 {
            bail!("RegisterClassW (janela principal): {}", io::Error::last_os_error());
        }
        // Hidden top-level window. Message-only windows would miss the TaskbarCreated broadcast.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name,
            w!("AcentoRust"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            bail!("CreateWindowExW (janela principal): {}", io::Error::last_os_error());
        }
        Ok(hwnd)
    }
}

/// Named mutex held for the process lifetime; a second instance sees it and exits silently.
struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire() -> Result<Option<Self>> {
        // SAFETY: static NUL-terminated name, default security; GetLastError is read right after.
        let (handle, already_running) = unsafe {
            let handle = CreateMutexW(ptr::null(), 0, w!("Local\\AcentoRust.SingleInstance"));
            (handle, GetLastError() == ERROR_ALREADY_EXISTS)
        };
        if handle.is_null() {
            bail!("CreateMutexW: {}", io::Error::last_os_error());
        }
        let instance = Self(handle);
        Ok((!already_running).then_some(instance))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

fn show_error(error: &anyhow::Error) {
    let text = wide(&format!("{error:#}"));
    // SAFETY: NUL-terminated buffers that outlive the call.
    unsafe {
        MessageBoxW(ptr::null_mut(), text.as_ptr(), w!("AcentoRust"), MB_OK | MB_ICONERROR)
    };
}

fn report(what: &str, result: Result<()>) {
    if let Err(error) = result {
        debug_log(&format!("{what}: {error:#}"));
    }
}

/// Diagnostics for DebugView (Sysinternals). Never pass key codes or characters here.
fn debug_log(message: &str) {
    let text = wide(&format!("AcentoRust: {message}\n"));
    // SAFETY: NUL-terminated buffer that outlives the call.
    unsafe { OutputDebugStringW(text.as_ptr()) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all lib tests pass, and the 3 bin tests in `hook` pass.

- [ ] **Step 6: Lint, build release, commit**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings; cargo build --release`. Expected: clean, and `target\release\acentorust.exe` exists.

```bash
git add src/hook.rs src/main.rs
git commit -m "feat: hook de teclado e loop principal (app completo)"
```

---

### Task 7: Verification (orchestrator)

- [ ] **Step 1:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` are all clean.
- [ ] **Step 2:** `cargo build --release`. The size of `target\release\acentorust.exe` is below 400 KB.
- [ ] **Step 3:** Start the exe, wait 2 s, and check it is still running.
  - Read `PrivateMemorySize64` via `Get-Process acentorust` and expect < 3 MB.
  - Start the exe a second time and check that exactly 1 `acentorust` process remains after 2 s.
  - Check that CPU time does not grow over 10 s of idle.
  - Stop the process.
- [ ] **Step 4:** Security pass on the hook/input code: privacy (no key data in logs or errors), injected-event filtering, fail-safe paths.
- [ ] **Step 5:** Hand the manual checklist to the user:
  - Notepad: hold `a` + Space, then Space ×2 and release → `â`. Hold `a` + Space, then Esc and release → `a`.
  - Fast typing `casa de praia ` never shows the bar or loses a space.
  - Shift + `a` + Space → uppercase. Caps Lock on → uppercase.
  - Chrome/Edge text field and VS Code: same results. The bar never steals focus (the caret keeps blinking in the app), even when clicking the bar.
  - Tray icon → "Iniciar com o Windows" toggles the checkmark, and the value appears or disappears in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
  - Restart Explorer (Task Manager → Windows Explorer → Restart): the icon comes back.
  - A fullscreen game or video: holding A + Space does not open the bar.
  - Tray → "Sair": the icon disappears and typing is normal.
