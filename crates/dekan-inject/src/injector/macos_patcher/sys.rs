use super::macho::CpuArch;

#[cfg(not(target_os = "macos"))]
pub fn patch_live_process(_pid: u32, _prefix: &str) -> Result<CpuArch, String> {
    Err("macOS Mach-O live process patching is only supported on macOS".into())
}

#[cfg(target_os = "macos")]
pub fn patch_live_process(pid: u32, prefix: &str) -> Result<CpuArch, String> {
    imp::patch_live_process(pid, prefix)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::super::macho::{
        CpuArch, MH_MAGIC_64, find_lazy_bind_offset, find_stub_arm64, find_stub_x86_64,
        parse_macho_commands, scan_wad_verify_arm64, scan_wad_verify_x86_64,
    };
    use super::super::payload::{
        WAD_VERIFY_RET1_ARM64, WAD_VERIFY_RET1_X86_64, build_payload_arm64, build_payload_x86_64,
        encode_arm64_stub_jmp, encode_x86_64_stub_jmp,
    };

    type KernReturn = i32;
    type MachPort = u32;
    type MachVmAddress = u64;
    type MachVmSize = u64;
    type VmProt = i32;

    const KERN_SUCCESS: KernReturn = 0;
    const VM_PROT_READ: VmProt = 0x01;
    const VM_PROT_WRITE: VmProt = 0x02;
    const VM_PROT_EXECUTE: VmProt = 0x04;
    const VM_PROT_COPY: VmProt = 0x10;
    const VM_REGION_BASIC_INFO_64: i32 = 9;
    const VM_REGION_BASIC_INFO_COUNT_64: u32 = 9;

    #[repr(C)]
    #[derive(Default)]
    struct VmRegionBasicInfo64 {
        protection: VmProt,
        max_protection: VmProt,
        inheritance: u32,
        shared: i32,
        reserved: i32,
        offset: u64,
        behavior: i32,
        user_wired_count: u16,
    }

    unsafe extern "C" {
        fn mach_task_self() -> MachPort;
        fn task_for_pid(target_tport: MachPort, pid: i32, t: *mut MachPort) -> KernReturn;
        fn task_suspend(target_task: MachPort) -> KernReturn;
        fn task_resume(target_task: MachPort) -> KernReturn;
        fn mach_vm_region(
            target_task: MachPort,
            address: *mut MachVmAddress,
            size: *mut MachVmSize,
            flavor: i32,
            info: *mut VmRegionBasicInfo64,
            info_cnt: *mut u32,
            object_name: *mut MachPort,
        ) -> KernReturn;
        fn mach_vm_read_overwrite(
            target_task: MachPort,
            address: MachVmAddress,
            size: MachVmSize,
            data: MachVmAddress,
            outsize: *mut MachVmSize,
        ) -> KernReturn;
        fn mach_vm_write(
            target_task: MachPort,
            address: MachVmAddress,
            data: usize,
            data_cnt: u32,
        ) -> KernReturn;
        fn mach_vm_protect(
            target_task: MachPort,
            address: MachVmAddress,
            size: MachVmSize,
            set_maximum: i32,
            new_protection: VmProt,
        ) -> KernReturn;
        fn sys_icache_invalidate(start: *const libc::c_void, len: usize);
    }

    struct TaskSuspendGuard(MachPort);

    impl Drop for TaskSuspendGuard {
        fn drop(&mut self) {
            let _ = unsafe { task_resume(self.0) }; // ignore-ok: resume remote task on drop
        }
    }

    fn read_mem(task: MachPort, address: u64, len: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; len];
        let mut out_size: MachVmSize = 0;
        let kr = unsafe {
            mach_vm_read_overwrite(
                task,
                address,
                len as u64,
                buf.as_mut_ptr() as u64,
                &mut out_size,
            )
        };
        if kr != KERN_SUCCESS || (out_size as usize) != len {
            return Err(format!(
                "mach_vm_read_overwrite failed at {address:#x} (len {len}, kr={kr})"
            ));
        }
        Ok(buf)
    }

    fn read_u64(task: MachPort, address: u64) -> Result<u64, String> {
        let bytes = read_mem(task, address, 8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_i32(task: MachPort, address: u64) -> Result<i32, String> {
        let bytes = read_mem(task, address, 4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn find_image_base(task: MachPort) -> Result<u64, String> {
        let mut address: MachVmAddress = 0;
        for _ in 0..4096 {
            let mut size: MachVmSize = 0;
            let mut info = VmRegionBasicInfo64::default();
            let mut count = VM_REGION_BASIC_INFO_COUNT_64;
            let mut object_name: MachPort = 0;
            let kr = unsafe {
                mach_vm_region(
                    task,
                    &mut address,
                    &mut size,
                    VM_REGION_BASIC_INFO_64,
                    &mut info,
                    &mut count,
                    &mut object_name,
                )
            };
            if kr != KERN_SUCCESS {
                break;
            }
            if let Ok(magic_bytes) = read_mem(task, address, 4) {
                let magic = u32::from_le_bytes([
                    magic_bytes[0],
                    magic_bytes[1],
                    magic_bytes[2],
                    magic_bytes[3],
                ]);
                if magic == MH_MAGIC_64 {
                    return Ok(address);
                }
            }
            if size == 0 {
                break;
            }
            address = address.saturating_add(size);
        }
        Err("could not locate Mach-O 64 image base in LeagueofLegends process".into())
    }

    fn write_code(task: MachPort, address: u64, data: &[u8]) -> Result<(), String> {
        let page_mask = 0xFFFu64;
        let page_start = address & !page_mask;
        let page_end = (address + data.len() as u64 + page_mask) & !page_mask;
        let page_len = page_end - page_start;

        let kr = unsafe {
            mach_vm_protect(
                task,
                page_start,
                page_len,
                0,
                VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY,
            )
        };
        if kr != KERN_SUCCESS {
            return Err(format!(
                "mach_vm_protect(RW+COPY) failed at {page_start:#x} (kr={kr})"
            ));
        }

        let write_kr =
            unsafe { mach_vm_write(task, address, data.as_ptr() as usize, data.len() as u32) };

        let restore_kr = unsafe {
            mach_vm_protect(
                task,
                page_start,
                page_len,
                0,
                VM_PROT_READ | VM_PROT_EXECUTE,
            )
        };
        unsafe {
            sys_icache_invalidate(address as *const libc::c_void, data.len());
        }

        if write_kr != KERN_SUCCESS {
            return Err(format!(
                "mach_vm_write failed at {address:#x} (kr={write_kr})"
            ));
        }
        if restore_kr != KERN_SUCCESS {
            return Err(format!(
                "mach_vm_protect(RX) failed at {page_start:#x} (kr={restore_kr})"
            ));
        }
        Ok(())
    }

    pub fn patch_live_process(pid: u32, prefix: &str) -> Result<CpuArch, String> {
        let raw_pid = i32::try_from(pid).map_err(|_| "invalid PID".to_owned())?;
        let mut task: MachPort = 0;
        let kr = unsafe { task_for_pid(mach_task_self(), raw_pid, &mut task) };
        if kr != KERN_SUCCESS {
            return Err(format!(
                "task_for_pid({pid}) failed (kr={kr}); administrator privileges are required"
            ));
        }

        let base = find_image_base(task)?;
        let header = read_mem(task, base, 32)?;
        let sizeofcmds =
            u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let header_and_cmds = read_mem(task, base, 32usize.saturating_add(sizeofcmds))?;
        let layout = parse_macho_commands(base, &header_and_cmds)?;

        let lazy_bind = read_mem(
            task,
            base.wrapping_add(u64::from(layout.lazy_bind_off)),
            layout.lazy_bind_size as usize,
        )?;
        let fopen_ptr_off = find_lazy_bind_offset(&lazy_bind, layout.la_sym_off, "_fopen")
            .ok_or_else(|| "could not find _fopen in Mach-O lazy bind table".to_owned())?;

        let stubs = read_mem(task, layout.stubs_addr, layout.stubs_size as usize)?;
        let stub_rel = match layout.arch {
            CpuArch::X86_64 => find_stub_x86_64(&stubs, layout.stubs_off, fopen_ptr_off),
            CpuArch::Arm64 => {
                find_stub_arm64(&stubs, layout.stubs_off, layout.text_vmaddr, fopen_ptr_off)
            }
        }
        .ok_or_else(|| "could not find _fopen stub in __TEXT.__stubs".to_owned())?;
        let fopen_stub_addr = layout.stubs_addr.wrapping_add(stub_rel);

        let chunk_size = 1024 * 1024usize;
        let overlap = 64usize;
        let mut offset = 0u64;
        let mut wad_verify_addr = None;
        while offset < layout.text_size {
            let read_len = ((layout.text_size - offset) as usize).min(chunk_size);
            if let Ok(chunk) = read_mem(task, layout.text_addr.wrapping_add(offset), read_len) {
                let found = match layout.arch {
                    CpuArch::X86_64 => scan_wad_verify_x86_64(&chunk),
                    CpuArch::Arm64 => scan_wad_verify_arm64(&chunk),
                };
                if let Some(rel) = found {
                    wad_verify_addr = Some(
                        layout
                            .text_addr
                            .wrapping_add(offset)
                            .wrapping_add(rel as u64),
                    );
                    break;
                }
            }
            if read_len <= overlap {
                break;
            }
            offset = offset.saturating_add((read_len - overlap) as u64);
        }
        let wad_verify = wad_verify_addr
            .ok_or_else(|| "could not find wad_verify signature in __TEXT.__text".to_owned())?;

        let fopen_orig_addr = match layout.arch {
            CpuArch::X86_64 => {
                let disp = read_i32(task, fopen_stub_addr.wrapping_add(2))?;
                let ptr_addr =
                    (fopen_stub_addr.wrapping_add(6) as i64).wrapping_add(i64::from(disp)) as u64;
                read_u64(task, ptr_addr)?
            }
            CpuArch::Arm64 => read_u64(task, base.wrapping_add(fopen_ptr_off))?,
        };

        let hook_fn_addr = wad_verify.wrapping_add(0x10);
        let payload = match layout.arch {
            CpuArch::X86_64 => build_payload_x86_64(fopen_orig_addr, prefix)?,
            CpuArch::Arm64 => build_payload_arm64(fopen_orig_addr, prefix)?,
        };

        let suspend_kr = unsafe { task_suspend(task) };
        let _guard = (suspend_kr == KERN_SUCCESS).then_some(TaskSuspendGuard(task));

        match layout.arch {
            CpuArch::X86_64 => {
                let jmp = encode_x86_64_stub_jmp(fopen_stub_addr, hook_fn_addr)?;
                write_code(task, wad_verify, &WAD_VERIFY_RET1_X86_64)?;
                write_code(task, hook_fn_addr, &payload)?;
                write_code(task, fopen_stub_addr, &jmp)?;
            }
            CpuArch::Arm64 => {
                let jmp = encode_arm64_stub_jmp(fopen_stub_addr, hook_fn_addr)?;
                write_code(task, wad_verify, &WAD_VERIFY_RET1_ARM64)?;
                write_code(task, hook_fn_addr, &payload)?;
                write_code(task, fopen_stub_addr, &jmp)?;
            }
        }

        Ok(layout.arch)
    }
}
