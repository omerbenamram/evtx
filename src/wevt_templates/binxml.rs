//! BinXML parsing helpers for the WEVT template “inline-name” dialect.
//!
//! Template BinXML (inside CRIM/TTBL/TEMP) does not use the normal EVTX string-table name
//! references. Instead, element/attribute names are stored inline (MS-EVEN6 Name structure).
//! We expose token streams so higher-level code can render skeletons or apply substitution
//! values without duplicating deserializer setup.
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - MS-EVEN6 (BinXml `Name` structure and NameHash)

use encoding::EncodingRef;

pub(super) const TEMP_BINXML_OFFSET: usize = 40;

/// Parse the BinXML fragment embedded inside a `TEMP` entry.
///
/// `TEMP` wraps a BinXML fragment plus trailing item descriptors/names; callers often want
/// *just the BinXML* (as tokens) for offline rendering or inspection.
///
/// Returns `(tokens, bytes_consumed)` where `bytes_consumed` is the number of bytes read from the
/// BinXML fragment (starting at offset 40 from the beginning of `TEMP`).
pub fn parse_temp_binxml_fragment<'a>(
    temp_bytes: &'a [u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<(
    Vec<crate::model::deserialized::BinXMLDeserializedTokens<'a>>,
    u32,
)> {
    use crate::binxml::deserializer::BinXmlDeserializer;
    use crate::binxml::name::BinXmlNameEncoding;
    use crate::err::EvtxError;

    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let de = BinXmlDeserializer::init_with_name_encoding(
        binxml,
        0,
        None,
        true,
        ansi_codec,
        BinXmlNameEncoding::WevtInline,
    );

    let mut iterator = de.iter_tokens(None)?;
    let mut tokens = vec![];
    for t in iterator.by_ref() {
        tokens.push(t?);
    }

    let bytes_consumed = u32::try_from(iterator.position())
        .map_err(|_| EvtxError::calculation_error("BinXML fragment too large".to_string()))?;

    Ok((tokens, bytes_consumed))
}

/// Parse a WEVT_TEMPLATE BinXML fragment (inline-name encoding).
///
/// Some callers already have the BinXML slice (e.g. from a parsed `manifest::TemplateDefinition`)
/// and need to deserialize it using the inline-name rules.
///
/// Returns `(tokens, bytes_consumed)` where `bytes_consumed` is the number of bytes read from `binxml`.
pub fn parse_wevt_binxml_fragment<'a>(
    binxml: &'a [u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<(
    Vec<crate::model::deserialized::BinXMLDeserializedTokens<'a>>,
    u32,
)> {
    use crate::binxml::deserializer::BinXmlDeserializer;
    use crate::binxml::name::BinXmlNameEncoding;
    use crate::err::EvtxError;

    let de = BinXmlDeserializer::init_with_name_encoding(
        binxml,
        0,
        None,
        true,
        ansi_codec,
        BinXmlNameEncoding::WevtInline,
    );

    let mut iterator = de.iter_tokens(None)?;
    let mut tokens = vec![];
    for t in iterator.by_ref() {
        tokens.push(t?);
    }

    let bytes_consumed = u32::try_from(iterator.position())
        .map_err(|_| EvtxError::calculation_error("BinXML fragment too large".to_string()))?;

    Ok((tokens, bytes_consumed))
}
