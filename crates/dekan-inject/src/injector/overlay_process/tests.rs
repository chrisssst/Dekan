use super::*;
use tokio::process::Command;

fn idle(signals: mpsc::UnboundedReceiver<PatcherSignal>) -> OverlayProcess {
    OverlayProcess {
        child: None,
        _stdin: None,
        signals,
        _stdout_reader: None,
        _stderr_reader: None,
        hooked: false,
    }
}

#[tokio::test]
async fn a_failed_injection_ends_the_wait() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut overlay = idle(rx);
    tx.send(PatcherSignal::Armed).expect("send");
    tx.send(PatcherSignal::Failed).expect("send");
    let waited = overlay
        .wait_for(PatcherSignal::Hooked, Duration::from_secs(30))
        .await;
    assert!(matches!(waited, Err(InjectError::HookFailed)));
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

#[tokio::test]
async fn a_hook_before_the_armed_line_counts_as_armed_and_stays_known() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut process = idle(rx);
    tx.send(PatcherSignal::Hooked).expect("send");
    process
        .wait_for(PatcherSignal::Armed, Duration::from_secs(5))
        .await
        .expect("a hooked patcher is armed");
    process
        .wait_for(PatcherSignal::Hooked, Duration::from_millis(50))
        .await
        .expect("the hook seen while arming is not lost");
}

#[tokio::test]
async fn a_reconnect_waits_for_a_new_hook_instead_of_the_first_games_one() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut overlay = idle(rx);
    tx.send(PatcherSignal::Hooked).expect("send");
    overlay
        .wait_for(PatcherSignal::Hooked, Duration::from_millis(200))
        .await
        .expect("first game hooked");
    tx.send(PatcherSignal::Hooked).expect("send");

    assert_eq!(overlay.ready_for_next_game(), Ok(()));
    let reconnect = overlay
        .wait_for(PatcherSignal::Hooked, Duration::from_millis(50))
        .await;
    assert!(matches!(
        reconnect,
        Err(InjectError::HookUnconfirmed { .. })
    ));
}

#[tokio::test]
async fn a_patcher_that_exited_is_not_reused_for_the_next_game() {
    let child = Command::new("cmd")
        .args(["/C", "exit /b 3"])
        .kill_on_drop(true)
        .spawn()
        .expect("spawn");
    let (_tx, rx) = mpsc::unbounded_channel();
    let mut overlay = idle(rx);
    overlay.child = Some(child);
    assert_eq!(overlay.exited_within(Duration::from_secs(5)).await, Some(3));
    assert_eq!(overlay.ready_for_next_game(), Err(3));
}
