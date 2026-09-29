//! Notification-area (tray) icon and its context menu.

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
                SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                WM_APP, WM_CONTEXTMENU, WM_NULL,
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
    /// Draws the icon without adding it: call [`Tray::register`] next. Callbacks arrive at `hwnd`
    /// as [`WM_TRAY`]. Registration is separate because Explorer may not be ready at logon.
    pub fn new(hwnd: HWND) -> Result<Self> {
        Ok(Self {
            hwnd,
            icon: draw_icon()?,
        })
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
        let check = if autostart_on {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        // SAFETY: the menu is created, used and destroyed inside this block; strings are
        // NUL-terminated statics; `cursor` is a valid out-param.
        let command = unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            AppendMenuW(
                menu,
                MF_STRING | check,
                ID_AUTOSTART,
                w!("Iniciar com o Windows"),
            );
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
