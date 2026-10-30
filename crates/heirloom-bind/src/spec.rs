//! Declarative schema types for the generic binding interpreter.

use alloc::string::String;
use alloc::vec::Vec;

/// Top-level binding specification for one file format / version family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingSpec {
    pub id: String,
    pub file: FileForm,
    pub group: GroupLayout,
    pub item: ItemLayout,
    /// Where the program records a group's window placement, keyed by the path of the file the group was read from (a separate
    /// record, often in another file of the same form): `None` = groups anchor by title or exact box only.
    pub placement: Option<PlacementLayout>,
    pub anchor: AnchorRule,
}

/// A placement record: the window box and the path of the file whose group opens there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementLayout {
    pub chunk: [u8; 4],
    pub min_size: u32,
    pub window_box: BoxField,
    pub path: CStrField,
}

/// IFF FORM identification rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileForm {
    pub form_type: [u8; 4],
    pub legacy_form_types: Vec<[u8; 4]>,
}

/// Layout of a group chunk (which corresponds to a window or panel).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupLayout {
    pub chunk: [u8; 4],
    pub size: u32,
    pub title: Option<CStrField>,
    pub window_box: Option<BoxField>,
    /// The format's documented border: the program draws each item's fill `inset` pixels inside its cell (its own edge lines in
    /// that ring), except when `no_inset_if` holds for the group (a borderless bank).
    pub inset: u32,
    pub no_inset_if: Option<FlagTest>,
}

/// Layout of an item chunk inside a group (which corresponds to a control/button).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemLayout {
    pub chunk: [u8; 4],
    pub min_size: u32,
    pub skip_if: Option<FlagTest>,
    pub label: StrListPick,
}

/// A NUL-terminated Latin-1 string field inside a chunk: at most `max` bytes from `at`, and never past the chunk's end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CStrField {
    pub at: u32,
    pub max: u32,
}

/// Four big-endian `i16` values `(left, top, width, height)` at `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxField {
    pub at: u32,
}

/// Bitmask test against an integer field inside a chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagTest {
    pub at: u32,
    pub width: IntWidth,
    pub mask: u32,
}

/// Integer width for big-endian field reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntWidth {
    U8,
    I16,
    U16,
    U32,
}

/// Rule for extracting a label from a packed list of null-terminated strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrListPick {
    pub strings_at: u32,
    pub count: CountRule,
    pub per_record: u32,
    pub pick: u32,
}

/// Rule for determining how many records exist in an item chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountRule {
    Field {
        at: u32,
        width: IntWidth,
        unless_flag: Option<(FlagTest, u32)>,
    },
}

/// Rule for anchoring parsed group labels onto a window's `DrawOp` stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorRule {
    LabelInFill {
        prefix_cut: bool,
        strip_lines_inside: bool,
        accent_from_swatch: bool,
    },
}