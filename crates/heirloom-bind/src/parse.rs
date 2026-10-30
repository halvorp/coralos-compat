//! Generic IFF FORM parser driven by a [`BindingSpec`].

use alloc::string::String;
use alloc::vec::Vec;

use crate::spec::{BindingSpec, BoxField, CStrField, CountRule, FlagTest, IntWidth, StrListPick};

/// A parsed group of items (typically corresponding to one window/panel).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: Option<String>,
    pub window_box: Option<(i16, i16, i16, i16)>,
    /// The border ring (pixels) between an item's fill and its cell: the spec's inset, or 0 when the group's flag says none.
    pub inset: u32,
    pub labels: Vec<Label>,
}

/// A placement record: a file path and the (left, top, width, height) the program opens that file's group window at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub path: String,
    pub window_box: (i16, i16, i16, i16),
}

/// Everything one file holds for a spec: its groups and its placement records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub groups: Vec<Group>,
    pub placements: Vec<Placement>,
}

/// A parsed item label within a [`Group`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub text: String,
}

/// Result of checking a file header against a [`BindingSpec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recognised {
    NotThisForm,
    Legacy,
    Ours,
}

/// Reason a recognised file was refused during parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Malformed,
}

/// Inspects the first 12 bytes of `bytes` to see if it is an IFF `FORM` matching `spec`.
pub fn recognise(spec: &BindingSpec, bytes: &[u8]) -> Recognised {
    if bytes.len() < 12 {
        return Recognised::NotThisForm;
    }
    if &bytes[0..4] != b"FORM" {
        return Recognised::NotThisForm;
    }
    let form_type: [u8; 4] = [bytes[8], bytes[9], bytes[10], bytes[11]];
    if form_type == spec.file.form_type {
        return Recognised::Ours;
    }
    if spec.file.legacy_form_types.iter().any(|t| *t == form_type) {
        return Recognised::Legacy;
    }
    Recognised::NotThisForm
}

/// Parses an IFF `FORM` buffer according to `spec`.
pub fn parse(spec: &BindingSpec, bytes: &[u8]) -> Result<Parsed, Refusal> {
    if recognise(spec, bytes) != Recognised::Ours {
        return Err(Refusal::Malformed);
    }

    let form_size = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if form_size < 4 {
        return Err(Refusal::Malformed);
    }
    let form_end = 8usize.checked_add(form_size).ok_or(Refusal::Malformed)?;
    if form_end > bytes.len() {
        return Err(Refusal::Malformed);
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut placements: Vec<Placement> = Vec::new();
    let mut offset = 12usize;

    while offset < form_end {
        // Need 8 bytes for chunk header (id + size)
        if offset + 8 > form_end {
            return Err(Refusal::Malformed);
        }
        let chunk_id: [u8; 4] = [
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ];
        let chunk_size = u32::from_be_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;

        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(chunk_size)
            .ok_or(Refusal::Malformed)?;
        if data_end > form_end {
            return Err(Refusal::Malformed);
        }

        let chunk_data = &bytes[data_start..data_end];

        if let Some(pl) = spec.placement.as_ref().filter(|pl| pl.chunk == chunk_id) {
            if chunk_size < pl.min_size as usize {
                return Err(Refusal::Malformed);
            }
            let path = read_cstr_field(chunk_data, &pl.path)?;
            if !path.is_empty() {
                placements.push(Placement { path, window_box: read_box_field(chunk_data, &pl.window_box)? });
            }
        } else if chunk_id == spec.group.chunk {
            if chunk_size != spec.group.size as usize {
                return Err(Refusal::Malformed);
            }
            let title = match &spec.group.title {
                Some(field) => {
                    let s = read_cstr_field(chunk_data, field)?;
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                }
                None => None,
            };
            let window_box = match &spec.group.window_box {
                Some(field) => Some(read_box_field(chunk_data, field)?),
                None => None,
            };
            let inset = match &spec.group.no_inset_if {
                Some(flag) if eval_flag(chunk_data, flag)? => 0,
                _ => spec.group.inset,
            };
            groups.push(Group {
                title,
                window_box,
                inset,
                labels: Vec::new(),
            });
        } else if chunk_id == spec.item.chunk {
            if let Some(current_group) = groups.last_mut() {
                if chunk_size < spec.item.min_size as usize {
                    return Err(Refusal::Malformed);
                }
                let skip = match &spec.item.skip_if {
                    Some(flag) => eval_flag(chunk_data, flag)?,
                    None => false,
                };
                if !skip {
                    read_labels(chunk_data, &spec.item.label, &mut current_group.labels)?;
                }
            } else {
                // An item before any group belongs to no group and is skipped, after the same size check every item gets.
                if chunk_size < spec.item.min_size as usize {
                    return Err(Refusal::Malformed);
                }
            }
        }

        offset = data_end;
        if chunk_size % 2 == 1 {
            if offset < form_end {
                offset += 1;
            }
            // If offset == form_end, tolerate missing final pad byte at end of FORM.
        }
    }

    Ok(Parsed { groups, placements })
}

fn read_int(data: &[u8], at: u32, width: IntWidth) -> Result<u32, Refusal> {
    let pos = at as usize;
    match width {
        IntWidth::U8 => {
            if pos + 1 > data.len() {
                return Err(Refusal::Malformed);
            }
            Ok(data[pos] as u32)
        }
        IntWidth::I16 => {
            if pos + 2 > data.len() {
                return Err(Refusal::Malformed);
            }
            let v = i16::from_be_bytes([data[pos], data[pos + 1]]);
            if v < 0 {
                return Err(Refusal::Malformed);
            }
            Ok(v as u32)
        }
        IntWidth::U16 => {
            if pos + 2 > data.len() {
                return Err(Refusal::Malformed);
            }
            Ok(u16::from_be_bytes([data[pos], data[pos + 1]]) as u32)
        }
        IntWidth::U32 => {
            if pos + 4 > data.len() {
                return Err(Refusal::Malformed);
            }
            Ok(u32::from_be_bytes([
                data[pos],
                data[pos + 1],
                data[pos + 2],
                data[pos + 3],
            ]))
        }
    }
}

fn eval_flag(data: &[u8], flag: &FlagTest) -> Result<bool, Refusal> {
    let val = read_int(data, flag.at, flag.width)?;
    Ok((val & flag.mask) != 0)
}

fn read_cstr_field(data: &[u8], field: &CStrField) -> Result<String, Refusal> {
    let start = field.at as usize;
    let max = field.max as usize;
    if start >= data.len() {
        return Err(Refusal::Malformed);
    }
    let end = start.saturating_add(max).min(data.len());
    let slice = &data[start..end];
    let nul_pos = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    Ok(latin1_to_utf8(&slice[..nul_pos]))
}

fn read_box_field(data: &[u8], field: &BoxField) -> Result<(i16, i16, i16, i16), Refusal> {
    let start = field.at as usize;
    let end = start.checked_add(8).ok_or(Refusal::Malformed)?;
    if end > data.len() {
        return Err(Refusal::Malformed);
    }
    let left = i16::from_be_bytes([data[start], data[start + 1]]);
    let top = i16::from_be_bytes([data[start + 2], data[start + 3]]);
    let width = i16::from_be_bytes([data[start + 4], data[start + 5]]);
    let height = i16::from_be_bytes([data[start + 6], data[start + 7]]);
    Ok((left, top, width, height))
}

fn read_labels(data: &[u8], rule: &StrListPick, out: &mut Vec<Label>) -> Result<(), Refusal> {
    if rule.per_record == 0 || rule.pick >= rule.per_record {
        return Err(Refusal::Malformed);
    }

    let count = match &rule.count {
        CountRule::Field {
            at,
            width,
            unless_flag,
        } => {
            if let Some((flag_test, fallback)) = unless_flag {
                // "unless_flag": unless the flag is set (i.e. if !flag_test), use fallback;
                // if the flag IS set, read the count field at `at`.
                if !eval_flag(data, flag_test)? {
                    *fallback
                } else {
                    read_int(data, *at, *width)?
                }
            } else {
                read_int(data, *at, *width)?
            }
        }
    };

    let mut pos = rule.strings_at as usize;
    if pos > data.len() {
        return Err(Refusal::Malformed);
    }

    // Every record is walked (so a list that runs past the chunk is refused), but the program draws only the FIRST record's
    // `pick` string: later records are its other functions, never a second button.
    for record in 0..count {
        for idx in 0..rule.per_record {
            let rel_nul = data[pos..]
                .iter()
                .position(|&b| b == 0)
                .ok_or(Refusal::Malformed)?;
            let s_bytes = &data[pos..pos + rel_nul];
            pos += rel_nul + 1;
            if record == 0 && idx == rule.pick && !s_bytes.is_empty() {
                out.push(Label {
                    text: latin1_to_utf8(s_bytes),
                });
            }
        }
    }

    Ok(())
}

/// The guest's strings are Latin-1 (ISO 8859-1): each byte IS the code point U+0000..U+00FF, so the conversion to a UTF-8 `String`
/// is exact and total (UTF8-EVERYWHERE: converted at the boundary, never carried as raw bytes).
fn latin1_to_utf8(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}