//! The machine-session frames: how a UI host drives a guest machine that runs in another process (docs/heirloom/23-ENGINE-SPLIT.md
//! phase 1: Heirloom.app <-> mac68k.engine). Same codec, same header, same bounds as the UI frames - ONE wire (ONE-PARSER-PER-FORMAT).
//!
//! - [`Control`] (kind 6, host -> engine): the `GuestMachine` calls the host makes, plus the shared canvas arena the host lends.
//!   A file or a staged volume larger than one frame travels as numbered chunks of one blob (`offset`, `total`).
//! - [`Report`] (kind 7, engine -> host): what those calls answered. A slice's UI calls ride beside it as ordinary Call/Lease
//!   frames, in the order the guest made them.
//!
//! The machine's own `Step` lives in heirloom-common (which depends on this crate), so the step crosses as [`WireStep`].
use super::*;

/// The largest chunk a blob is split into: well inside [`MAX_FRAME`] with the header and fields around it.
pub const BLOB_CHUNK: usize = 512 * 1024;

/// The named inputs ([`Control::Input`]) a family takes. The names are the contract between the host that sends a file and the
/// family library that takes it; one home here, beside the frame.
pub mod inputs {
    /// Symbian (ekaarm): heirloom-fmt-e32's eka-ordinals.tsv, the S60 export names every import binds by.
    pub const EKA_ORDINALS: &str = "eka-ordinals";
    /// Symbian (ekaarm): the device's layout data (the text `cdlextract` makes).
    pub const EKA_LAYOUT: &str = "s60-layout";
    /// Symbian (ekaarm): the device ROM (`Z:`'s read-only files) as a heirloom-volume snapshot of exactly one volume.
    pub const EKA_ROM: &str = "rom";
}

/// A host -> engine call on the guest machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// The machine this session runs: the family's profile name ("" = its default, e.g. mac-ii). The FIRST frame of a session;
    /// the engine process creates the guest machine on it (a library never sees it).
    Machine { profile: String },
    /// The shared region guest canvases are written into: the delegation slot the engine maps, and its byte length.
    Canvases { slot: u16, len: u32 },
    /// One chunk of a heirloom-volume snapshot of the staged guest volumes (`total` bytes in all).
    Volume { offset: u32, total: u32, chunk: Vec<u8> },
    /// One chunk of the program file (`total` bytes in all).
    Program { offset: u32, total: u32, chunk: Vec<u8> },
    /// `GuestMachine::start_file` on the program assembled from the Program chunks, under `name`.
    StartFile { name: String },
    SetWallClock { local_unix_secs: i64 },
    StopOnUnimplemented(bool),
    /// `GuestMachine::advance_clock_to`.
    Advance { virtual_us: u64 },
    /// `GuestMachine::step`; answered by [`Report::Stepped`].
    Step { budget: u64 },
    /// A guest-OS path alias the host's manifest set up before the program starts (AmigaDOS `Assign NAME: target`, `add` = the
    /// `ADD` form of a multi-directory assign). Only a family whose OS has assigns takes it; another refuses it by name.
    Assign { name: String, target: String, add: bool },
    /// The host's control metrics a guest toolkit lays its windows out for (doc 23 Phase 3, MUI's HostProfile). Numbers only:
    /// the host's text measurer cannot cross a process, so the engine sizes text by the guest's own font (the named fallback).
    HostProfile(HostProfile),
    /// One chunk of a named input file the family's machine takes before `StartFile` (`total` bytes in all): data the guest OS
    /// reads that is neither the program nor a volume - Symbian's ordinal table, its layout data and its device ROM. The family
    /// names what it takes; any other name is refused by name.
    Input { name: String, offset: u32, total: u32, chunk: Vec<u8> },
    /// The storage directory the staged volume's STREAM files are read from (the bundle's root on CoralFS): their locators are
    /// relative to it. The ENGINE process takes it (a library never sees it) and reads each stream there, refusing a locator that
    /// would leave it. Sent before the Volume chunks when the volume holds a stream.
    StreamRoot { path: String },
}

/// The host's measured control metrics, in window pixels (the kit's tokens on CoralOS). A field of 0 / None = the toolkit's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostProfile {
    /// Smallest check box / toggle (width, height).
    pub toggle: (i32, i32),
    /// Smallest height of a button, field, cycle, slider and gauge.
    pub control_h: i32,
    /// Extra width the host adds to a button, field and cycle (its inset on both sides).
    pub pad_w: i32,
    /// The host draws a check box's label this far to the RIGHT of its toggle (None = in the toolkit's label column).
    pub label_right: Option<i32>,
    /// The title band the host draws above a group's panel.
    pub group_head: i32,
    /// The host panel's padding around a group's contents.
    pub group_inset: i32,
    /// Room the host keeps between a label and the control it labels.
    pub label_gap: i32,
    /// The host's gap between neighbouring objects (horizontal, vertical).
    pub spacing: (i32, i32),
    /// The host's inset of a window's contents.
    pub win_pad: i32,
}

/// heirloom-common's `Step` as it crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireStep {
    Exited,
    Fault(String),
    BudgetSpent,
    Blocked,
}

/// An engine -> host answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// `start_file`'s answer (Err = the engine's named refusal).
    Started(Result<(), String>),
    /// `set_wall_clock`'s answer.
    WallClock(Result<(), String>),
    /// A slice ran: the step, its counts, the next timer deadline (virtual µs), the refusals the machine drained, and the native
    /// events this batch carried that reached no guest window (each named).
    Stepped { step: WireStep, insns: u64, bus_faults: u32, deadline_us: Option<u64>, refused: Vec<String>, undelivered: Vec<String> },
}

fn put_blob(out: &mut Vec<u8>, tag: u8, offset: u32, total: u32, chunk: &[u8]) {
    out.push(tag);
    put_blob_body(out, offset, total, chunk);
}

fn put_blob_body(out: &mut Vec<u8>, offset: u32, total: u32, chunk: &[u8]) {
    put_u32(out, offset);
    put_u32(out, total);
    put_len(out, chunk.len());
    out.extend_from_slice(chunk);
}

fn put_result(out: &mut Vec<u8>, r: &Result<(), String>) {
    match r {
        Ok(()) => out.push(0),
        Err(why) => {
            out.push(1);
            put_string(out, why);
        }
    }
}

fn put_strings(out: &mut Vec<u8>, v: &[String]) {
    put_vec(out, v, |o, s| put_string(o, s));
}

pub(super) fn put_machine_control(out: &mut Vec<u8>, c: &Control) {
    match c {
        Control::Canvases { slot, len } => {
            out.push(1);
            put_u16(out, *slot);
            put_u32(out, *len);
        }
        Control::Volume { offset, total, chunk } => put_blob(out, 2, *offset, *total, chunk),
        Control::Program { offset, total, chunk } => put_blob(out, 3, *offset, *total, chunk),
        Control::StartFile { name } => {
            out.push(4);
            put_string(out, name);
        }
        Control::SetWallClock { local_unix_secs } => {
            out.push(5);
            put_u64(out, *local_unix_secs as u64);
        }
        Control::StopOnUnimplemented(on) => {
            out.push(6);
            put_bool(out, *on);
        }
        Control::Advance { virtual_us } => {
            out.push(7);
            put_u64(out, *virtual_us);
        }
        Control::Step { budget } => {
            out.push(8);
            put_u64(out, *budget);
        }
        Control::Machine { profile } => {
            out.push(9);
            put_string(out, profile);
        }
        Control::Assign { name, target, add } => {
            out.push(10);
            put_string(out, name);
            put_string(out, target);
            put_bool(out, *add);
        }
        Control::HostProfile(p) => {
            out.push(11);
            for v in [p.toggle.0, p.toggle.1, p.control_h, p.pad_w, p.group_head, p.group_inset, p.label_gap, p.spacing.0, p.spacing.1, p.win_pad] {
                put_u32(out, v as u32);
            }
            put_option(out, &p.label_right, |o, v| put_u32(o, *v as u32));
        }
        Control::Input { name, offset, total, chunk } => {
            out.push(12);
            put_string(out, name);
            put_blob_body(out, *offset, *total, chunk);
        }
        Control::StreamRoot { path } => {
            out.push(13);
            put_string(out, path);
        }
    }
}

fn get_blob(r: &mut Reader<'_>) -> Result<(u32, u32, Vec<u8>), WireError> {
    let offset = r.u32()?;
    let total = r.u32()?;
    let len = r.u32()? as usize;
    let chunk = r.take(len)?.to_vec();
    if (offset as u64) + (len as u64) > total as u64 {
        return Err(WireError::TooLong { what: "blob chunk past its total", len: len as u32 });
    }
    Ok((offset, total, chunk))
}

pub(super) fn get_machine_control(r: &mut Reader<'_>) -> Result<Control, WireError> {
    let tag = r.u8()?;
    Ok(match tag {
        1 => Control::Canvases { slot: r.u16()?, len: r.u32()? },
        2 => {
            let (offset, total, chunk) = get_blob(r)?;
            Control::Volume { offset, total, chunk }
        }
        3 => {
            let (offset, total, chunk) = get_blob(r)?;
            Control::Program { offset, total, chunk }
        }
        4 => Control::StartFile { name: r.string()? },
        5 => Control::SetWallClock { local_unix_secs: r.u64()? as i64 },
        6 => Control::StopOnUnimplemented(r.boolean()?),
        7 => Control::Advance { virtual_us: r.u64()? },
        8 => Control::Step { budget: r.u64()? },
        9 => Control::Machine { profile: r.string()? },
        10 => Control::Assign { name: r.string()?, target: r.string()?, add: r.boolean()? },
        11 => {
            let mut v = [0i32; 10];
            for x in v.iter_mut() {
                *x = r.u32()? as i32;
            }
            Control::HostProfile(HostProfile {
                toggle: (v[0], v[1]),
                control_h: v[2],
                pad_w: v[3],
                group_head: v[4],
                group_inset: v[5],
                label_gap: v[6],
                spacing: (v[7], v[8]),
                win_pad: v[9],
                label_right: r.option(|r| r.u32().map(|x| x as i32))?,
            })
        }
        12 => {
            let name = r.string()?;
            let (offset, total, chunk) = get_blob(r)?;
            Control::Input { name, offset, total, chunk }
        }
        13 => Control::StreamRoot { path: r.string()? },
        tag => return Err(WireError::UnknownTag { what: "Control", tag }),
    })
}

fn get_result(r: &mut Reader<'_>) -> Result<Result<(), String>, WireError> {
    match r.u8()? {
        0 => Ok(Ok(())),
        1 => Ok(Err(r.string()?)),
        tag => Err(WireError::UnknownTag { what: "Result", tag }),
    }
}

fn get_strings(r: &mut Reader<'_>) -> Result<Vec<String>, WireError> {
    r.vec(4, |r| r.string())
}

pub(super) fn put_machine_report(out: &mut Vec<u8>, rep: &Report) {
    match rep {
        Report::Started(res) => {
            out.push(1);
            put_result(out, res);
        }
        Report::WallClock(res) => {
            out.push(2);
            put_result(out, res);
        }
        Report::Stepped { step, insns, bus_faults, deadline_us, refused, undelivered } => {
            out.push(3);
            match step {
                WireStep::Exited => out.push(0),
                WireStep::Fault(why) => {
                    out.push(1);
                    put_string(out, why);
                }
                WireStep::BudgetSpent => out.push(2),
                WireStep::Blocked => out.push(3),
            }
            put_u64(out, *insns);
            put_u32(out, *bus_faults);
            put_option(out, deadline_us, |o, d| put_u64(o, *d));
            put_strings(out, refused);
            put_strings(out, undelivered);
        }
    }
}

pub(super) fn get_machine_report(r: &mut Reader<'_>) -> Result<Report, WireError> {
    let tag = r.u8()?;
    Ok(match tag {
        1 => Report::Started(get_result(r)?),
        2 => Report::WallClock(get_result(r)?),
        3 => {
            let step = match r.u8()? {
                0 => WireStep::Exited,
                1 => WireStep::Fault(r.string()?),
                2 => WireStep::BudgetSpent,
                3 => WireStep::Blocked,
                tag => return Err(WireError::UnknownTag { what: "WireStep", tag }),
            };
            Report::Stepped {
                step,
                insns: r.u64()?,
                bus_faults: r.u32()?,
                deadline_us: r.option(|r| r.u64())?,
                refused: get_strings(r)?,
                undelivered: get_strings(r)?,
            }
        }
        tag => return Err(WireError::UnknownTag { what: "Report", tag }),
    })
}
