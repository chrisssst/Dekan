use std::sync::{Arc, Mutex};

use dekan_core::supervisor::Supervisor;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer")).into_owned()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_task_that_dies_mid_session_is_reported_at_once_not_at_shutdown() {
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let mut supervisor = Supervisor::new(CancellationToken::new());
    supervisor.spawn("injection-trigger", |_token| async move {
        panic!("overlay state lost");
    });
    supervisor.spawn("returns-early", |_token| async move {});
    supervisor.spawn(
        "well-behaved",
        |token| async move { token.cancelled().await },
    );
    for _ in 0..100_000 {
        let text = logs.text();
        if text.contains("panicked") && text.contains("ended before shutdown") {
            break;
        }
        tokio::task::yield_now().await;
    }

    let text = logs.text();
    let trigger = reports(&text, "injection-trigger");
    assert!(
        trigger.iter().any(|l| l.contains("ERROR")
            && l.contains("panicked")
            && l.contains("overlay state lost")),
        "{text}"
    );
    assert!(
        reports(&text, "returns-early")
            .iter()
            .any(|l| l.contains("WARN") && l.contains("ended before shutdown")),
        "{text}"
    );
    assert!(
        reports(&text, "well-behaved").is_empty(),
        "a task still running is not reported: {text}"
    );

    supervisor.shutdown().await;
    let after = logs.text();
    assert!(!after.contains("watch failed"), "{after}");
    assert_eq!(
        after.matches("panicked").count(),
        1,
        "reported once: {after}"
    );
    assert!(
        reports(&after, "well-behaved").is_empty(),
        "a cancelled task is not reported: {after}"
    );
}

fn reports<'a>(text: &'a str, task: &str) -> Vec<&'a str> {
    text.lines()
        .filter(|l| l.contains(&format!("task=\"{task}\"")) && !l.contains("Spawned"))
        .collect()
}
