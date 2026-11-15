use super::*;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn test_mock_provider() {
    let mock = MockProvider::default();
    assert_eq!(mock.name(), "mock");
    let status = mock.check_compatibility(&PathBuf::from("a"), &PathBuf::from("b"));
    assert_eq!(status, ProviderStatus::Ready);
}

#[tokio::test]
async fn test_mock_provider_execute() {
    let mock = MockProvider {
        status_to_return: Some(InjectionStatus::Confirmed),
    };
    let config = PipelineConfig {
        provider_kind: ProviderKind::Mock,
        native_host_exe: None,
        native_dll_path: None,
        native_target: None,
        overlay_config: crate::overlay::OverlayConfig {
            overlay_dir: PathBuf::from("overlay"),
            mods_dir: PathBuf::from("mods"),
            game_dir: PathBuf::from("game"),
        },
        hook_timeout: Duration::from_secs(5),
        build_timeout: Duration::from_secs(5),
        late_budget: Duration::from_secs(5),
    };

    let outcome = mock.execute(&config, &[], 1234).await.expect("execute");
    assert_eq!(outcome.status, InjectionStatus::Confirmed);
}

#[test]
fn test_native_provider_compatibility_missing_binaries() {
    let provider = DekanNativeProvider::new();
    assert_eq!(provider.name(), "dekan-native");
    let status = provider.check_compatibility(
        &PathBuf::from(r"C:\NonExistent\host.exe"),
        &PathBuf::from(r"C:\NonExistent\dll.dll"),
    );
    assert!(matches!(status, ProviderStatus::Unsupported { .. }));
}

#[test]
fn test_native_provider_compatibility_present_binaries() {
    let dir = std::env::temp_dir().join(format!("dekan_test_provider_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let host = dir.join("host.exe");
    let dll = dir.join("dll.dll");
    std::fs::write(&host, b"host").expect("write host");
    std::fs::write(&dll, b"dll").expect("write dll");

    let provider = DekanNativeProvider::new();
    let status = provider.check_compatibility(&host, &dll);
    assert!(matches!(status, ProviderStatus::LaboratoryOnly { .. }));

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup test fixture
}

#[tokio::test]
async fn test_native_provider_refuses_fake_confirmed_execution() {
    let provider = DekanNativeProvider::new();
    let config = PipelineConfig {
        provider_kind: ProviderKind::Mock,
        native_host_exe: None,
        native_dll_path: None,
        native_target: None,
        overlay_config: crate::overlay::OverlayConfig {
            overlay_dir: PathBuf::from("overlay"),
            mods_dir: PathBuf::from("mods"),
            game_dir: PathBuf::from("game"),
        },
        hook_timeout: Duration::from_secs(5),
        build_timeout: Duration::from_secs(5),
        late_budget: Duration::from_secs(5),
    };

    let result = provider.execute(&config, &[], 9999).await;
    assert!(
        result.is_err(),
        "Native provider must not report Confirmed on unverified target"
    );
    match result {
        Err(InjectError::Process(msg)) => {
            assert!(msg.contains("NATIVE_GAME_TARGET_MISSING"));
        }
        Err(other) => panic!("Expected InjectError::Process, got {other}"),
        Ok(_) => panic!("Expected error, got Ok outcome"),
    }
}
