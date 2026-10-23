use std::io::ErrorKind;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

use tracing::debug;

use crate::error::PlatformError;

fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dekan-activate-{name}.sock"))
}

pub struct ActivationListener {
    socket: UnixDatagram,
    path: PathBuf,
}

impl ActivationListener {
    pub fn create(name: &str) -> Result<Self, PlatformError> {
        let path = socket_path(name);
        let _ = std::fs::remove_file(&path); // ignore-ok: a socket left by a dead instance; bind reports the real outcome
        let socket = UnixDatagram::bind(&path).map_err(|source| PlatformError::Io {
            context: format!("binding the activation socket {}", path.display()),
            source,
        })?;
        socket
            .set_nonblocking(true)
            .map_err(|source| PlatformError::Io {
                context: "making the activation socket non-blocking".into(),
                source,
            })?;
        Ok(Self { socket, path })
    }

    pub fn take_request(&self) -> bool {
        let mut buffer = [0u8; 16];
        let mut requested = false;
        while self.socket.recv(&mut buffer).is_ok() {
            requested = true;
        }
        requested
    }
}

impl Drop for ActivationListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path); // ignore-ok: nothing to recover from on teardown
    }
}

pub fn request_activation(name: &str) -> Result<bool, PlatformError> {
    let socket = UnixDatagram::unbound().map_err(|source| PlatformError::Io {
        context: "creating the activation request socket".into(),
        source,
    })?;
    match socket.send_to(b"activate", socket_path(name)) {
        Ok(_) => Ok(true),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            debug!(error = %e, "No running instance is listening for activation");
            Ok(false)
        }
        Err(source) => Err(PlatformError::Io {
            context: "sending the activation request".into(),
            source,
        }),
    }
}

pub fn notify_already_running() {
    let text = crate::i18n::text();
    crate::shell::message_box(text.already_running_title, text.already_running_body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reaches_the_listener_once() {
        let name = format!("probe_{}", std::process::id());
        let listener = ActivationListener::create(&name).expect("listener");
        assert!(!listener.take_request());
        assert!(request_activation(&name).expect("send"));
        assert!(listener.take_request());
        assert!(!listener.take_request());
        drop(listener);
        assert!(!request_activation(&name).expect("no listener"));
    }
}
