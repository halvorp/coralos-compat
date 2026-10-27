use alloc::format;
use heirloom_cpu_arm::Exit;
use crate::layout::STUB_SVC_BASE;
use crate::loader::Process;
use crate::natives::{self, NativeOutcome};

#[derive(Debug)]
pub enum Outcome {
    Exited(i32),
    Refused(alloc::string::String),
    Fault(alloc::string::String),
    BudgetSpent,
    Blocked,
}

impl Process {
    pub fn run(&mut self, budget: u64) -> Outcome {
        // step() keeps the instruction count exact even when execution stops at an SVC or fault.
        loop {
            if self.insns >= budget {
                return Outcome::BudgetSpent;
            }
            let exit = self.cpu.step(&mut self.mem);
            self.insns += 1;
            match exit {
                Exit::Svc(n) => {
                    if n == crate::layout::EXIT_SVC {
                        return Outcome::Exited(self.cpu.regs[0] as i32);
                    }
                    if n == crate::callback::CALLBACK_SVC {
                        match self.callback_return() {
                            Ok(()) => continue,
                            Err(outcome) => return outcome,
                        }
                    }
                    if n >= STUB_SVC_BASE {
                        let idx = (n - STUB_SVC_BASE) as usize;
                        match natives::call(self, idx) {
                            NativeOutcome::Return => continue,
                            NativeOutcome::Exit(code) => return Outcome::Exited(code),
                            NativeOutcome::Refused(msg) => match self.census.as_mut() {
                                Some(seen) if msg.starts_with("NEXT-NEEDED") => { seen.push(msg); self.cpu.regs[0] = 0; continue }
                                _ => return Outcome::Refused(msg),
                            },
                        }
                    }
                    return Outcome::Refused(format!(
                        "NEXT-NEEDED exec call SVC #0x{:X} at PC 0x{:08X}",
                        n, self.cpu.pc()
                    ));
                }
                Exit::Bkpt(n) => return Outcome::Fault(format!(
                    "BKPT #0x{:X} at PC 0x{:08X}", n, self.cpu.pc()
                )),
                Exit::Trap(t) => return Outcome::Fault(format!(
                    "TRAP pc=0x{:08X} insn=0x{:08X} thumb={} kind={:?}",
                    t.pc, t.insn, t.thumb, t.kind
                )),
                Exit::Fault { pc, fault } => {
                    // The CApaProcess placeholder has vtable pointer 0, so a virtual call through it reads near address 0.
                    let note = match self.nat.apa_process {
                        Some(a) if fault.addr < 0x1000 => format!(
                            " (null/low address: a virtual call through a vtable-less object, e.g. the CApaProcess placeholder at 0x{:08X})", a),
                        _ => alloc::string::String::new(),
                    };
                    return Outcome::Fault(format!(
                        "FAULT pc=0x{:08X} addr=0x{:08X} access={:?}{}",
                        pc, fault.addr, fault.access, note
                    ));
                }
                Exit::Wait => return Outcome::Blocked,
                // step() reports one retired instruction as a spent budget of one
                Exit::BudgetSpent => continue,
            }
        }
    }
}