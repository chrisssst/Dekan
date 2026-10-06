use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(40);

pub struct Check {
    pub scenario: &'static str,
    pub expectation: String,
    pub ok: bool,
}

fn logs_dir() -> Option<PathBuf> {
    dekan_platform::paths::logs_dir().ok()
}

fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dekan.log"))
        })
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

struct LogCursor {
    dir: PathBuf,
    start: Option<(PathBuf, u64)>,
}

impl LogCursor {
    fn here() -> Option<Self> {
        let dir = logs_dir()?;
        let start = newest_log(&dir).and_then(|p| {
            let len = std::fs::metadata(&p).ok()?.len();
            Some((p, len))
        });
        Some(Self { dir, start })
    }

    fn since(&self) -> String {
        let Some(current) = newest_log(&self.dir) else {
            return String::new();
        };
        let bytes = std::fs::read(&current).unwrap_or_default();
        let from = match &self.start {
            Some((path, len)) if *path == current => usize::try_from(*len).unwrap_or(0),
            _ => 0,
        };
        String::from_utf8_lossy(bytes.get(from..).unwrap_or_default()).into_owned()
    }

    fn wait_for(&self, needles: &[&str]) -> bool {
        let started = Instant::now();
        while started.elapsed() < WAIT {
            let text = self.since();
            if needles.iter().all(|n| text.contains(n)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        false
    }
}

fn spawn(exe: &Path) -> std::io::Result<Child> {
    Command::new(exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

fn kill(child: &mut Child) {
    let _ = child.kill(); // ignore-ok: the process may already be gone, which is the wanted state
    let _ = child.wait(); // ignore-ok: reaping a killed child; its exit status carries nothing here
}

fn window_exists(class: &str) -> bool {
    let script_path = std::env::temp_dir().join("dekan_smoke_find_window.ps1");
    let script = [
        "Add-Type -Namespace W -Name U -MemberDefinition '[DllImport(\"user32.dll\", CharSet = CharSet.Unicode)] public static extern System.IntPtr FindWindow(string c, string t);'".to_owned(),
        format!("if ([W.U]::FindWindow('{class}', [NullString]::Value) -ne [System.IntPtr]::Zero) {{ exit 0 }} else {{ exit 1 }}"),
    ]
    .join("\r\n");
    if std::fs::write(&script_path, script).is_err() {
        return false;
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        let found = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script_path)
            .status()
            .is_ok_and(|s| s.success());
        if found {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn stage(root: &Path, exe: &Path, name: &str) -> std::io::Result<PathBuf> {
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: the stage may not exist yet
    std::fs::create_dir_all(&dir)?;
    let staged = dir.join("dekan.exe");
    std::fs::copy(exe, &staged)?;
    Ok(staged)
}

fn check(checks: &mut Vec<Check>, scenario: &'static str, expectation: &str, ok: bool) {
    println!("  [{}] {expectation}", if ok { "ok" } else { "FALHA" });
    checks.push(Check {
        scenario,
        expectation: expectation.to_owned(),
        ok,
    });
}

pub fn run(exe: &Path, tools: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let root = std::env::temp_dir().join("dekan_smoke");

    let installed_injector = dekan_platform::paths::install_dir()
        .map(|dir| dir.join("tools"))
        .is_ok_and(|tools| {
            tools.join("ltk_patcher_host.exe").is_file()
                && tools.join("ltk_patcher_dll.dll").is_file()
        });
    if installed_injector {
        println!(
            "A/B nao executados: este PC tem o injetor em %ProgramFiles%/Dekan/tools, que a descoberta usa primeiro e que o Windows nao deixa isolar por variavel; a logica de recusa e coberta por dekan_app::startup"
        );
        return run_audited(exe, tools, &root, checks);
    }

    println!("A. sem a pasta tools");
    match (stage(&root, exe, "no_tools"), LogCursor::here()) {
        (Ok(staged), Some(cursor)) => match spawn(&staged) {
            Ok(mut child) => {
                let refused = cursor.wait_for(&[
                    "Dekan starting",
                    "Dekan stopped at startup: the injector files are missing",
                ]);
                check(
                    &mut checks,
                    "A",
                    "recusa iniciar e registra o motivo",
                    refused,
                );
                check(
                    &mut checks,
                    "A",
                    "nenhuma tarefa do Dekan foi iniciada",
                    !cursor.since().contains("Spawned supervised task"),
                );
                check(
                    &mut checks,
                    "A",
                    "cria a pasta tools para o usuário",
                    staged.with_file_name("tools").is_dir(),
                );
                kill(&mut child);
            }
            Err(e) => check(&mut checks, "A", &format!("executa: {e}"), false),
        },
        _ => check(&mut checks, "A", "prepara a pasta e o log", false),
    }

    println!("B. tools com arquivos que não são os auditados");
    match (stage(&root, exe, "bad_tools"), LogCursor::here()) {
        (Ok(staged), Some(cursor)) => {
            let tools_dir = staged.with_file_name("tools");
            let written = std::fs::create_dir_all(&tools_dir)
                .and_then(|()| {
                    std::fs::write(tools_dir.join("ltk_patcher_host.exe"), b"not the host")
                })
                .and_then(|()| {
                    std::fs::write(tools_dir.join("ltk_patcher_dll.dll"), b"not the dll")
                });
            match written.and_then(|()| spawn(&staged)) {
                Ok(mut child) => {
                    let refused = cursor.wait_for(&["an injector file is not the audited build"]);
                    check(
                        &mut checks,
                        "B",
                        "recusa arquivos com hash diferente",
                        refused,
                    );
                    check(
                        &mut checks,
                        "B",
                        "nenhuma tarefa do Dekan foi iniciada",
                        !cursor.since().contains("Spawned supervised task"),
                    );
                    kill(&mut child);
                }
                Err(e) => check(&mut checks, "B", &format!("executa: {e}"), false),
            }
        }
        _ => check(&mut checks, "B", "prepara a pasta e o log", false),
    }

    run_audited(exe, tools, &root, checks)
}

fn run_audited(exe: &Path, tools: &Path, root: &Path, mut checks: Vec<Check>) -> Vec<Check> {
    println!("C. injetor auditado");
    match (stage(root, exe, "good_tools"), LogCursor::here()) {
        (Ok(staged), Some(cursor)) => {
            let tools_dir = staged.with_file_name("tools");
            let copied = std::fs::create_dir_all(&tools_dir).and_then(|()| {
                for file in ["ltk_patcher_host.exe", "ltk_patcher_dll.dll"] {
                    std::fs::copy(tools.join(file), tools_dir.join(file))?;
                }
                Ok(())
            });
            match copied.and_then(|()| spawn(&staged)) {
                Ok(mut child) => {
                    let started = cursor.wait_for(&[
                        "Injection tools found in Dekan's own folder",
                        "Random skin when none is chosen setting loaded",
                        "Automatic match accept setting loaded",
                        "task=\"tray-events\"",
                        "task=\"lcu-observer\"",
                        "task=\"activation-listener\"",
                        "Overlay window and WebView surface created",
                        "task=\"injection-trigger\"",
                    ]);
                    check(
                        &mut checks,
                        "C",
                        "sobe todas as tarefas supervisionadas e o overlay",
                        started,
                    );
                    let alive = child.try_wait().is_ok_and(|status| status.is_none());
                    check(&mut checks, "C", "continua rodando", alive);
                    let text = cursor.since();
                    check(
                        &mut checks,
                        "C",
                        "nenhum ERROR no log da inicialização",
                        !text.lines().any(|l| l.contains(" ERROR ")),
                    );
                    match spawn(&staged) {
                        Ok(mut second) => {
                            let surfaced = cursor.wait_for(&["Another launch was detected"]);
                            check(
                                &mut checks,
                                "C",
                                "segunda execução chama a primeira",
                                surfaced,
                            );
                            check(
                                &mut checks,
                                "C",
                                "painel de controle aberto",
                                window_exists("DekanPanelWindowClass"),
                            );
                            let exited = second.wait().is_ok_and(|status| status.success());
                            check(&mut checks, "C", "segunda execução encerra sozinha", exited);
                        }
                        Err(e) => check(&mut checks, "C", &format!("segunda execução: {e}"), false),
                    }
                    kill(&mut child);
                }
                Err(e) => check(&mut checks, "C", &format!("executa: {e}"), false),
            }
        }
        _ => check(&mut checks, "C", "prepara a pasta e o log", false),
    }
    let _ = std::fs::remove_dir_all(root); // ignore-ok: smoke scratch folder
    checks
}
