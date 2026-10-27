//! The native S60 application start (`EikStart::RunApplication`) as a state machine of callback continuations (`callback`):
//! factory -> `AppDllUid` check against the image's UID3 -> `CreateDocumentL` -> `CreateAppUiL` -> the AppUi's `ConstructL`,
//! every step run by the guest's OWN code through the callback machine. Facts below come from the S60 5th edition SDK headers
//! (`epoc32/include`, EPL; facts only, no code), each with the header and line it was read at.
//!
//! VTABLE-ANCHOR CHECK: a virtual slot number is counted by hand from a header, and a miscount would silently call another
//! virtual. Before every virtual call the slot is read and must hold either an address inside the loaded code chunk (a guest
//! override) or one of OUR import stubs bound to the expected base-class method (an inherited one); anything else is a named
//! Fault, so a wrong slot count is caught at run time.
use alloc::format;
use alloc::string::String;
use crate::callback::Continuation;
use crate::layout::{CODE_BASE, STUB_BASE};
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::run::Outcome;

// ---- TApaApplicationFactory (apparc.h:393). Trivially copyable (no user copy ctor/dtor), 16 bytes: AAPCS passes such a
// composite BY VALUE in r0-r3 (not by hidden reference); RunApplication checks the words it gets are the ctor's.
pub const FACTORY_I_TYPE: u32 = 0; // apparc.h:427 `TType iType`
pub const FACTORY_I_DATA: u32 = 4; // apparc.h:428 `TUint iData` (the TFunction when iType is ETypeFunction)
pub const FACTORY_I_APPLICATION: u32 = 8; // apparc.h:429 `mutable CApaApplication* iApplication`
pub const FACTORY_I_SPARE2: u32 = 12; // apparc.h:430 `TInt iSpare2`
pub const FACTORY_SIZE: u32 = 16; // four words, apparc.h:427-430
pub const FACTORY_TYPE_FUNCTION: u32 = 0; // apparc.h:416 `ETypeFunction` is the first TType enumerator

// ---- Object layouts (ARM EABI: vptr at offset 0, members in declaration order, 4-byte words).
pub const DOC_I_CONTAINER: u32 = 4; // apparc.h:384 CApaDocument::iContainer (after the CBase vptr)
pub const DOC_I_APPLICATION: u32 = 8; // apparc.h:386 CApaDocument::iApplication
pub const DOC_I_APA_PROCESS: u32 = 12; // apparc.h:387 CApaDocument::iApaProcess
pub const DOC_I_SPARE: u32 = 16; // apparc.h:388 CApaDocument::iSpare
pub const CAPA_DOCUMENT_SIZE: u32 = 20;
/// CEikDocument adds MSaveObserver (vptr at 20, savenotf.h:20; the `_ZThn20_` thunks) then `iAppUi` (eikdoc.h:84) at 24.
pub const EIKDOC_I_APP_UI: u32 = 24;
/// CEikAppUi: CCoeAppUi is 24 bytes (vptr, iCoeEnv 4, iViewManager 8, iStack 12, iExtra 16, reserved 20; coeaui.h:~221-227), then
/// MEikMenuObserver at 24 (vptr, MEikCommandObserver reserved word, iMEikMenuObserver_Spare1: 12 bytes, eikcmobs.h/eikmobs.h; the
/// import `_ZThn24_N13CAknViewAppUi15ProcessCommandLEi` confirms 24), MCoeMessageObserver at 36 (vptr only), MObjectProvider at 40
/// (vptr + iMObjectProvider_Reserved1, coemop.h:167; `_ZThn40_N13CAknAppUiBase15MopSupplyObject` confirms 40), then the members
/// of eikappui.h:147-163.
pub const EIKAPPUI_I_DOCUMENT: u32 = 48; // eikappui.h:147
pub const EIKAPPUI_I_CONTAINER_APPUI: u32 = 52; // eikappui.h:150 // end of iSpare; the MSaveObserver subobject starts here (the `_ZThn20_` thunks in the imports)
/// CApaProcess: vptr + iAppList, iDocList, iMainDoc, iMainDocFileName, iFsSession (RFs, one handle word), iApplicationRemover,
/// iMonitor, iCApaProcess_Reserved1 = 9 words (apparc.h:435 class, members at the lines of its `private:` block).
pub const CAPA_PROCESS_SIZE: u32 = 36;

// ---- Virtual slot numbers. A vtable slot is a word index from the vtable address; a virtual destructor takes TWO slots
// (complete, deleting); a base's slots come first and an override reuses its base's slot.
// CBase: ~CBase 0,1 (e32base.h:48), Extension_ 2 (e32base.h:56).
/// CApaApplication: PreDocConstructL 3 (apparc.h:91), CreateDocumentL(CApaProcess*) 4 (:103), AppDllUid 5 (:112),
/// OpenIniFileLC 6 (:130), AppFullName 7 (:133), Capability 8 (:148), NewAppServerL 9 (:156), Reserved1 10 (:160), Reserved2 11 (:161).
pub const SLOT_APP_CREATE_DOCUMENT: u32 = 4; // apparc.h:103
pub const SLOT_APP_DLL_UID: u32 = 5; // apparc.h:112
/// CEikApplication adds GetDefaultDocumentFileName 12 (eikapp.h:37), BitmapStoreName 13 (:38), ResourceFileName 14 (:51),
/// Reserved1 15 (:53), Reserved2 16 (:54), then the private pure `CreateDocumentL()` 17 (:60). CAknApplication (aknapp.h) adds none.
pub const SLOT_EIK_APP_CREATE_DOCUMENT_NOARG: u32 = 17; // eikapp.h:60
/// CApaDocument: NewDocumentL 3 (apparc.h:221), CreateFileStoreLC 4 (:235), GlassPictureL 5 (:238), EditL 6 (:252), PrintL 7 (:265),
/// SaveL 8 (:284), StoreL 9 (:301), RestoreL 10 (:317), DetachFromStoreL 11 (:340), ExternalizeL 12 (:342), IsEmpty 13 (:358),
/// ValidatePasswordL 14 (:359), Capability 15 (:361), HasChanged 16 (:373), OpenFileL(CFileStore*&,RFile&) 17 (:380), Reserved_2 18 (:382);
/// CEikDocument's first new virtual is `CreateAppUiL` (eikdoc.h:46).
pub const SLOT_DOC_CREATE_APP_UI: u32 = 19; // eikdoc.h:46
/// CCoeAppUi: InputCapabilities 3 (coeaui.h:121), HandleWsEventL 4 (:175), PrepareToExit 5 (:176), HandleScreenDeviceChangedL 6 (:178),
/// HandleKeyEventL 7 (:180), HandleForegroundEventL 8 (:181), HandleSwitchOnEventL 9 (:182), HandleSystemEventL 10 (:183),
/// HandleApplicationSpecificEventL 11 (:184), SetAndDrawFocus 12 (:185), HelpContextL 13 (:186), FrameworkCallsRendezvous 14 (:188),
/// CCoeAppUi_Reserved_2 15 (:209); CEikAppUi's first new virtual is `ConstructL` (eikappui.h:58). Its other bases (MEikMenuObserver,
/// MCoeMessageObserver, MObjectProvider) are secondary: the `_ZThn..` thunks, their own vtables.
pub const SLOT_APPUI_CONSTRUCT_L: u32 = 16; // eikappui.h:58

/// The base-class implementations a slot may legitimately hold (an import stub). Empty = pure virtual: only a guest override.
pub const INHERITED_APP_CREATE_DOCUMENT: &[&str] = &["_ZN15CEikApplication15CreateDocumentLEP11CApaProcess"];
pub const INHERITED_APPUI_CONSTRUCT_L: &[&str] = &["_ZN9CEikAppUi10ConstructLEv"];

/// The slot numbers in force; `Default` is the header-derived constants. A test changes one to prove the anchor check bites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slots {
    pub app_dll_uid: u32,
    pub app_create_document: u32,
    pub eik_app_create_document_noarg: u32,
    pub doc_create_app_ui: u32,
    pub appui_construct_l: u32,
}

impl Default for Slots {
    fn default() -> Self {
        Slots {
            app_dll_uid: SLOT_APP_DLL_UID,
            app_create_document: SLOT_APP_CREATE_DOCUMENT,
            eik_app_create_document_noarg: SLOT_EIK_APP_CREATE_DOCUMENT_NOARG,
            doc_create_app_ui: SLOT_DOC_CREATE_APP_UI,
            appui_construct_l: SLOT_APPUI_CONSTRUCT_L,
        }
    }
}

/// One virtual call about to be made: where the slot is and what it may hold.
#[derive(Debug, Clone, Copy)]
pub struct SlotSpec {
    pub class: &'static str,
    pub method: &'static str,
    pub slot: u32,
    pub inherited: &'static [&'static str],
}

/// The vtable-anchor check: read slot `spec.slot` of `obj`'s vtable and return the call target, or a named refusal.
pub fn anchor(p: &Process, obj: u32, spec: &SlotSpec) -> Result<u32, String> {
    let vptr = p.mem.vm.read_u32(obj).map_err(|_| format!("{} object at {obj:#x} is unreadable (reading its vtable pointer)", spec.class))?;
    if vptr == 0 {
        return Err(format!("{} object at {obj:#x} has vtable pointer 0 (a vtable-less placeholder, or not constructed): virtual call {} refused", spec.class, spec.method));
    }
    let slot_addr = vptr.wrapping_add(spec.slot.wrapping_mul(4));
    let v = p.mem.vm.read_u32(slot_addr).map_err(|_| format!("vtable slot {} of {} ({}) at {slot_addr:#x} is unreadable (vtable pointer {vptr:#x})", spec.slot, spec.class, spec.method))?;
    let head = format!("vtable slot {} of {} ({}) at {slot_addr:#x} holds", spec.slot, spec.class, spec.method);
    let a = v & !1;
    if a >= CODE_BASE && a < CODE_BASE.saturating_add(p.code_len) {
        return Ok(v);
    }
    if v >= STUB_BASE && v & 7 == 0 {
        let idx = ((v - STUB_BASE) / 8) as usize;
        if let Some(stub) = p.stubs.get(idx) {
            let name = stub.name.as_deref().unwrap_or("");
            if spec.inherited.contains(&name) {
                return Ok(v);
            }
            return Err(format!("{head} import stub {}/{} {} ({v:#x}); expected a guest override or one of {:?}", stub.dll_base, stub.ordinal, if name.is_empty() { "<unnamed>" } else { name }, spec.inherited));
        }
    }
    Err(format!("{head} {v:#x}, which is neither in the loaded code [{CODE_BASE:#x}..+{:#x}) nor one of the {} import stubs", p.code_len, p.stubs.len()))
}

/// The steps of `RunApplication`; each is the continuation of the guest call before it.
#[derive(Debug, Clone)]
pub enum Step {
    /// The factory function returned the application object.
    Factory,
    /// `app->AppDllUid()` returned.
    AppDllUid { app: u32, process: u32 },
    /// `app->CreateDocumentL(process)` returned.
    CreateDocument { app: u32, process: u32 },
    /// `doc->CreateAppUiL()` returned.
    CreateAppUi { app: u32, doc: u32 },
    /// `appui->ConstructL()` returned.
    ConstructL { appui: u32 },
    /// A queued view activation returned (`viewappui::start_activation`); its scratch block is freed.
    ViewActivated { appui: u32, block: u32 },
}

fn fault(msg: String) -> Outcome {
    Outcome::Fault(msg)
}

/// Anchor-check `spec` on `obj`, then start the guest call of that slot.
fn virtual_call(p: &mut Process, obj: u32, spec: &SlotSpec, args: &[u32], cont: Continuation) -> Result<(), Outcome> {
    let target = anchor(p, obj, spec).map_err(fault)?;
    p.call_guest(target, args, cont).map_err(fault)
}

/// `EikStart::RunApplication(TApaApplicationFactory)`: r0-r3 are the factory's four words (by value, see above).
pub fn run_application(p: &mut Process) -> NativeOutcome {
    let [ty, func, app, spare] = [p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]];
    if ty != FACTORY_TYPE_FUNCTION || app != 0 || spare != 0 {
        return NativeOutcome::Refused(format!(
            "NEXT-NEEDED eikcore EikStart::RunApplication: factory words r0-r3 = {ty:#x} {func:#x} {app:#x} {spare:#x} are not a function-type TApaApplicationFactory as its ctor writes it (iType {FACTORY_TYPE_FUNCTION}, iApplication 0, iSpare2 0)"
        ));
    }
    let a = func & !1;
    if !(a >= CODE_BASE && a < CODE_BASE.saturating_add(p.code_len)) {
        return NativeOutcome::Refused(format!("NEXT-NEEDED eikcore EikStart::RunApplication: factory function {func:#x} is not in the loaded code"));
    }
    p.log.push(format!("[appstart] RunApplication: factory function {func:#x}"));
    match p.call_guest(func, &[], Continuation::App(Step::Factory)) {
        Ok(()) => NativeOutcome::Return,
        Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED eikcore EikStart::RunApplication: {e}")),
    }
}

/// One step of the start: `r0` is the result of the guest call that just returned.
pub(crate) fn step(p: &mut Process, s: Step, r0: u32) -> Result<(), Outcome> {
    let slots = p.slots;
    match s {
        Step::Factory => {
            if r0 == 0 {
                return Err(Outcome::Refused(String::from("factory returned no application")));
            }
            p.log.push(format!("[appstart] factory -> CApaApplication at {r0:#x}"));
            let process = match p.nat.apa_process {
                Some(a) => a,
                None => {
                    let a = p.alloc_zeroed(CAPA_PROCESS_SIZE).ok_or_else(|| Outcome::Refused(String::from("NEXT-NEEDED apparc CApaProcess placeholder: no guest heap")))?;
                    p.nat.apa_process = Some(a);
                    a
                }
            };
            let spec = SlotSpec { class: "CApaApplication", method: "AppDllUid", slot: slots.app_dll_uid, inherited: &[] };
            virtual_call(p, r0, &spec, &[r0], Continuation::App(Step::AppDllUid { app: r0, process }))
        }
        Step::AppDllUid { app, process } => {
            if r0 != p.uid3 {
                return Err(fault(format!("AppDllUid() returned {r0:#010x} but the image's UID3 is {:#010x}: a wrong vtable slot or a wrong application", p.uid3)));
            }
            p.log.push(format!("[appstart] AppDllUid {r0:#010x} == UID3 {:#010x}", p.uid3));
            let spec = SlotSpec { class: "CApaApplication", method: "CreateDocumentL(CApaProcess*)", slot: slots.app_create_document, inherited: INHERITED_APP_CREATE_DOCUMENT };
            virtual_call(p, app, &spec, &[app, process], Continuation::App(Step::CreateDocument { app, process }))
        }
        Step::CreateDocument { app, process } => {
            if r0 == 0 {
                return Err(Outcome::Refused(String::from("CreateDocumentL returned no document")));
            }
            p.log.push(format!("[appstart] CreateDocumentL(process {process:#x}) -> CApaDocument at {r0:#x}"));
            let spec = SlotSpec { class: "CEikDocument", method: "CreateAppUiL", slot: slots.doc_create_app_ui, inherited: &[] };
            virtual_call(p, r0, &spec, &[r0], Continuation::App(Step::CreateAppUi { app, doc: r0 }))
        }
        Step::CreateAppUi { app: _, doc } => {
            if r0 == 0 {
                return Err(Outcome::Refused(String::from("CreateAppUiL returned no AppUi")));
            }
            p.log.push(format!("[appstart] CreateAppUiL(doc {doc:#x}) -> CEikAppUi at {r0:#x}"));
            // CEikDocument::ConstructAppUiL: iAppUi = the new AppUi, and the AppUi's SetDocument(doc) (eikappui.h:63) sets its iDocument.
            for (obj, off, val) in [(doc, EIKDOC_I_APP_UI, r0), (r0, EIKAPPUI_I_DOCUMENT, doc)] {
                p.mem.vm.write_u32(obj.wrapping_add(off), val).map_err(|_| fault(format!("CreateAppUiL: the object at {obj:#x} is unwritable")))?;
            }
            let spec = SlotSpec { class: "CEikAppUi", method: "ConstructL", slot: slots.appui_construct_l, inherited: INHERITED_APPUI_CONSTRUCT_L };
            virtual_call(p, r0, &spec, &[r0], Continuation::App(Step::ConstructL { appui: r0 }))
        }
        Step::ConstructL { appui } => {
            p.log.push(String::from("[appstart] AppUi::ConstructL returned"));
            next_activation_or_idle(p, appui)
        }
        Step::ViewActivated { appui, block } => {
            crate::viewappui::finish_activation(p, block);
            p.log.push(String::from("[appstart] view activation returned"));
            next_activation_or_idle(p, appui)
        }
    }
}

/// The thread is back in its scheduler: the view server delivers the queued activations, then the event loop would start.
fn next_activation_or_idle(p: &mut Process, appui: u32) -> Result<(), Outcome> {
    if crate::viewappui::has_pending(p, appui) {
        return crate::viewappui::start_activation(p, appui, |block| Continuation::App(Step::ViewActivated { appui, block })).map_err(|m| if m.starts_with("NEXT-NEEDED") { Outcome::Refused(m) } else { fault(m) });
    }
    // `CEikonEnv::ExecuteD` -> `CActiveScheduler::Start()`: the thread enters its scheduler and stays there until `Stop`.
    p.sched_start().map_err(fault)
}

/// The native `CEikApplication::CreateDocumentL(CApaProcess*)` (the slot the apps inherit): call the application's private
/// `CreateDocumentL()` and, when it returns the document, set the document's `iApplication` and `iApaProcess`.
fn eik_create_document(p: &mut Process) -> NativeOutcome {
    let (app, process) = (p.cpu.regs[0], p.cpu.regs[1]);
    let spec = SlotSpec { class: "CEikApplication", method: "CreateDocumentL()", slot: p.slots.eik_app_create_document_noarg, inherited: &[] };
    let go = anchor(p, app, &spec).and_then(|t| p.call_guest(t, &[app], Continuation::EikCreateDocument { app, process }));
    match go {
        Ok(()) => NativeOutcome::Return,
        Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED eikcore CEikApplication::CreateDocumentL: {e}")),
    }
}

/// The private `CreateDocumentL()` returned `doc`.
pub(crate) fn finish_create_document(p: &mut Process, app: u32, process: u32, doc: u32) -> Result<(), Outcome> {
    if doc != 0 {
        for (off, val) in [(DOC_I_APPLICATION, app), (DOC_I_APA_PROCESS, process)] {
            p.mem.vm.write_u32(doc.wrapping_add(off), val).map_err(|_| fault(format!("CreateDocumentL: the document at {doc:#x} is unwritable")))?;
        }
    }
    p.cpu.regs[0] = doc;
    Ok(())
}

fn zero(p: &mut Process, base: u32, from: u32, to: u32) -> Result<(), String> {
    for off in (from..to).step_by(4) {
        p.mem.vm.write_u32(base.wrapping_add(off), 0).map_err(|_| format!("object at {base:#x} is unwritable"))?;
    }
    Ok(())
}

/// The natives of the application start: the factory, `RunApplication`, and the base-class constructors / destructors the
/// apps import. `None` = not one of ours. Base constructors zero the base part, write the fields a derived class may read and
/// return `this`; they never write a vtable pointer (the derived constructor does). Base destructors own nothing here (every
/// object came from `User::AllocZ`, no base owns a handle yet): they return `this`.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let ctor = |p: &mut Process, r: Result<(), String>| match r {
        Ok(()) => { p.cpu.regs[0] = this; NativeOutcome::Return }
        Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED {name}: {e}")),
    };
    Some(match name {
        "_ZN22TApaApplicationFactoryC1EPFP15CApaApplicationvE" => {
            let func = p.cpu.regs[1];
            let words = [(FACTORY_I_TYPE, FACTORY_TYPE_FUNCTION), (FACTORY_I_DATA, func), (FACTORY_I_APPLICATION, 0), (FACTORY_I_SPARE2, 0)];
            let r = words.iter().try_for_each(|&(off, v)| p.mem.vm.write_u32(this.wrapping_add(off), v).map_err(|_| format!("factory object at {this:#x} is unwritable")));
            ctor(p, r)
        }
        "_ZN8EikStart14RunApplicationE22TApaApplicationFactory" => run_application(p),
        "_ZN15CEikApplication15CreateDocumentLEP11CApaProcess" => eik_create_document(p),
        "_ZN12CAknDocumentC2ER15CEikApplication" => {
            let app = p.cpu.regs[1];
            let r = zero(p, this, DOC_I_CONTAINER, CAPA_DOCUMENT_SIZE).and_then(|_| p.mem.vm.write_u32(this.wrapping_add(DOC_I_APPLICATION), app).map_err(|_| format!("document at {this:#x} is unwritable")));
            ctor(p, r)
        }
        // Nothing the derived code may read is set by these (iCoeEnv stays 0: there is no CCoeEnv yet, a read through it is a named null fault).
        "_ZN15CEikApplicationC2Ev" | "_ZN13CAknAppUiBaseC1Ev" | "_ZN13CAknAppUiBaseC2Ev" => ctor(p, Ok(())),
        "_ZN15CEikApplicationD2Ev" | "_ZN12CEikDocumentD2Ev" | "_ZN13CAknAppUiBaseD1Ev" | "_ZN13CAknAppUiBaseD2Ev"
        | "_ZN9CAknAppUiD2Ev" | "_ZN5CBaseD1Ev" | "_ZN5CBaseD2Ev" => ctor(p, Ok(())),
        _ => return None,
    })
}
