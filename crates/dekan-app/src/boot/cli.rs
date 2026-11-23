use anyhow::Result;

pub async fn handle_cli_args(args: &[String]) -> Result<Option<i32>> {
    match dekan_inject::macos_patcher::run_cli_patcher(args) {
        Ok(Some(code)) => return Ok(Some(code)),
        Ok(None) => {}
        Err(e) => {
            eprintln!("macOS patcher CLI failed: {e}");
            return Ok(Some(1));
        }
    }

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

#[cfg(target_os = "macos")]
pub fn run_macos_main(
    grace: std::time::Duration,
    entry: fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>,
) -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .any(|a| a == "--macos-patcher" || a == "--native-diagnostics" || a == "--diagnostics")
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        if let Some(code) = runtime.block_on(handle_cli_args(&args))? {
            std::process::exit(code);
        }
    }
    let worker = std::thread::Builder::new()
        .name("dekan-runtime".into())
        .spawn(move || -> Result<()> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let outcome = runtime.block_on(entry());
            runtime.shutdown_timeout(grace);
            std::process::exit(i32::from(outcome.is_err()));
        })?;
    if let Err(e) = dekan_platform::run_interface_on_main_thread() {
        tracing::warn!(error = %e, "macOS main-thread interface loop returned");
    }
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("dekan-runtime thread panicked"))?
}
