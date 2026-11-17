use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_STYLE, GetParent, GetWindow, GetWindowThreadProcessId, IsWindow,
    IsWindowVisible, SET_WINDOW_POS_FLAGS, SHOW_WINDOW_CMD, SW_HIDE, SWP_SHOWWINDOW, ShowWindow,
    WINDOW_LONG_PTR_INDEX, WINDOWPLACEMENT, WS_VISIBLE,
};
use windows::core::{BOOL, PCSTR, PCWSTR, s};

type FnShowWindow = unsafe extern "system" fn(HWND, SHOW_WINDOW_CMD) -> BOOL;
type FnShowWindowAsync = unsafe extern "system" fn(HWND, SHOW_WINDOW_CMD) -> BOOL;
type FnSetWindowPos =
    unsafe extern "system" fn(HWND, HWND, i32, i32, i32, i32, SET_WINDOW_POS_FLAGS) -> BOOL;
type FnSetWindowPlacement = unsafe extern "system" fn(HWND, *const WINDOWPLACEMENT) -> BOOL;
type FnSetWindowLongPtrW = unsafe extern "system" fn(HWND, WINDOW_LONG_PTR_INDEX, isize) -> isize;
type FnSetWindowLongPtrA = unsafe extern "system" fn(HWND, WINDOW_LONG_PTR_INDEX, isize) -> isize;
type FnSetWindowLongW = unsafe extern "system" fn(HWND, WINDOW_LONG_PTR_INDEX, i32) -> i32;
type FnSetWindowLongA = unsafe extern "system" fn(HWND, WINDOW_LONG_PTR_INDEX, i32) -> i32;

static mut REAL_SHOW_WINDOW: Option<FnShowWindow> = None;
static mut REAL_SHOW_WINDOW_ASYNC: Option<FnShowWindowAsync> = None;
static mut REAL_SET_WINDOW_POS: Option<FnSetWindowPos> = None;
static mut REAL_SET_WINDOW_PLACEMENT: Option<FnSetWindowPlacement> = None;
static mut REAL_SET_WINDOW_LONG_PTR_W: Option<FnSetWindowLongPtrW> = None;
static mut REAL_SET_WINDOW_LONG_PTR_A: Option<FnSetWindowLongPtrA> = None;
static mut REAL_SET_WINDOW_LONG_W: Option<FnSetWindowLongW> = None;
static mut REAL_SET_WINDOW_LONG_A: Option<FnSetWindowLongA> = None;

static HIDE_RUNNING: AtomicBool = AtomicBool::new(false);

unsafe fn is_own_top_level_window(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    if !IsWindow(Some(hwnd)).as_bool() {
        return false;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != GetCurrentProcessId() {
        return false;
    }
    GetWindow(hwnd, GW_OWNER).is_err() && GetParent(hwnd).is_err()
}

unsafe extern "system" fn show_window_hidden(hwnd: HWND, cmd: SHOW_WINDOW_CMD) -> BOOL {
    if cmd != SW_HIDE && is_own_top_level_window(hwnd) {
        if let Some(real) = REAL_SHOW_WINDOW {
            return real(hwnd, SW_HIDE);
        }
    }
    if let Some(real) = REAL_SHOW_WINDOW {
        real(hwnd, cmd)
    } else {
        BOOL(0)
    }
}

unsafe extern "system" fn show_window_async_hidden(hwnd: HWND, cmd: SHOW_WINDOW_CMD) -> BOOL {
    if cmd != SW_HIDE && is_own_top_level_window(hwnd) {
        if let Some(real) = REAL_SHOW_WINDOW_ASYNC {
            return real(hwnd, SW_HIDE);
        }
    }
    if let Some(real) = REAL_SHOW_WINDOW_ASYNC {
        real(hwnd, cmd)
    } else {
        BOOL(0)
    }
}

unsafe extern "system" fn set_window_pos_hidden(
    hwnd: HWND,
    after: HWND,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    mut flags: SET_WINDOW_POS_FLAGS,
) -> BOOL {
    if (flags & SWP_SHOWWINDOW) == SWP_SHOWWINDOW && is_own_top_level_window(hwnd) {
        flags = SET_WINDOW_POS_FLAGS(flags.0 & !SWP_SHOWWINDOW.0);
    }
    if let Some(real) = REAL_SET_WINDOW_POS {
        real(hwnd, after, x, y, cx, cy, flags)
    } else {
        BOOL(0)
    }
}

unsafe extern "system" fn set_window_placement_hidden(
    hwnd: HWND,
    placement: *const WINDOWPLACEMENT,
) -> BOOL {
    if !placement.is_null()
        && (*placement).showCmd != SW_HIDE.0 as u32
        && is_own_top_level_window(hwnd)
    {
        let mut hidden = *placement;
        hidden.showCmd = SW_HIDE.0 as u32;
        if let Some(real) = REAL_SET_WINDOW_PLACEMENT {
            return real(hwnd, &hidden);
        }
    }
    if let Some(real) = REAL_SET_WINDOW_PLACEMENT {
        real(hwnd, placement)
    } else {
        BOOL(0)
    }
}

unsafe extern "system" fn set_window_long_ptr_w_hidden(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    mut value: isize,
) -> isize {
    if index == GWL_STYLE && (value as u32 & WS_VISIBLE.0) != 0 && is_own_top_level_window(hwnd) {
        value &= !(WS_VISIBLE.0 as isize);
    }
    if let Some(real) = REAL_SET_WINDOW_LONG_PTR_W {
        real(hwnd, index, value)
    } else {
        0
    }
}

unsafe extern "system" fn set_window_long_ptr_a_hidden(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    mut value: isize,
) -> isize {
    if index == GWL_STYLE && (value as u32 & WS_VISIBLE.0) != 0 && is_own_top_level_window(hwnd) {
        value &= !(WS_VISIBLE.0 as isize);
    }
    if let Some(real) = REAL_SET_WINDOW_LONG_PTR_A {
        real(hwnd, index, value)
    } else {
        0
    }
}

unsafe extern "system" fn set_window_long_w_hidden(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    mut value: i32,
) -> i32 {
    if index == GWL_STYLE && (value as u32 & WS_VISIBLE.0) != 0 && is_own_top_level_window(hwnd) {
        value &= !(WS_VISIBLE.0 as i32);
    }
    if let Some(real) = REAL_SET_WINDOW_LONG_W {
        real(hwnd, index, value)
    } else {
        0
    }
}

unsafe extern "system" fn set_window_long_a_hidden(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    mut value: i32,
) -> i32 {
    if index == GWL_STYLE && (value as u32 & WS_VISIBLE.0) != 0 && is_own_top_level_window(hwnd) {
        value &= !(WS_VISIBLE.0 as i32);
    }
    if let Some(real) = REAL_SET_WINDOW_LONG_A {
        real(hwnd, index, value)
    } else {
        0
    }
}

#[repr(C)]
struct ImageDosHeader {
    e_magic: u16,
    _pad: [u8; 58],
    e_lfanew: i32,
}

#[repr(C)]
struct ImageImportDescriptor {
    original_first_thunk: u32,
    time_date_stamp: u32,
    forwarder_chain: u32,
    name: u32,
    first_thunk: u32,
}

unsafe fn patch_import(module: HMODULE, api: PCSTR, detour: usize, real: &mut usize) -> usize {
    let target = match GetProcAddress(module, api) {
        Some(proc) => proc as usize,
        None => return 0,
    };
    *real = target;

    let base = match GetModuleHandleW(None) {
        Ok(h) => h.0 as *const u8,
        Err(_) => return 0,
    };
    if base.is_null() {
        return 0;
    }

    let dos = &*(base as *const ImageDosHeader);
    if dos.e_magic != 0x5A4D {
        return 0;
    }

    let nt_ptr = base.offset(dos.e_lfanew as isize);
    let signature = *(nt_ptr as *const u32);
    if signature != 0x00004550 {
        return 0;
    }

    let import_dir_ptr = nt_ptr.offset(144);
    let import_rva = *(import_dir_ptr as *const u32);
    let import_size = *(import_dir_ptr.offset(4) as *const u32);
    if import_rva == 0 || import_size == 0 {
        return 0;
    }

    let mut desc = base.offset(import_rva as isize) as *const ImageImportDescriptor;
    let mut patched = 0;

    while (*desc).name != 0 {
        let mut slot = base.offset((*desc).first_thunk as isize) as *mut usize;
        while *slot != 0 {
            if *slot == target {
                let mut old_protect = PAGE_PROTECTION_FLAGS::default();
                if VirtualProtect(
                    slot as _,
                    std::mem::size_of::<usize>(),
                    PAGE_READWRITE,
                    &mut old_protect,
                )
                .is_ok()
                {
                    *slot = detour;
                    let size = std::mem::size_of::<usize>();
                    let _ = VirtualProtect(slot.cast(), size, old_protect, &mut old_protect); // ignore-ok: restore memory protection
                    patched += 1;
                }
            }
            slot = slot.offset(1);
        }
        desc = desc.offset(1);
    }

    patched
}

#[allow(clippy::missing_safety_doc)]
pub unsafe fn patch_window_show_imports() -> bool {
    let user32_name: Vec<u16> = "user32.dll\0".encode_utf16().collect();
    let user32 = match GetModuleHandleW(PCWSTR(user32_name.as_ptr())) {
        Ok(m) => m,
        Err(_) => return false,
    };

    let mut real_ptr = 0usize;

    if patch_import(
        user32,
        s!("ShowWindow"),
        show_window_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SHOW_WINDOW = Some(std::mem::transmute::<usize, FnShowWindow>(real_ptr));
    }
    if patch_import(
        user32,
        s!("ShowWindowAsync"),
        show_window_async_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SHOW_WINDOW_ASYNC = Some(std::mem::transmute::<usize, FnShowWindowAsync>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowPos"),
        set_window_pos_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_POS = Some(std::mem::transmute::<usize, FnSetWindowPos>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowPlacement"),
        set_window_placement_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_PLACEMENT =
            Some(std::mem::transmute::<usize, FnSetWindowPlacement>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowLongPtrW"),
        set_window_long_ptr_w_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_LONG_PTR_W =
            Some(std::mem::transmute::<usize, FnSetWindowLongPtrW>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowLongPtrA"),
        set_window_long_ptr_a_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_LONG_PTR_A =
            Some(std::mem::transmute::<usize, FnSetWindowLongPtrA>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowLongW"),
        set_window_long_w_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_LONG_W = Some(std::mem::transmute::<usize, FnSetWindowLongW>(real_ptr));
    }
    if patch_import(
        user32,
        s!("SetWindowLongA"),
        set_window_long_a_hidden as *const () as usize,
        &mut real_ptr,
    ) > 0
    {
        REAL_SET_WINDOW_LONG_A = Some(std::mem::transmute::<usize, FnSetWindowLongA>(real_ptr));
    }

    true
}

unsafe extern "system" fn hide_one_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let pid = lparam.0 as u32;
    let mut wpid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    if wpid != pid {
        return BOOL(1);
    }
    if GetWindow(hwnd, GW_OWNER).is_ok() {
        return BOOL(1);
    }
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let _ = ShowWindow(hwnd, SW_HIDE); // ignore-ok: best effort hide
    BOOL(1)
}

pub fn spawn_hide_thread(stop_flag: &'static AtomicBool) {
    if HIDE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let pid = unsafe { GetCurrentProcessId() };
        while !stop_flag.load(Ordering::Relaxed) {
            unsafe {
                let _ = EnumWindows(Some(hide_one_window_proc), LPARAM(pid as isize)); // ignore-ok: periodic enum
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        HIDE_RUNNING.store(false, Ordering::SeqCst);
    });
}
