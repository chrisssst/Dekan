use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, OpenEventW, SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
};
use windows::core::HSTRING;

use crate::error::PlatformError;

fn event_name(name: &str) -> HSTRING {
    HSTRING::from(format!("Local\\Dekan_Activate_{name}"))
}

pub struct ActivationListener {
    handle_raw: isize,
}

impl ActivationListener {
    pub fn create(name: &str) -> Result<Self, PlatformError> {
        let handle = unsafe { CreateEventW(None, false, false, &event_name(name)) }?;
        Ok(Self {
            handle_raw: handle.0 as isize,
        })
    }

    pub fn take_request(&self) -> bool {
        let handle = HANDLE(self.handle_raw as *mut _);

        let result = unsafe { WaitForSingleObject(handle, 0) };
        result == WAIT_OBJECT_0
    }
}

impl Drop for ActivationListener {
    fn drop(&mut self) {
        let handle = HANDLE(self.handle_raw as *mut _);

        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: nothing to recover from on teardown
        }
    }
}

pub fn request_activation(name: &str) -> Result<bool, PlatformError> {
    let handle = match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &event_name(name)) } {
        Ok(handle) => handle,
        Err(_) => return Ok(false),
    };

    let signaled = unsafe { SetEvent(handle) };

    unsafe {
        let _ = CloseHandle(handle); // ignore-ok: the handle is dropped either way
    }

    signaled?;
    Ok(true)
}

pub fn notify_already_running() {
    unsafe {
        // ignore-ok: the process is exiting regardless of which button came back
        let _ = MessageBoxW(
            None,
            &HSTRING::from(crate::i18n::text().already_running_body),
            &HSTRING::from(crate::i18n::text().already_running_title),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_without_listener_reports_no_instance() {
        let name = format!("test_absent_{}", std::process::id());
        assert!(
            !request_activation(&name).expect("opening a missing event is not an error"),
            "no listener exists, so nothing can be activated"
        );
    }

    #[test]
    fn listener_receives_exactly_one_request_per_signal() {
        let name = format!("test_activation_{}", std::process::id());
        let listener = ActivationListener::create(&name).expect("listener should be creatable");

        assert!(
            !listener.take_request(),
            "a fresh listener must start unsignaled"
        );

        assert!(
            request_activation(&name).expect("signaling an existing listener should succeed"),
            "the listener exists, so the request must be delivered"
        );
        assert!(listener.take_request(), "the signaled request must arrive");
        assert!(
            !listener.take_request(),
            "auto-reset: one signal must not yield two requests"
        );
    }

    #[test]
    fn listener_released_on_drop_stops_answering() {
        let name = format!("test_activation_drop_{}", std::process::id());
        let listener = ActivationListener::create(&name).expect("listener should be creatable");
        drop(listener);

        assert!(
            !request_activation(&name).expect("opening a closed event is not an error"),
            "the event dies with its owner, so a stale request must not be delivered"
        );
    }
}
