use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::w;

use crate::error::PlatformError;

const RUN_SUBKEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

const VALUE_NAME: windows::core::PCWSTR = w!("Dekan");

pub fn is_enabled() -> Result<bool, PlatformError> {
    let exe = current_exe()?;
    Ok(read_run_value()?.is_some_and(|value| run_value_matches(&value, &exe)))
}

pub fn enable() -> Result<(), PlatformError> {
    let exe = current_exe()?;
    let command = run_command(&exe);
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = u32::try_from(wide.len() * std::mem::size_of::<u16>())
        .map_err(|_| PlatformError::Path(format!("executable path too long: {command}")))?;

    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            VALUE_NAME,
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            bytes,
        )
    };
    win32_result(status)
}

pub fn disable() -> Result<(), PlatformError> {
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_SUBKEY, VALUE_NAME) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    win32_result(status)
}

pub fn toggle() -> Result<bool, PlatformError> {
    if is_enabled()? {
        disable()?;
        Ok(false)
    } else {
        enable()?;
        Ok(true)
    }
}

#[must_use]
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[must_use]
pub fn run_value_matches(value: &str, exe: &Path) -> bool {
    let program = value.trim();
    let program = match program.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => program,
    };
    program.eq_ignore_ascii_case(&exe.display().to_string())
}

fn current_exe() -> Result<std::path::PathBuf, PlatformError> {
    std::env::current_exe().map_err(|e| PlatformError::Io {
        context: "failed to resolve the running executable".into(),
        source: e,
    })
}

fn read_run_value() -> Result<Option<String>, PlatformError> {
    let mut buffer = vec![0u16; 1024];
    loop {
        let mut bytes = u32::try_from(buffer.len() * std::mem::size_of::<u16>())
            .map_err(|_| PlatformError::Path("registry value too large".into()))?;

        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_SUBKEY,
                VALUE_NAME,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status == ERROR_MORE_DATA {
            let needed = (bytes as usize).div_ceil(std::mem::size_of::<u16>());
            if needed <= buffer.len() {
                return Err(PlatformError::Path(
                    "registry reported more data without a larger size".into(),
                ));
            }
            buffer.resize(needed, 0);
            continue;
        }
        win32_result(status)?;
        let chars = (bytes as usize / std::mem::size_of::<u16>()).min(buffer.len());
        let text = &buffer[..chars];
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        return Ok(Some(String::from_utf16_lossy(&text[..end])));
    }
}

fn win32_result(status: WIN32_ERROR) -> Result<(), PlatformError> {
    Ok(status.ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_the_command_is_quoted_so_spaces_do_not_split_it() {
        let exe = PathBuf::from(r"C:\Program Files\Dekan\dekan.exe");
        assert_eq!(run_command(&exe), r#""C:\Program Files\Dekan\dekan.exe""#);
    }

    #[test]
    fn test_a_value_matches_quoted_unquoted_and_in_any_case() {
        let exe = PathBuf::from(r"C:\Program Files\Dekan\dekan.exe");
        assert!(run_value_matches(&run_command(&exe), &exe));
        assert!(run_value_matches(r"c:\program files\dekan\DEKAN.EXE", &exe));
        assert!(run_value_matches(
            r#"  "C:\Program Files\Dekan\dekan.exe" --tray "#,
            &exe
        ));
    }

    #[test]
    fn test_a_value_for_another_install_does_not_match() {
        let exe = PathBuf::from(r"C:\Program Files\Dekan\dekan.exe");
        assert!(!run_value_matches(
            r#""C:\Users\u\AppData\Local\Programs\Dekan\dekan.exe""#,
            &exe
        ));
        assert!(!run_value_matches("", &exe));
    }
}
