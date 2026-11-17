use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows::core::PCWSTR;

pub const EXIT_OK: i32 = 0;
pub const EXIT_DLL_MISSING: i32 = 1;
pub const EXIT_EXE_MISSING: i32 = 2;
pub const EXIT_SPAWN_FAILED: i32 = 3;
pub const EXIT_INJECT_FAILED: i32 = 4;
pub const EXIT_ALREADY_RUNNING: i32 = 5;
pub const EXIT_NO_LIVE_INSTANCE: i32 = 6;

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub struct CliArgs {
    pub log_dir: Option<PathBuf>,
    pub exe_path: Option<PathBuf>,
    pub pid_file: Option<PathBuf>,
    pub as_user_child: bool,
    pub quiet: bool,
    pub no_hide: bool,
}

pub fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    parse_args_from(&args)
}

pub fn parse_args_from(args: &[String]) -> CliArgs {
    let mut log_dir = None;
    let mut exe_path = None;
    let mut pid_file = None;
    let mut as_user_child = false;
    let mut quiet = false;
    let mut no_hide = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--no-hide" => {
                no_hide = true;
            }
            "--as-user-child" => {
                as_user_child = true;
            }
            "--quiet" => {
                quiet = true;
            }
            "--pid-file" => {
                if i + 1 < args.len() {
                    i += 1;
                    pid_file = Some(PathBuf::from(&args[i]));
                }
            }
            "--log-dir" => {
                if i + 1 < args.len() {
                    i += 1;
                    log_dir = Some(PathBuf::from(&args[i]));
                }
            }
            "--exe" => {
                if i + 1 < args.len() {
                    i += 1;
                    exe_path = Some(PathBuf::from(&args[i]));
                }
            }
            other => {
                if !other.starts_with('-') && exe_path.is_none() {
                    exe_path = Some(PathBuf::from(other));
                }
            }
        }
        i += 1;
    }

    CliArgs {
        log_dir,
        exe_path,
        pid_file,
        as_user_child,
        quiet,
        no_hide,
    }
}

pub struct Logger {
    primary_path: PathBuf,
    fallback_path: PathBuf,
}

impl Logger {
    fn new(log_dir: Option<&Path>) -> Self {
        let primary_path = if let Some(dir) = log_dir {
            let _ = std::fs::create_dir_all(dir); // ignore-ok: directory creation failure handled via fallback
            dir.join("dekan_launcher.log")
        } else {
            let _ = std::fs::create_dir_all(r"C:\Users\Public"); // ignore-ok: fallback path will be used if directory creation fails
            PathBuf::from(r"C:\Users\Public\dekan_launcher.log")
        };
        let fallback_path = std::env::temp_dir().join("dekan_launcher.log");
        Self {
            primary_path,
            fallback_path,
        }
    }

    fn write_line(&self, text: &str) {
        if self.append_to(&self.primary_path, text).is_err() {
            let _ = self.append_to(&self.fallback_path, text); // ignore-ok: log emission is best-effort
        }
    }

    fn append_to(&self, path: &Path, text: &str) -> std::io::Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{text}")?;
        file.flush()
    }
}

pub fn init_logger(log_dir: Option<&Path>) {
    let _ = LOGGER.set(Logger::new(log_dir)); // ignore-ok: logger is initialized once at startup
}

pub fn log(msg: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.write_line(msg);
    }
}

pub fn fail(quiet: bool, code: i32, msg: &str) -> i32 {
    log(&format!("[err] {msg}"));
    if !quiet {
        let wide_msg: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let wide_title: Vec<u16> = "Mod Manager Bridge Launcher\0".encode_utf16().collect();
        let (m, t) = (PCWSTR(wide_msg.as_ptr()), PCWSTR(wide_title.as_ptr()));
        let _ = unsafe { MessageBoxW(None, m, t, MB_ICONERROR) }; // ignore-ok: message box failure does not alter exit code
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_from_handles_flags_and_positional_exe() {
        let args: Vec<String> = vec![
            "dekan_launcher.exe".into(),
            "--quiet".into(),
            "--no-hide".into(),
            "--log-dir".into(),
            r"C:\Temp\logs".into(),
            "--pid-file".into(),
            r"C:\Temp\pid.txt".into(),
            r"C:\Program Files\LTK Manager\ltk-manager.exe".into(),
        ];
        let parsed = parse_args_from(&args);
        assert!(parsed.quiet);
        assert!(parsed.no_hide);
        assert!(!parsed.as_user_child);
        assert_eq!(parsed.log_dir, Some(PathBuf::from(r"C:\Temp\logs")));
        assert_eq!(parsed.pid_file, Some(PathBuf::from(r"C:\Temp\pid.txt")));
        assert_eq!(
            parsed.exe_path,
            Some(PathBuf::from(
                r"C:\Program Files\LTK Manager\ltk-manager.exe"
            ))
        );
    }
}
