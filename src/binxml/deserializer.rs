use byteorder::{LittleEndian, ReadBytesExt};

use log::{debug, log, trace};
use std::io::{Seek, SeekFrom};

use crate::binxml::tokens::read_open_start_element;
use crate::binxml::value_variant::BinXMLValue;
use crate::string_cache::CachedString;
use crate::{
    binxml::tokens::{
        read_attribute, read_entity_ref, read_fragment_header, read_substitution, read_template,
    },
    error::Error,
    evtx::ReadSeek,
    guid::Guid,
    model::{deserialized::*, raw::*, xml::*},
    string_cache::StringCache,
    template_cache::TemplateCache,
    utils::datetime_from_filetime,
    utils::*,
    xml_output::BinXMLOutput,
    Offset,
};
use std::io::Cursor;
use std::rc::Rc;

pub struct BinXmlDeserializer<'a, T: ReadSeek + 'a> {
    cursor: Cursor<&'a T>,
    ctx: ParsingContext<'a>,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsingContext<'a> {
    offset: u64,
    string_cache: Option<&'a StringCache>,
    template_cache: Option<&'a TemplateCache<'a>>,
}

impl<'a> ParsingContext<'a> {
    pub fn cached_string_at_offset(&self, offset: Offset) -> Option<&CachedString> {
        match self.string_cache {
            Some(cache) => cache.get_string_and_hash(offset),
            None => None,
        }
    }
}

pub struct IterTokens<'a, T: ReadSeek + 'a> {
    state: BinXmlDeserializer<'a, T>,
    data_size: Option<u32>,
    data_read_so_far: u32,
    eof: bool,
}

impl<'a, T: ReadSeek + 'a> BinXmlDeserializer<'a, T> {
    pub fn init(
        data: &'a T,
        start_offset: u64,
        string_cache: &'a StringCache,
        template_cache: &'a TemplateCache,
    ) -> Self {
        let cursor = Cursor::new(data);
        let ctx = ParsingContext {
            offset: start_offset,
            string_cache: Some(string_cache),
            template_cache: Some(template_cache),
        };

        BinXmlDeserializer { cursor, ctx }
    }

    pub fn from_ctx(data: &'a T, ctx: &ParsingContext<'a>) -> Self {
        let cursor = Cursor::new(data);
        BinXmlDeserializer {
            cursor,
            ctx: ctx.clone(),
        }
    }

    pub fn init_without_cache(data: &'a T, start_offset: u64) -> Self {
        let cursor = Cursor::new(data);
        let ctx = ParsingContext {
            offset: start_offset,
            string_cache: None,
            template_cache: None,
        };

        BinXmlDeserializer { cursor, ctx }
    }

    pub fn wrap_cursor(cursor: Cursor<&'a T>) -> Self {
        let offset = cursor.position();

        let ctx = ParsingContext {
            offset,
            string_cache: None,
            template_cache: None,
        };

        BinXmlDeserializer { cursor, ctx }
    }

    /// Reads `data_size` bytes of binary xml, or until EOF marker.
    pub fn iter_tokens(self, data_size: Option<u32>) -> IterTokens<'a, T> {
        IterTokens {
            state: self,
            data_size,
            data_read_so_far: 0,
            eof: false,
        }
    }
}

impl<'a, T: ReadSeek + 'a> IterTokens<'a, T> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&self, cursor: &mut T) -> Result<BinXMLRawToken, Error> {
        let token = cursor
            .read_u8()
            .map_err(|e| Error::unexpected_eof(e, cursor.stream_position().unwrap()))?;

        Ok(BinXMLRawToken::from_u8(token).ok_or_else(|| {
            Error::not_a_valid_binxml_token(token, cursor.stream_position().unwrap())
        })?)
    }

    fn visit_token(
        &self,
        cursor: &'a mut Cursor<T>,
        ctx: &'a ParsingContext<'a>,
        raw_token: BinXMLRawToken,
    ) -> Result<BinXMLDeserializedTokens<'a>, Error> {
        match raw_token {
            BinXMLRawToken::EndOfStream => Ok(BinXMLDeserializedTokens::EndOfStream),
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    read_open_start_element(cursor, &ctx, token_information.has_attributes)?,
                ))
            }
            BinXMLRawToken::CloseStartElement => Ok(BinXMLDeserializedTokens::CloseStartElement),
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => Ok(BinXMLDeserializedTokens::CloseElement),
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(
                BinXMLValue::from_binxml_stream(cursor, ctx)?,
            )),
            BinXMLRawToken::Attribute(token_information) => Ok(
                BinXMLDeserializedTokens::Attribute(read_attribute(cursor, ctx)?),
            ),
            BinXMLRawToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLRawToken::EntityReference => Ok(BinXMLDeserializedTokens::EntityRef(
                read_entity_ref(cursor, ctx)?,
            )),
            BinXMLRawToken::ProcessingInstructionTarget => {
                unimplemented!("BinXMLToken::ProcessingInstructionTarget")
            }
            BinXMLRawToken::ProcessingInstructionData => {
                unimplemented!("BinXMLToken::ProcessingInstructionData")
            }
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                read_template(cursor, &ctx)?,
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

impl<'a, T: ReadSeek + 'a> Iterator for IterTokens<'a, T> {
    type Item = Result<BinXMLDeserializedTokens<'a>, Error>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let mut cursor = &mut self.state.cursor;
        let offset_from_chunk_start = cursor.position();

        trace!("offset_from_chunk_start: {}", offset_from_chunk_start);
        trace!(
            "need to read: {:?}, read so far: {}",
            self.data_size,
            self.data_read_so_far
        );

        // Finished reading
        match (self.data_size, self.eof) {
            (_, true) => return None,
            (Some(sz), _) => {
                if self.data_read_so_far >= sz {
                    return None;
                }
            }
        }

        match self.read_next_token(cursor) {
            Ok(t) => {
                if let BinXMLRawToken::EndOfStream = t {
                    self.eof = true;
                }

                trace!("{:?} at {}", t, offset_from_chunk_start);
                let token = self.visit_token(cursor, &self.state.ctx, t);
                trace!("{:?} position at stream {}", token, cursor.position());

                debug_assert!(
                    cursor.position() >= offset_from_chunk_start,
                    "Invalid state, cursor position at entering loop {}, now at {}",
                    offset_from_chunk_start,
                    cursor.position()
                );

                let total_read = cursor.position() - offset_from_chunk_start;
                offset_from_chunk_start += total_read;
                self.data_read_so_far += total_read as u32;

                Some(token)
            }
            Err(e) => {
                // Cursor might have not been moved if this error was thrown in middle of seek.
                // So seek all the way to end.
                debug_assert!(
                    if let Some(limit) = self.data_size {
                        limit >= self.data_read_so_far
                    } else {
                        false
                    },
                    "Invalid state! read too much data! data_size is {:?}, read to {}",
                    self.data_size,
                    self.data_read_so_far
                );

                let total_read = self.data_size - self.data_read_so_far;
                self.start_offset += u64::from(total_read);
                self.data_read_so_far += total_read as u32;

                Some(Err(e))
            }
        }
    }
}

mod tests {
    use super::*;
    use crate::ensure_env_logger_initialized;
    use crate::evtx_chunk::EvtxChunk;
    use crate::evtx_record::EvtxRecordHeader;
    use std::borrow::BorrowMut;
    use std::io::Read;

    const EVTX_CHUNK_SIZE: usize = 65536;
    const EVTX_HEADER_SIZE: usize = 4096;
    const EVTX_RECORD_HEADER_SIZE: usize = 24;

    #[test]
    fn test_read_name_bug() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");

        let mut cursor = Cursor::new(&evtx_file[EVTX_HEADER_SIZE + EVTX_CHUNK_SIZE..]);
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        cursor
            .borrow_mut()
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .unwrap();

        let chunk = EvtxChunk::new(chunk_data).unwrap();
        let mut cursor = Cursor::new(chunk.data.as_slice());

        // Seek to bad record position
        cursor.seek(SeekFrom::Start(3872)).unwrap();

        let record_header = EvtxRecordHeader::from_reader(&mut cursor).unwrap();
        let mut data = Vec::with_capacity(record_header.data_size as usize);

        cursor
            .take(u64::from(record_header.data_size))
            .read_to_end(&mut data)
            .unwrap();

        let deser = BinXmlDeserializer::from_chunk_at_offset(
            &chunk,
            (3872_usize + EVTX_RECORD_HEADER_SIZE) as u64,
            record_header.data_size - 4 - 4 - 4 - 8 - 8,
        );

        for token in deser {
            if let Err(e) = token {
                let mut cursor = Cursor::new(chunk.data.as_slice());
                println!("{}", e);
                cursor.seek(SeekFrom::Start(e.offset())).unwrap();
                dump_cursor(&mut cursor, 10);
                panic!();
            }
        }
    }

    #[test]
    fn test_reads_a_single_record() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(from_start_of_chunk.to_vec()).unwrap();

        for record in chunk.into_iter().take(1) {
            assert!(record.is_ok(), record.unwrap())
        }
    }

    #[test]
    fn test_reads_a_ten_records() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(from_start_of_chunk.to_vec()).unwrap();

        for record in chunk.into_iter().take(10) {
            println!("{:?}", record);
        }
    }

}
