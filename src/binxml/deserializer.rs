use crate::err::{DeserializationError, DeserializationResult as Result};

use byteorder::ReadBytesExt;

use log::trace;
use std::io::{Seek, SeekFrom};

use crate::binxml::tokens::{
    read_open_start_element, read_processing_instruction_data, read_processing_instruction_target,
};
use crate::binxml::value_variant::BinXmlValue;

use crate::{
    binxml::tokens::{
        read_attribute, read_entity_ref, read_fragment_header, read_substitution_descriptor,
        read_template,
    },
    model::{deserialized::*, raw::*},
};

use crate::evtx_chunk::EvtxChunk;
use encoding::EncodingRef;

use std::io::Cursor;
use std::mem;

pub struct IterTokens<'a> {
    cursor: Cursor<&'a [u8]>,
    chunk: Option<&'a EvtxChunk<'a>>,
    data_size: Option<u32>,
    data_read_so_far: u32,
    eof: bool,
    is_inside_substitution: bool,
    ansi_codec: EncodingRef,
}

pub struct BinXmlDeserializer<'a> {
    data: &'a [u8],
    offset: u64,
    chunk: Option<&'a EvtxChunk<'a>>,
    // if called from substitution token with value type: Binary XML (0x21)
    is_inside_substitution: bool,
    ansi_codec: EncodingRef,
}

impl<'a> BinXmlDeserializer<'a> {
    pub fn init(
        data: &'a [u8],
        start_offset: u64,
        chunk: Option<&'a EvtxChunk<'a>>,
        is_inside_substitution: bool,
        ansi_codec: EncodingRef,
    ) -> Self {
        BinXmlDeserializer {
            data,
            offset: start_offset,
            chunk,
            is_inside_substitution,
            ansi_codec,
        }
    }

    /// Returns a tuple of the tokens.
    pub fn read_binxml_fragment(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        data_size: Option<u32>,
        is_inside_substitution: bool,
        ansi_codec: EncodingRef,
    ) -> Result<Vec<BinXMLDeserializedTokens<'a>>> {
        let offset = cursor.position();

        let de = BinXmlDeserializer::init(
            cursor.get_ref(),
            offset,
            chunk,
            is_inside_substitution,
            ansi_codec,
        );

        let mut tokens = vec![];
        let mut iterator = de.iter_tokens(data_size)?;

        loop {
            let token = iterator.next();
            if let Some(t) = token {
                tokens.push(t?);
            } else {
                break;
            }
        }

        let seek_ahead = iterator.cursor.position() - offset;
        cursor.seek(SeekFrom::Current(seek_ahead as i64))?;

        Ok(tokens)
    }

    /// Reads `data_size` bytes of binary xml, or until EOF marker.
    pub fn iter_tokens(self, data_size: Option<u32>) -> Result<IterTokens<'a>> {
        let mut cursor = Cursor::new(self.data);
        cursor.seek(SeekFrom::Start(self.offset))?;

        Ok(IterTokens {
            cursor,
            chunk: self.chunk,
            data_size,
            data_read_so_far: 0,
            eof: false,
            is_inside_substitution: self.is_inside_substitution,
            ansi_codec: self.ansi_codec,
        })
    }
}

impl<'a> IterTokens<'a> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&self, cursor: &mut Cursor<&'a [u8]>) -> Result<BinXMLRawToken> {
        let token = try_read!(cursor, u8)?;

        BinXMLRawToken::from_u8(token).ok_or(DeserializationError::InvalidToken {
            value: token,
            offset: cursor.position(),
        })
    }

    fn visit_token(
        &self,
        cursor: &mut Cursor<&'a [u8]>,
        raw_token: BinXMLRawToken,
    ) -> Result<BinXMLDeserializedTokens<'a>> {
        match raw_token {
            BinXMLRawToken::EndOfStream => Ok(BinXMLDeserializedTokens::EndOfStream),
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    read_open_start_element(
                        cursor,
                        self.chunk,
                        token_information.has_attributes,
                        self.is_inside_substitution,
                    )?,
                ))
            }
            BinXMLRawToken::CloseStartElement => Ok(BinXMLDeserializedTokens::CloseStartElement),
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => Ok(BinXMLDeserializedTokens::CloseElement),
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(
                BinXmlValue::from_binxml_stream(cursor, self.chunk, None, self.ansi_codec)?,
            )),
            BinXMLRawToken::Attribute(_token_information) => {
                Ok(BinXMLDeserializedTokens::Attribute(read_attribute(cursor)?))
            }
            BinXMLRawToken::CDataSection => Err(DeserializationError::UnimplementedToken {
                name: "CDataSection",
                offset: cursor.position(),
            }),
            BinXMLRawToken::CharReference => Err(DeserializationError::UnimplementedToken {
                name: "CharReference",
                offset: cursor.position(),
            }),
            BinXMLRawToken::EntityReference => Ok(BinXMLDeserializedTokens::EntityRef(
                read_entity_ref(cursor)?,
            )),
            BinXMLRawToken::ProcessingInstructionTarget => Ok(BinXMLDeserializedTokens::PITarget(
                read_processing_instruction_target(cursor)?,
            )),
            BinXMLRawToken::ProcessingInstructionData => Ok(BinXMLDeserializedTokens::PIData(
                read_processing_instruction_data(cursor)?,
            )),
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                read_template(cursor, self.chunk, self.ansi_codec)?,
            )),
            BinXMLRawToken::NormalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                read_substitution_descriptor(cursor, false)?,
            )),
            BinXMLRawToken::ConditionalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                read_substitution_descriptor(cursor, true)?,
            )),
            BinXMLRawToken::StartOfStream => Ok(BinXMLDeserializedTokens::FragmentHeader(
                read_fragment_header(cursor)?,
            )),
        }
    }
}

impl<'a> IterTokens<'a> {
    fn inner_next(&mut self) -> Option<Result<BinXMLDeserializedTokens<'a>>> {
        let mut cursor = self.cursor.clone();
        let offset_from_chunk_start = cursor.position();

        trace!(
            "Offset `0x{offset:08x} ({offset})`: need to read: {data:?}, read so far: {pos}",
            offset = offset_from_chunk_start,
            data = self.data_size,
            pos = self.data_read_so_far
        );

        // Finished reading
        match (self.data_size, self.eof) {
            (_, true) => {
                trace!("Finished reading - EOF reached");
                return None;
            }
            (Some(sz), _) => {
                if self.data_read_so_far >= sz {
                    trace!("Finished reading - end of data");
                    return None;
                }
            }
            _ => {}
        }

        let yield_value = match self.read_next_token(&mut cursor) {
            Ok(t) => {
                if let BinXMLRawToken::EndOfStream = t {
                    self.eof = true;
                }
                let deserialized_token_result = self.visit_token(&mut cursor, t);

                debug_assert!(
                    cursor.position() >= offset_from_chunk_start,
                    "Invalid state, cursor position at entering loop {}, now at {}",
                    offset_from_chunk_start,
                    cursor.position()
                );

                Some(deserialized_token_result)
            }
            Err(e) => Some(Err(e)),
        };
        let total_read = cursor.position() - offset_from_chunk_start;
        self.data_read_so_far += total_read as u32;

        mem::swap(&mut self.cursor, &mut cursor);
        yield_value
    }
}

impl<'a> Iterator for IterTokens<'a> {
    type Item = Result<BinXMLDeserializedTokens<'a>>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        self.inner_next()
    }
}

#[cfg(test)]
mod tests {
    use crate::evtx_chunk::EvtxChunkData;
    use crate::{ensure_env_logger_initialized, ParserSettings};
    use std::sync::Arc;

    #[test]
    fn test_reads_a_single_record() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let mut chunk = EvtxChunkData::new(from_start_of_chunk.to_vec(), true).unwrap();
        let settings = ParserSettings::default();
        let mut evtx_chunk = chunk.parse(Arc::new(settings)).unwrap();
        let records = evtx_chunk.iter();

        for record in records.take(1) {
            assert!(record.is_ok(), "Record failed to parse")
        }
    }

    #[test]
    fn test_record_formatting_does_not_contain_nul_bytes() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let mut chunk = EvtxChunkData::new(from_start_of_chunk.to_vec(), true).unwrap();
        let settings = ParserSettings::default();
        let mut evtx_chunk = chunk.parse(Arc::new(settings)).unwrap();
        let records = evtx_chunk.iter();

        for record in records.take(100) {
            assert!(!record
                .unwrap()
                .into_xml()
                .unwrap()
                .data
                .chars()
                .any(|c| c == '\0'))
        }
    }

    #[test]
    fn test_record_formatting_does_not_contain_nul_bytes_another_sample() {
        ensure_env_logger_initialized();
        let evtx_file =
            include_bytes!("../../samples/2-system-Microsoft-Windows-LiveId%4Operational.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let mut chunk = EvtxChunkData::new(from_start_of_chunk.to_vec(), true).unwrap();
        let settings = ParserSettings::default();
        let mut evtx_chunk = chunk.parse(Arc::new(settings)).unwrap();
        let records = evtx_chunk.iter();

        for record in records {
            let r = record.unwrap().into_xml().unwrap();
            for line in r.data.lines() {
                for char in line.chars() {
                    if char == '\0' {
                        panic!("{}", line);
                    }
                }
            }
        }
    }
}
