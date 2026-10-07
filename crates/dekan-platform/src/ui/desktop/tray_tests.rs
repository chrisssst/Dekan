use super::*;

#[tokio::test]
async fn test_an_event_wakes_the_receiver_without_polling() {
    let mut tray = SystemTray::spawn("Dekan Unit Test").expect("spawn tray");
    let controller = tray.controller();
    controller.update_status("Testing");

    let idle = tokio::time::timeout(std::time::Duration::from_millis(150), tray.recv_event()).await;
    assert!(idle.is_err(), "no event arrives on its own");

    controller.events().send(TrayEvent::Quit).expect("send");
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), tray.recv_event())
        .await
        .expect("the event wakes the receiver");
    assert!(matches!(event, Some(TrayEvent::Quit)));

    drop(tray);
}

#[test]
fn test_notice_text_is_cut_to_fit_and_always_terminated() {
    let mut field = [7u16; 5];
    fill_wide(&mut field, "Dekan 1.2");
    assert_eq!(String::from_utf16_lossy(&field[..4]), "Deka");
    assert_eq!(field[4], 0);
    fill_wide(&mut field, "ok");
    assert_eq!(field, [u16::from(b'o'), u16::from(b'k'), 0, 0, 0]);
}
