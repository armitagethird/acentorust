//! The accent bar: a non-activating topmost popup drawn with GDI.

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
                BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
                CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
                DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject,
                DrawTextW, EndPaint, FF_DONTCARE, FW_SEMIBOLD, FillRect, GetMonitorInfoW,
                GetStockObject, HFONT, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
                MonitorFromWindow, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RoundRect, SRCCOPY,
                SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, HWND_TOPMOST,
                IDC_ARROW, LoadCursorW, MA_NOACTIVATE, RegisterClassW, SW_HIDE, SWP_NOACTIVATE,
                SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WM_DPICHANGED, WM_ERASEBKGND,
                WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WS_EX_TOPMOST, WS_POPUP,
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
                bail!(
                    "RegisterClassW (barra de acentos): {}",
                    io::Error::last_os_error()
                );
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
            bail!(
                "CreateWindowExW (barra de acentos): {}",
                io::Error::last_os_error()
            );
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
        CONTENT.set(Content {
            variants,
            index,
            dpi,
        });
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
        RoundRect(
            mem,
            cell.left,
            cell.top,
            cell.right + 1,
            cell.bottom + 1,
            corner,
            corner,
        );
        SelectObject(mem, old_pen);
        SelectObject(mem, old_brush);
        DeleteObject(accent);

        let old_font = SelectObject(mem, font(content.dpi));
        SetBkMode(mem, TRANSPARENT as i32);
        for (i, ch) in content.variants.iter().enumerate() {
            SetTextColor(
                mem,
                if i == content.index {
                    TEXT_SELECTED
                } else {
                    TEXT
                },
            );
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
        assert_eq!(
            (first.left, first.top, first.right, first.bottom),
            (6, 6, 50, 58)
        );
        assert_eq!(
            (second.left, second.top, second.right, second.bottom),
            (52, 6, 96, 58)
        );
    }

    #[test]
    fn last_cell_ends_one_padding_before_the_edge() {
        let (width, _) = window_size(4, 144);
        assert_eq!(cell_rect(3, 144).right + px(PAD, 144), width);
    }
}
