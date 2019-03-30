use byteorder::{LittleEndian, ReadBytesExt};

use log::{debug, log, trace};
use std::io::{Seek, SeekFrom};

use crate::binxml::tokens::read_open_start_element;
use crate::binxml::value_variant::BinXmlValue;
use crate::string_cache::CachedString;
use crate::template_cache::CachedTemplate;
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
use core::borrow::BorrowMut;
use std::io::Cursor;
use std::mem;
use std::rc::Rc;
use std::sync::RwLock;

// Alias that will make it easier to change context type if needed.
pub type Context<'b> = Rc<Cache<'b>>;

#[derive(Clone, Debug)]
pub struct Cache<'c> {
    offset: u64,
    string_cache: Option<&'c StringCache>,
    template_cache: Option<&'c TemplateCache<'c>>,
}

impl<'c> Cache<'c> {
    pub fn cached_string_at_offset(&self, offset: Offset) -> Option<&'c CachedString> {
        match self.string_cache {
            Some(cache) => cache.get_string_and_hash(offset),
            None => None,
        }
    }

    pub fn cached_template_at_offset(&'c self, offset: Offset) -> Option<&CachedTemplate<'c>> {
        None
    }
}

pub struct IterTokens<'c> {
    cursor: Cursor<&'c [u8]>,
    ctx: Context<'c>,
    data_size: Option<u32>,
    data_read_so_far: u32,
    eof: bool,
}

pub struct BinXmlDeserializer<'c> {
    data: &'c [u8],
    ctx: Context<'c>,
}

impl<'c> BinXmlDeserializer<'c> {
    pub fn init(
        data: &'c [u8],
        start_offset: u64,
        string_cache: &'c StringCache,
        template_cache: &'c TemplateCache<'c>,
    ) -> Self {
        let ctx = Cache {
            offset: start_offset,
            string_cache: Some(string_cache),
            template_cache: Some(template_cache),
        };

        BinXmlDeserializer {
            data,
            ctx: Rc::new(ctx),
        }
    }

    pub fn from_ctx(data: &'c [u8], ctx: Context<'c>) -> Self {
        BinXmlDeserializer {
            data,
            ctx: Rc::clone(&ctx),
        }
    }

    pub fn init_without_cache(data: &'c [u8], start_offset: u64) -> Self {
        let ctx = Rc::new(Cache {
            offset: start_offset,
            string_cache: None,
            template_cache: None,
        });

        BinXmlDeserializer { data, ctx }
    }

    /// Reads `data_size` bytes of binary xml, or until EOF marker.
    pub fn iter_tokens(self, data_size: Option<u32>) -> IterTokens<'c> {
        let mut cursor = Cursor::new(self.data);
        cursor.seek(SeekFrom::Start(self.ctx.offset)).unwrap();

        IterTokens {
            cursor,
            ctx: Rc::clone(&self.ctx),
            data_size,
            data_read_so_far: 0,
            eof: false,
        }
    }
}

impl<'c> IterTokens<'c> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&self, cursor: &mut Cursor<&'c [u8]>) -> Result<BinXMLRawToken, Error> {
        let token = cursor
            .read_u8()
            .map_err(|e| Error::unexpected_eof(e, cursor.stream_position().unwrap()))?;

        Ok(BinXMLRawToken::from_u8(token).ok_or_else(|| {
            Error::not_a_valid_binxml_token(token, cursor.stream_position().unwrap())
        })?)
    }

    fn visit_token(
        &self,
        cursor: &mut Cursor<&'c [u8]>,
        ctx: Context<'c>,
        raw_token: BinXMLRawToken,
    ) -> Result<BinXMLDeserializedTokens<'c>, Error> {
        match raw_token {
            BinXMLRawToken::EndOfStream => Ok(BinXMLDeserializedTokens::EndOfStream),
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    read_open_start_element(cursor, ctx, token_information.has_attributes)?,
                ))
            }
            BinXMLRawToken::CloseStartElement => Ok(BinXMLDeserializedTokens::CloseStartElement),
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => Ok(BinXMLDeserializedTokens::CloseElement),
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(
                BinXmlValue::from_binxml_stream(cursor, ctx)?,
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
                read_template(cursor, ctx)?,
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

impl<'c> IterTokens<'c> {
    fn inner_next(&mut self) -> Option<Result<BinXMLDeserializedTokens<'c>, Error>> {
        let mut cursor = self.cursor.clone();
        let offset_from_chunk_start = cursor.stream_position().expect("Tell failed");

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
                let deserialized_token_result =
                    self.visit_token(&mut cursor, Rc::clone(&self.ctx), t);

                trace!(
                    "{:?} position at stream {}",
                    deserialized_token_result,
                    cursor.stream_position().unwrap()
                );

                debug_assert!(
                    cursor.stream_position().unwrap() >= offset_from_chunk_start,
                    "Invalid state, cursor position at entering loop {}, now at {}",
                    offset_from_chunk_start,
                    cursor.stream_position().unwrap()
                );

                Some(deserialized_token_result)
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
                Some(Err(e))
            }
        };
        let total_read = cursor.stream_position().unwrap() - offset_from_chunk_start;
        self.data_read_so_far += total_read as u32;

        mem::swap(&mut self.cursor, &mut cursor);
        yield_value
    }
}

impl<'c> Iterator for IterTokens<'c> {
    type Item = Result<BinXMLDeserializedTokens<'c>, Error>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        self.inner_next()
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

        let deser = BinXmlDeserializer::init_without_cache(
            &chunk.data,
            (3872_usize + EVTX_RECORD_HEADER_SIZE) as u64,
        );

        for token in deser.iter_tokens(Some(record_header.data_size - 4 - 4 - 4 - 8 - 8)) {
            if let Err(e) = token {
                let mut cursor = Cursor::new(chunk.data.as_slice());
                println!("{}", e);
                cursor.seek(SeekFrom::Start(e.offset().unwrap())).unwrap();
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
