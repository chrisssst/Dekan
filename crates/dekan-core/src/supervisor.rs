use std::future::Future;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub struct Supervisor {
    token: CancellationToken,
    handles: Vec<(&'static str, JoinHandle<()>)>,
}

impl Supervisor {
    /// Create a new supervisor with the given cancellation token.
    #[must_use]
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            handles: Vec::new(),
        }
    }

    /// Get the cancellation token for this supervisor.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Spawn a named task under this supervisor.
    ///
    /// The task receives a `CancellationToken` and should select on it
    /// to support graceful shutdown.
    pub fn spawn<F, Fut>(&mut self, name: &'static str, f: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let child_token = self.token.child_token();
        let handle = tokio::spawn(f(child_token));
        self.handles.push((name, handle));
        info!(task = name, "Spawned supervised task");
    }

    pub async fn shutdown(self) {
        info!("Supervisor: initiating shutdown");
        self.token.cancel();

        for (name, handle) in self.handles {
            match handle.await {
                Ok(()) => {
                    info!(task = name, "Task completed cleanly");
                }
                Err(e) if e.is_panic() => {
                    error!(task = name, error = %e, "Task panicked during shutdown");
                }
                Err(e) => {
                    error!(task = name, error = %e, "Task failed during shutdown");
                }
            }
        }

        info!("Supervisor: all tasks shut down");
    }
}
