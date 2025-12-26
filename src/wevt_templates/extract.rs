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

use goblin::pe::header;
use goblin::pe::options::ParseOptions;
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

fn parse_image_resource_directory(
    rsrc: &[u8],
    offset: usize,
) -> Result<ImageResourceDirectory, WevtTemplateExtractError> {
    if offset + IMAGE_RESOURCE_DIRECTORY_HEADER_SIZE > rsrc.len() {
        return Err(WevtTemplateExtractError::MalformedResource {
            message: "resource directory header out of bounds",
        });
    }

    Ok(ImageResourceDirectory {
        characteristics: bytes::read_u32_le(rsrc, offset).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory characteristics out of bounds",
            },
        )?,
        time_date_stamp: bytes::read_u32_le(rsrc, offset + 4).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory time_date_stamp out of bounds",
            },
        )?,
        major_version: bytes::read_u16_le(rsrc, offset + 8).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory major_version out of bounds",
            },
        )?,
        minor_version: bytes::read_u16_le(rsrc, offset + 10).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory minor_version out of bounds",
            },
        )?,
        number_of_named_entries: bytes::read_u16_le(rsrc, offset + 12).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory number_of_named_entries out of bounds",
            },
        )?,
        number_of_id_entries: bytes::read_u16_le(rsrc, offset + 14).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource directory number_of_id_entries out of bounds",
            },
        )?,
    })
}

fn parse_resource_name(rsrc: &[u8], offset: usize) -> Result<String, WevtTemplateExtractError> {
    let char_count =
        bytes::read_u16_le(rsrc, offset).ok_or(WevtTemplateExtractError::MalformedResource {
            message: "resource name length out of bounds",
        })? as usize;

    let bytes_off = offset
        .checked_add(2)
        .ok_or(WevtTemplateExtractError::MalformedResource {
            message: "resource name offset overflow",
        })?;
    let bytes_len =
        char_count
            .checked_mul(2)
            .ok_or(WevtTemplateExtractError::MalformedResource {
                message: "resource name length overflow",
            })?;
    let bytes_end =
        bytes_off
            .checked_add(bytes_len)
            .ok_or(WevtTemplateExtractError::MalformedResource {
                message: "resource name end overflow",
            })?;

    let buf =
        rsrc.get(bytes_off..bytes_end)
            .ok_or(WevtTemplateExtractError::MalformedResource {
                message: "resource name out of bounds",
            })?;

    let mut chars = Vec::with_capacity(char_count);
    for i in 0..char_count {
        let c =
            bytes::read_u16_le(buf, i * 2).ok_or(WevtTemplateExtractError::MalformedResource {
                message: "resource name read out of bounds",
            })?;
        chars.push(c);
    }

    String::from_utf16(&chars).map_err(|_| WevtTemplateExtractError::InvalidResourceName)
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
    let entries_offset = dir_offset
        .checked_add(IMAGE_RESOURCE_DIRECTORY_HEADER_SIZE)
        .ok_or(WevtTemplateExtractError::MalformedResource {
            message: "resource directory entries offset overflow",
        })?;

    let it = dir.next_iter(entries_offset, rsrc).map_err(|_| {
        WevtTemplateExtractError::MalformedResource {
            message: "resource directory entries out of bounds",
        }
    })?;

    it.collect::<Result<Vec<_>, _>>()
        .map_err(|_| WevtTemplateExtractError::MalformedResource {
            message: "failed to parse resource directory entries",
        })
}

fn parse_resource_data_entry(
    rsrc: &[u8],
    offset: usize,
) -> Result<ResourceDataEntry, WevtTemplateExtractError> {
    if offset + RESOURCE_DATA_ENTRY_SIZE > rsrc.len() {
        return Err(WevtTemplateExtractError::MalformedResource {
            message: "resource data entry out of bounds",
        });
    }

    Ok(ResourceDataEntry {
        offset_to_data: bytes::read_u32_le(rsrc, offset).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource data entry RVA out of bounds",
            },
        )?,
        size: bytes::read_u32_le(rsrc, offset + 4).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource data entry size out of bounds",
            },
        )?,
        code_page: bytes::read_u32_le(rsrc, offset + 8).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource data entry code_page out of bounds",
            },
        )?,
        reserved: bytes::read_u32_le(rsrc, offset + 12).ok_or(
            WevtTemplateExtractError::MalformedResource {
                message: "resource data entry reserved out of bounds",
            },
        )?,
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
    // Note: We intentionally avoid `goblin::pe::PE::parse*` here.
    //
    // `PE::parse` eagerly parses multiple data directories (including resources) and will hard-fail
    // on some synthetic/minimal fixtures where those directories are "valid enough" for our use
    // but violate stricter PE invariants (e.g. `FileAlignment == 0`).
    //
    // We only need:
    // - the header (to locate the resource data directory)
    // - the section table (to map RVAs to file offsets)
    let header =
        header::Header::parse(pe_bytes).map_err(|_| WevtTemplateExtractError::InvalidPe {
            message: "failed to parse PE via goblin",
        })?;

    let Some(optional_header) = header.optional_header else {
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
    let opts = ParseOptions::default();

    let optional_header_offset = header.dos_header.pe_pointer as usize
        + header::SIZEOF_PE_MAGIC
        + header::SIZEOF_COFF_HEADER;
    let mut sections_offset =
        optional_header_offset + header.coff_header.size_of_optional_header as usize;
    let sections = header
        .coff_header
        .sections(pe_bytes, &mut sections_offset)
        .map_err(|_| WevtTemplateExtractError::MalformedPe {
            message: "failed to parse section headers",
        })?;

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
