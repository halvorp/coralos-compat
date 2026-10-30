// These tests build raw resource forks directly rather than assuming an
// unconfirmed heirloom_fmt_resfork::writer API. MacBinary is unnecessary:
// machine::partition_bytes also accepts a raw resource fork.
//
// API gaps: no test GuestUi implementation or public Gestalt test helper is
// visible here, so with_ram's UI-driven error path and the full Gestalt call
// path are not fabricated. LOGICAL_RAM is checked through its public API.

use heirloom_macppc_engine::machine::{partition_bytes, ram_for_program};

fn size_resource(flags: u16, preferred: u32, minimum: u32) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&flags.to_be_bytes());
    d.extend_from_slice(&preferred.to_be_bytes());
    d.extend_from_slice(&minimum.to_be_bytes());
    d
}

fn put16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn put32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

/// A minimal resource fork containing resources of one type and no names.
fn resource_fork(ty: [u8; 4], resources: &[(i16, Vec<u8>)]) -> Vec<u8> {
    let data_offset = 256usize;
    let data_len = resources.iter().map(|(_, d)| 4 + d.len()).sum::<usize>();
    let map_offset = data_offset + data_len;
    let map_len = 28 + 2 + 8 + 12 * resources.len();
    let mut out = vec![0u8; map_offset + map_len];

    put32(&mut out, 0, data_offset as u32);
    put32(&mut out, 4, map_offset as u32);
    put32(&mut out, 8, data_len as u32);
    put32(&mut out, 12, map_len as u32);

    let mut at = data_offset;
    for (_, data) in resources {
        put32(&mut out, at, data.len() as u32);
        out[at + 4..at + 4 + data.len()].copy_from_slice(data);
        at += 4 + data.len();
    }

    out.copy_within(..16, map_offset);
    put16(&mut out, map_offset + 24, 28); // type-list offset
    put16(&mut out, map_offset + 26, map_len as u16); // name-list offset
    let types = map_offset + 28;
    put16(&mut out, types, 0); // one type, minus one
    out[types + 2..types + 6].copy_from_slice(&ty);
    put16(&mut out, types + 6, resources.len() as u16 - 1);
    put16(&mut out, types + 8, 10); // reference list follows this type entry

    let refs = types + 10;
    let mut offset = 0usize;
    for (i, (id, data)) in resources.iter().enumerate() {
        let r = refs + 12 * i;
        put16(&mut out, r, *id as u16);
        put16(&mut out, r + 2, u16::MAX); // no name
        out[r + 5] = (offset >> 16) as u8;
        out[r + 6] = (offset >> 8) as u8;
        out[r + 7] = offset as u8;
        offset += 4 + data.len();
    }
    out
}

#[test]
fn partition_from_size_id0() {
    let prog = resource_fork(*b"SIZE", &[(0, size_resource(0, 156 << 20, 156 << 20))]);
    assert_eq!(partition_bytes(&prog), Some(156 << 20));
    // ram_for_program == base + 164 MiB (156 + 8), rounded to 1 MiB.
    let base = 16 << 20;
    assert_eq!(ram_for_program(base, &prog), base + (164 << 20));
}

#[test]
fn size_id_neg1_wins_over_id0() {
    let prog = resource_fork(
        *b"SIZE",
        &[
            (-1, size_resource(0, 200 << 20, 0)),
            (0, size_resource(0, 100 << 20, 0)),
        ],
    );
    assert_eq!(partition_bytes(&prog), Some(200 << 20));
}

#[test]
fn no_size_returns_none() {
    let prog = resource_fork(*b"CODE", &[(0, vec![1, 2, 3, 4])]);
    assert!(partition_bytes(&prog).is_none());
}

#[test]
fn no_size_uses_the_finder_default_partition() {
    let base = 16 << 20;
    assert_eq!(ram_for_program(base, &[]), base + (12 << 20));
}

#[test]
fn gestalt_lram_uses_logical_ram_constant() {
    assert_eq!(heirloom_macppc_engine::osroutines::LOGICAL_RAM, 512 << 20);
}
#[test]
fn a_machine_made_before_its_program_grows_to_the_programs_partition_and_never_shrinks() {
    use heirloom_macppc_engine::MacPpc;
    use heirloom_ui_abi::RecordingGuestUi;
    // os-mac.library makes the PowerPC machine before its program arrives (macppc.engine); start_file sizes it, as for_program does.
    let prog = resource_fork(*b"SIZE", &[(0, size_resource(0, 156 << 20, 156 << 20))]);
    let sized = MacPpc::for_program(RecordingGuestUi::new(), &prog).unwrap();
    let mut late = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert!(late.mac.mem.len() < sized.mac.mem.len());
    late.size_for_program(&prog).unwrap();
    assert_eq!(late.mac.mem.len(), sized.mac.mem.len());
    late.size_for_program(&[]).unwrap();
    assert_eq!(late.mac.mem.len(), sized.mac.mem.len(), "a smaller ask never shrinks guest RAM");
}
