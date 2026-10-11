use super::*;

#[test]
fn the_user_supplied_tools_are_the_injector_files_dekan_loads() {
    assert_eq!(
        USER_SUPPLIED_TOOLS,
        ["ltk_patcher_host.exe", "ltk_patcher_dll.dll"]
    );
}
