use alloc::vec::Vec;

use crate::field::{
    self, array_element_type, for_each_array_element, single_field, Reader,
};
use crate::SisError;

pub(crate) struct FileData {
    /// SISData's data units, each its SISFileData blobs in order. Unit 0 belongs to the package's own controller.
    units: Vec<Vec<Vec<u8>>>,
}

impl FileData {
    /// The bytes a SISFileDescription's FileIndex names: an index into data unit 0's file data.
    pub(crate) fn file_bytes(&self, index: u32) -> Result<&[u8], SisError> {
        let i = usize::try_from(index).map_err(|_| SisError::DataIndex(index))?;
        self.units.first().and_then(|u| u.get(i)).map(Vec::as_slice).ok_or(SisError::DataIndex(index))
    }
}

pub(crate) fn decompress_field(bytes: &[u8]) -> Result<Vec<u8>, SisError> {
    let mut reader = Reader::new(bytes);
    let algorithm = reader.read_u32()?;
    let uncompressed_size = reader.read_u64()?;
    // SISCompressed: algorithm u32, uncompressed size u64, then the data - a 12-byte head.
    let compressed = bytes.get(12..).ok_or(SisError::Malformed("SISCompressed head"))?;

    match algorithm {
        0 => {
            if compressed.len() as u64 != uncompressed_size {
                return Err(SisError::SizeMismatch {
                    expected: uncompressed_size,
                    actual: compressed.len() as u64,
                });
            }
            Ok(compressed.to_vec())
        }
        1 => {
            let limit = usize::try_from(uncompressed_size)
                .map_err(|_| SisError::Malformed("uncompressed size does not fit in memory"))?;
            let output = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                compressed,
                limit,
            )
            .map_err(|_| SisError::Decompression)?;
            if output.len() as u64 != uncompressed_size {
                return Err(SisError::SizeMismatch {
                    expected: uncompressed_size,
                    actual: output.len() as u64,
                });
            }
            Ok(output)
        }
        _ => Err(SisError::Unsupported("SIS compression algorithm")),
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<FileData, SisError> {
    let mut units = Vec::new();
    let mut reader = Reader::new(bytes);
    while !reader.is_empty() {
        let current = reader.read_field()?;
        match current.kind {
            field::SIS_DATA_UNIT => units.push(parse_data_unit(current.data)?),
            field::SIS_ARRAY
                if array_element_type(current.data)? == field::SIS_DATA_UNIT =>
            {
                for_each_array_element(current.data, |element| {
                    units.push(parse_data_unit(element)?);
                    Ok(())
                })?;
            }
            _ => {}
        }
    }
    Ok(FileData { units })
}

fn parse_data_unit(bytes: &[u8]) -> Result<Vec<Vec<u8>>, SisError> {
    let mut alternatives = Vec::new();
    let mut reader = Reader::new(bytes);
    while !reader.is_empty() {
        let current = reader.read_field()?;
        match current.kind {
            field::SIS_FILE_DATA => alternatives.push(parse_file_data(current.data)?),
            field::SIS_ARRAY
                if array_element_type(current.data)? == field::SIS_FILE_DATA =>
            {
                for_each_array_element(current.data, |element| {
                    alternatives.push(parse_file_data(element)?);
                    Ok(())
                })?;
            }
            _ => {}
        }
    }
    Ok(alternatives)
}

fn parse_file_data(bytes: &[u8]) -> Result<Vec<u8>, SisError> {
    decompress_field(single_field(bytes, field::SIS_COMPRESSED)?)
}