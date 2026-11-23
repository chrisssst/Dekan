pub const MH_MAGIC_64: u32 = 0xFEED_FACF;
pub const CPU_TYPE_X86_64: i32 = 0x0100_0007;
pub const CPU_TYPE_ARM64: i32 = 0x0100_000C;

pub const LC_SEGMENT_64: u32 = 0x19;
pub const LC_DYLD_INFO: u32 = 0x22;
pub const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;

pub const BIND_OPCODE_MASK: u8 = 0xF0;
pub const BIND_IMMEDIATE_MASK: u8 = 0x0F;
pub const BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM: u8 = 0x40;
pub const BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x70;
pub const BIND_OPCODE_DO_BIND: u8 = 0x90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    X86_64,
    Arm64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachoLayout {
    pub arch: CpuArch,
    pub text_addr: u64,
    pub text_size: u64,
    pub text_off: u64,
    pub text_vmaddr: u64,
    pub stubs_addr: u64,
    pub stubs_size: u64,
    pub stubs_off: u64,
    pub la_sym_off: u64,
    pub lazy_bind_off: u32,
    pub lazy_bind_size: u32,
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let slice = buf.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_i32_le(buf: &[u8], off: usize) -> Option<i32> {
    let slice = buf.get(off..off.checked_add(4)?)?;
    Some(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(buf: &[u8], off: usize) -> Option<u64> {
    let slice = buf.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn trim_nul(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

pub fn parse_macho_commands(image_base: u64, buf: &[u8]) -> Result<MachoLayout, String> {
    if buf.len() < 32 {
        return Err("Mach-O header buffer truncated".into());
    }
    let magic = read_u32_le(buf, 0).ok_or_else(|| "cannot read magic".to_owned())?;
    if magic != MH_MAGIC_64 {
        return Err(format!("invalid Mach-O 64 magic: {magic:#x}"));
    }
    let cputype = read_i32_le(buf, 4).ok_or_else(|| "cannot read cputype".to_owned())?;
    let arch = match cputype {
        CPU_TYPE_X86_64 => CpuArch::X86_64,
        CPU_TYPE_ARM64 => CpuArch::Arm64,
        other => return Err(format!("unsupported Mach-O CPU type: {other:#x}")),
    };
    let ncmds = read_u32_le(buf, 16).ok_or_else(|| "cannot read ncmds".to_owned())?;
    let sizeofcmds =
        read_u32_le(buf, 20).ok_or_else(|| "cannot read sizeofcmds".to_owned())? as usize;
    if buf.len() < 32usize.saturating_add(sizeofcmds) {
        return Err("Mach-O load commands truncated".into());
    }

    let mut layout = MachoLayout {
        arch,
        text_addr: 0,
        text_size: 0,
        text_off: 0,
        text_vmaddr: 0,
        stubs_addr: 0,
        stubs_size: 0,
        stubs_off: 0,
        la_sym_off: 0,
        lazy_bind_off: 0,
        lazy_bind_size: 0,
    };

    let mut off = 32usize;
    for _ in 0..ncmds {
        let cmd =
            read_u32_le(buf, off).ok_or_else(|| "load command header truncated".to_owned())?;
        let cmdsize = read_u32_le(buf, off + 4)
            .ok_or_else(|| "load command size truncated".to_owned())?
            as usize;
        if cmdsize < 8 || off.saturating_add(cmdsize) > 32usize.saturating_add(sizeofcmds) {
            return Err("invalid Mach-O load command size".into());
        }

        if cmd == LC_SEGMENT_64 && cmdsize >= 72 {
            let segname = trim_nul(&buf[off + 8..off + 24]);
            let vmaddr =
                read_u64_le(buf, off + 24).ok_or_else(|| "segment vmaddr truncated".to_owned())?;
            let nsects = read_u32_le(buf, off + 64)
                .ok_or_else(|| "segment nsects truncated".to_owned())?
                as usize;
            if segname == b"__TEXT" {
                layout.text_vmaddr = vmaddr;
            }
            for s in 0..nsects {
                let soff = off + 72 + s * 80;
                if soff + 80 > off + cmdsize {
                    break;
                }
                let sectname = trim_nul(&buf[soff..soff + 16]);
                let s_addr = read_u64_le(buf, soff + 32).unwrap_or(0);
                let s_size = read_u64_le(buf, soff + 40).unwrap_or(0);
                let s_offset = u64::from(read_u32_le(buf, soff + 48).unwrap_or(0));
                if segname == b"__TEXT" && sectname == b"__text" {
                    layout.text_addr = image_base.wrapping_add(s_offset);
                    layout.text_size = s_size;
                    layout.text_off = s_offset;
                } else if segname == b"__TEXT" && sectname == b"__stubs" {
                    layout.stubs_addr = image_base.wrapping_add(s_offset);
                    layout.stubs_size = s_size;
                    layout.stubs_off = s_offset;
                } else if (segname == b"__DATA" || segname == b"__DATA_CONST")
                    && sectname == b"__la_symbol_ptr"
                {
                    layout.la_sym_off = if layout.text_vmaddr > 0 && s_addr >= layout.text_vmaddr {
                        s_addr - layout.text_vmaddr
                    } else {
                        s_offset
                    };
                }
            }
        } else if (cmd == LC_DYLD_INFO || cmd == LC_DYLD_INFO_ONLY) && cmdsize >= 48 {
            layout.lazy_bind_off = read_u32_le(buf, off + 32).unwrap_or(0);
            layout.lazy_bind_size = read_u32_le(buf, off + 36).unwrap_or(0);
        }

        off += cmdsize;
    }

    if layout.text_size == 0 || layout.stubs_size == 0 || layout.la_sym_off == 0 {
        return Err("missing required Mach-O sections (__text, __stubs, __la_symbol_ptr)".into());
    }
    Ok(layout)
}

fn read_uleb128(data: &[u8], pos: &mut usize) -> u64 {
    let mut result = 0u64;
    let mut shift = 0u32;
    while *pos < data.len() {
        let byte = data[*pos];
        *pos += 1;
        result |= u64::from(byte & 0x7F) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift = shift.saturating_add(7);
    }
    result
}

#[must_use]
pub fn find_lazy_bind_offset(lazy_bind: &[u8], la_sym_off: u64, symbol: &str) -> Option<u64> {
    let mut pos = 0usize;
    let mut seg_offset = 0u64;
    let mut current_sym = String::new();
    while pos < lazy_bind.len() {
        let byte = lazy_bind[pos];
        pos += 1;
        let opcode = byte & BIND_OPCODE_MASK;
        let _imm = byte & BIND_IMMEDIATE_MASK;
        match opcode {
            BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                seg_offset = read_uleb128(lazy_bind, &mut pos);
            }
            BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM => {
                let start = pos;
                while pos < lazy_bind.len() && lazy_bind[pos] != 0 {
                    pos += 1;
                }
                current_sym = String::from_utf8_lossy(&lazy_bind[start..pos]).into_owned();
                if pos < lazy_bind.len() {
                    pos += 1;
                }
            }
            BIND_OPCODE_DO_BIND if current_sym == symbol => {
                return Some(la_sym_off.wrapping_add(seg_offset));
            }
            _ => {}
        }
    }
    None
}

#[must_use]
pub fn find_stub_x86_64(stubs: &[u8], stubs_off: u64, target_ptr_off: u64) -> Option<u64> {
    let mut i = 0usize;
    while i + 6 <= stubs.len() {
        if stubs[i] == 0xFF && stubs[i + 1] == 0x25 {
            let rip_rel =
                i32::from_le_bytes([stubs[i + 2], stubs[i + 3], stubs[i + 4], stubs[i + 5]]);
            let stub_off = stubs_off.wrapping_add(i as u64);
            let target = (stub_off.wrapping_add(6) as i64).wrapping_add(i64::from(rip_rel)) as u64;
            if target == target_ptr_off {
                return Some(i as u64);
            }
        }
        i += 6;
    }
    None
}

#[must_use]
pub fn find_stub_arm64(
    stubs: &[u8],
    stubs_off: u64,
    text_vmaddr: u64,
    target_ptr_off: u64,
) -> Option<u64> {
    let target_vmaddr = text_vmaddr.wrapping_add(target_ptr_off);
    let mut i = 0usize;
    while i + 12 <= stubs.len() {
        let i0 = read_u32_le(stubs, i)?;
        let i1 = read_u32_le(stubs, i + 4)?;
        let i2 = read_u32_le(stubs, i + 8)?;
        let is_adrp_x16 = (i0 & 0x9F00_001F) == 0x9000_0010;
        let is_ldr_x16 = (i1 & 0xFFC0_03FF) == 0xF940_0210;
        let is_br_x16 = i2 == 0xD61F_0200;
        if is_adrp_x16 && is_ldr_x16 && is_br_x16 {
            let immlo = i64::from((i0 >> 29) & 0x3);
            let immhi = i64::from((i0 >> 5) & 0x7FFFF);
            let mut imm = (immhi << 2) | immlo;
            if (imm & (1 << 20)) != 0 {
                imm |= !((1 << 21) - 1);
            }
            let pc = text_vmaddr.wrapping_add(stubs_off).wrapping_add(i as u64);
            let page = ((pc & !0xFFF) as i64).wrapping_add(imm << 12) as u64;
            let ldr_off = u64::from((i1 >> 10) & 0xFFF) * 8;
            if page.wrapping_add(ldr_off) == target_vmaddr {
                return Some(i as u64);
            }
        }
        i += 12;
    }
    None
}

const WAD_VERIFY_PAT_X86_64: &[i16] = &[
    0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x81, 0xEC, -1, -1, 0x00, 0x00,
    0x49, 0x89, -1, 0x48, 0x8B, 0x05, -1, -1, -1, -1, 0x48, 0x31, 0xE0, 0x48, 0x89, 0x84, 0x24, -1,
    -1, 0x00, 0x00, -1, 0x85, -1, 0x0F, 0x84, -1, -1, 0x00, 0x00, 0x4D, 0x85,
];

const WAD_VERIFY_PAT_ARM64: &[i16] = &[
    0xFF, -1, -1, 0xD1, -1, -1, -1, 0xA9, -1, -1, -1, 0xA9, -1, -1, -1, 0xA9, -1, -1, -1, 0xA9, -1,
    -1, -1, 0xA9, 0xFD, -1, -1, 0x91, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0xF9, -1, -1, 0x00, 0xB4,
];

fn scan_pattern(haystack: &[u8], pattern: &[i16], step: usize) -> Option<usize> {
    if haystack.len() < pattern.len() {
        return None;
    }
    let limit = haystack.len() - pattern.len();
    let mut i = 0usize;
    while i <= limit {
        let matched = pattern
            .iter()
            .enumerate()
            .all(|(j, &pat)| pat < 0 || i16::from(haystack[i + j]) == pat);
        if matched {
            return Some(i);
        }
        i += step;
    }
    None
}

#[must_use]
pub fn scan_wad_verify_x86_64(text: &[u8]) -> Option<usize> {
    let idx = scan_pattern(text, WAD_VERIFY_PAT_X86_64, 1)?;
    let lookback = idx.min(16);
    for back in 1..=lookback {
        let pos = idx - back;
        if pos + 4 <= text.len() && text[pos..pos + 4] == [0x55, 0x48, 0x89, 0xE5] {
            return Some(pos);
        }
    }
    Some(idx)
}

#[must_use]
pub fn scan_wad_verify_arm64(text: &[u8]) -> Option<usize> {
    let limit = text.len().saturating_sub(WAD_VERIFY_PAT_ARM64.len());
    let mut i = 0usize;
    while i <= limit {
        let matched = WAD_VERIFY_PAT_ARM64
            .iter()
            .enumerate()
            .all(|(j, &pat)| pat < 0 || i16::from(text[i + j]) == pat);
        if matched
            && (text[i + 4] & 0xF0) == 0xF0
            && (text[i + 8] & 0xF0) == 0xF0
            && (text[i + 12] & 0xF0) == 0xF0
            && (text[i + 16] & 0xF0) == 0xF0
            && (text[i + 20] & 0xF0) == 0xF0
        {
            return Some(i);
        }
        i += 4;
    }
    None
}
