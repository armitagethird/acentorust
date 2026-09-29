//! Low-level keyboard hook: turns raw key events into engine events and blocks what the handler
//! says to block. Runs inside the installing thread's message loop, so it must return fast:
//! Windows silently removes slow low-level hooks.

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
            UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
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
        bail!(
            "SetWindowsHookExW(WH_KEYBOARD_LL): {}",
            io::Error::last_os_error()
        );
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
        let a = RawKey {
            vk: 0x41,
            scan: 0x1E,
            extended: false,
        };
        assert_eq!(
            to_event(WM_KEYDOWN as WPARAM, &info(0x41, 0)),
            Some(Event::KeyDown(a))
        );
        assert_eq!(
            to_event(WM_SYSKEYDOWN as WPARAM, &info(0x41, 0)),
            Some(Event::KeyDown(a))
        );
        assert_eq!(
            to_event(WM_KEYUP as WPARAM, &info(0x41, 0)),
            Some(Event::KeyUp(a))
        );
        assert_eq!(
            to_event(WM_SYSKEYUP as WPARAM, &info(0x41, 0)),
            Some(Event::KeyUp(a))
        );
    }

    #[test]
    fn extended_flag_is_preserved() {
        let right = RawKey {
            vk: 0x27,
            scan: 0x1E,
            extended: true,
        };
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
