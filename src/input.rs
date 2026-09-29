//! Keyboard injection (SendInput): commit an accented char, replay swallowed keys.

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
        assert_eq!(
            (keys[2].wVk, keys[2].wScan, keys[2].dwFlags),
            (0, A_TILDE, KEYEVENTF_UNICODE)
        );
        assert_eq!(
            (keys[3].wVk, keys[3].wScan, keys[3].dwFlags),
            (0, A_TILDE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)
        );
    }

    #[test]
    fn replay_sends_space_then_only_the_key_down() {
        let right = RawKey {
            vk: 0x27,
            scan: 0x4D,
            extended: true,
        };
        let keys: Vec<KEYBDINPUT> = replay_inputs(Some(right)).iter().map(ki).collect();
        assert_eq!(keys.len(), 3);
        assert_eq!((keys[0].wVk, keys[0].dwFlags), (VK_SPACE, 0));
        assert_eq!((keys[1].wVk, keys[1].dwFlags), (VK_SPACE, KEYEVENTF_KEYUP));
        assert_eq!(
            (keys[2].wVk, keys[2].wScan, keys[2].dwFlags),
            (0x27, 0x4D, KEYEVENTF_EXTENDEDKEY)
        );
    }

    #[test]
    fn replay_without_key_sends_only_space() {
        assert_eq!(replay_inputs(None).len(), 2);
    }
}
