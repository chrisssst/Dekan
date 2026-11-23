use super::macho::*;
use super::payload::*;

fn synthetic_macho64(cputype: i32) -> Vec<u8> {
    let mut buf = vec![0u8; 512];
    buf[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
    buf[4..8].copy_from_slice(&cputype.to_le_bytes());
    buf[16..20].copy_from_slice(&3u32.to_le_bytes());
    buf[20..24].copy_from_slice(&432u32.to_le_bytes());

    let seg_text = 32usize;
    buf[seg_text..seg_text + 4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
    buf[seg_text + 4..seg_text + 8].copy_from_slice(&232u32.to_le_bytes());
    buf[seg_text + 8..seg_text + 14].copy_from_slice(b"__TEXT");
    buf[seg_text + 24..seg_text + 32].copy_from_slice(&0x100000000u64.to_le_bytes());
    buf[seg_text + 64..seg_text + 68].copy_from_slice(&2u32.to_le_bytes());

    let sect_text = seg_text + 72;
    buf[sect_text..sect_text + 6].copy_from_slice(b"__text");
    buf[sect_text + 16..sect_text + 22].copy_from_slice(b"__TEXT");
    buf[sect_text + 32..sect_text + 40].copy_from_slice(&0x100001000u64.to_le_bytes());
    buf[sect_text + 40..sect_text + 48].copy_from_slice(&0x50000u64.to_le_bytes());
    buf[sect_text + 48..sect_text + 52].copy_from_slice(&0x1000u32.to_le_bytes());

    let sect_stubs = sect_text + 80;
    buf[sect_stubs..sect_stubs + 7].copy_from_slice(b"__stubs");
    buf[sect_stubs + 16..sect_stubs + 22].copy_from_slice(b"__TEXT");
    buf[sect_stubs + 32..sect_stubs + 40].copy_from_slice(&0x100051000u64.to_le_bytes());
    buf[sect_stubs + 40..sect_stubs + 48].copy_from_slice(&0x300u64.to_le_bytes());
    buf[sect_stubs + 48..sect_stubs + 52].copy_from_slice(&0x51000u32.to_le_bytes());

    let seg_data = seg_text + 232;
    buf[seg_data..seg_data + 4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
    buf[seg_data + 4..seg_data + 8].copy_from_slice(&152u32.to_le_bytes());
    buf[seg_data + 8..seg_data + 14].copy_from_slice(b"__DATA");
    buf[seg_data + 24..seg_data + 32].copy_from_slice(&0x100080000u64.to_le_bytes());
    buf[seg_data + 64..seg_data + 68].copy_from_slice(&1u32.to_le_bytes());

    let sect_la = seg_data + 72;
    buf[sect_la..sect_la + 15].copy_from_slice(b"__la_symbol_ptr");
    buf[sect_la + 16..sect_la + 22].copy_from_slice(b"__DATA");
    buf[sect_la + 32..sect_la + 40].copy_from_slice(&0x100080000u64.to_le_bytes());
    buf[sect_la + 40..sect_la + 48].copy_from_slice(&0x100u64.to_le_bytes());
    buf[sect_la + 48..sect_la + 52].copy_from_slice(&0x80000u32.to_le_bytes());

    let dyld = seg_data + 152;
    buf[dyld..dyld + 4].copy_from_slice(&LC_DYLD_INFO_ONLY.to_le_bytes());
    buf[dyld + 4..dyld + 8].copy_from_slice(&48u32.to_le_bytes());
    buf[dyld + 32..dyld + 36].copy_from_slice(&0x90000u32.to_le_bytes());
    buf[dyld + 36..dyld + 40].copy_from_slice(&64u32.to_le_bytes());
    buf
}

#[test]
fn parses_x86_64_and_arm64_macho_headers_and_sections() {
    let x86 = parse_macho_commands(0x100000000, &synthetic_macho64(CPU_TYPE_X86_64)).expect("x86");
    assert_eq!(x86.arch, CpuArch::X86_64);
    assert_eq!(x86.text_off, 0x1000);
    assert_eq!(x86.stubs_off, 0x51000);
    assert_eq!(x86.la_sym_off, 0x80000);
    assert_eq!(x86.lazy_bind_off, 0x90000);

    let arm = parse_macho_commands(0x100000000, &synthetic_macho64(CPU_TYPE_ARM64)).expect("arm64");
    assert_eq!(arm.arch, CpuArch::Arm64);
}

#[test]
fn resolves_fopen_from_lazy_bind_and_matches_x86_64_and_arm64_stubs() {
    let lazy_bind = [
        BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 2,
        0x10,
        BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM,
        b'_',
        b'm',
        b'a',
        b'l',
        b'l',
        b'o',
        b'c',
        0,
        BIND_OPCODE_DO_BIND,
        BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 2,
        0x28,
        BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM,
        b'_',
        b'f',
        b'o',
        b'p',
        b'e',
        b'n',
        0,
        BIND_OPCODE_DO_BIND,
    ];
    let ptr_off = find_lazy_bind_offset(&lazy_bind, 0x80000, "_fopen").expect("lazy bind");
    assert_eq!(ptr_off, 0x80028);

    let stubs_off = 0x51000u64;
    let disp = (ptr_off as i64 - (stubs_off + 6 + 6) as i64) as i32;
    let mut x86_stubs = vec![0x90u8; 12];
    x86_stubs[6] = 0xFF;
    x86_stubs[7] = 0x25;
    x86_stubs[8..12].copy_from_slice(&disp.to_le_bytes());
    assert_eq!(find_stub_x86_64(&x86_stubs, stubs_off, ptr_off), Some(6));

    let text_vmaddr = 0x100000000u64;
    let page_diff =
        (((text_vmaddr + ptr_off) & !0xFFF) as i64) - (((text_vmaddr + stubs_off) & !0xFFF) as i64);
    let imm21 = (page_diff >> 12) as u32;
    let adrp = 0x9000_0010u32 | ((imm21 & 0x3) << 29) | (((imm21 >> 2) & 0x7FFFF) << 5);
    let ldr_off = (((text_vmaddr + ptr_off) & 0xFFF) / 8) as u32;
    let ldr = 0xF940_0210u32 | (ldr_off << 10);
    let br = 0xD61F_0200u32;
    let mut arm_stubs = [0u8; 12];
    arm_stubs[0..4].copy_from_slice(&adrp.to_le_bytes());
    arm_stubs[4..8].copy_from_slice(&ldr.to_le_bytes());
    arm_stubs[8..12].copy_from_slice(&br.to_le_bytes());
    assert_eq!(
        find_stub_arm64(&arm_stubs, stubs_off, text_vmaddr, ptr_off),
        Some(0)
    );
}

#[test]
fn scans_wad_verify_signatures_and_encodes_payloads_for_both_architectures() {
    let mut x86_text = vec![0x90u8; 128];
    x86_text[16..20].copy_from_slice(&[0x55, 0x48, 0x89, 0xE5]);
    let sig_x86 = [
        0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x81, 0xEC, 0x40, 0x02, 0x00,
        0x00, 0x49, 0x89, 0xFE, 0x48, 0x8B, 0x05, 0x11, 0x22, 0x33, 0x44, 0x48, 0x31, 0xE0, 0x48,
        0x89, 0x84, 0x24, 0x30, 0x02, 0x00, 0x00, 0x48, 0x85, 0xF6, 0x0F, 0x84, 0x80, 0x00, 0x00,
        0x00, 0x4D, 0x85,
    ];
    x86_text[20..20 + sig_x86.len()].copy_from_slice(&sig_x86);
    assert_eq!(scan_wad_verify_x86_64(&x86_text), Some(16));

    let mut arm_text = vec![0u8; 128];
    let sig_arm = [
        0xFF, 0x83, 0x02, 0xD1, 0xFA, 0x67, 0x04, 0xA9, 0xF8, 0x5F, 0x05, 0xA9, 0xF6, 0x57, 0x06,
        0xA9, 0xF4, 0x4F, 0x07, 0xA9, 0xFD, 0x7B, 0x08, 0xA9, 0xFD, 0x03, 0x02, 0x91, 0x01, 0x02,
        0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0xE8, 0x1F, 0x00, 0xF9, 0x94,
        0x02, 0x00, 0xB4,
    ];
    arm_text[32..32 + sig_arm.len()].copy_from_slice(&sig_arm);
    assert_eq!(scan_wad_verify_arm64(&arm_text), Some(32));

    let p_x86 = build_payload_x86_64(0x7FFF_1234_5678, "/tmp/dekan_ovl/").expect("x86 payload");
    assert!(p_x86.len() > 264);
    let p_arm = build_payload_arm64(0x7FFF_1234_5678, "/tmp/dekan_ovl/").expect("arm64 payload");
    assert!(p_arm.len() > 264);

    let jmp_x86 = encode_x86_64_stub_jmp(0x100051000, 0x100002010).expect("x86 jmp");
    assert_eq!(jmp_x86[0], 0xE9);
    assert_eq!(jmp_x86[5], 0x90);

    let jmp_arm = encode_arm64_stub_jmp(0x100051000, 0x100002010).expect("arm64 jmp");
    assert_eq!(&jmp_arm[8..12], &0xD61F_0200u32.to_le_bytes());
}
