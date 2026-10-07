use std::future::Future;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub struct Supervisor {
    token: CancellationToken,
    handles: Vec<(&'static str, JoinHandle<()>)>,
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
        let child_token = self.token.child_token();
        let task = tokio::spawn(f(child_token.clone()));
        let handle = tokio::spawn(async move {
            match task.await {
                Ok(()) if child_token.is_cancelled() => {}
                Ok(()) => warn!(
                    task = name,
                    "Supervised task ended before shutdown; its work has stopped"
                ),
                Err(e) => error!(
                    task = name,
                    panic = %panic_message(e),
                    "Supervised task panicked; its work has stopped"
                ),
            }
        });
        self.handles.push((name, handle));
        info!(task = name, "Spawned supervised task");
    }

    pub async fn shutdown(self) {
        info!("Supervisor: initiating shutdown");
        self.token.cancel();

        for (name, handle) in self.handles {
            if let Err(e) = handle.await {
                error!(task = name, error = %e, "Supervisor watch failed during shutdown");
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
