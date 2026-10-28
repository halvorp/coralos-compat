//! exec's library construction: MakeLibrary, MakeFunctions, InitStruct (exec.library LVOs -84, -90, -78), used by
//! both the bridged calls (a guest init routine may call them) and the disk-library loader.
//!
//! Facts: exec/initializers.h and the InitStruct autodoc (instruction byte `iisscccc`: ii = 0 copy c+1 elements,
//! 1 repeat one element c+1 times, 2 = 1-byte offset then copy, 3 = 24-bit offset then copy; ss = element size
//! 0 LONG / 1 WORD / 2 BYTE; a 0 byte ends the table; elements and instruction bytes are word-aligned on the 68k).
//! MakeFunctions writes `JMP abs.l` ($4EF9) vectors, 6 bytes each, downwards from the base; a function list starting
//! with the WORD -1 holds 16-bit offsets from its own address, otherwise absolute pointers; both end with -1.
//! `struct Library` is 34 bytes (m68k-linux-gnu-gcc -mno-align-int over AROS exec/libraries.h): ln_Type 8,
//! ln_Name 10, lib_Flags 14, lib_NegSize 16, lib_PosSize 18, lib_Version 20, lib_Revision 22, lib_IdString 24,
//! lib_Sum 28, lib_OpenCnt 32.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::guest_call::{self, Next};
use crate::machine::{Amiga, EXEC_BASE};
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const LVO_INIT_STRUCT: i32 = -78;
pub const LVO_MAKE_LIBRARY: i32 = -84;
pub const LVO_MAKE_FUNCTIONS: i32 = -90;

pub const LIB_SIZE: u32 = 34;
pub const LIB_NEG_SIZE: u32 = 16;
pub const LIB_POS_SIZE: u32 = 18;
pub const LIB_VERSION: u32 = 20;
const JMP_ABS_L: u16 = 0x4EF9;
const VECTOR_SIZE: u32 = 6;
/// A function table with more entries than this is a corrupt table, not a library.
const MAX_VECTORS: usize = 2048;
const MAX_INIT_INSTRUCTIONS: u32 = 8192;

/// A library's memory: where it was allocated (for freeing) and its base.
pub struct Made { pub base: u32, pub block: u32, pub block_size: u32 }

fn rd<U: GuestUi>(a: &Amiga<U>, at: u32, what: &str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }

/// The function pointers of a table: relative if `rel_base` is given (WORD offsets, -1 ends), else absolute (LONGs, -1 ends).
pub fn collect_vectors<U: GuestUi>(a: &Amiga<U>, table: u32, rel_base: Option<u32>) -> Result<Vec<u32>, Refusal> {
    let mut out = Vec::new();
    let mut p = table;
    loop {
        if out.len() >= MAX_VECTORS { return Err(Refusal(format!("function table at {table:#x} has no end after {MAX_VECTORS} entries"))); }
        match rel_base {
            Some(base) => {
                let w = a.mem.read_i16(p).map_err(Refusal::mem("function table"))?;
                if w == -1 { break; }
                out.push(base.wrapping_add(w as i32 as u32));
                p += 2;
            }
            None => {
                let v = rd(a, p, "function table")?;
                if v == 0xFFFF_FFFF { break; }
                out.push(v);
                p += 4;
            }
        }
    }
    Ok(out)
}

/// Write the `JMP` vectors for `funcs` below `base`; returns the table size in bytes.
pub fn write_vectors<U: GuestUi>(a: &mut Amiga<U>, base: u32, funcs: &[u32]) -> Result<u32, Refusal> {
    for (i, &f) in funcs.iter().enumerate() {
        let at = base - VECTOR_SIZE * (i as u32 + 1);
        a.mem.write_u16(at, JMP_ABS_L).map_err(Refusal::mem("library vector"))?;
        a.mem.write_u32(at + 2, f).map_err(Refusal::mem("library vector"))?;
    }
    Ok(funcs.len() as u32 * VECTOR_SIZE)
}

/// exec InitStruct: fill `mem` (already zeroed by the caller if it wants that) from the init table at `table`.
pub fn init_struct<U: GuestUi>(a: &mut Amiga<U>, table: u32, mem: u32) -> Result<(), Refusal> {
    let (mut it, mut dst) = (table, mem);
    let align2 = |x: u32| (x + 1) & !1;
    for _ in 0..MAX_INIT_INSTRUCTIONS {
        let cmd = a.mem.read_u8(it).map_err(Refusal::mem("InitStruct table"))?;
        if cmd == 0 { return Ok(()); }
        let (t, s, cnt) = (cmd >> 6, (cmd >> 4) & 3, (cmd & 15) as u32 + 1);
        let mut offset = None;
        match t {
            2 => { offset = Some(a.mem.read_u8(it + 1).map_err(Refusal::mem("InitStruct table"))? as u32); it += 2; }
            3 => { offset = Some(rd(a, it, "InitStruct table")? & 0xFF_FFFF); it += 4; }
            _ => it += 1,
        }
        let size = match s { 0 => 4, 1 => 2, 2 => 1, _ => return Err(Refusal("InitStruct: QUAD elements are not modelled".into())) };
        if size > 1 { it = align2(it); dst = align2(dst); }
        if let Some(o) = offset { dst = mem + o; }
        let m = Refusal::mem("InitStruct");
        let get = |a: &Amiga<U>, at: u32| -> Result<u32, Refusal> {
            match size { 4 => a.mem.read_u32(at).map_err(Refusal::mem("InitStruct")), 2 => a.mem.read_u16(at).map(u32::from).map_err(Refusal::mem("InitStruct")), _ => a.mem.read_u8(at).map(u32::from).map_err(Refusal::mem("InitStruct")) }
        };
        let put = |a: &mut Amiga<U>, at: u32, v: u32| match size { 4 => a.mem.write_u32(at, v), 2 => a.mem.write_u16(at, v as u16), _ => a.mem.write_u8(at, v as u8) };
        if t == 1 {
            let v = get(a, it)?;
            it += size;
            for _ in 0..cnt { put(a, dst, v).map_err(&m)?; dst += size; }
        } else {
            for _ in 0..cnt { let v = get(a, it)?; put(a, dst, v).map_err(&m)?; it += size; dst += size; }
        }
        it = align2(it);
    }
    Err(Refusal(format!("InitStruct table at {table:#x} has no end")))
}

/// exec MakeLibrary without its init routine: allocate `neg + data_size` bytes, build the vectors, InitStruct the
/// data, set NegSize/PosSize.
pub fn make_library<U: GuestUi>(a: &mut Amiga<U>, func_init: u32, struct_init: u32, data_size: u32) -> Result<Made, Refusal> {
    if data_size < LIB_SIZE { return Err(Refusal(format!("MakeLibrary: dataSize {data_size} is smaller than a struct Library ({LIB_SIZE})"))); }
    let funcs = if func_init == 0 { Vec::new() } else if a.mem.read_u16(func_init).map_err(Refusal::mem("function table"))? == 0xFFFF {
        collect_vectors(a, func_init + 2, Some(func_init))?
    } else { collect_vectors(a, func_init, None)? };
    let neg = (funcs.len() as u32 * VECTOR_SIZE + 3) & !3;
    let total = neg.checked_add(data_size).ok_or_else(|| Refusal("MakeLibrary: size overflows".into()))?;
    let block = a.heap.alloc(total).ok_or_else(|| Refusal(format!("MakeLibrary: no guest memory for {total} bytes (NOMEM)")))?;
    a.mem.zero(block, total).map_err(Refusal::mem("MakeLibrary"))?;
    let base = block + neg;
    write_vectors(a, base, &funcs)?;
    if struct_init != 0 { init_struct(a, struct_init, base)?; }
    a.mem.write_u16(base + LIB_NEG_SIZE, neg as u16).map_err(Refusal::mem("lib_NegSize"))?;
    a.mem.write_u16(base + LIB_POS_SIZE, data_size as u16).map_err(Refusal::mem("lib_PosSize"))?;
    Ok(Made { base, block, block_size: total })
}

/// InitStruct(a1 = table, a2 = memory, d0 = size): zero `size` bytes (if non-zero), then fill from the table.
pub fn init_struct_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (table, mem, size) = (cpu.areg(1), cpu.areg(2), cpu.dreg(0) & 0xFFFF);
    a.mem.zero(mem, size).map_err(Refusal::mem("InitStruct.memory"))?;
    init_struct(a, table, mem)?;
    Ok(format!("[exec] InitStruct table={table:#x} memory={mem:#x}"))
}

/// MakeFunctions(a0 = target, a1 = funcArray, a2 = funcDispBase) -> the table size in bytes.
pub fn make_functions_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (target, array, disp) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let funcs = collect_vectors(a, array, (disp != 0).then_some(disp))?;
    let n = write_vectors(a, target, &funcs)?;
    cpu.set_d0(n);
    Ok(format!("[exec] MakeFunctions target={target:#x} {} vectors", funcs.len()))
}

/// MakeLibrary(a0 = funcInit, a1 = structInit, a2 = libInit, d0 = dataSize, d1 = segList) -> the library, or NULL.
/// A non-NULL libInit is called in the GUEST (D0 = the library, A0 = segList, A6 = ExecBase) and its D0 is the result.
pub fn make_library_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (func_init, struct_init, lib_init, data_size, seg) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.dreg(0), cpu.dreg(1));
    let made = make_library(a, func_init, struct_init, data_size)?;
    let next = if lib_init == 0 { Next::Return(made.base) } else {
        Next::Call { target: lib_init, d0: made.base, aregs: alloc::vec![(0, seg), (6, EXEC_BASE)], then: Box::new(|_, _, d0| Ok(Next::Return(d0))) }
    };
    guest_call::start(a, cpu, next)?;
    Ok(format!("[exec] MakeLibrary -> {:#x}{}", made.base, if lib_init == 0 { "" } else { " (calling its init routine in the guest)" }))
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.func(LVO_INIT_STRUCT, "InitStruct", init_struct_lvo::<U>)
        .func(LVO_MAKE_LIBRARY, "MakeLibrary", make_library_lvo::<U>)
        .func(LVO_MAKE_FUNCTIONS, "MakeFunctions", make_functions_lvo::<U>)
}
