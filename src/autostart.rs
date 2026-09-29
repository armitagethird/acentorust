//! "Start with Windows" toggle via HKCU Run key.

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
        bail!(
            "não consegui ativar o início com o Windows (erro {status}) para {}",
            exe.display()
        );
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
