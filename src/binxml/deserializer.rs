use crate::err::{self, Result};
use crate::evtx_parser::ReadSeek;
use snafu::{OptionExt, ResultExt};

use byteorder::ReadBytesExt;

use log::trace;
use std::io::{Seek, SeekFrom};

use crate::binxml::tokens::read_open_start_element;
use crate::binxml::value_variant::BinXmlValue;

use crate::{
    binxml::tokens::{
        read_attribute, read_entity_ref, read_fragment_header, read_substitution, read_template,
    },
    model::{deserialized::*, raw::*},
};

use crate::evtx_chunk::EvtxChunk;
use std::borrow::Cow;
use std::io::Cursor;
use std::mem;

pub struct IterTokens<'a> {
    cursor: Cursor<&'a [u8]>,
    chunk: Option<&'a EvtxChunk<'a>>,
    data_size: Option<u32>,
    data_read_so_far: u32,
    eof: bool,
}

pub struct BinXmlDeserializer<'a> {
    data: &'a [u8],
    offset: u64,
    chunk: Option<&'a EvtxChunk<'a>>,
}

impl<'a> BinXmlDeserializer<'a> {
    pub fn init(data: &'a [u8], start_offset: u64, chunk: Option<&'a EvtxChunk<'a>>) -> Self {
        BinXmlDeserializer {
            data,
            offset: start_offset,
            chunk,
        }
    }

    /// Returns a tuple of the tokens.
    pub fn read_binxml_fragment(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        data_size: Option<u32>,
    ) -> Result<Vec<BinXMLDeserializedTokens<'a>>> {
        let offset = cursor.position();

        let de = BinXmlDeserializer::init(*cursor.get_ref(), offset, chunk);

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

        trace!(
            "Position is {}, seeking {} bytes ahead",
            cursor.position(),
            seek_ahead
        );
        cursor
            .seek(SeekFrom::Current(seek_ahead as i64))
            .context(err::IO)?;

        Ok(tokens)
    }

    /// Reads `data_size` bytes of binary xml, or until EOF marker.
    pub fn iter_tokens(self, data_size: Option<u32>) -> Result<IterTokens<'a>> {
        let mut cursor = Cursor::new(self.data);
        cursor.seek(SeekFrom::Start(self.offset)).context(err::IO)?;

        Ok(IterTokens {
            cursor,
            chunk: self.chunk,
            data_size,
            data_read_so_far: 0,
            eof: false,
        })
    }
}

impl<'a> IterTokens<'a> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&self, cursor: &mut Cursor<&'a [u8]>) -> Result<BinXMLRawToken> {
        let token = try_read!(cursor, u8);

        Ok(BinXMLRawToken::from_u8(token).context(err::InvalidToken {
            value: token,
            offset: cursor.position(),
        })?)
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
                    read_open_start_element(cursor, self.chunk, token_information.has_attributes)?,
                ))
            }
            BinXMLRawToken::CloseStartElement => Ok(BinXMLDeserializedTokens::CloseStartElement),
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => Ok(BinXMLDeserializedTokens::CloseElement),
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(Cow::Owned(
                BinXmlValue::from_binxml_stream(cursor, self.chunk)?,
            ))),
            BinXMLRawToken::Attribute(_token_information) => Ok(
                BinXMLDeserializedTokens::Attribute(read_attribute(cursor, self.chunk)?),
            ),
            BinXMLRawToken::CDataSection => err::UnimplementedToken {
                name: "CDataSection",
                offset: cursor.position(),
            }
            .fail(),
            BinXMLRawToken::EntityReference => Ok(BinXMLDeserializedTokens::EntityRef(
                read_entity_ref(cursor, self.chunk)?,
            )),
            BinXMLRawToken::ProcessingInstructionTarget => err::UnimplementedToken {
                name: "ProcessingInstructionTarget",
                offset: cursor.position(),
            }
            .fail(),
            BinXMLRawToken::ProcessingInstructionData => err::UnimplementedToken {
                name: "ProcessingInstructionData",
                offset: cursor.position(),
            }
            .fail(),
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                read_template(cursor, self.chunk)?,
            )),
            BinXMLRawToken::NormalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                read_substitution(cursor, false)?,
            )),
            BinXMLRawToken::ConditionalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                read_substitution(cursor, true)?,
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

        trace!("offset_from_chunk_start: {}", offset_from_chunk_start);
        trace!(
            "need to read: {:?}, read so far: {}",
            self.data_size,
            self.data_read_so_far
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
                trace!("{:?} at {}", t, offset_from_chunk_start);
                let deserialized_token_result = self.visit_token(&mut cursor, t);

                trace!(
                    "{:?} position at stream {}",
                    deserialized_token_result,
                    cursor.position()
                );

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

    #[test]
    fn test_reads_a_single_record() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let mut chunk = EvtxChunkData::new(from_start_of_chunk.to_vec(), true).unwrap();
        let settings = ParserSettings::default();
        let mut evtx_chunk = chunk.parse(&settings).unwrap();
        let records = evtx_chunk.iter();

        for record in records.take(1) {
            assert!(record.is_ok(), record.unwrap().into_xml())
        }
    }

    #[test]
    fn test_record_formatting_does_not_contain_nul_bytes() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let mut chunk = EvtxChunkData::new(from_start_of_chunk.to_vec(), true).unwrap();
        let settings = ParserSettings::default();
        let mut evtx_chunk = chunk.parse(&settings).unwrap();
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
        let mut evtx_chunk = chunk.parse(&settings).unwrap();
        let records = evtx_chunk.iter();

        for record in records.into_iter() {
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
