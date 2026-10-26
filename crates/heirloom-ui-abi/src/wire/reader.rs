//! The bounded reader: every length is checked against what remains BEFORE anything is allocated.
use super::*;

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub(super) fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    pub(super) fn take(&mut self, count: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .at
            .checked_add(count)
            .ok_or(WireError::Truncated("length"))?;
        if end > self.bytes.len() {
            return Err(WireError::Truncated("payload"));
        }
        let result = &self.bytes[self.at..end];
        self.at = end;
        Ok(result)
    }

    pub(super) fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut result = [0; N];
        result.copy_from_slice(self.take(N)?);
        Ok(result)
    }

    pub(super) fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(super) fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(super) fn boolean(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(WireError::UnknownTag { what: "bool", tag }),
        }
    }

    pub(super) fn string(&mut self) -> Result<String, WireError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        let text = core::str::from_utf8(bytes).map_err(|_| WireError::BadUtf8("string"))?;
        Ok(String::from(text))
    }

    pub(super) fn character(&mut self) -> Result<char, WireError> {
        core::char::from_u32(self.u32()?).ok_or(WireError::BadUtf8("char"))
    }

    pub(super) fn usize(&mut self) -> Result<usize, WireError> {
        usize::try_from(self.u64()?).map_err(|_| WireError::Truncated("usize"))
    }

    pub(super) fn vec<T>(
        &mut self,
        minimum_encoded_size: usize,
        mut get: impl FnMut(&mut Reader<'_>) -> Result<T, WireError>,
    ) -> Result<Vec<T>, WireError> {
        let count = self.u32()? as usize;
        // Reject impossible counts before reserving. Every element reachable from
        // this codec occupies at least one byte, and callers provide tighter bounds.
        if minimum_encoded_size == 0
            || count > self.remaining() / minimum_encoded_size
        {
            return Err(WireError::Truncated("vector element count"));
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(get(self)?);
        }
        Ok(values)
    }

    pub(super) fn option<T>(
        &mut self,
        get: impl FnOnce(&mut Reader<'_>) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(get(self)?)),
            tag => Err(WireError::UnknownTag { what: "Option", tag }),
        }
    }
}
