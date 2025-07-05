use evtx::{EvtxParser, ParserSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Cursor;
use wasm_bindgen::prelude::*;

// Set panic hook for better error messages in the browser
#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
}

#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    pub records: Vec<Value>,
    pub total_records: usize,
    pub chunk_count: usize,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChunkInfo {
    pub chunk_number: u64,
    /// Number of records in the chunk. Serialised as string to avoid
    /// potential 64-bit overflow issues on the JS side.
    pub record_count: String,
    /// These IDs may exceed JavaScript's safe integer range, so we serialise
    /// them as strings.
    pub first_record_id: String,
    pub last_record_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct FileInfo {
    pub chunks: Vec<ChunkInfo>,
    pub total_chunks: usize,
    pub first_chunk: u64,
    pub last_chunk: u64,
    /// Use a string here to avoid `serde_wasm_bindgen` errors when the value
    /// exceeds JavaScript's safe integer range (2^53-1).
    pub next_record_id: String,
    pub is_dirty: bool,
    pub is_full: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub struct BucketCounts {
    pub level: HashMap<String, u64>,
    pub provider: HashMap<String, u64>,
    pub channel: HashMap<String, u64>,
    pub event_id: HashMap<String, u64>,
}

/// Compute distinct values + counts for common facets across **all** records.
/// Returned object shape (JSON):
/// {
///   level:    { "0": 123, "4": 456, ... },
///   provider: { "Microsoft-Windows-Security-Auditing": 789, ... },
///   channel:  { "Security": 789, ... },
///   event_id: { "4688": 321, ... }
/// }
#[wasm_bindgen]
pub fn compute_buckets(data: &[u8]) -> Result<JsValue, JsError> {
    let cursor = Cursor::new(data);
    let settings = ParserSettings::default()
        .separate_json_attributes(true)
        .indent(false);

    let mut parser = EvtxParser::from_read_seek(cursor)
        .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?
        .with_configuration(settings);

    let mut buckets: BucketCounts = BucketCounts::default();
    let mut record_counter = 0u64;

    for record in parser.records_json_value() {
        record_counter += 1;
        let rec = match record {
            Ok(r) => r.data,
            Err(_) => continue,
        };

        // Navigate to Event.System if present
        let sys = match rec.get("Event").and_then(|v| v.get("System")) {
            Some(s) => s,
            None => continue,
        };

        // Level
        if let Some(level_val) = sys.get("Level") {
            let key = level_val.to_string();
            *buckets.level.entry(key).or_insert(0) += 1;
        }

        // Provider.Name – might be nested under Provider or Provider_attributes
        if let Some(provider_name) = sys
            .get("Provider")
            .and_then(|p| p.get("Name"))
            .or_else(|| sys.get("Provider_attributes").and_then(|p| p.get("Name")))
        {
            let key_owned = provider_name
                .as_str()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| provider_name.to_string());
            *buckets.provider.entry(key_owned).or_insert(0) += 1;
        }

        // Channel
        if let Some(ch) = sys.get("Channel") {
            let key_owned = ch
                .as_str()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| ch.to_string());
            *buckets.channel.entry(key_owned).or_insert(0) += 1;
        }

        // EventID (may be object when attributes enabled)
        if let Some(eid) = sys.get("EventID") {
            let id_str = if eid.is_object() {
                eid.get("#text")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&eid.to_string())
                    .to_owned()
            } else if eid.is_string() {
                eid.as_str().unwrap().to_owned()
            } else {
                eid.to_string()
            };
            *buckets.event_id.entry(id_str).or_insert(0) += 1;
        }
    }

    // DEBUG: emit some stats to the browser console so we can confirm logic works.
    #[cfg(target_arch = "wasm32")]
    {
        use web_sys::console;
        console::log_1(&JsValue::from_str(&format!(
            "compute_buckets finished – processed {} records, level keys={} provider keys={} channel keys={} event_id keys={}",
            record_counter,
            buckets.level.len(),
            buckets.provider.len(),
            buckets.channel.len(),
            buckets.event_id.len()
        )));
    }

    let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);

    buckets
        .serialize(&serializer)
        .map_err(|e| JsError::new(&format!("Failed to serialise buckets: {}", e)))
}

#[wasm_bindgen]
pub struct EvtxWasmParser {
    data: Vec<u8>,
}

#[wasm_bindgen]
impl EvtxWasmParser {
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8]) -> Result<EvtxWasmParser, JsError> {
        Ok(EvtxWasmParser {
            data: data.to_vec(),
        })
    }

    /// Get file header information
    #[wasm_bindgen]
    pub fn get_file_info(&self) -> Result<JsValue, JsError> {
        // Parse header from raw data
        let mut header_cursor = Cursor::new(&self.data[..4096.min(self.data.len())]);
        let header = evtx::EvtxFileHeader::from_stream(&mut header_cursor)
            .map_err(|e| JsError::new(&format!("Failed to parse header: {}", e)))?;

        let cursor = Cursor::new(&self.data);
        let mut parser = EvtxParser::from_read_seek(cursor)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?;

        let mut chunks = Vec::new();

        // Collect chunk information
        for (chunk_number, chunk) in parser.chunks().enumerate() {
            match chunk {
                Ok(mut chunk_data) => {
                    let chunk_settings = ParserSettings::default();
                    match chunk_data.parse(std::sync::Arc::new(chunk_settings)) {
                        Ok(chunk) => {
                            // In rare corrupted files `last_event_record_number` can be
                            // lower than `first_event_record_number`, which would wrap the
                            // subtraction and produce a huge `u64`.  Guard against that and
                            // clamp to 0.
                            let safe_record_count = if chunk.header.last_event_record_number
                                >= chunk.header.first_event_record_number
                            {
                                chunk.header.last_event_record_number
                                    - chunk.header.first_event_record_number
                                    + 1
                            } else {
                                0
                            };

                            chunks.push(ChunkInfo {
                                chunk_number: chunk_number as u64,
                                record_count: safe_record_count.to_string(),
                                first_record_id: chunk.header.first_event_record_id.to_string(),
                                last_record_id: chunk.header.last_event_record_id.to_string(),
                            });
                        }
                        Err(_) => continue,
                    }
                }
                Err(_) => continue,
            }
        }

        let file_info = FileInfo {
            total_chunks: chunks.len(),
            chunks,
            first_chunk: header.first_chunk_number,
            last_chunk: header.last_chunk_number,
            next_record_id: header.next_record_id.to_string(),
            is_dirty: header.flags.contains(evtx::HeaderFlags::DIRTY),
            is_full: header.flags.contains(evtx::HeaderFlags::FULL),
        };

        serde_wasm_bindgen::to_value(&file_info)
            .map_err(|e| JsError::new(&format!("Failed to serialize file info: {}", e)))
    }

    /// Parse all records in the file
    #[wasm_bindgen]
    pub fn parse_all(&self) -> Result<JsValue, JsError> {
        self.parse_with_limit(None)
    }

    /// Parse records with an optional limit
    #[wasm_bindgen]
    pub fn parse_with_limit(&self, limit: Option<usize>) -> Result<JsValue, JsError> {
        let cursor = Cursor::new(&self.data);
        let settings = ParserSettings::default()
            .separate_json_attributes(true) // This might help with the structure
            .indent(false);

        let mut parser = EvtxParser::from_read_seek(cursor)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?
            .with_configuration(settings);

        let mut records = Vec::new();
        let mut errors = Vec::new();

        // Use records_json_value iterator for JSON values
        for (idx, record) in parser.records_json_value().enumerate() {
            if let Some(limit) = limit {
                if records.len() >= limit {
                    break;
                }
            }

            match record {
                Ok(record_data) => {
                    // The record_data.data already contains the full event structure
                    records.push(record_data.data);
                }
                Err(e) => errors.push(format!("Record {} error: {}", idx, e)),
            }
        }

        // Count chunks separately
        let cursor2 = Cursor::new(&self.data);
        let mut parser2 = EvtxParser::from_read_seek(cursor2)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?;
        let chunk_count = parser2.chunks().count();

        let result = ParseResult {
            total_records: records.len(),
            records,
            chunk_count,
            errors,
        };

        serde_wasm_bindgen::to_value(&result)
            .map_err(|e| JsError::new(&format!("Failed to serialize result: {}", e)))
    }

    /// Parse a specific chunk
    #[wasm_bindgen]
    pub fn parse_chunk(&self, chunk_index: usize) -> Result<JsValue, JsError> {
        let cursor = Cursor::new(&self.data);
        let mut parser = EvtxParser::from_read_seek(cursor)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?;

        let mut records = Vec::new();
        let mut errors = Vec::new();

        for (idx, chunk) in parser.chunks().enumerate() {
            if idx != chunk_index {
                continue;
            }

            match chunk {
                Ok(mut chunk_data) => {
                    let chunk_settings = ParserSettings::default()
                        .separate_json_attributes(true)
                        .indent(false);
                    match chunk_data.parse(std::sync::Arc::new(chunk_settings)) {
                        Ok(mut chunk) => {
                            for record in chunk.iter() {
                                match record {
                                    Ok(record_data) => {
                                        match record_data.into_json_value() {
                                            Ok(json_record) => {
                                                // Use the full data structure
                                                records.push(json_record.data);
                                            }
                                            Err(e) => {
                                                errors.push(format!("Record JSON error: {}", e))
                                            }
                                        }
                                    }
                                    Err(e) => errors.push(format!("Record error: {}", e)),
                                }
                            }
                        }
                        Err(e) => errors.push(format!("Chunk parse error: {}", e)),
                    }
                }
                Err(e) => errors.push(format!("Chunk error: {}", e)),
            }

            break;
        }

        let result = ParseResult {
            total_records: records.len(),
            records,
            chunk_count: 1,
            errors,
        };

        serde_wasm_bindgen::to_value(&result)
            .map_err(|e| JsError::new(&format!("Failed to serialize result: {}", e)))
    }

    /// Get a specific record by its ID
    #[wasm_bindgen]
    pub fn get_record_by_id(&self, record_id: u64) -> Result<JsValue, JsError> {
        let cursor = Cursor::new(&self.data);
        let mut parser = EvtxParser::from_read_seek(cursor)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?;

        for record in parser.records_json_value() {
            match record {
                Ok(record_data) => {
                    if record_data.event_record_id == record_id {
                        return serde_wasm_bindgen::to_value(&record_data.data).map_err(|e| {
                            JsError::new(&format!("Failed to serialize record: {}", e))
                        });
                    }
                }
                Err(_) => continue,
            }
        }

        Err(JsError::new(&format!(
            "Record with ID {} not found",
            record_id
        )))
    }

    /// Parse records from a specific chunk with offset/limit.
    /// `chunk_index` – zero-based index of the chunk.
    /// `start` – zero-based record offset within the chunk to begin at.
    /// `limit` – maximum number of records to return (0 = no limit).
    #[wasm_bindgen]
    pub fn parse_chunk_records(
        &self,
        chunk_index: usize,
        start: usize,
        limit: Option<usize>,
    ) -> Result<JsValue, JsError> {
        let cursor = Cursor::new(&self.data);
        let mut parser = EvtxParser::from_read_seek(cursor)
            .map_err(|e| JsError::new(&format!("Failed to create parser: {}", e)))?;

        let mut records = Vec::new();
        let mut errors = Vec::new();

        for (idx, chunk) in parser.chunks().enumerate() {
            if idx != chunk_index {
                continue;
            }

            match chunk {
                Ok(mut chunk_data) => {
                    let chunk_settings = ParserSettings::default()
                        .separate_json_attributes(true)
                        .indent(false);
                    match chunk_data.parse(std::sync::Arc::new(chunk_settings)) {
                        Ok(mut chunk) => {
                            for (rec_idx, record) in chunk.iter().enumerate() {
                                if rec_idx < start {
                                    continue;
                                }

                                if let Some(max) = limit {
                                    if records.len() >= max {
                                        break;
                                    }
                                }

                                match record {
                                    Ok(record_data) => match record_data.into_json_value() {
                                        Ok(json_record) => records.push(json_record.data),
                                        Err(e) => errors.push(format!("Record JSON error: {}", e)),
                                    },
                                    Err(e) => errors.push(format!("Record error: {}", e)),
                                }
                            }
                        }
                        Err(e) => errors.push(format!("Chunk parse error: {}", e)),
                    }
                }
                Err(e) => errors.push(format!("Chunk error: {}", e)),
            }

            break; // Only process the requested chunk
        }

        let result = ParseResult {
            total_records: records.len(),
            records,
            chunk_count: 1,
            errors,
        };

        serde_wasm_bindgen::to_value(&result)
            .map_err(|e| JsError::new(&format!("Failed to serialize result: {}", e)))
    }
}

/// Utility function to get basic file info without creating a parser instance
#[wasm_bindgen]
pub fn quick_file_info(data: &[u8]) -> Result<JsValue, JsError> {
    let parser = EvtxWasmParser::new(data)?;
    parser.get_file_info()
}
