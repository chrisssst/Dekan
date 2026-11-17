#![allow(non_snake_case)]
#![allow(unsafe_op_in_unsafe_fn)]

pub mod cdp;
pub mod commands;
pub mod hook;
pub mod server;

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HINSTANCE, HMODULE};
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
use windows::core::BOOL;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static ATTACHED: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "system" fn DllMain(
    hinst: HINSTANCE,
    fdw_reason: u32,
    _lp_reserved: *mut std::ffi::c_void,
) -> BOOL {
    match fdw_reason {
        DLL_PROCESS_ATTACH => {
            if ATTACHED
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return BOOL(1);
            }
            let _ = DisableThreadLibraryCalls(HMODULE(hinst.0)); // ignore-ok: disable thread calls

            let no_cdp = std::env::var("DEKAN_BRIDGE_NO_CDP").is_ok()
                || std::env::var("CELESTIAL_NO_CDP").is_ok();
            if !no_cdp {
                cdp::setup_cdp_port();
            }

            server::spawn_tcp_server(&STOP_REQUESTED);
            server::spawn_auto_spoof_thread(&STOP_REQUESTED);

            let no_hide = std::env::var("DEKAN_BRIDGE_NO_HIDE").is_ok()
                || std::env::var("CELESTIAL_NO_HIDE").is_ok();
            if !no_hide {
                let _ = hook::patch_window_show_imports(); // ignore-ok: best effort hook
                hook::spawn_hide_thread(&STOP_REQUESTED);
            }
        }
        DLL_PROCESS_DETACH => {
            STOP_REQUESTED.store(true, Ordering::SeqCst);
            cdp::close_cdp();
            server::clear_port_broadcast();
            ATTACHED.store(false, Ordering::SeqCst);
        }
        _ => {}
    }

    BOOL(1)
}
