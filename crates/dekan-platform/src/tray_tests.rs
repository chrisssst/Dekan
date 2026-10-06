use super::*;

#[test]
fn test_tray_lifecycle() {
    let tray = SystemTray::spawn("Dekan Unit Test").expect("spawn tray");
    let controller = tray.controller();

    controller.update_status("Testing");
    assert!(tray.try_recv_event().is_none());

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
