use heirloom_mem_paged32::{Prot, Vm, VmError, PAGE_SIZE};

#[test]
fn overlap_refusal() {
    let mut vm = Vm::new();
    vm.create_chunk(0x1000, 0x3000, Prot::RW).unwrap();
    assert_eq!(
        vm.create_chunk(0x2000, 0x1000, Prot::RW),
        Err(VmError::Overlap)
    );
}

#[test]
fn sparse_commit() {
    let mut vm = Vm::new();
    let id = vm.create_chunk(0x1000, 0x4000_0000, Prot::RW).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();
    assert_eq!(vm.committed_bytes(), 4096);
}

#[test]
fn cross_page_read_write() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, 0x2000, Prot::RW).unwrap();
    vm.commit(id, 0, 0x2000).unwrap();
    vm.write_u32(base + 4092, 0x7856_3412).unwrap();
    assert_eq!(vm.read_u32(base + 4092), Ok(0x7856_3412));
}

#[test]
fn wx_fault_write_to_x() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, PAGE_SIZE, Prot::RX).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();
    assert!(matches!(
        vm.write_u32(base, 1),
        Err(VmError::WriteToExec(_))
    ));
}

#[test]
fn wx_fault_exec_of_w() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, PAGE_SIZE, Prot::RW).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();
    assert!(matches!(
        vm.fetch_u32(base),
        Err(VmError::ExecOfWrite(_))
    ));
}

#[test]
fn uncommitted_fault() {
    let mut vm = Vm::new();
    let base = 0x1000;
    vm.create_chunk(base, PAGE_SIZE, Prot::RW).unwrap();
    assert!(matches!(vm.read_u32(base), Err(VmError::Fault(_))));
}

#[test]
fn guard_fault() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, PAGE_SIZE, Prot::RW).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();
    vm.decommit(id, 0, PAGE_SIZE).unwrap();
    assert!(matches!(vm.read_u32(base), Err(VmError::Fault(_))));
}

#[test]
fn decommit() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, 0x2000, Prot::RW).unwrap();
    vm.commit(id, 0, 0x2000).unwrap();
    vm.decommit(id, 0, PAGE_SIZE).unwrap();
    assert_eq!(vm.committed_bytes(), 4096);
    assert!(matches!(vm.read_u32(base), Err(VmError::Fault(_))));
}

#[test]
fn set_prot_transitions() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, PAGE_SIZE, Prot::RW).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();

    vm.set_prot(id, Prot::RX).unwrap();
    assert!(matches!(
        vm.write_u32(base, 1),
        Err(VmError::WriteToExec(_))
    ));

    vm.set_prot(id, Prot::RW).unwrap();
    assert!(matches!(
        vm.fetch_u32(base),
        Err(VmError::ExecOfWrite(_))
    ));
}

#[test]
fn budget_refusal() {
    let mut vm = Vm::new();
    let id = vm.create_chunk(0x1000, 0x3000, Prot::RW).unwrap();
    vm.set_budget(8192);
    vm.commit(id, 0, 0x2000).unwrap();
    assert_eq!(
        vm.commit(id, 0x2000, PAGE_SIZE),
        Err(VmError::BudgetExceeded)
    );
}

#[test]
fn x_page_write_generation_bump() {
    let mut vm = Vm::new();
    let base = 0x1000;
    let id = vm.create_chunk(base, PAGE_SIZE, Prot::RX).unwrap();
    vm.commit(id, 0, PAGE_SIZE).unwrap();
    let generation = vm.generation(id).unwrap();
    assert!(matches!(
        vm.write_u32(base, 1),
        Err(VmError::WriteToExec(_))
    ));
    assert_eq!(vm.generation(id), Ok(generation + 1));
}