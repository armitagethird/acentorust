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
                MB_ICONERROR, MB_OK, MSG, MessageBoxW, PBT_APMRESUMEAUTOMATIC, PostMessageW,
                PostQuitMessage, RegisterClassW, SetTimer, WM_APP, WM_CLOSE, WM_POWERBROADCAST,
                WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
            },
        },
    },
    w,
};

/// Posted by the hook: run queued actions outside the hook callback.
const WM_APP_FLUSH: u32 = WM_APP + 2;
const CONFIRM_TIMER: usize = 1;
const TRAY_RETRY_TIMER: usize = 2;
const TRAY_RETRY_MS: u32 = 2000;

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
// - HOOK is mutably borrowed only by `take`/`set` in run() and reinstall_hook(), never while a
//   message is pumped. Dropped before UI so no key event arrives after the UI is gone.
thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
    static PENDING: RefCell<VecDeque<Action>> = RefCell::new(VecDeque::with_capacity(16));
    static MAIN_WINDOW: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static HOOK: RefCell<Option<hook::Hook>> = const { RefCell::new(None) };
}

fn main() {
    if let Err(error) = run() {
        show_error(&error);
        // exit() skips thread-local destructors: unhook and remove the tray icon explicitly.
        drop(HOOK.take());
        drop(UI.take());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    // SAFETY: plain call. It fails if awareness was already set or on Windows 10 before 1703
    // (no per-monitor v2); either way only the scaling degrades.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let hwnd = create_main_window()?;
    MAIN_WINDOW.set(hwnd);
    TASKBAR_CREATED.set(tray::taskbar_created_message());
    let ui = Ui {
        popup: Popup::create()?,
        tray: Tray::new(hwnd)?,
    };
    // Explorer's notification area may not exist yet when started at logon: keep running and retry.
    if let Err(error) = ui.tray.register() {
        debug_log(&format!("bandeja: {error:#}"));
        start_tray_retry();
    }
    UI.set(Some(ui));

    // Keep the Run entry pointing at this exe in case it was moved.
    if autostart::is_enabled()
        && let Err(error) = autostart::set(true)
    {
        debug_log(&format!("autostart: {error:#}"));
    }

    HOOK.set(Some(hook::install(on_key)?));
    message_loop();
    drop(HOOK.take());
    drop(UI.take());
    Ok(())
}

fn message_loop() {
    // SAFETY: all-zero is a valid MSG; standard loop. GetMessageW returns 0 on WM_QUIT and -1
    // on error; both end the loop.
    let failed = unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            match GetMessageW(&mut msg, ptr::null_mut(), 0, 0) {
                0 => break false,
                -1 => break true,
                _ => DispatchMessageW(&msg),
            };
        }
    };
    if failed {
        debug_log(&format!("GetMessageW: {}", io::Error::last_os_error()));
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
            // SAFETY: periodic timer on our own window; re-arming replaces a pending one.
            let armed =
                unsafe { SetTimer(MAIN_WINDOW.get(), CONFIRM_TIMER, CONFIRM_DELAY_MS, None) };
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

/// Not killed here: the periodic timer keeps ticking as a watchdog for a lost letter key-up.
/// `end_session` kills it, and every session end goes through there.
fn on_timer() {
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

/// Windows silently removes a low-level hook whose callback times out, typically on resume from
/// sleep when our pages are paged back in. Order matters: dropping the old `Hook` clears the
/// handler that `install` sets again, and the engine must be reset outside the hook.
fn reinstall_hook() {
    drop(HOOK.take());
    ENGINE.with_borrow_mut(|engine| *engine = Engine::new());
    PENDING.with_borrow_mut(VecDeque::clear);
    UI.with_borrow(|ui| {
        if let Some(ui) = ui {
            end_session(ui);
        }
    });
    match hook::install(on_key) {
        Ok(hook) => HOOK.set(Some(hook)),
        Err(error) => debug_log(&format!("hook: {error:#}")),
    }
}

fn start_tray_retry() {
    // SAFETY: periodic timer on our own window.
    if unsafe { SetTimer(MAIN_WINDOW.get(), TRAY_RETRY_TIMER, TRAY_RETRY_MS, None) } == 0 {
        debug_log(&format!("SetTimer: {}", io::Error::last_os_error()));
    }
}

/// Registers the tray icon (on `TaskbarCreated` or a retry tick) and stops the retry timer once
/// it succeeds.
fn register_tray() {
    let registered = UI.with_borrow(|ui| {
        let Some(ui) = ui else { return false };
        ui.tray
            .register()
            .inspect_err(|error| debug_log(&format!("bandeja: {error:#}")))
            .is_ok()
    });
    if registered {
        // SAFETY: killing a timer that may not exist is harmless.
        unsafe { KillTimer(MAIN_WINDOW.get(), TRAY_RETRY_TIMER) };
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // DefWindowProcW would destroy the window without WM_QUIT, leaving the hook installed
        // and every PostMessageW failing.
        WM_CLOSE => {
            // SAFETY: plain call; ends the message loop.
            unsafe { PostQuitMessage(0) }
        }
        WM_POWERBROADCAST if wparam == PBT_APMRESUMEAUTOMATIC as usize => {
            reinstall_hook();
            // SAFETY: forwarding the unmodified arguments to the default window procedure.
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_APP_FLUSH => flush(),
        WM_TIMER if wparam == CONFIRM_TIMER => on_timer(),
        WM_TIMER if wparam == TRAY_RETRY_TIMER => register_tray(),
        WM_TRAY if tray::is_menu_request(lparam) => on_tray_menu(),
        _ if msg != 0 && msg == TASKBAR_CREATED.get() => register_tray(),
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
            bail!(
                "RegisterClassW (janela principal): {}",
                io::Error::last_os_error()
            );
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
            bail!(
                "CreateWindowExW (janela principal): {}",
                io::Error::last_os_error()
            );
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
        MessageBoxW(
            ptr::null_mut(),
            text.as_ptr(),
            w!("AcentoRust"),
            MB_OK | MB_ICONERROR,
        )
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
