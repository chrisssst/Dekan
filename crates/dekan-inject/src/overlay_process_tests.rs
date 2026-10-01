use super::*;
use std::path::PathBuf;

fn cmd_exe() -> PathBuf {
    PathBuf::from(
        std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()),
    )
}

#[tokio::test]
async fn test_spawn_missing_program_is_reported() {
    let res = OverlayProcess::spawn(&PathBuf::from(r"C:\definitely\not\here.exe"), &[]);
    assert!(
        matches!(res, Err(InjectError::Process(_))),
        "missing program must be reported, not swallowed"
    );
}

#[tokio::test]
async fn test_waits_for_matching_line() {
    let args = vec!["/C".to_string(), "echo hooked into game".to_string()];
    let mut proc = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn echo");

    let line = proc
        .wait_for_line(|l| l.contains("hooked"), Duration::from_secs(5))
        .await
        .expect("should observe the line");

    assert!(line.text.contains("hooked"));
    assert!(!line.is_stderr);
    proc.shutdown().await;
}

#[tokio::test]
async fn test_reports_timeout_when_signal_never_comes() {
    let args = vec!["/C".to_string(), "echo something else".to_string()];
    let mut proc = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn echo");

    let err = proc
        .wait_for_line(|l| l.contains("never-appears"), Duration::from_millis(400))
        .await
        .expect_err("must not report success without the signal");

    assert!(matches!(
        err,
        InjectError::HookUnconfirmed { .. } | InjectError::Process(_)
    ));
    proc.shutdown().await;
}
