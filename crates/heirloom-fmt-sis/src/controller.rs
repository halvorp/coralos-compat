use alloc::{string::String, vec::Vec};

use crate::field::{
    self, array_element_type, exact_u32, for_each_array_element, single_field, Reader,
};
use crate::SisError;

pub(crate) struct FileDescription {
    pub(crate) target: String,
    pub(crate) mime: String,
    pub(crate) operation: u32,
    pub(crate) data_index: u32,
    pub(crate) is_conditional: bool,
}

pub(crate) struct Description {
    pub(crate) package_name: String,
    pub(crate) vendor: String,
    pub(crate) version: (u32, u32, u32),
    pub(crate) supported_languages: Vec<u32>,
    pub(crate) files: Vec<FileDescription>,
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Description, SisError> {
    let controller = single_field(bytes, field::SIS_CONTROLLER)?;
    let mut reader = Reader::new(controller);
    let mut package_name = None;
    let mut vendor = None;
    let mut version = None;
    let mut supported_languages = None;
    let mut files = Vec::new();

    while !reader.is_empty() {
        let current = reader.read_field()?;
        match current.kind {
            field::SIS_INFO => {
                let info = parse_info(current.data)?;
                if package_name.is_some() || vendor.is_some() || version.is_some() {
                    return Err(SisError::Malformed("duplicate SISInfo"));
                }
                package_name = Some(info.0);
                vendor = Some(info.1);
                version = Some(info.2);
            }
            field::SIS_SUPPORTED_LANGUAGES => {
                if supported_languages.is_some() {
                    return Err(SisError::Malformed("duplicate supported languages"));
                }
                supported_languages = Some(parse_languages(current.data)?);
            }
            field::SIS_INSTALL_BLOCK => parse_install_block(current.data, &mut files)?,
            _ => {}
        }
    }

    Ok(Description {
        package_name: package_name.ok_or(SisError::Missing("package name"))?,
        vendor: vendor.ok_or(SisError::Missing("vendor"))?,
        version: version.ok_or(SisError::Missing("package version"))?,
        supported_languages: supported_languages
            .ok_or(SisError::Missing("supported languages"))?,
        files,
    })
}

/// SISInfo: Uid, UniqueVendorName (a plain string), Names and VendorNames (one string per language), Version,
/// CreationTime, then two bytes. The package and vendor names are the first language's entries of the two arrays.
fn parse_info(bytes: &[u8]) -> Result<(String, String, (u32, u32, u32)), SisError> {
    let mut reader = Reader::new(bytes);
    let mut localized = Vec::new();
    let mut version = None;

    while reader.remaining() >= 8 {
        let current = reader.read_field()?;
        match current.kind {
            field::SIS_VERSION => version = Some(field::version(current.data)?),
            field::SIS_ARRAY if array_element_type(current.data)? == field::SIS_STRING => {
                let mut first = None;
                for_each_array_element(current.data, |element| {
                    if first.is_none() {
                        first = Some(field::string(element)?);
                    }
                    Ok(())
                })?;
                localized.push(first.ok_or(SisError::Missing("localized SISInfo string"))?);
            }
            // The Uid, UniqueVendorName and CreationTime do not name the package.
            _ => {}
        }
        if version.is_some() && localized.len() >= 2 {
            break;
        }
    }

    if localized.len() < 2 {
        return Err(SisError::Missing("SISInfo package and vendor names"));
    }
    Ok((
        localized.remove(0),
        localized.remove(0),
        version.ok_or(SisError::Missing("SISInfo version"))?,
    ))
}

fn parse_languages(bytes: &[u8]) -> Result<Vec<u32>, SisError> {
    let mut languages = Vec::new();
    let mut reader = Reader::new(bytes);
    while !reader.is_empty() {
        let current = reader.read_field()?;
        match current.kind {
            field::SIS_LANGUAGE => languages.push(exact_u32(current.data)?),
            field::SIS_ARRAY
                if array_element_type(current.data)? == field::SIS_LANGUAGE =>
            {
                for_each_array_element(current.data, |element| {
                    languages.push(exact_u32(element)?);
                    Ok(())
                })?;
            }
            _ => {}
        }
    }
    Ok(languages)
}

struct InstallFrame<'a> {
    reader: Reader<'a>,
    conditional: bool,
}

fn parse_install_block(
    bytes: &[u8],
    files: &mut Vec<FileDescription>,
) -> Result<(), SisError> {
    // The install-block tree is guest-controlled; use an explicit stack rather
    // than recursive descent so deeply nested conditions cannot overflow it.
    let mut stack = Vec::new();
    stack.push(InstallFrame {
        reader: Reader::new(bytes),
        conditional: false,
    });

    while !stack.is_empty() {
        let (kind, data, conditional) = {
            let frame = stack.last_mut().unwrap();
            if frame.reader.is_empty() {
                stack.pop();
                continue;
            }
            let current = frame.reader.read_field()?;
            (current.kind, current.data, frame.conditional)
        };

        match kind {
            field::SIS_FILE_DESCRIPTION => {
                let mut file = parse_file_description(data)?;
                file.is_conditional = conditional;
                files.push(file);
            }
            field::SIS_INSTALL_BLOCK => stack.push(InstallFrame {
                reader: Reader::new(data),
                conditional,
            }),
            field::SIS_IF | field::SIS_ELSE_IF => stack.push(InstallFrame {
                reader: Reader::new(data),
                conditional: true,
            }),
            // InstallBlock holds Array<FileDescription>, Array<Controller> (embedded packages, their own data unit,
            // not this package's files) and Array<If>; If holds Array<ElseIf>.
            field::SIS_ARRAY => match array_element_type(data)? {
                field::SIS_FILE_DESCRIPTION => for_each_array_element(data, |body| {
                    let mut file = parse_file_description(body)?;
                    file.is_conditional = conditional;
                    files.push(file);
                    Ok(())
                })?,
                field::SIS_IF | field::SIS_ELSE_IF => {
                    let mut bodies = Vec::new();
                    for_each_array_element(data, |body| {
                        bodies.push(body);
                        Ok(())
                    })?;
                    // Last first, so the stack visits the branches in package order.
                    for body in bodies.into_iter().rev() {
                        stack.push(InstallFrame { reader: Reader::new(body), conditional: true });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}

/// SISFileDescription: Target, MIMEType, Capabilities (optional), Hash, Operation u32, OperationOptions u32,
/// Length u64, UncompressedLength u64, FileIndex u32.
fn parse_file_description(bytes: &[u8]) -> Result<FileDescription, SisError> {
    let mut reader = Reader::new(bytes);

    let target = reader.read_field()?;
    if target.kind != field::SIS_STRING {
        return Err(SisError::Malformed("file target is not a string"));
    }
    let mime = reader.read_field()?;
    if mime.kind != field::SIS_STRING {
        return Err(SisError::Malformed("file MIME type is not a string"));
    }
    let mut next = reader.read_field()?;
    if next.kind == field::SIS_CAPABILITIES {
        next = reader.read_field()?;
    }
    if next.kind != field::SIS_HASH {
        return Err(SisError::Malformed("missing file hash"));
    }

    let operation = reader.read_u32()?;
    let _options = reader.read_u32()?;
    let _length = reader.read_u64()?;
    let _uncompressed_length = reader.read_u64()?;
    let data_index = reader.read_u32()?;
    reader.finish()?;

    Ok(FileDescription {
        target: field::string(target.data)?,
        mime: field::string(mime.data)?,
        operation,
        data_index,
        is_conditional: false,
    })
}