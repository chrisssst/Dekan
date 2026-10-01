use super::*;
use crate::dll_validator::compute_sha256;
use dekan_core::state::new_state_channel;

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

fn temp_config(dir: &std::path::Path, host_exe: PathBuf, host_hash: String) -> PipelineConfig {
    PipelineConfig {
        ltk_host_exe: host_exe,
        ltk_host_hash: host_hash,
        ltk_dll_path: dir.join("ltk_patcher_dll.dll"),
        ltk_dll_hash: String::new(),
        ltk_flags: 0,
        overlay_config: OverlayConfig {
            mods_dir: dir.join("mods"),
            overlay_dir: dir.join("overlay"),
            game_dir: dir.join("game"),
        },
        state_dir: dir.join("state"),
        hook_timeout: Duration::from_millis(50),
        build_timeout: Duration::from_secs(5),
        max_suspension: Duration::from_secs(30),
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

    let config = temp_config(&dir, dir.join("host"), String::new());
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
async fn test_pipeline_aborts_before_suspending_on_bad_dll_hash() {
    let temp_dir = std::env::temp_dir().join("dekan_test_pipeline_hash");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let host_file = temp_dir.join("ltk_patcher_host.exe");
    std::fs::write(&host_file, b"test host contents").unwrap();

    let (tx, rx) = new_state_channel();
    let config = temp_config(
        &temp_dir,
        host_file,
        "0000000000000000000000000000000000000000000000000000000000000000".into(),
    );

    let pipeline = InjectionPipeline::new(config, Some(tx));
    let result = pipeline.execute(&["my_skin_mod".into()], 1234, 5678).await;

    assert!(result.is_err(), "should abort on wrong hash");
    assert!(matches!(
        rx.borrow().injection,
        InjectionStatus::Failed { .. }
    ));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[tokio::test]
async fn test_pipeline_resumes_game_when_the_overlay_cannot_be_built() {
    let temp_dir = std::env::temp_dir().join("dekan_test_pipeline_resume");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let host_file = temp_dir.join("ltk_patcher_host.exe");
    let host_bytes = b"valid audited host binary sample";
    std::fs::write(&host_file, host_bytes).unwrap();

    let (tx, rx) = new_state_channel();
    let config = temp_config(&temp_dir, host_file, compute_sha256(host_bytes));

    let pipeline = InjectionPipeline::new(config, Some(tx));
    let current_pid = std::process::id();
    let current_tid = dekan_platform::process::ProcessFinder::find_first_thread_id(current_pid)
        .unwrap()
        .unwrap();

    let result = pipeline
        .execute(&["my_skin_mod".into()], current_pid, current_tid)
        .await;

    assert!(result.is_err(), "an unbuildable overlay must fail loudly");
    assert!(
        matches!(rx.borrow().injection, InjectionStatus::Failed { .. }),
        "failure must be published, not swallowed"
    );
    assert!(
        !temp_dir.join("state").join("suspend.lock").exists(),
        "the suspension sentinel must be cleared, proving the thread was resumed"
    );

    std::fs::remove_dir_all(&temp_dir).ok();
}

fn fake_runoverlay(dir: &std::path::Path, script: &str) -> (PipelineConfig, Vec<String>) {
    let config = temp_config(dir, dir.join("dll"), String::new());
    let args = vec!["/C".to_string(), script.to_string()];
    (config, args)
}

fn cmd_exe() -> PathBuf {
    PathBuf::from(
        std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()),
    )
}

#[tokio::test]
async fn test_hook_confirmed_only_on_the_patcher_ready_line() {
    let dir = std::env::temp_dir();
    let (mut config, args) = fake_runoverlay(
        &dir,
        "echo Status: Waiting for league match to start&echo Status: Patching&echo Status: Waiting for exit&ping -n 4 127.0.0.1 >nul",
    );
    config.hook_timeout = Duration::from_secs(10);

    let config_timeout = config.hook_timeout;
    let pipeline = InjectionPipeline::new(config, None);
    let mut overlay = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn fake");

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, config_timeout).await,
        InjectionStatus::Confirmed
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_progress_lines_alone_are_not_a_confirmation() {
    let dir = std::env::temp_dir();
    let (mut config, args) = fake_runoverlay(
        &dir,
        "echo Status: Waiting for league match to start&echo Status: Patching&ping -n 6 127.0.0.1 >nul",
    );
    config.hook_timeout = Duration::from_millis(700);

    let config_timeout = config.hook_timeout;
    let pipeline = InjectionPipeline::new(config, None);
    let mut overlay = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn fake");

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, config_timeout).await,
        InjectionStatus::Unconfirmed,
        "reaching 'Patching' is progress, not a hook"
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_overlay_dying_early_is_a_failure_not_an_unconfirmed() {
    let dir = std::env::temp_dir();
    let (mut config, args) =
        fake_runoverlay(&dir, "echo Status: Waiting for league match to start");
    config.hook_timeout = Duration::from_secs(3);

    let config_timeout = config.hook_timeout;
    let pipeline = InjectionPipeline::new(config, None);
    let mut overlay = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn fake");

    assert!(
        matches!(
            pipeline.confirm_hook(&mut overlay, config_timeout).await,
            InjectionStatus::Failed { .. }
        ),
        "an overlay process that died cannot be reported as merely unconfirmed"
    );
}

#[tokio::test]
async fn test_spent_suspension_budget_resumes_unconfirmed() {
    let dir = std::env::temp_dir();
    let (config, args) = fake_runoverlay(&dir, "ping -n 4 127.0.0.1 >nul");

    let pipeline = InjectionPipeline::new(config, None);
    let mut overlay = OverlayProcess::spawn(&cmd_exe(), &args).expect("spawn fake");

    let started = std::time::Instant::now();
    let status = pipeline.confirm_hook(&mut overlay, Duration::ZERO).await;

    assert_eq!(status, InjectionStatus::Unconfirmed);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "an exhausted budget must not wait at all"
    );
    overlay.shutdown().await;
}
