use heirloom_cpu_arm::{Access, Fault, Memory};
use heirloom_mem_paged32::{Vm, VmError};

pub struct GuestMem {
    pub(crate) vm: Vm,
}

impl GuestMem {
    pub fn new() -> Self {
        Self { vm: Vm::new() }
    }
}

fn map_error(err: VmError, addr: u32, access: Access) -> Fault {
    let addr = match err {
        VmError::Fault(a) => a,
        _ => addr,
    };
    Fault { addr, access }
}

impl Memory for GuestMem {
    fn read_u8(&mut self, addr: u32) -> Result<u8, Fault> {
        self.vm.read_u8(addr).map_err(|e| map_error(e, addr, Access::Read))
    }

    fn read_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        self.vm.read_u16(addr).map_err(|e| map_error(e, addr, Access::Read))
    }

    fn read_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        self.vm.read_u32(addr).map_err(|e| map_error(e, addr, Access::Read))
    }

    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault> {
        self.vm.write_u8(addr, value).map_err(|e| map_error(e, addr, Access::Write))
    }

    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault> {
        self.vm.write_u16(addr, value).map_err(|e| map_error(e, addr, Access::Write))
    }

    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault> {
        self.vm.write_u32(addr, value).map_err(|e| map_error(e, addr, Access::Write))
    }

    fn fetch_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        // Vm exposes only fetch_u32; we assume the CPU delivers word-aligned addresses for Thumb fetches and mask odd bits here.
        let word = self.vm.fetch_u32(addr & !3).map_err(|e| map_error(e, addr, Access::Fetch))?;
        if addr & 2 != 0 {
            Ok((word >> 16) as u16)
        } else {
            Ok(word as u16)
        }
    }

    fn fetch_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        self.vm.fetch_u32(addr).map_err(|e| map_error(e, addr, Access::Fetch))
    }
}
