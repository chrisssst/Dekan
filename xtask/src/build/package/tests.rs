use super::*;

#[test]
fn the_shipped_tools_are_the_native_engine_files_dekan_loads() {
    assert_eq!(
        SHIPPED_TOOLS,
        [
            "dekan_launcher.exe",
            "dekan_bridge.dll",
            "bullet_bridge.dll"
        ]
    );
}
