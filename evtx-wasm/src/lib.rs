use arrow2::{
    array::{Array, MutableArray, MutablePrimitiveArray, MutableUtf8Array},
    chunk::Chunk,
    datatypes::{DataType, Field, Schema, TimeUnit},
    io::ipc::write::{StreamWriter, WriteOptions},
};
use evtx::{EvtxChunk, EvtxChunkHeader, EvtxParser, Offset, ParserSettings};
use serde_json::Value;
use std::io::Cursor;
use wasm_bindgen::prelude::*;

// Set panic hook for better error messages in the browser
#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
}

fn json_settings() -> ParserSettings {
    ParserSettings::default()
        .separate_json_attributes(true)
        .indent(false)
}

/// Integer from a JSON number or numeric string.
fn json_i32(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

/// Microseconds since the Unix epoch for `YYYY-MM-DDTHH:MM:SS[.f]Z` with 1-9 fraction
/// digits: the core renders 6, forwarded events carry their original text (often 9).
// ponytail: hand parse because jiff's string parsers add ~125KB to the wasm; other
// shapes become null. Swap in `time.parse::<Timestamp>()` if that stops being worth it.
fn system_time_micros(time: &str) -> Option<i64> {
    let (datetime, fraction) = time.strip_suffix('Z')?.split_at_checked(19)?;
    let b = datetime.as_bytes();
    if [b[4], b[7], b[10], b[13], b[16]] != *b"--T::" {
        return None;
    }
    let digits = |s: &[u8]| {
        s.iter().try_fold(0i32, |n, &d| {
            d.is_ascii_digit().then(|| n * 10 + i32::from(d - b'0'))
        })
    };
    let num = |at: usize, len: usize| digits(&b[at..at + len]);
    let nanos = match fraction.as_bytes() {
        [] => 0,
        [b'.', f @ ..] if (1..=9).contains(&f.len()) => digits(f)? * 10i32.pow(9 - f.len() as u32),
        _ => return None,
    };
    let datetime = jiff::civil::DateTime::new(
        num(0, 4)? as i16,
        num(5, 2)? as i8,
        num(8, 2)? as i8,
        num(11, 2)? as i8,
        num(14, 2)? as i8,
        num(17, 2)? as i8,
        nanos,
    )
    .ok()?;
    Some(Offset::UTC.to_timestamp(datetime).ok()?.as_microsecond())
}

#[wasm_bindgen]
pub struct EvtxWasmParser {
    data: Vec<u8>,
    // Logical chunk indices skip empty physical slots, matching EvtxParser::chunks.
    chunk_offsets: Vec<usize>,
}

const FILE_HEADER_SIZE: usize = 4096;
const CHUNK_SIZE: usize = 65536;

impl EvtxWasmParser {
    fn from_data(data: Vec<u8>) -> Result<Self, String> {
        EvtxParser::from_read_seek(Cursor::new(&data))
            .map_err(|e| format!("Failed to create parser: {e}"))?;
        let chunks = data
            .get(FILE_HEADER_SIZE..)
            .ok_or_else(|| "Incomplete EVTX file header".to_owned())?;
        let chunk_offsets = chunks
            .chunks(CHUNK_SIZE)
            .enumerate()
            .filter(|(_, bytes)| bytes.iter().any(|byte| *byte != 0))
            .map(|(index, _)| FILE_HEADER_SIZE + index * CHUNK_SIZE)
            .collect();
        Ok(Self {
            data,
            chunk_offsets,
        })
    }

    fn chunk_data(&self, index: usize) -> Result<(&[u8], EvtxChunkHeader), String> {
        let offset = *self
            .chunk_offsets
            .get(index)
            .ok_or_else(|| format!("Chunk index {index} out of range"))?;
        let bytes = &self.data[offset..self.data.len().min(offset + CHUNK_SIZE)];
        if bytes.len() != CHUNK_SIZE {
            return Err(format!("incomplete chunk ({} bytes)", bytes.len()));
        }
        let header = EvtxChunkHeader::from_bytes(bytes).map_err(|e| e.to_string())?;
        Ok((bytes, header))
    }

    fn arrow_chunk(&self, chunk_index: usize) -> Result<ArrowChunkIPC, String> {
        let (bytes, header) = self.chunk_data(chunk_index)?;
        let mut chunk = EvtxChunk::new(bytes, &header, std::sync::Arc::new(json_settings()))
            .map_err(|e| e.to_string())?;
        let mut event_id = MutablePrimitiveArray::<i32>::new();
        let mut level = MutablePrimitiveArray::<i32>::new();
        let mut provider = MutableUtf8Array::<i32>::new();
        let mut channel = MutableUtf8Array::<i32>::new();
        let mut time_created = MutablePrimitiveArray::<i64>::new()
            .to(DataType::Timestamp(TimeUnit::Microsecond, None));
        let mut computer = MutableUtf8Array::<i32>::new();
        let mut user_id = MutableUtf8Array::<i32>::new();
        let mut task = MutablePrimitiveArray::<i32>::new();
        let mut opcode = MutablePrimitiveArray::<i32>::new();
        let mut keywords = MutableUtf8Array::<i32>::new();
        let mut raw = MutableUtf8Array::<i32>::new();
        let mut raw_json = Vec::new();
        let mut errors = Vec::new();

        for record in chunk.iter() {
            let rec = match record.and_then(|record| record.into_json_value()) {
                Ok(record) => record.data,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            let sys = &rec["Event"]["System"];
            event_id.push(json_i32(&sys["EventID"]));
            level.push(json_i32(&sys["Level"]));
            provider.push(Some(
                sys["Provider_attributes"]["Name"].as_str().unwrap_or(""),
            ));
            channel.push(Some(sys["Channel"].as_str().unwrap_or("")));
            time_created.push(
                sys["TimeCreated_attributes"]["SystemTime"]
                    .as_str()
                    .and_then(system_time_micros),
            );
            computer.push(sys["Computer"].as_str());
            user_id.push(sys["Security_attributes"]["UserID"].as_str());
            task.push(json_i32(&sys["Task"]));
            opcode.push(json_i32(&sys["Opcode"]));
            keywords.push(sys["Keywords"].as_str());
            // Re-serialising the parsed Value keeps one copy of a duplicated EventData key, so
            // DuckDB's json_extract (first match) and JSON.parse (last match) agree.
            raw_json.clear();
            serde_json::to_writer(&mut raw_json, &rec)
                .map_err(|e| format!("Record JSON error: {e}"))?;
            raw.push(Some(
                std::str::from_utf8(&raw_json).map_err(|e| format!("Record JSON error: {e}"))?,
            ));
        }

        // Positional: the viewer inserts this batch into DuckDB `logs` by column order.
        let columns: [(&str, bool, Box<dyn Array>); 11] = [
            ("EventID", true, event_id.as_box()),
            ("Level", true, level.as_box()),
            ("Provider", false, provider.as_box()),
            ("Channel", false, channel.as_box()),
            ("TimeCreated", true, time_created.as_box()),
            ("Computer", true, computer.as_box()),
            ("UserID", true, user_id.as_box()),
            ("Task", true, task.as_box()),
            ("Opcode", true, opcode.as_box()),
            ("Keywords", true, keywords.as_box()),
            ("Raw", false, raw.as_box()),
        ];
        let schema = Schema::from(
            columns
                .iter()
                .map(|(name, nullable, array)| {
                    Field::new(*name, array.data_type().clone(), *nullable)
                })
                .collect::<Vec<_>>(),
        );
        let batch = Chunk::new(columns.into_iter().map(|(_, _, array)| array).collect());

        let mut buf = Vec::new();
        let mut writer = StreamWriter::new(&mut buf, WriteOptions { compression: None });
        writer
            .start(&schema, None)
            .and_then(|()| writer.write(&batch, None))
            .and_then(|()| writer.finish())
            .map_err(|e| format!("Arrow IPC write failed: {e}"))?;

        Ok(ArrowChunkIPC {
            bytes: buf,
            rows: batch.len(),
            errors,
        })
    }
}

#[wasm_bindgen]
impl EvtxWasmParser {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<EvtxWasmParser, JsError> {
        Self::from_data(data).map_err(|e| JsError::new(&e))
    }

    /// Nonempty chunk count; empty slots are skipped, so a malformed nonempty chunk keeps
    /// its logical index.
    #[wasm_bindgen(getter)]
    pub fn total_chunks(&self) -> usize {
        self.chunk_offsets.len()
    }

    pub fn chunk_arrow_ipc(&self, chunk_index: usize) -> Result<ArrowChunkIPC, JsError> {
        self.arrow_chunk(chunk_index).map_err(|e| JsError::new(&e))
    }
}

/// One chunk as a single-batch Arrow IPC stream, plus per-record parse errors.
#[wasm_bindgen]
pub struct ArrowChunkIPC {
    bytes: Vec<u8>,
    rows: usize,
    errors: Vec<String>,
}

#[wasm_bindgen]
impl ArrowChunkIPC {
    #[wasm_bindgen(getter)]
    pub fn ipc(&self) -> js_sys::Uint8Array {
        js_sys::Uint8Array::from(&self.bytes[..])
    }

    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[wasm_bindgen(getter)]
    pub fn errors(&self) -> Vec<String> {
        self.errors.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow2::array::{PrimitiveArray, Utf8Array};
    use arrow2::io::ipc::read::{read_stream_metadata, StreamReader, StreamState};
    use evtx::Timestamp;

    /// Decodes the Raw rows, asserting each typed column agrees with its row's JSON.
    fn arrow_records(ipc: ArrowChunkIPC) -> Vec<Value> {
        let mut cursor = Cursor::new(ipc.bytes);
        let metadata = read_stream_metadata(&mut cursor).unwrap();
        let names: Vec<_> = metadata
            .schema
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            names,
            "EventID Level Provider Channel TimeCreated Computer UserID Task Opcode Keywords Raw"
                .split(' ')
                .collect::<Vec<_>>()
        );
        let mut records = Vec::new();
        for batch in StreamReader::new(cursor, metadata, None) {
            let StreamState::Some(batch) = batch.unwrap() else {
                continue;
            };
            let col = |i: usize| batch.arrays()[i].as_any();
            let text = |i| col(i).downcast_ref::<Utf8Array<i32>>().unwrap();
            let int = |i| col(i).downcast_ref::<PrimitiveArray<i32>>().unwrap();
            let time = col(4).downcast_ref::<PrimitiveArray<i64>>().unwrap();
            assert_eq!(
                time.data_type(),
                &DataType::Timestamp(TimeUnit::Microsecond, None)
            );
            for (row, json) in text(10).values_iter().enumerate() {
                let rec: Value = serde_json::from_str(json).unwrap();
                let sys = &rec["Event"]["System"];
                let system_time = sys["TimeCreated_attributes"]["SystemTime"].as_str();
                assert_eq!(
                    time.get(row),
                    system_time.map(|t| t.parse::<Timestamp>().unwrap().as_microsecond())
                );
                assert_eq!(text(5).get(row), sys["Computer"].as_str());
                assert_eq!(
                    text(6).get(row),
                    sys["Security_attributes"]["UserID"].as_str()
                );
                assert_eq!(int(7).get(row), json_i32(&sys["Task"]));
                assert_eq!(int(8).get(row), json_i32(&sys["Opcode"]));
                assert_eq!(text(9).get(row), sys["Keywords"].as_str());
                records.push(rec);
            }
        }
        assert_eq!(records.len(), ipc.rows);
        records
    }

    #[test]
    fn arrow_chunks_match_json_and_preserve_indices_around_corruption() {
        for name in [
            "Security_short_selected.evtx",
            "system.evtx",
            "sample_with_a_bad_chunk_magic.evtx",
            "Archive-ForwardedEvents-test.evtx",
        ] {
            let data =
                std::fs::read(format!("{}/../samples/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
            let mut native = EvtxParser::from_read_seek(Cursor::new(&data))
                .unwrap()
                .with_configuration(json_settings());
            let expected: Vec<Value> = native
                .records_json_value()
                .filter_map(Result::ok)
                .map(|r| r.data)
                .collect();
            let reader = EvtxWasmParser::from_data(data).unwrap();
            let mut actual = Vec::new();
            for index in 0..reader.chunk_offsets.len() {
                if let Ok(ipc) = reader.arrow_chunk(index) {
                    actual.extend(arrow_records(ipc));
                }
            }
            assert!(!expected.is_empty(), "{name}");
            assert_eq!(actual, expected, "{name}");
        }

        let sample = include_bytes!("../../samples/Security_short_selected.evtx");
        let chunk = &sample[FILE_HEADER_SIZE..FILE_HEADER_SIZE + CHUNK_SIZE];
        let mut damaged = chunk.to_vec();
        damaged[0] = 0;
        let mut data = sample[..FILE_HEADER_SIZE].to_vec();
        data.extend_from_slice(chunk);
        data.extend_from_slice(&vec![0; CHUNK_SIZE]);
        data.extend_from_slice(&damaged);
        data.extend_from_slice(chunk);
        let reader = EvtxWasmParser::from_data(data).unwrap();
        assert_eq!(reader.chunk_offsets.len(), 3);
        assert!(reader.arrow_chunk(1).is_err());
        assert!(reader.arrow_chunk(3).is_err());
        assert_eq!(
            arrow_records(reader.arrow_chunk(0).unwrap()),
            arrow_records(reader.arrow_chunk(2).unwrap())
        );

        let mut truncated = sample.to_vec();
        truncated.extend_from_slice(&chunk[..100]);
        let reader = EvtxWasmParser::from_data(truncated).unwrap();
        assert_eq!(reader.chunk_offsets.len(), 2);
        assert!(reader
            .arrow_chunk(1)
            .err()
            .unwrap()
            .contains("incomplete chunk"));
        assert!(EvtxWasmParser::from_data(vec![0; FILE_HEADER_SIZE]).is_err());
        for time in ["2016-06-29T15:24:34Z", "2016-06-29T15:24:34.3Z"] {
            let expected = time.parse::<Timestamp>().unwrap().as_microsecond();
            assert_eq!(system_time_micros(time), Some(expected));
        }
        for time in [
            "2016-06-29 15:24:34Z",
            "2016-06-29T15:24:34.Z",
            "2016-13-29T15:24:34Z",
        ] {
            assert_eq!(system_time_micros(time), None);
        }
    }
}
