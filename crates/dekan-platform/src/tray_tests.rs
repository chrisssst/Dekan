use super::*;

#[test]
fn test_tray_lifecycle() {
    let tray = SystemTray::spawn("Dekan Unit Test").expect("spawn tray");
    let controller = tray.controller();

    controller.update_status("Testing");
    assert!(tray.try_recv_event().is_none());

    drop(tray);
}
