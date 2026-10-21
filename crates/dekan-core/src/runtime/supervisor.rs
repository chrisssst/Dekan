use std::future::Future;
use std::time::Duration;
use tokio::task::{AbortHandle, JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

struct Supervised {
    name: &'static str,
    watcher: JoinHandle<()>,
    task: AbortHandle,
}

pub struct Supervisor {
    token: CancellationToken,
    handles: Vec<Supervised>,
}

impl Supervisor {
    #[must_use]
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            handles: Vec::new(),
        }
    }

    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn spawn<F, Fut>(&mut self, name: &'static str, f: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawn_with(name, false, f);
    }

    pub fn spawn_essential<F, Fut>(&mut self, name: &'static str, f: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawn_with(name, true, f);
    }

    fn spawn_with<F, Fut>(&mut self, name: &'static str, essential: bool, f: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let child_token = self.token.child_token();
        let task = tokio::spawn(f(child_token.clone()));
        let abort = task.abort_handle();
        let watcher = tokio::spawn(async move {
            match task.await {
                Ok(()) if child_token.is_cancelled() => {}
                Ok(()) if essential => error!(
                    task = name,
                    "Essential supervised task ended before shutdown; its work has stopped"
                ),
                Ok(()) => warn!(
                    task = name,
                    "Supervised task ended before shutdown; its work has stopped"
                ),
                Err(e) if e.is_cancelled() => {}
                Err(e) => error!(
                    task = name,
                    panic = %panic_message(e),
                    "Supervised task panicked; its work has stopped"
                ),
            }
        });
        self.handles.push(Supervised {
            name,
            watcher,
            task: abort,
        });
        info!(task = name, essential, "Spawned supervised task");
    }

    pub async fn shutdown(self) {
        self.shutdown_within(SHUTDOWN_GRACE).await;
    }

    async fn shutdown_within(self, grace: Duration) {
        info!("Supervisor: initiating shutdown");
        self.token.cancel();

        for supervised in self.handles {
            let abort = supervised.watcher.abort_handle();
            match tokio::time::timeout(grace, supervised.watcher).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    error!(task = supervised.name, error = %e, "Supervisor watch failed during shutdown");
                }
                Err(_) => {
                    supervised.task.abort();
                    abort.abort();
                    warn!(
                        task = supervised.name,
                        grace_ms = grace.as_millis(),
                        "Supervised task did not stop when cancelled; aborted"
                    );
                }
            }
        }

        info!("Supervisor: all tasks shut down");
    }
}

fn panic_message(error: JoinError) -> String {
    if !error.is_panic() {
        return error.to_string();
    }
    let payload = error.into_panic();
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-text panic payload".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_shutdown_cancels_every_task_and_waits_for_it() {
        let mut supervisor = Supervisor::new(CancellationToken::new());
        let finished = Arc::new(AtomicUsize::new(0));
        for name in ["first", "second"] {
            let finished = Arc::clone(&finished);
            supervisor.spawn(name, move |token| async move {
                token.cancelled().await;
                finished.fetch_add(1, Ordering::SeqCst);
            });
        }
        supervisor.shutdown().await;
        assert_eq!(finished.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_a_panicking_task_does_not_stop_the_others_from_shutting_down() {
        let mut supervisor = Supervisor::new(CancellationToken::new());
        let finished = Arc::new(AtomicUsize::new(0));
        supervisor.spawn("panics", |_token| async move {
            panic!("task failure under test");
        });
        let after = Arc::clone(&finished);
        supervisor.spawn("well-behaved", move |token| async move {
            token.cancelled().await;
            after.fetch_add(1, Ordering::SeqCst);
        });
        supervisor.shutdown().await;
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_a_task_that_ignores_cancellation_is_aborted_after_the_grace() {
        let mut supervisor = Supervisor::new(CancellationToken::new());
        let finished = Arc::new(AtomicUsize::new(0));
        let after = Arc::clone(&finished);
        supervisor.spawn("stuck", move |_token| async move {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            after.fetch_add(1, Ordering::SeqCst);
        });
        supervisor.shutdown_within(Duration::from_millis(50)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_the_token_is_the_one_the_supervisor_was_given() {
        let root = CancellationToken::new();
        let supervisor = Supervisor::new(root.clone());
        root.cancel();
        assert!(supervisor.token().is_cancelled());
    }

    #[tokio::test]
    async fn test_tasks_get_a_child_of_the_supervisor_token() {
        let root = CancellationToken::new();
        let mut supervisor = Supervisor::new(root.clone());
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        supervisor.spawn("observer", move |token| async move {
            token.cancelled().await;
            let _ = seen_tx.send(()); // ignore-ok: the receiver outlives this task in the test
        });
        assert!(!supervisor.token().is_cancelled());
        root.cancel();
        assert!(
            seen_rx.await.is_ok(),
            "cancelling the root reaches the task"
        );
        supervisor.shutdown().await;
    }
}
