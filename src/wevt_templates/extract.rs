//! PE resource extraction for `WEVT_TEMPLATE` blobs (via `goblin`).
//!
//! Template definitions are shipped as PE resources (not inside EVTX files), so building an
//! *offline template cache* requires extracting those blobs without relying on Windows APIs.
//! We use `goblin` for PE + resource-directory parsing and keep only minimal glue code here.
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - Microsoft PE/COFF specification (resource directory layout)

use super::error::WevtTemplateExtractError;
use super::types::{ResourceIdentifier, WevtTemplateResource};
use crate::utils::bytes;

use goblin::pe::options::{ParseMode, ParseOptions};
use goblin::pe::resource::{ImageResourceDirectory, ResourceDataEntry, ResourceEntry};
use goblin::pe::section_table::SectionTable;

const IMAGE_RESOURCE_DIRECTORY_HEADER_SIZE: usize = 16;
const RESOURCE_DATA_ENTRY_SIZE: usize = 16;

fn rva_to_file_offset(
    sections: &[SectionTable],
    file_alignment: u32,
    opts: &ParseOptions,
    rva: u32,
) -> Option<usize> {
    goblin::pe::utils::find_offset(rva as usize, sections, file_alignment, opts)
}

const fn malformed(message: &'static str) -> WevtTemplateExtractError {
    WevtTemplateExtractError::MalformedResource { message }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(bytes::le(&b[o..]))
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(bytes::le(&b[o..]))
}

fn parse_image_resource_directory(
    rsrc: &[u8],
    offset: usize,
) -> Result<ImageResourceDirectory, WevtTemplateExtractError> {
    let b = bytes::read_array::<IMAGE_RESOURCE_DIRECTORY_HEADER_SIZE>(rsrc, offset)
        .ok_or(malformed("resource directory header out of bounds"))?;
    Ok(ImageResourceDirectory {
        characteristics: u32_at(&b, 0),
        time_date_stamp: u32_at(&b, 4),
        major_version: u16_at(&b, 8),
        minor_version: u16_at(&b, 10),
        number_of_named_entries: u16_at(&b, 12),
        number_of_id_entries: u16_at(&b, 14),
    })
}

fn parse_resource_name(rsrc: &[u8], offset: usize) -> Result<String, WevtTemplateExtractError> {
    let char_count = bytes::read_u16_le(rsrc, offset)
        .ok_or(malformed("resource name length out of bounds"))? as usize;
    // `start <= rsrc.len()` and `char_count < 2^16`, so this cannot overflow.
    let start = offset + 2;
    let buf = rsrc
        .get(start..start + char_count * 2)
        .ok_or(malformed("resource name out of bounds"))?;
    let chars = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]));
    char::decode_utf16(chars)
        .collect::<Result<String, _>>()
        .map_err(|_| WevtTemplateExtractError::InvalidResourceName)
}

fn entry_identifier(
    entry: ResourceEntry,
    rsrc: &[u8],
) -> Result<ResourceIdentifier, WevtTemplateExtractError> {
    if entry.name_is_string() {
        let name_offset = entry.name_offset() as usize;
        Ok(ResourceIdentifier::Name(parse_resource_name(
            rsrc,
            name_offset,
        )?))
    } else {
        Ok(ResourceIdentifier::Id(entry.name_offset()))
    }
}

fn directory_entries(
    rsrc: &[u8],
    dir_offset: usize,
) -> Result<Vec<ResourceEntry>, WevtTemplateExtractError> {
    let dir = parse_image_resource_directory(rsrc, dir_offset)?;
    // The header parse above already bounds `dir_offset + 16` by `rsrc.len()`.
    let entries_offset = dir_offset + IMAGE_RESOURCE_DIRECTORY_HEADER_SIZE;

    dir.next_iter(entries_offset, rsrc)
        .map_err(|_| malformed("resource directory entries out of bounds"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| malformed("failed to parse resource directory entries"))
}

fn parse_resource_data_entry(
    rsrc: &[u8],
    offset: usize,
) -> Result<ResourceDataEntry, WevtTemplateExtractError> {
    let b = bytes::read_array::<RESOURCE_DATA_ENTRY_SIZE>(rsrc, offset)
        .ok_or(malformed("resource data entry out of bounds"))?;
    Ok(ResourceDataEntry {
        offset_to_data: u32_at(&b, 0),
        size: u32_at(&b, 4),
        code_page: u32_at(&b, 8),
        reserved: u32_at(&b, 12),
    })
}

/// Extract `WEVT_TEMPLATE` resource blobs from a PE file.
///
/// `WEVT_TEMPLATE` is where providers store the CRIM/WEVT template metadata needed to render
/// events offline (e.g. for an on-disk cache, or for CLI tooling like
/// `evtx_dump extract-wevt-templates`). This function lets us obtain those blobs cross-platform.
///
/// Returns an empty vector if the PE has no resources or no `WEVT_TEMPLATE` resources.
pub fn extract_wevt_template_resources(
    pe_bytes: &[u8],
) -> Result<Vec<WevtTemplateResource>, WevtTemplateExtractError> {
    let mut opts = ParseOptions::default();
    // We only need sections + data directories; parsing TLS/certificates is wasted work here.
    opts.parse_tls_data = false;
    opts.parse_attribute_certificates = false;
    // Prefer permissive mode: we want template extraction to succeed even if unrelated tables
    // (imports/debug/etc) are slightly malformed.
    opts.parse_mode = ParseMode::Permissive;

    let pe = goblin::pe::PE::parse_with_opts(pe_bytes, &opts).map_err(|_| {
        WevtTemplateExtractError::InvalidPe {
            message: "failed to parse PE via goblin",
        }
    })?;

    let Some(optional_header) = pe.header.optional_header else {
        return Err(WevtTemplateExtractError::InvalidPe {
            message: "missing optional header",
        });
    };

    let Some(resource_table) = optional_header.data_directories.get_resource_table() else {
        return Ok(Vec::new());
    };

    if resource_table.virtual_address == 0 || resource_table.size == 0 {
        return Ok(Vec::new());
    }

    let file_alignment = optional_header.windows_fields.file_alignment;
    let sections = pe.sections;

    let rsrc_offset = rva_to_file_offset(
        &sections,
        file_alignment,
        &opts,
        resource_table.virtual_address,
    )
    .ok_or(WevtTemplateExtractError::UnmappedRva {
        rva: resource_table.virtual_address,
    })?;

    let rsrc_end = rsrc_offset
        .checked_add(resource_table.size as usize)
        .ok_or(WevtTemplateExtractError::MalformedPe {
            message: "resource directory overflow",
        })?;

    let rsrc =
        pe_bytes
            .get(rsrc_offset..rsrc_end)
            .ok_or(WevtTemplateExtractError::MalformedPe {
                message: "resource directory out of bounds",
            })?;

    let root_entries = directory_entries(rsrc, 0)?;

    let mut wevt_template_entry = None;
    for entry in root_entries {
        if !entry.name_is_string() {
            continue;
        }
        let name = parse_resource_name(rsrc, entry.name_offset() as usize)?;
        if name == "WEVT_TEMPLATE" {
            wevt_template_entry = Some(entry);
            break;
        }
    }

    let Some(wevt_template_entry) = wevt_template_entry else {
        return Ok(Vec::new());
    };
    if !wevt_template_entry.data_is_directory() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();

    let wevt_dir_offset = wevt_template_entry.offset_to_directory() as usize;
    for resource_entry in directory_entries(rsrc, wevt_dir_offset)? {
        if !resource_entry.data_is_directory() {
            continue;
        }
        let resource_id = entry_identifier(resource_entry, rsrc)?;

        let lang_dir_offset = resource_entry.offset_to_directory() as usize;
        for lang_entry in directory_entries(rsrc, lang_dir_offset)? {
            if lang_entry.name_is_string() {
                continue;
            }
            let lang_id = lang_entry.name_offset();

            let Some(data_entry_offset) = lang_entry.offset_to_data() else {
                continue;
            };
            let data_entry = parse_resource_data_entry(rsrc, data_entry_offset as usize)?;
            let data_rva = data_entry.offset_to_data;
            let data_size = data_entry.size as usize;
            if data_size == 0 {
                continue;
            }

            let data_offset = rva_to_file_offset(&sections, file_alignment, &opts, data_rva)
                .ok_or(WevtTemplateExtractError::UnmappedRva { rva: data_rva })?;

            let data_end = data_offset.checked_add(data_size).ok_or(
                WevtTemplateExtractError::MalformedPe {
                    message: "resource data overflow",
                },
            )?;
            let data = pe_bytes
                .get(data_offset..data_end)
                .ok_or(WevtTemplateExtractError::MalformedPe {
                    message: "resource data out of bounds",
                })?
                .to_vec();

            out.push(WevtTemplateResource {
                resource: resource_id.clone(),
                lang_id,
                data,
            });
        }
    }

    Ok(out)
}
