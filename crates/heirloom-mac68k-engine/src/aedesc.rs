//! The Apple Event Manager's DATA MODEL (Inside Macintosh: Interapplication Communication, chapter 4; `AEDataModel.h`): descriptors,
//! descriptor lists, records and Apple events with their attributes and parameters, as one engine-side model both Mac engines call
//! with argument VALUES (the PowerPC adapter is `heirloom-macppc-engine` `ae_c.rs`; the 68k `_Pack8` keeps its handler registry and
//! can adopt these bodies selector by selector).
//!
//! An `AEDesc` in guest memory is 8 bytes: `descriptorType` then `dataHandle`. A SIMPLE descriptor's handle holds its bytes as they are
//! (a program may read `**dataHandle`, and the Carbon accessors `AEGetDescData` read the same bytes). A LIST, RECORD or APPLE EVENT
//! keeps a private format in its handle (the real Manager's is private too: programs only reach it through these calls):
//!
//! ```text
//! 'hAE1' kind:u32 (0 list, 1 record, 2 event) nattr:u32 nitem:u32, then nattr + nitem entries:
//!   keyword:u32 (0 for a list item) type:u32 flags:u32 (bit 0 READ, bit 1 STRUCTURED = the bytes are a nested blob) len:u32 bytes, padded to 4
//! ```
//! A nested list/record/event is stored as its own blob (STRUCTURED) and comes back out as a new handle holding that blob. Every
//! table here is a growable `Vec`; malformed handle contents answer `errAECorruptData` by name.
//!
//! Coercion ([`coerce`]): identity, `typeWildCard`, the four-character code types (`type`/`keyw`/`enum`) among themselves, the integer widths `shor`/`long` (range-checked), `bool` from either width and the
//! `true`/`fals` types; anything else is `errAECoercionFail` (-1700) and the caller logs which pair.
//! `keyMissedKeywordAttr` (`miss`) is computed like the real Manager's: the first parameter of the event the handler has not read
//! (not listed in `keyOptionalKeywordAttr` `optk`), `errAEDescNotFound` once all were read.
use crate::machine::Mac;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;

pub const ERR_AE_COERCION_FAIL: i16 = -1700;
pub const ERR_AE_DESC_NOT_FOUND: i16 = -1701;
pub const ERR_AE_CORRUPT_DATA: i16 = -1702;
pub const ERR_AE_WRONG_DATA_TYPE: i16 = -1703;
pub const ERR_AE_NOT_AE_DESC: i16 = -1704;
pub const ERR_AE_BAD_LIST_ITEM: i16 = -1705;
pub const ERR_AE_NOT_APPLE_EVENT: i16 = -1707;
pub const ERR_AE_ILLEGAL_INDEX: i16 = -1719;
pub const MEM_FULL: i16 = -108;

pub const fn fc(b: &[u8; 4]) -> u32 { u32::from_be_bytes(*b) }
pub const TYPE_NULL: u32 = fc(b"null");
pub const TYPE_LIST: u32 = fc(b"list");
pub const TYPE_RECORD: u32 = fc(b"reco");
pub const TYPE_APPLE_EVENT: u32 = fc(b"aevt");
pub const TYPE_WILD_CARD: u32 = fc(b"****");
pub const TYPE_SHORT: u32 = fc(b"shor");
pub const TYPE_LONG: u32 = fc(b"long");
pub const TYPE_BOOLEAN: u32 = fc(b"bool");
pub const TYPE_TRUE: u32 = fc(b"true");
pub const TYPE_FALSE: u32 = fc(b"fals");
pub const TYPE_TYPE: u32 = fc(b"type");
pub const TYPE_KEYWORD: u32 = fc(b"keyw");
pub const TYPE_ENUMERATED: u32 = fc(b"enum");
pub const KEY_EVENT_CLASS: u32 = fc(b"evcl");
pub const KEY_EVENT_ID: u32 = fc(b"evid");
pub const KEY_ADDRESS: u32 = fc(b"addr");
pub const KEY_RETURN_ID: u32 = fc(b"rtid");
pub const KEY_TRANSACTION_ID: u32 = fc(b"tran");
pub const KEY_MISSED_KEYWORD: u32 = fc(b"miss");
pub const KEY_OPTIONAL_KEYWORD: u32 = fc(b"optk");
pub const KEY_ERROR_NUMBER: u32 = fc(b"errn");
/// `kAutoGenerateReturnID`: AECreateAppleEvent makes a fresh return id.
pub const AUTO_GENERATE_RETURN_ID: i16 = -1;

const MAGIC: u32 = fc(b"hAE1");
const F_READ: u32 = 1;
const F_STRUCTURED: u32 = 2;

/// Four characters for a log line.
pub fn tag(v: u32) -> String { v.to_be_bytes().iter().map(|&b| if (0x20..0x7F).contains(&b) { char::from(b) } else { '?' }).collect() }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { List, Record, Event }

/// One attribute, parameter or list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry { pub key: u32, pub ty: u32, pub structured: bool, pub read: bool, pub data: Vec<u8> }

/// A list, record or event, decoded from its handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob { pub kind: Kind, pub attrs: Vec<Entry>, pub items: Vec<Entry> }

impl Blob {
    pub fn new(kind: Kind) -> Self { Blob { kind, attrs: Vec::new(), items: Vec::new() } }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let kind = match self.kind { Kind::List => 0u32, Kind::Record => 1, Kind::Event => 2 };
        for w in [MAGIC, kind, self.attrs.len() as u32, self.items.len() as u32] { out.extend_from_slice(&w.to_be_bytes()); }
        for e in self.attrs.iter().chain(&self.items) {
            let flags = if e.read { F_READ } else { 0 } | if e.structured { F_STRUCTURED } else { 0 };
            for w in [e.key, e.ty, flags, e.data.len() as u32] { out.extend_from_slice(&w.to_be_bytes()); }
            out.extend_from_slice(&e.data);
            while out.len() % 4 != 0 { out.push(0); }
        }
        out
    }

    /// The blob in `b`, or None when `b` is not one (a simple descriptor's bytes) or is malformed.
    pub fn decode(b: &[u8]) -> Option<Blob> {
        let w = |at: usize| -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) };
        if w(0)? != MAGIC { return None }
        let kind = match w(4)? { 0 => Kind::List, 1 => Kind::Record, 2 => Kind::Event, _ => return None };
        let (na, ni) = (w(8)? as usize, w(12)? as usize);
        let mut at = 16;
        let mut entries = Vec::new();
        for _ in 0..na.checked_add(ni)? {
            let (key, ty, flags, len) = (w(at)?, w(at + 4)?, w(at + 8)?, w(at + 12)? as usize);
            let data = b.get(at + 16..at + 16 + len)?.to_vec();
            at = (at + 16 + len).next_multiple_of(4);
            entries.push(Entry { key, ty, structured: flags & F_STRUCTURED != 0, read: flags & F_READ != 0, data });
        }
        let items = entries.split_off(na);
        Some(Blob { kind, attrs: entries, items })
    }

    fn keyed(&mut self, attr: bool) -> &mut Vec<Entry> { if attr { &mut self.attrs } else { &mut self.items } }

    /// Put (replace or append) `key` among the attributes or the parameters.
    pub fn put_key(&mut self, attr: bool, e: Entry) {
        let v = self.keyed(attr);
        match v.iter_mut().find(|x| x.key == e.key) { Some(x) => *x = e, None => v.push(e) }
    }

    pub fn key(&self, attr: bool, key: u32) -> Option<&Entry> { if attr { &self.attrs } else { &self.items }.iter().find(|x| x.key == key) }

    /// The first parameter not read and not optional (keyMissedKeywordAttr).
    pub fn missed(&self) -> Option<u32> {
        let optional: Vec<u32> = self.key(true, KEY_OPTIONAL_KEYWORD).and_then(|e| Blob::decode(&e.data)).map(|l| l.items.iter().filter_map(|i| i.data.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))).collect()).unwrap_or_default();
        self.items.iter().find(|e| !e.read && !optional.contains(&e.key)).map(|e| e.key)
    }
}

/// `data` (of type `from`) as type `to`: Ok((type, bytes)), or ERR_AE_COERCION_FAIL.
pub fn coerce(from: u32, data: &[u8], to: u32) -> Result<(u32, Vec<u8>), i16> {
    if to == from || to == TYPE_WILD_CARD { return Ok((from, data.to_vec())) }
    // the four-character code types carry the same 4 bytes (typeType, typeKeyword, typeEnumerated)
    let code = |t| matches!(t, TYPE_TYPE | TYPE_KEYWORD | TYPE_ENUMERATED);
    if code(from) && code(to) && data.len() == 4 { return Ok((to, data.to_vec())) }
    let int = || -> Option<i64> {
        match (from, data.len()) {
            (TYPE_SHORT, 2) => Some(i64::from(i16::from_be_bytes([data[0], data[1]]))),
            (TYPE_LONG, 4) => Some(i64::from(i32::from_be_bytes([data[0], data[1], data[2], data[3]]))),
            (TYPE_BOOLEAN, 1) => Some(i64::from(data[0] != 0)),
            (TYPE_TRUE, _) => Some(1),
            (TYPE_FALSE, _) => Some(0),
            _ => None,
        }
    };
    let v = int().ok_or(ERR_AE_COERCION_FAIL)?;
    match to {
        TYPE_SHORT => i16::try_from(v).map(|s| (to, s.to_be_bytes().to_vec())).map_err(|_| ERR_AE_COERCION_FAIL),
        TYPE_LONG => i32::try_from(v).map(|s| (to, s.to_be_bytes().to_vec())).map_err(|_| ERR_AE_COERCION_FAIL),
        TYPE_BOOLEAN if v == 0 || v == 1 => Ok((to, alloc::vec![v as u8])),
        _ => Err(ERR_AE_COERCION_FAIL),
    }
}

/// What the data-model calls fail with: an OSErr the guest gets, or a guest pointer that leaves memory (a named refusal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AeErr { Os(i16), Fault(String) }
impl From<i16> for AeErr { fn from(e: i16) -> Self { AeErr::Os(e) } }
pub type AeResult<T> = Result<T, AeErr>;

fn fault(what: &str, at: u32) -> AeErr { AeErr::Fault(alloc::format!("{what}: {at:#x} leaves guest RAM")) }

/// A descriptor's contents as the model sees it: simple bytes or a decoded blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content { Simple(Vec<u8>), Structured(Blob) }

impl<U: GuestUi> Mac<U> {
    /// (descriptorType, dataHandle) of the AEDesc at `at`.
    pub fn ae_desc(&self, at: u32) -> AeResult<(u32, u32)> {
        Ok((self.mem.read_u32(at).map_err(|_| fault("AEDesc", at))?, self.mem.read_u32(at + 4).map_err(|_| fault("AEDesc", at))?))
    }
    pub fn ae_set_desc(&mut self, at: u32, ty: u32, h: u32) -> AeResult<()> {
        if at == 0 { return Err(AeErr::Fault(String::from("AEDesc result pointer is NIL"))) }
        self.mem.write_u32(at, ty).and_then(|_| self.mem.write_u32(at + 4, h)).map_err(|_| fault("AEDesc result", at))
    }

    /// A new handle holding `bytes` (NIL for no bytes and type null).
    pub fn ae_new_handle(&mut self, bytes: &[u8]) -> AeResult<u32> {
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, bytes.len() as u32, false).map_err(|_| AeErr::Os(MEM_FULL))?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(AeErr::Os(MEM_FULL))?;
        self.mem.write_bytes(p, bytes).map_err(|_| fault("a descriptor's handle", p))?;
        Ok(h)
    }

    fn ae_write_handle(&mut self, h: u32, bytes: &[u8]) -> AeResult<()> {
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, bytes.len() as u32).map_err(|_| AeErr::Os(MEM_FULL))?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(AeErr::Os(ERR_AE_CORRUPT_DATA))?;
        self.mem.write_bytes(p, bytes).map_err(|_| fault("a descriptor's handle", p))
    }

    /// The contents of the descriptor at `at`. A list/record/event type whose handle is not a blob is errAECorruptData.
    pub fn ae_content(&self, at: u32) -> AeResult<(u32, Content)> {
        let (ty, h) = self.ae_desc(at)?;
        let bytes = if h == 0 { Vec::new() } else { self.handle_bytes(h).ok_or(AeErr::Os(ERR_AE_CORRUPT_DATA))? };
        if let Some(b) = Blob::decode(&bytes) { return Ok((ty, Content::Structured(b))) }
        if matches!(ty, TYPE_LIST | TYPE_RECORD | TYPE_APPLE_EVENT) && h != 0 { return Err(AeErr::Os(ERR_AE_CORRUPT_DATA)) }
        Ok((ty, Content::Simple(bytes)))
    }

    /// The blob of a list/record/event descriptor (errAEWrongDataType for a simple one).
    pub fn ae_blob(&self, at: u32) -> AeResult<Blob> {
        match self.ae_content(at)? {
            (_, Content::Structured(b)) => Ok(b),
            // a list/record/event descriptor with a NIL handle (never filled) is an empty one
            (TYPE_LIST, Content::Simple(_)) => Ok(Blob::new(Kind::List)),
            (TYPE_RECORD, Content::Simple(_)) => Ok(Blob::new(Kind::Record)),
            (TYPE_APPLE_EVENT, Content::Simple(_)) => Ok(Blob::new(Kind::Event)),
            _ => Err(AeErr::Os(ERR_AE_WRONG_DATA_TYPE)),
        }
    }

    /// Write `blob` back into the descriptor at `at` (its handle resized; a NIL handle is made).
    pub fn ae_store_blob(&mut self, at: u32, blob: &Blob) -> AeResult<()> {
        let (ty, h) = self.ae_desc(at)?;
        let bytes = blob.encode();
        if h == 0 { let h = self.ae_new_handle(&bytes)?; return self.ae_set_desc(at, ty, h) }
        self.ae_write_handle(h, &bytes)
    }

    /// A new descriptor at `out` from type and bytes (AECreateDesc); `structured` bytes are a nested blob.
    pub fn ae_create_desc(&mut self, out: u32, ty: u32, bytes: &[u8]) -> AeResult<()> {
        if ty == TYPE_NULL && bytes.is_empty() { return self.ae_set_desc(out, TYPE_NULL, 0) }
        let h = self.ae_new_handle(bytes)?;
        self.ae_set_desc(out, ty, h)
    }

    /// AEDisposeDesc: free the handle, the descriptor becomes the null descriptor.
    pub fn ae_dispose_desc(&mut self, at: u32) -> AeResult<()> {
        let (_, h) = self.ae_desc(at)?;
        if h != 0 { let _ = self.dispose_handle(h); }
        self.ae_set_desc(at, TYPE_NULL, 0)
    }

    /// AEDuplicateDesc.
    pub fn ae_duplicate_desc(&mut self, src: u32, out: u32) -> AeResult<()> {
        let (ty, h) = self.ae_desc(src)?;
        if h == 0 { return self.ae_set_desc(out, ty, 0) }
        let bytes = self.handle_bytes(h).ok_or(AeErr::Os(ERR_AE_CORRUPT_DATA))?;
        let h2 = self.ae_new_handle(&bytes)?;
        self.ae_set_desc(out, ty, h2)
    }

    /// AECreateList(factoringPtr, factoredSize, isRecord, &list): factoring is an optimisation of the real Manager's storage and changes
    /// nothing a program can see through the calls.
    pub fn ae_create_list(&mut self, out: u32, is_record: bool) -> AeResult<()> {
        let blob = Blob::new(if is_record { Kind::Record } else { Kind::List });
        let h = self.ae_new_handle(&blob.encode())?;
        self.ae_set_desc(out, if is_record { TYPE_RECORD } else { TYPE_LIST }, h)
    }

    /// The descriptor at `at` as an entry to store inside another (its type, its bytes or blob).
    pub fn ae_entry_of(&self, key: u32, at: u32) -> AeResult<Entry> {
        let (ty, c) = self.ae_content(at)?;
        Ok(match c { Content::Simple(data) => Entry { key, ty, structured: false, read: false, data }, Content::Structured(b) => Entry { key, ty, structured: true, read: false, data: b.encode() } })
    }

    /// A new descriptor at `out` holding an entry's value (a nested blob becomes its own handle).
    pub fn ae_desc_from_entry(&mut self, out: u32, e: &Entry) -> AeResult<()> { self.ae_create_desc(out, e.ty, &e.data) }

    /// AECountItems: items of a list, keyed items of a record, parameters of an event.
    pub fn ae_count_items(&self, at: u32) -> AeResult<u32> {
        match self.ae_content(at)? { (_, Content::Structured(b)) => Ok(b.items.len() as u32), _ => Err(AeErr::Os(ERR_AE_NOT_AE_DESC)) }
    }

    /// AEPutPtr / AEPutDesc / AEPutKeyPtr into a list: `index` 1..=count replaces, 0 or count+1 appends.
    pub fn ae_put_item(&mut self, list: u32, index: i32, e: Entry) -> AeResult<()> {
        let mut b = self.ae_blob(list)?;
        let n = b.items.len() as i32;
        if index == 0 || index == n + 1 { b.items.push(e) }
        else if (1..=n).contains(&index) { b.items[index as usize - 1] = e }
        else { return Err(AeErr::Os(ERR_AE_ILLEGAL_INDEX)) }
        self.ae_store_blob(list, &b)
    }

    /// The `index`th (1-based) item of a list or record: its keyword and entry.
    pub fn ae_nth(&self, list: u32, index: i32) -> AeResult<Entry> {
        let b = self.ae_blob(list)?;
        if index < 1 || index as usize > b.items.len() { return Err(AeErr::Os(ERR_AE_BAD_LIST_ITEM)) }
        Ok(b.items[index as usize - 1].clone())
    }

    pub fn ae_delete_item(&mut self, list: u32, index: i32) -> AeResult<()> {
        let mut b = self.ae_blob(list)?;
        if index < 1 || index as usize > b.items.len() { return Err(AeErr::Os(ERR_AE_BAD_LIST_ITEM)) }
        b.items.remove(index as usize - 1);
        self.ae_store_blob(list, &b)
    }

    /// AEPutParam* / AEPutAttribute* / AEPutKey*.
    pub fn ae_put_key(&mut self, at: u32, attr: bool, e: Entry) -> AeResult<()> {
        let mut b = self.ae_blob(at)?;
        if attr && b.kind != Kind::Event { return Err(AeErr::Os(ERR_AE_NOT_APPLE_EVENT)) }
        b.put_key(attr, e);
        self.ae_store_blob(at, &b)
    }

    /// AEGetParam* / AEGetAttribute* / AEGetKey*: the entry; reading a parameter marks it read (keyMissedKeywordAttr). `miss` is computed.
    pub fn ae_get_key(&mut self, at: u32, attr: bool, key: u32) -> AeResult<Entry> {
        let mut b = self.ae_blob(at)?;
        if attr && key == KEY_MISSED_KEYWORD {
            return b.missed().map(|k| Entry { key, ty: TYPE_KEYWORD, structured: false, read: true, data: k.to_be_bytes().to_vec() }).ok_or(AeErr::Os(ERR_AE_DESC_NOT_FOUND));
        }
        let found = b.keyed(attr).iter_mut().find(|x| x.key == key).ok_or(AeErr::Os(ERR_AE_DESC_NOT_FOUND))?;
        let first = !found.read;
        found.read = true;
        let e = found.clone();
        if !attr && first && b.kind == Kind::Event { self.ae_store_blob(at, &b)?; }
        Ok(e)
    }

    pub fn ae_delete_key(&mut self, at: u32, key: u32) -> AeResult<()> {
        let mut b = self.ae_blob(at)?;
        let n = b.items.len();
        b.items.retain(|x| x.key != key);
        if b.items.len() == n { return Err(AeErr::Os(ERR_AE_DESC_NOT_FOUND)) }
        self.ae_store_blob(at, &b)
    }

    /// AECreateAppleEvent(class, id, target AEAddressDesc*, returnID, transactionID, &event).
    pub fn ae_create_apple_event(&mut self, out: u32, class: u32, id: u32, target: u32, return_id: i16, transaction: u32) -> AeResult<()> {
        let mut b = Blob::new(Kind::Event);
        let rtid = if return_id == AUTO_GENERATE_RETURN_ID {
            self.ae.next_return_id = self.ae.next_return_id.wrapping_add(1).max(1);
            self.ae.next_return_id
        } else { return_id as u16 as u32 };
        let a = |key, ty, data: Vec<u8>| Entry { key, ty, structured: false, read: false, data };
        b.attrs.push(a(KEY_EVENT_CLASS, TYPE_TYPE, class.to_be_bytes().to_vec()));
        b.attrs.push(a(KEY_EVENT_ID, TYPE_TYPE, id.to_be_bytes().to_vec()));
        if target != 0 { b.attrs.push(self.ae_entry_of(KEY_ADDRESS, target)?); }
        b.attrs.push(a(KEY_RETURN_ID, TYPE_SHORT, (rtid as u16).to_be_bytes().to_vec()));
        b.attrs.push(a(KEY_TRANSACTION_ID, TYPE_LONG, transaction.to_be_bytes().to_vec()));
        let h = self.ae_new_handle(&b.encode())?;
        self.ae_set_desc(out, TYPE_APPLE_EVENT, h)
    }

    /// An event's class and id attributes.
    pub fn ae_class_id(&self, at: u32) -> AeResult<(u32, u32)> {
        let b = self.ae_blob(at)?;
        let get = |k| b.key(true, k).and_then(|e| e.data.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))).ok_or(AeErr::Os(ERR_AE_NOT_APPLE_EVENT));
        Ok((get(KEY_EVENT_CLASS)?, get(KEY_EVENT_ID)?))
    }

    /// Copy an entry's value, coerced to `desired`, into a guest buffer: (type, full size). The AEGet*Ptr body.
    pub fn ae_copy_out(&mut self, e: &Entry, desired: u32, type_out: u32, data: u32, max: u32, size_out: u32) -> AeResult<(u32, u32)> {
        let (ty, bytes) = if e.structured { if desired == e.ty || desired == TYPE_WILD_CARD { (e.ty, e.data.clone()) } else { return Err(AeErr::Os(ERR_AE_COERCION_FAIL)) } } else { coerce(e.ty, &e.data, desired)? };
        let n = (bytes.len() as u32).min(max);
        if data != 0 && n > 0 { self.mem.write_bytes(data, &bytes[..n as usize]).map_err(|_| fault("the data buffer", data))?; }
        if type_out != 0 { self.mem.write_u32(type_out, ty).map_err(|_| fault("typeCode result", type_out))?; }
        if size_out != 0 { self.mem.write_u32(size_out, bytes.len() as u32).map_err(|_| fault("actualSize result", size_out))?; }
        Ok((ty, bytes.len() as u32))
    }

    /// The bytes of a simple descriptor (or the blob of a structured one) - AEGetDescData / AEGetDescDataSize.
    pub fn ae_desc_bytes(&self, at: u32) -> AeResult<Vec<u8>> {
        let (_, h) = self.ae_desc(at)?;
        if h == 0 { return Ok(Vec::new()) }
        self.handle_bytes(h).ok_or(AeErr::Os(ERR_AE_CORRUPT_DATA))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blob_round_trips_with_nested_entries_and_flags() {
        let mut inner = Blob::new(Kind::List);
        inner.items.push(Entry { key: 0, ty: TYPE_LONG, structured: false, read: false, data: 7i32.to_be_bytes().to_vec() });
        let mut b = Blob::new(Kind::Event);
        b.attrs.push(Entry { key: KEY_EVENT_CLASS, ty: TYPE_TYPE, structured: false, read: false, data: fc(b"Alad").to_be_bytes().to_vec() });
        b.items.push(Entry { key: fc(b"kocl"), ty: fc(b"blob"), structured: false, read: true, data: alloc::vec![1, 2, 3] });
        b.items.push(Entry { key: fc(b"----"), ty: TYPE_LIST, structured: true, read: false, data: inner.encode() });
        let enc = b.encode();
        assert_eq!(enc.len() % 4, 0);
        assert_eq!(Blob::decode(&enc), Some(b.clone()));
        assert_eq!(Blob::decode(&b.items[1].data), Some(inner));
        assert_eq!(b.missed(), Some(fc(b"----")), "the unread parameter is the missed keyword");
    }

    #[test]
    fn simple_bytes_and_truncated_blobs_are_not_blobs() {
        assert_eq!(Blob::decode(b"ASer"), None);
        let mut enc = Blob::new(Kind::Record).encode();
        enc.extend_from_slice(&[0; 8]); // claims nothing more; trailing bytes are tolerated
        assert!(Blob::decode(&enc).is_some());
        let mut bad = Blob::new(Kind::Record);
        bad.items.push(Entry { key: 1, ty: 2, structured: false, read: false, data: alloc::vec![9; 10] });
        let e = bad.encode();
        assert_eq!(Blob::decode(&e[..e.len() - 4]), None, "an entry running past the end is malformed");
    }

    #[test]
    fn coercion_covers_identity_wildcard_integers_and_booleans_only() {
        assert_eq!(coerce(fc(b"TEXT"), b"hi", TYPE_WILD_CARD), Ok((fc(b"TEXT"), b"hi".to_vec())));
        assert_eq!(coerce(TYPE_SHORT, &(-3i16).to_be_bytes(), TYPE_LONG), Ok((TYPE_LONG, (-3i32).to_be_bytes().to_vec())));
        assert_eq!(coerce(TYPE_LONG, &70000i32.to_be_bytes(), TYPE_SHORT), Err(ERR_AE_COERCION_FAIL));
        assert_eq!(coerce(TYPE_TRUE, &[], TYPE_BOOLEAN), Ok((TYPE_BOOLEAN, alloc::vec![1])));
        assert_eq!(coerce(fc(b"TEXT"), b"12", TYPE_LONG), Err(ERR_AE_COERCION_FAIL));
    }
}
