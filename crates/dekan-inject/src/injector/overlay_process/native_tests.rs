use super::*;

#[test]
fn ipc_configuration_never_means_game_hook_installed() {
    for line in [
        "DLL confirmed overlay configured",
        "Native DLL connected and acknowledged handshake",
    ] {
        assert!(!matches!(native_signal(line), Some(PatcherSignal::Hooked)));
    }
    assert!(matches!(
        native_signal("Native file-open interception confirmed by DLL"),
        Some(PatcherSignal::Hooked)
    ));
    assert!(matches!(
        native_signal("Waiting for DLL to connect to Named Pipe"),
        Some(PatcherSignal::Armed)
    ));
}

#[test]
fn parse_native_failure_handles_mixed_case_and_extracts_win32_codes() {
    let err1 = "Error: Supervision(\"SetWindowsHookExW: A operação foi concluída com êxito. (0x00000000)\")";
    let parsed1 = parse_native_failure(err1).expect("parse err1");
    assert_eq!(parsed1.category, NativeFailureCategory::HookRegistration);
    assert_eq!(parsed1.win32_code, Some(0));

    let err2 = "Error: Supervision(\"SetWindowsHookExW: Acesso negado. (0x00000005)\")";
    let parsed2 = parse_native_failure(err2).expect("parse err2");
    assert_eq!(parsed2.category, NativeFailureCategory::HookRegistration);
    assert_eq!(parsed2.win32_code, Some(5));

    let err_kernel = "Error: Supervision(\"SetWindowsHookExW rejected by kernel/anti-cheat (NULL returned with GetLastError=0x00000000). Target PID: 19248, Thread ID: 15080, DLL: dekan_native_dll.dll. Windows Code Integrity / Vanguard rejected hook injection into protected process.\")";
    let parsed_kernel = parse_native_failure(err_kernel).expect("parse err_kernel");
    assert_eq!(
        parsed_kernel.category,
        NativeFailureCategory::HookRegistration
    );
    assert_eq!(parsed_kernel.win32_code, Some(0));

    assert!(parse_native_failure("ERROR: fatal pipe error").is_some());
    assert!(parse_native_failure("error: something failed").is_some());
    assert!(parse_native_failure("Failed to connect to Named Pipe").is_some());
    assert!(parse_native_failure("HOST_INVALID_ARGUMENT: option requires a value").is_some());

    let parsed_pipe = parse_native_failure("Timed out waiting for DLL to connect to pipe")
        .expect("parse pipe error");
    assert_eq!(parsed_pipe.category, NativeFailureCategory::Ipc);

    let parsed_startup =
        parse_native_failure("Error: Session(\"HOST_INVALID_ARGUMENT: option requires a value\")")
            .expect("parse startup error");
    assert_eq!(parsed_startup.category, NativeFailureCategory::Startup);

    assert!(parse_native_failure("Native target watcher armed").is_none());
    assert!(parse_native_failure("Waiting for DLL to connect to Named Pipe...").is_none());
    assert!(parse_native_failure("Native file-open interception confirmed by DLL").is_none());
}

#[test]
fn native_signal_triggers_failed_on_mixed_case_errors() {
    let rust_display_err = "Error: Supervision(\"SetWindowsHookExW: A operação foi concluída com êxito. (0x00000000)\")";
    assert_eq!(native_signal(rust_display_err), Some(PatcherSignal::Failed));

    let peer_mismatch = "Handshake peer mismatch: expected 1234, got 5678";
    assert_eq!(native_signal(peer_mismatch), Some(PatcherSignal::Failed));
}

#[test]
fn adversarial_error_parsing_and_warning_discrimination() {
    assert!(
        parse_native_failure("2026-10-25T00:00:00Z WARN Failed to close process query handle")
            .is_none()
    );
    assert!(
        parse_native_failure(
            "WARN dekan_native_host::window_hook: Target window title not found yet"
        )
        .is_none()
    );
    assert!(parse_native_failure("warning: optional feature disabled").is_none());

    assert!(parse_native_failure("Native DLL connected and acknowledged handshake").is_none());
    assert!(parse_native_failure("Handshake completed without error").is_none());
    assert!(parse_native_failure("0 errors encountered during warm-up").is_none());
    assert!(parse_native_failure("Cache invalidated successfully for next match").is_none());

    assert!(parse_native_failure("").is_none());
    assert!(parse_native_failure("   \t\r\n  ").is_none());
    assert!(parse_native_failure("Error: (0x)").is_some());
    assert_eq!(
        parse_native_failure("Error: (0x)").unwrap().win32_code,
        None
    );
    assert_eq!(
        parse_native_failure("Error: (0xZZZZ)").unwrap().win32_code,
        None
    );
    assert_eq!(
        parse_native_failure("Error: (0x1f)").unwrap().win32_code,
        Some(31)
    );

    let panic_line =
        "thread 'main' panicked at 'called `Option::unwrap()` on a `None` value', src/main.rs:42";
    let parsed_panic = parse_native_failure(panic_line).expect("parse panic");
    assert_eq!(parsed_panic.category, NativeFailureCategory::General);
    assert_eq!(native_signal(panic_line), Some(PatcherSignal::Failed));

    assert!(parse_native_failure("fatal: failed to spawn host process").is_some());
    assert!(parse_native_failure("FaTaL runtime error").is_some());
    assert!(parse_native_failure("Host_Session_Failed: connection refused").is_some());
}

#[tokio::test]
async fn process_creation_alone_is_not_native_readiness() {
    let exe = std::env::current_exe().expect("owned test binary");
    let mut process = spawn_native_host(&exe, &std::env::temp_dir(), "invalid-pipe", None, None)
        .await
        .expect("create owned process");
    let result = process
        .wait_for(PatcherSignal::Armed, std::time::Duration::from_secs(2))
        .await;
    process.shutdown().await;
    assert!(
        result.is_err(),
        "a process that cannot start the native server must not be Armed"
    );
}
