use super::*;
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

fn temp_config(dir: &std::path::Path, host_exe: PathBuf) -> PipelineConfig {
    PipelineConfig {
        ltk_host_exe: host_exe,
        ltk_dll_path: dir.join("ltk_patcher_dll.dll"),
        ltk_flags: 0,
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
async fn test_pipeline_aborts_before_building_on_an_unsigned_injector() {
    let temp_dir = std::env::temp_dir().join("dekan_test_pipeline_hash");
    std::fs::create_dir_all(&temp_dir).unwrap();

    let host_file = temp_dir.join("ltk_patcher_host.exe");
    std::fs::write(&host_file, b"test host contents").unwrap();

    let (tx, rx) = new_state_channel();
    let config = temp_config(&temp_dir, host_file);

    let pipeline = InjectionPipeline::new(config, Some(tx));
    let result = pipeline.execute(&["my_skin_mod".into()], 1234).await;

    assert!(
        matches!(result, Err(InjectError::UntrustedInjector { .. })),
        "an injector without the publisher's signature must stop the pipeline"
    );
    assert!(matches!(
        rx.borrow().injection,
        InjectionStatus::Failed { .. }
    ));

    std::fs::remove_dir_all(&temp_dir).ok();
}

async fn fake_host(
    name: &str,
    script: &[&str],
    hook_timeout: Duration,
) -> (InjectionPipeline, OverlayProcess) {
    let dir = std::env::temp_dir().join(format!("dekan_fake_host_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let host = dir.join("fake_host.cmd");
    let body: String = std::iter::once("@echo off")
        .chain(script.iter().copied())
        .map(|line| format!("{line}\r\n"))
        .collect();
    std::fs::write(&host, body).expect("fake host script");
    let mut config = temp_config(&dir, host);
    config.hook_timeout = hook_timeout;
    let pipeline = InjectionPipeline::new(config, None);
    let overlay = pipeline.spawn_patcher().await.expect("spawn fake host");
    (pipeline, overlay)
}

const STALL: &str = "ping -n 4 127.0.0.1 >nul";

#[tokio::test]
async fn test_hook_confirmed_only_when_the_host_reports_injected() {
    let budget = Duration::from_secs(10);
    let (pipeline, mut overlay) = fake_host(
        "confirmed",
        &[
            "echo status 0.01 injecting scanning for the game",
            "echo dll 1.00 1 2 INFO ltk_patcher_dll: redirected wad: Zed.wad.client",
            "echo status 1.10 injected dll attached",
            STALL,
        ],
        budget,
    )
    .await;

    assert_eq!(
        pipeline.confirm_hook(&mut overlay, budget).await,
        InjectionStatus::Confirmed
    );
    overlay.shutdown().await;
}

#[tokio::test]
async fn test_arming_and_legacy_text_alone_are_not_a_confirmation() {
    let budget = Duration::from_millis(700);
    let (pipeline, mut overlay) = fake_host(
        "progress",
        &[
            "echo status 0.01 injecting scanning for the game",
            "echo Status: Waiting for exit",
            "ping -n 6 127.0.0.1 >nul",
        ],
        budget,
    )
    .await;

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
    let (pipeline, mut overlay) = fake_host(
        "dying",
        &[
            "echo status 0.01 injecting scanning for the game",
            "ping -n 2 127.0.0.1 >nul",
        ],
        budget,
    )
    .await;

    assert!(
        matches!(
            pipeline.confirm_hook(&mut overlay, budget).await,
            InjectionStatus::Failed { .. }
        ),
        "an overlay process that died cannot be reported as merely unconfirmed"
    );
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
