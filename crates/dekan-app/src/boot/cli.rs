use anyhow::Result;

pub async fn handle_cli_args(args: &[String]) -> Result<Option<i32>> {
    if args
        .iter()
        .any(|a| a == "--native-diagnostics" || a == "--diagnostics")
    {
        let exe = std::env::current_exe()?;
        let tools = exe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Executable has no parent directory"))?;
        let output = match args.iter().position(|a| a == "--diagnostics-output") {
            Some(index) => std::path::PathBuf::from(
                args.get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("--diagnostics-output requires a path"))?,
            ),
            None => std::env::current_dir()?.join("native-integration-diagnostics.json"),
        };
        if let Err(e) = dekan_app::native_integration::run_native_diagnostics(tools, &output).await
        {
            eprintln!("Diagnostic execution failed: {e}");
            return Ok(Some(1));
        }
        return Ok(Some(0));
    }

    Ok(None)
}

#[cfg(windows)]
unsafe extern "system" {
    fn AttachConsole(dw_process_id: u32) -> i32;
}

pub fn attach_parent_console() {
    #[cfg(windows)]
    unsafe {
        let _ = AttachConsole(0xFFFF_FFFF); // ignore-ok: attach console to parent when run from terminal
    }
}
