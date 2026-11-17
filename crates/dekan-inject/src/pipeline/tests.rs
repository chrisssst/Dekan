use super::overlay::measure_overlay;
use super::*;
use dekan_core::state::new_state_channel;

#[tokio::test]
async fn native_game_paths_reject_before_filesystem_or_process_work() {
    let dir = std::env::temp_dir().join(format!("dekan_unavailable_{}", std::process::id()));
    let mut config = temp_config(&dir, dir.join("missing_host.exe"));
    config.native_target = None;
    let (tx, rx) = new_state_channel();
    let pipeline = InjectionPipeline::new(config, Some(tx));
    for arm in [false, true] {
        let error = if arm {
            pipeline
                .arm(&[])
                .await
                .err()
                .expect("native arm unsupported")
        } else {
            pipeline
                .execute(&[], 0)
                .await
                .err()
                .expect("native execute unsupported")
        };
        let message = error.to_string();
        assert!(message.contains("NATIVE_GAME_TARGET_MISSING"), "{message}");
        let InjectionStatus::Failed { error } = &rx.borrow().injection else {
            panic!("unsupported integration must publish failure");
        };
        assert!(
            !error.contains("Vanguard"),
            "unattempted operation is not an OS rejection"
        );
        assert!(!dir.exists());
    }
}

#[tokio::test]
async fn configured_native_target_reaches_binary_validation() {
    let dir = std::env::temp_dir().join(format!("dekan_native_target_{}", std::process::id()));
    let mut config = temp_config(&dir, dir.join("missing_host.exe"));
    config.native_host_exe = Some(dir.join("missing_native_host.exe"));
    config.native_dll_path = Some(dir.join("missing_native_dll.dll"));
    config.native_target = Some(NativeWindowTarget {
        window_title: "Owned Test Window".into(),
        image_name: "owned-test.exe".into(),
    });
    let pipeline = InjectionPipeline::new(config, None);
    let error = pipeline
        .arm(&[])
        .await
        .err()
        .expect("missing host must fail");
    assert!(error.to_string().contains("Native host not found"));
    assert!(!dir.exists());
}

#[tokio::test]
async fn mock_pipeline_never_reaches_the_native_engine() {
    let dir = std::env::temp_dir().join(format!("dekan_mock_guard_{}", std::process::id()));
    let mut config = temp_config(&dir, dir.join("missing_host.exe"));
    config.provider_kind = crate::provider::ProviderKind::Mock;
    let pipeline = InjectionPipeline::new(config, None);
    let error = pipeline
        .arm(&[])
        .await
        .err()
        .expect("mock is not executable");
    assert!(error.to_string().contains("MOCK_PROVIDER_NOT_EXECUTABLE"));
    assert!(!dir.exists());
}

#[tokio::test]
async fn mock_provider_spawn_patcher_is_refused() {
    let dir = std::env::temp_dir().join(format!("dekan_mock_spawn_{}", std::process::id()));
    let mut config = temp_config(&dir, dir.join("missing_host.exe"));
    config.provider_kind = crate::provider::ProviderKind::Mock;
    let error = InjectionPipeline::new(config, None)
        .spawn_patcher(&[])
        .await
        .err()
        .expect("mock cannot spawn a patcher");
    assert!(
        error
            .to_string()
            .contains("Mock provider cannot spawn a patcher process")
    );
}

#[tokio::test]
async fn native_spawn_patcher_reports_each_missing_configuration() {
    let dir = std::env::temp_dir().join(format!("dekan_native_cfg_{}", std::process::id()));

    let mut no_host = temp_config(&dir, dir.join("host.exe"));
    no_host.native_host_exe = None;
    let error = InjectionPipeline::new(no_host, None)
        .spawn_patcher(&[])
        .await
        .err()
        .expect("missing host path");
    assert!(
        error
            .to_string()
            .contains("Native host path not configured")
    );

    let mut no_dll = temp_config(&dir, dir.join("host.exe"));
    no_dll.native_dll_path = None;
    let error = InjectionPipeline::new(no_dll, None)
        .spawn_patcher(&[])
        .await
        .err()
        .expect("missing dll path");
    assert!(error.to_string().contains("Native DLL path not configured"));

    let mut no_target = temp_config(&dir, dir.join("host.exe"));
    no_target.native_target = None;
    let error = InjectionPipeline::new(no_target, None)
        .spawn_patcher(&[])
        .await
        .err()
        .expect("missing game target");
    assert!(
        error
            .to_string()
            .contains("Native game target not configured")
    );
}

#[test]
fn test_measure_overlay_counts_only_wads_and_reports_an_empty_tree() {
    let root = std::env::temp_dir().join(format!("dekan_measure_overlay_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet

    assert_eq!(measure_overlay(&root), (0, 0));

    let nested = root.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&nested).expect("fixture tree");
    std::fs::write(nested.join("Morgana.wad.client"), b"0123456789").expect("wad");
    std::fs::write(root.join("DATA").join("UI.wad.client"), b"012345").expect("second wad");

    std::fs::write(root.join("notes.txt"), b"not a wad").expect("decoy");

    let (files, bytes) = measure_overlay(&root);
    assert_eq!(
        files, 2,
        "both WADs must be found across nested directories"
    );
    assert_eq!(bytes, 16);

    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

fn temp_config(dir: &std::path::Path, host_exe: PathBuf) -> PipelineConfig {
    PipelineConfig {
        provider_kind: crate::provider::ProviderKind::Native,
        native_host_exe: Some(host_exe),
        native_dll_path: Some(dir.join("dekan-br.dll")),
        native_target: Some(NativeWindowTarget {
            window_title: "League of Legends (TM) Client".into(),
            image_name: "League of Legends.exe".into(),
        }),
        overlay_config: OverlayConfig {
            mods_dir: dir.join("mods"),
            overlay_dir: dir.join("overlay"),
            game_dir: dir.join("game"),
        },
        hook_timeout: Duration::from_millis(50),
        build_timeout: Duration::from_secs(5),
        late_budget: Duration::from_secs(30),
    }
}

#[tokio::test]
async fn test_the_native_builder_builds_and_an_empty_merge_is_an_error() {
    use dekan_wad::writer::{WadWriter, optimal_raw};

    let dir = std::env::temp_dir().join(format!(
        "dekan_test_pipeline_native_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    let wad = |path: &std::path::Path, entries: &[(u64, &[u8])]| {
        let mut writer = WadWriter::default();
        for (hash, bytes) in entries {
            writer.insert(*hash, optimal_raw(bytes.to_vec()).expect("entry"));
        }
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, writer.to_bytes().expect("wad")).expect("write");
    };
    let game = dir.join("game");
    wad(
        &game.join("DATA/FINAL/Champions/Zed.wad.client"),
        &[(1, b"skin0"), (2, b"model")],
    );
    std::fs::write(game.join("League of Legends.exe"), b"exe").expect("exe");
    let mod_dir = dir.join("mods").join("zed");
    std::fs::create_dir_all(mod_dir.join("META")).expect("meta");
    std::fs::write(mod_dir.join("META/info.json"), "{}").expect("info");
    wad(&mod_dir.join("WAD/Zed.wad.client"), &[(1, b"new skin0")]);

    let config = temp_config(&dir, dir.join("host"));
    let pipeline = InjectionPipeline::new(config, None);

    let build = pipeline
        .build_overlay(&["zed".into()], Duration::from_secs(30))
        .await
        .expect("native build");
    assert_eq!(build.wad_files, 1);

    let empty = dir.join("mods").join("empty");
    std::fs::create_dir_all(empty.join("META")).expect("meta");
    std::fs::write(empty.join("META/info.json"), "{}").expect("info");
    std::fs::remove_dir_all(dir.join("overlay")).expect("clear overlay");
    assert!(
        pipeline
            .build_overlay(&["empty".into()], Duration::from_secs(30))
            .await
            .is_err()
    );

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[tokio::test]
async fn test_a_missing_native_dll_stops_the_pipeline_before_building() {
    let dir = std::env::temp_dir().join(format!("dekan_test_missing_dll_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let host = dir.join("dekan-br.exe");
    std::fs::write(&host, b"host").expect("host");

    let (tx, rx) = new_state_channel();
    let pipeline = InjectionPipeline::new(temp_config(&dir, host), Some(tx));
    let result = pipeline.execute(&["my_skin_mod".into()], 1234).await;

    assert!(
        matches!(&result, Err(InjectError::Process(message)) if message.contains("Native DLL not found")),
        "{:?}",
        result.err()
    );
    assert!(matches!(
        rx.borrow().injection,
        InjectionStatus::Failed { .. }
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

async fn fake_host(
    name: &str,
    script: &[&str],
    hook_timeout: Duration,
) -> (InjectionPipeline, OverlayProcess) {
    let dir = std::env::temp_dir().join(format!("dekan_fake_host_{name}_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("overlay")).expect("fixture dir");
    let host = dir.join("fake_host.cmd");
    let body: String = std::iter::once("@echo off")
        .chain(script.iter().copied())
        .map(|line| format!("{line}\r\n"))
        .collect();
    std::fs::write(&host, body).expect("fake host script");
    std::fs::write(dir.join("dekan-br.dll"), b"dll").expect("fake dll");
    let mut config = temp_config(&dir, host);
    config.hook_timeout = hook_timeout;
    let pipeline = InjectionPipeline::new(config, None);
    let overlay = pipeline.spawn_patcher(&[]).await.expect("spawn fake host");
    (pipeline, overlay)
}

const STALL: &str = "ping -n 4 127.0.0.1 >nul";
const ARMED: &str = "echo Native target watcher armed";
const HOOKED: &str = "echo Native file-open interception confirmed by DLL";

#[tokio::test]
async fn test_hook_confirmed_only_when_the_host_reports_interception() {
    let budget = Duration::from_secs(10);
    let (pipeline, mut overlay) = fake_host("confirmed", &[ARMED, HOOKED, STALL], budget).await;

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, budget).await,
        InjectionStatus::Confirmed
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_arming_alone_is_not_a_confirmation() {
    let budget = Duration::from_millis(700);
    let (pipeline, mut overlay) =
        fake_host("progress", &[ARMED, "ping -n 6 127.0.0.1 >nul"], budget).await;

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, budget).await,
        InjectionStatus::Unconfirmed,
        "arming is progress, not a hook"
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_overlay_dying_early_is_a_failure_not_an_unconfirmed() {
    let budget = Duration::from_secs(5);
    let (pipeline, mut overlay) =
        fake_host("dying", &[ARMED, "ping -n 2 127.0.0.1 >nul"], budget).await;

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, budget).await,
        InjectionStatus::PatcherExited { code: 0 },
        "an overlay process that died cannot be reported as merely unconfirmed"
    );
}

#[tokio::test]
async fn test_a_patcher_reporting_failure_is_failed_not_unconfirmed() {
    let budget = Duration::from_secs(10);
    let (pipeline, mut overlay) = fake_host(
        "failed",
        &[
            ARMED,
            "echo Error: Supervision(SetWindowsHookExW failed with ERROR_ACCESS_DENIED (0x00000005))",
            STALL,
        ],
        budget,
    )
    .await;

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, budget).await,
        InjectionStatus::Failed {
            error: InjectError::HookFailed.to_string()
        }
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_a_patcher_terminated_by_another_program_reports_its_exit_code() {
    let budget = Duration::from_secs(5);
    let (pipeline, mut overlay) =
        fake_host("terminated", &[ARMED, "exit /b 1073807364"], budget).await;

    let status = pipeline.confirm_hook(&mut overlay, budget).await;

    assert_eq!(status, InjectionStatus::PatcherExited { code: 0x4001_0004 });
    assert!(closed_by_another_program(0x4001_0004));
}

#[tokio::test]
async fn test_a_spent_late_budget_ends_unconfirmed() {
    let (pipeline, mut overlay) = fake_host("spent", &[STALL], Duration::from_secs(10)).await;

    let started = std::time::Instant::now();
    let status = pipeline.confirm_hook(&mut overlay, Duration::ZERO).await;

    assert_eq!(status, InjectionStatus::Unconfirmed);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "an exhausted budget must not wait at all"
    );
    overlay.shutdown().await;
}

#[test]
fn test_ltf_prepare_summary_skips_native_wad_build() {
    let dir = std::env::temp_dir().join(format!("dekan_ltf_summary_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: clean temp dir
    let mod_dir = dir.join("mods").join("std_sett_1");
    std::fs::create_dir_all(&mod_dir).expect("mkdir");
    std::fs::write(mod_dir.join("skin0.bin"), b"prop").expect("write");

    let mut config = temp_config(&dir, dir.join("host.exe"));
    config.provider_kind = crate::provider::ProviderKind::Ltf;
    let pipeline = InjectionPipeline::new(config, None);
    let build = pipeline
        .prepare_ltf_summary(&["std_sett_1".to_string()])
        .expect("summary");
    assert_eq!(build.wad_files, 1);
    assert_eq!(build.bytes, 4);
    assert!(!dir.join("overlay").join("overlay_manifest.json").exists());
    assert!(
        pipeline
            .prepare_ltf_summary(&["missing".to_string()])
            .is_err()
    );

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup temp dir
}

#[test]
fn test_exit_codes_are_shown_as_ntstatus_hex() {
    assert_eq!(exit_code_hex(-1_073_741_819), "0xC0000005");
    assert_eq!(exit_code_hex(1), "0x00000001");
}
