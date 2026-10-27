//! The icon calls of the AVKON icon library (`AknIconUtils`, akniconutils.h; `AknsUtils`, aknsutils.h) and `CGulIcon` (gulicon.h).
//! Each call makes a real guest object (a bitmap, a `CGulIcon`) and RECORDS which icon was asked for (file, bitmap id, mask id): nothing is
//! decoded or drawn. DELETE WHEN the GuestUi List control carries icons (then decode the MIF/MBM through a datatype and project them).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

/// `KAvkonIconFileName` (AknIconUtils.cpp:41, the library's own constant).
const AVKON_ICON_FILE: &str = "Z:\\Resource\\Apps\\avkon2.mbm";

#[derive(Debug, Default)]
pub struct IconState {
    /// The guest `TBufC16` answering `AknIconUtils::AvkonIconFileName()`.
    pub avkon_name: Option<u32>,
    /// The `MAknsSkinInstance` answering `AknsUtils::SkinInstance()`: an engine object whose every virtual refuses by slot until a guest call confirms it.
    pub skin_instance: Option<u32>,
}

/// A bitmap (`CFbsBitmap`) the icon library made for an icon: which one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconBitmap {
    pub file: String,
    pub id: i32,
    /// The skin item the icon is themed from (`TAknsItemID`: iMajor, iMinor), when the call named one.
    pub skin_item: Option<(u32, u32)>,
    /// The colour group item and index a coloured icon is tinted with (`CreateColorIconL`).
    pub colour: Option<((u32, u32), i32)>,
}

/// Slots of the `MAknsSkinInstance` table: more than the interface has (aknsskininstance.h), so a guest call past one is a named refusal.
const SKIN_INSTANCE_SLOTS: usize = 40;

impl Process {
    fn make_skin_instance(&mut self) -> Result<u32, String> {
        let o = self.alloc_zeroed(8).ok_or_else(|| String::from("no guest heap for the skin instance"))?;
        let vptr = self.build_vtable("MAknsSkinInstance", &[None; SKIN_INSTANCE_SLOTS])?;
        self.wr(o, vptr)?;
        Ok(o)
    }

    /// A guest `TBufC16` (a constant descriptor: length word, then the elements) holding `text`.
    fn const_des16(&mut self, text: &str) -> Result<u32, String> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let a = self.alloc_zeroed(4 + 2 * units.len() as u32).ok_or_else(|| String::from("no guest heap for a descriptor"))?;
        self.wr(a, units.len() as u32)?; // type EBufC = 0 in the top nibble
        for (i, u) in units.iter().enumerate() {
            self.mem.vm.write_u16(a + 4 + 2 * i as u32, *u).map_err(|_| format!("{a:#x} is unwritable"))?;
        }
        Ok(a)
    }

    /// A bitmap object standing for the icon `icon`: recorded, never decoded.
    fn icon_bitmap(&mut self, icon: IconBitmap) -> Result<u32, String> {
        let b = self.plain_object("CFbsBitmap", 0x20)?;
        self.obj_set(b, HostObj::IconBitmap(icon))?;
        Ok(b)
    }

    /// A `CGulIcon` (vptr, iBitmap 4, iMask 8, iBitmapsOwnedExternally 12; gulicon.h:38-40) over the two bitmaps.
    fn gul_icon(&mut self, bitmap: u32, mask: u32) -> Result<u32, String> {
        let o = self.plain_object("CGulIcon", 16)?;
        self.wr(o + 4, bitmap)?;
        self.wr(o + 8, mask)?;
        Ok(o)
    }

    fn item_id(&self, at: u32) -> Result<(u32, u32), String> {
        Ok((self.rd(at)?, self.rd(at + 4)?))
    }

    fn create_gul_icon(&mut self) -> Result<u32, String> {
        let item = self.item_id(self.cpu.regs[1])?;
        let file = crate::desc::to_utf8(&crate::desc::read16(self, self.cpu.regs[2])?);
        let (bitmap_id, mask_id) = (self.cpu.regs[3] as i32, self.rd(self.cpu.regs[13])? as i32);
        let mk = |id| IconBitmap { file: file.clone(), id, skin_item: Some(item), colour: None };
        let (bitmap, mask) = (self.icon_bitmap(mk(bitmap_id))?, self.icon_bitmap(mk(mask_id))?);
        self.gul_icon(bitmap, mask)
    }

    /// `AknsUtils::CreateColorIconL(skin, itemId, colourItemId, colourIndex, CFbsBitmap*& bitmap, CFbsBitmap*& mask, const TDesC& file,
    /// TInt bitmapId, TInt maskId, TRgb default)` (aknsutils.h:517): the two out pointers get recorded icon bitmaps. A mask id of -1 (none
    /// in a MIF) is still a mask object, as the guest stores it; the default colour is ignored (no skin supplies a colour).
    fn create_color_icon(&mut self) -> Result<(u32, u32), String> {
        let (item, colour_item, colour_index) = (self.item_id(self.cpu.regs[1])?, self.item_id(self.cpu.regs[2])?, self.cpu.regs[3] as i32);
        let sp = self.cpu.regs[13];
        let (out_bitmap, out_mask, file) = (self.rd(sp)?, self.rd(sp + 4)?, self.rd(sp + 8)?);
        let (bitmap_id, mask_id) = (self.rd(sp + 12)? as i32, self.rd(sp + 16)? as i32);
        let file = crate::desc::to_utf8(&crate::desc::read16(self, file)?);
        let mk = |id| IconBitmap { file: file.clone(), id, skin_item: Some(item), colour: Some((colour_item, colour_index)) };
        let (bitmap, mask) = (self.icon_bitmap(mk(bitmap_id))?, self.icon_bitmap(mk(mask_id))?);
        self.wr(out_bitmap, bitmap)?;
        self.wr(out_mask, mask)?;
        Ok((bitmap, mask))
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let refuse = |m: String| Some(NativeOutcome::Refused(format!("NEXT-NEEDED AknIcon {name}: {m}")));
    match name {
        // const TDesC& AknIconUtils::AvkonIconFileName()
        "_ZN12AknIconUtils17AvkonIconFileNameEv" => {
            let a = match p.nat.icons.avkon_name {
                Some(a) => Ok(a),
                None => p.const_des16(AVKON_ICON_FILE),
            };
            if let Ok(a) = a { p.nat.icons.avkon_name = Some(a); }
            match a {
                Ok(a) => { p.cpu.regs[0] = a; Some(NativeOutcome::Return) }
                Err(m) => refuse(m),
            }
        }
        // MAknsSkinInstance* AknsUtils::SkinInstance() (aknsutils.h:155)
        "_ZN9AknsUtils12SkinInstanceEv" => {
            let a = match p.nat.icons.skin_instance {
                Some(a) => Ok(a),
                None => p.make_skin_instance(),
            };
            if let Ok(a) = a { p.nat.icons.skin_instance = Some(a); }
            match a {
                Ok(a) => { p.cpu.regs[0] = a; Some(NativeOutcome::Return) }
                Err(m) => refuse(m),
            }
        }
        // AknsUtils::CreateColorIconL (aknsutils.h:517); the LC form is NEXT-NEEDED until a guest calls it.
        "_ZN9AknsUtils16CreateColorIconLEP17MAknsSkinInstanceRK11TAknsItemIDS4_iRP10CFbsBitmapS7_RK7TDesC16ii4TRgb" => match p.create_color_icon() {
            Ok(_) => Some(NativeOutcome::Return),
            Err(m) => refuse(m),
        },
        // CGulIcon* CGulIcon::NewL(CFbsBitmap* aBitmap, CFbsBitmap* aMask) (gulicon.h:25).
        "_ZN8CGulIcon4NewLEP10CFbsBitmapS1_" => match p.gul_icon(p.cpu.regs[0], p.cpu.regs[1]) {
            Ok(o) => { p.cpu.regs[0] = o; Some(NativeOutcome::Return) }
            Err(m) => refuse(m),
        },
        // CGulIcon* AknsUtils::CreateGulIconL(skin, const TAknsItemID&, const TDesC& aFileName, TInt aFileBitmapId, TInt aFileMaskId) (aknsutils.h:374).
        "_ZN9AknsUtils14CreateGulIconLEP17MAknsSkinInstanceRK11TAknsItemIDRK7TDesC16ii" => match p.create_gul_icon() {
            Ok(o) => { p.cpu.regs[0] = o; Some(NativeOutcome::Return) }
            Err(m) => refuse(m),
        },
        // void CGulIcon::SetBitmapsOwnedExternally(TBool) (gulicon.h:31): iBitmapsOwnedExternally.
        "_ZN8CGulIcon25SetBitmapsOwnedExternallyEi" => match p.wr(p.cpu.regs[0] + 12, p.cpu.regs[1]) {
            Ok(()) => Some(NativeOutcome::Return),
            Err(m) => refuse(m),
        },
        _ => None,
    }
}
