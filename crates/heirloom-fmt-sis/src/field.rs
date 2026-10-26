use alloc::{string::String, vec::Vec};

use crate::SisError;

pub(crate) const SIS_STRING: u32 = 1;
pub(crate) const SIS_ARRAY: u32 = 2;
pub(crate) const SIS_COMPRESSED: u32 = 3;
pub(crate) const SIS_VERSION: u32 = 4;
pub(crate) const SIS_LANGUAGE: u32 = 11;
pub(crate) const SIS_CONTENTS: u32 = 12;
pub(crate) const SIS_CONTROLLER: u32 = 13;
pub(crate) const SIS_INFO: u32 = 14;
pub(crate) const SIS_SUPPORTED_LANGUAGES: u32 = 15;
pub(crate) const SIS_FILE_DESCRIPTION: u32 = 24;
pub(crate) const SIS_HASH: u32 = 25;
pub(crate) const SIS_IF: u32 = 26;
pub(crate) const SIS_ELSE_IF: u32 = 27;
pub(crate) const SIS_INSTALL_BLOCK: u32 = 28;
pub(crate) const SIS_DATA: u32 = 30;
pub(crate) const SIS_DATA_UNIT: u32 = 31;
pub(crate) const SIS_FILE_DATA: u32 = 32;
pub(crate) const SIS_CAPABILITIES: u32 = 41;

#[derive(Clone, Copy)]
pub(crate) struct Field<'a> {
    pub(crate) kind: u32,
    pub(crate) data: &'a [u8],
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    pub(crate) fn finish(&self) -> Result<(), SisError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(SisError::Malformed("trailing field data"))
        }
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, SisError> {
        let end = self
            .position
            .checked_add(4)
            .ok_or(SisError::Malformed("offset overflow"))?;
        if end > self.bytes.len() {
            return Err(SisError::Truncated("u32"));
        }
        let value = u32::from_le_bytes(
            self.bytes[self.position..end]
                .try_into()
                .map_err(|_| SisError::Truncated("u32"))?,
        );
        self.position = end;
        Ok(value)
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64, SisError> {
        let end = self
            .position
            .checked_add(8)
            .ok_or(SisError::Malformed("offset overflow"))?;
        if end > self.bytes.len() {
            return Err(SisError::Truncated("u64"));
        }
        let value = u64::from_le_bytes(
            self.bytes[self.position..end]
                .try_into()
                .map_err(|_| SisError::Truncated("u64"))?,
        );
        self.position = end;
        Ok(value)
    }

    pub(crate) fn read_field(&mut self) -> Result<Field<'a>, SisError> {
        let kind = self.read_u32()?;
        let data = self.read_sized_data()?;
        Ok(Field { kind, data })
    }

    /// Read a field body. SISArray elements use this same size header but omit
    /// the type word because their common type is stored in the array header.
    pub(crate) fn read_sized_data(&mut self) -> Result<&'a [u8], SisError> {
        let first = self.read_u32()?;
        let length = if first & 0x8000_0000 == 0 {
            u64::from(first)
        } else {
            let high = self.read_u32()?;
            u64::from(first & 0x7fff_ffff) | (u64::from(high) << 31)
        };

        let length = usize::try_from(length)
            .map_err(|_| SisError::Malformed("field does not fit in memory"))?;
        let start = self.position;
        let data_end = start
            .checked_add(length)
            .ok_or(SisError::Malformed("field length overflow"))?;
        if data_end > self.bytes.len() {
            return Err(SisError::Truncated("field data"));
        }

        let padded_length = length
            .checked_add(3)
            .ok_or(SisError::Malformed("field padding overflow"))?
            & !3;
        let padded_end = start
            .checked_add(padded_length)
            .ok_or(SisError::Malformed("field padding overflow"))?;
        if padded_end > self.bytes.len() {
            return Err(SisError::Truncated("field padding"));
        }

        self.position = padded_end;
        Ok(&self.bytes[start..data_end])
    }
}

pub(crate) fn single_field(bytes: &[u8], kind: u32) -> Result<&[u8], SisError> {
    let mut reader = Reader::new(bytes);
    let field = reader.read_field()?;
    reader.finish()?;
    if field.kind != kind {
        return Err(SisError::Malformed("unexpected field type"));
    }
    Ok(field.data)
}

pub(crate) fn array_element_type(bytes: &[u8]) -> Result<u32, SisError> {
    let mut reader = Reader::new(bytes);
    reader.read_u32()
}

pub(crate) fn for_each_array_element<'a>(
    bytes: &'a [u8],
    mut element: impl FnMut(&'a [u8]) -> Result<(), SisError>,
) -> Result<(), SisError> {
    let mut reader = Reader::new(bytes);
    let _element_type = reader.read_u32()?;
    while !reader.is_empty() {
        element(reader.read_sized_data()?)?;
    }
    Ok(())
}

pub(crate) fn exact_u32(bytes: &[u8]) -> Result<u32, SisError> {
    if bytes.len() != 4 {
        return Err(SisError::Malformed("u32 field length"));
    }
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

pub(crate) fn version(bytes: &[u8]) -> Result<(u32, u32, u32), SisError> {
    if bytes.len() != 12 {
        return Err(SisError::Malformed("SISVersion length"));
    }
    Ok((
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
    ))
}

pub(crate) fn string(bytes: &[u8]) -> Result<String, SisError> {
    // SISX strings are UTF-16LE field bodies. Accept the count-prefixed
    // spelling used by a few producers as well, but only when the count
    // consumes the complete field body.
    let mut body = bytes;
    if bytes.len() >= 4 {
        let count = u64::from(u32::from_le_bytes(bytes[0..4].try_into().unwrap()));
        if count
            .checked_mul(2)
            .and_then(|n| n.checked_add(4))
            == Some(bytes.len() as u64)
            || count.checked_add(4) == Some(bytes.len() as u64)
        {
            body = &bytes[4..];
        }
    }

    if body.len() % 2 != 0 {
        return Err(SisError::InvalidUtf16);
    }
    let mut units = Vec::with_capacity(body.len() / 2);
    for pair in body.chunks_exact(2) {
        units.push(u16::from_le_bytes([pair[0], pair[1]]));
    }
    if units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16(&units).map_err(|_| SisError::InvalidUtf16)
}