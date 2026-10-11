use super::*;
use std::path::PathBuf;

async fn signals_of(output: &str) -> Vec<PatcherSignal> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    read_host_stdout(output.as_bytes(), tx).await;
    let mut signals = Vec::new();
    while let Ok(signal) = rx.try_recv() {
        signals.push(signal);
    }
    signals
}

fn idle(signals: mpsc::UnboundedReceiver<PatcherSignal>) -> OverlayProcess {
    OverlayProcess {
        child: None,
        _stdin: None,
        signals,
        _stdout_reader: None,
        _stderr_reader: None,
    }
}

#[tokio::test]
async fn a_missing_host_is_reported_not_swallowed() {
    let res = OverlayProcess::spawn_ltk_host(
        &PathBuf::from(r"C:\definitely\not\here.exe"),
        &PathBuf::from(r"C:\overlay"),
        0,
        HostLogLevel::Info,
    )
    .await;
    assert!(matches!(res, Err(InjectError::Process(_))));
}

#[tokio::test]
async fn a_full_host_session_yields_armed_then_hooked_and_nothing_else() {
    let output = "ok 0.00 config loglevel\r\n\
                  status 0.01 injecting scanning for the game\r\n\
                  dll 3.10 1234 5678 INFO ltk_patcher_dll: redirected wad: Zed.wad.client\r\n\
                  dll 3.20 1234 5678 ERROR ltk_patcher_dll::verify: WAD scan failed status with c0000229\r\n\
                  error 3.30 something odd\r\n\
                  noise that is not the host protocol\r\n\
                  status 3.40 injected dll attached\r\n\
                  status 9.00 waiting for exit\r\n\
                  status 60.0 exited game closed\r\n";
    assert_eq!(
        signals_of(output).await,
        vec![
            PatcherSignal::Armed,
            PatcherSignal::Hooked,
            PatcherSignal::Hooked
        ]
    );
}

#[tokio::test]
async fn a_failed_injection_never_reads_as_hooked() {
    let output = "status 0.01 injecting scanning\n\
                  status 60.0 failed DLL never attached after 60s\n\
                  status 60.1 exited\n";
    assert_eq!(signals_of(output).await, vec![PatcherSignal::Armed]);
}

#[tokio::test]
async fn legacy_status_text_is_not_a_signal() {
    let output = "Status: Waiting for exit\nStatus: Waiting for league match to start\n";
    assert!(signals_of(output).await.is_empty());
}

#[tokio::test]
async fn waiting_skips_other_signals_until_the_wanted_one() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut process = idle(rx);
    tx.send(PatcherSignal::Armed).expect("send");
    tx.send(PatcherSignal::Hooked).expect("send");
    process
        .wait_for(PatcherSignal::Hooked, Duration::from_secs(5))
        .await
        .expect("hooked arrives after armed");
}

#[tokio::test]
async fn silence_is_unconfirmed_and_a_closed_host_is_a_process_error() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut process = idle(rx);
    tx.send(PatcherSignal::Armed).expect("send");
    let silent = process
        .wait_for(PatcherSignal::Hooked, Duration::from_millis(200))
        .await;
    assert!(matches!(silent, Err(InjectError::HookUnconfirmed { .. })));

    drop(tx);
    let closed = process
        .wait_for(PatcherSignal::Hooked, Duration::from_secs(5))
        .await;
    assert!(matches!(closed, Err(InjectError::Process(_))));
}
