use anyhow::{Context, Result};
use bitflags::Flags;
use bitflags::parser::WriteHex;
use clap::{Arg, ArgMatches, Command};
use evtx::{ChunkFlags, EvtxChunkData, EvtxFileHeader, HeaderFlags};
use indoc::indoc;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// Chunks start after the header block, at fixed offsets: where evtx reads them.
const HEADER_BLOCK_SIZE: u64 = 4096;
const CHUNK_SIZE: u64 = 65536;

pub fn command() -> Command {
    Command::new("info")
        .about("Show an EVTX file's header and each chunk's record IDs, flags and CRC32s")
        .long_about(indoc!(
            r#"
            Show what an EVTX file's header and chunk headers say: the format version, the
            dirty and full flags, the chunk numbers and the next record ID, whether the
            header's CRC32 matches, and for each chunk its record numbers and IDs, its flags,
            and whether its header's and records' CRC32s match.

            For triaging dirty, wrapped or damaged logs. Records aren't parsed.
        "#
        ))
        .arg(
            Arg::new("input")
                .long("input")
                .short('i')
                .required(true)
                .value_name("EVTX")
                .help("Input EVTX file path."),
        )
}

pub fn run(matches: &ArgMatches) -> Result<()> {
    let input = PathBuf::from(matches.get_one::<String>("input").expect("required"));
    let mut file =
        File::open(&input).with_context(|| format!("Failed to open {}", input.display()))?;
    let mut out = BufWriter::new(io::stdout().lock());
    write_info(&mut file, &mut out)?;
    out.flush()?;
    Ok(())
}

/// A row of the chunk table.
enum Row {
    /// A chunk's header fields, one per column.
    Chunk([String; 7]),
    /// A chunk's number and offset, and why it has no header fields to show.
    Note(String, String, String),
}

const COLUMNS: [&str; 7] = [
    "Chunk",
    "Offset",
    "Record numbers",
    "Record IDs",
    "Flags",
    "Header CRC32",
    "Records CRC32",
];

/// What an EVTX file's header and chunk headers say, as `evtx_dump info` prints it.
fn write_info(file: &mut (impl Read + Seek), out: &mut impl Write) -> Result<()> {
    let len = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(0))?;
    let header = EvtxFileHeader::from_stream(file).context("Failed to read the file header")?;
    let chunks = len.saturating_sub(HEADER_BLOCK_SIZE) / CHUNK_SIZE;
    let trailing = len.saturating_sub(HEADER_BLOCK_SIZE) % CHUNK_SIZE;

    let header_crc = if header.flags.contains(HeaderFlags::NO_CRC32) {
        "none (NO_CRC32)".to_string()
    } else if header.validate_checksum() {
        format!("ok ({:#010x})", header.checksum)
    } else {
        format!(
            "BAD (stored {:#010x}, computed {:#010x})",
            header.checksum, header.computed_checksum
        )
    };
    let mut fields = vec![
        (
            "Version",
            format!("{}.{}", header.major_version, header.minor_version),
        ),
        ("Flags", flag_names(&header.flags)),
        ("CRC32", header_crc),
        ("First chunk", header.first_chunk_number.to_string()),
        ("Last chunk", header.last_chunk_number.to_string()),
        ("Next record ID", header.next_record_id.to_string()),
        (
            "Chunk count",
            format!("{} (the file has room for {chunks})", header.chunk_count),
        ),
        (
            "Header size",
            format!(
                "{} (block {})",
                header.header_size, header.header_block_size
            ),
        ),
    ];
    if trailing > 0 {
        fields.push((
            "Trailing bytes",
            format!("{trailing}, after the last chunk"),
        ));
    }
    writeln!(out, "File header")?;
    for (name, value) in fields {
        writeln!(out, "  {name:<16} {value}")?;
    }

    let mut rows = vec![Row::Chunk(COLUMNS.map(String::from))];
    let (mut used, mut empty, mut failing, mut records) = (0_u64, 0_u64, 0_u64, 0_u64);
    for number in 0..chunks {
        let offset = HEADER_BLOCK_SIZE + number * CHUNK_SIZE;
        let mut data = vec![0; CHUNK_SIZE as usize];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut data)
            .with_context(|| format!("Failed to read chunk {number}"))?;
        let (number, offset) = (number.to_string(), format!("{offset:#010x}"));
        if data.iter().all(|&byte| byte == 0) {
            empty += 1;
            rows.push(Row::Note(number, offset, "empty".into()));
            continue;
        }
        let chunk = match EvtxChunkData::new(data, false) {
            Ok(chunk) => chunk,
            Err(error) => {
                let error = anyhow::Error::from(error);
                rows.push(Row::Note(number, offset, format!("unreadable: {error:#}")));
                continue;
            }
        };
        used += 1;
        let h = &chunk.header;
        let crc32 = |valid: bool| match (h.flags.contains(ChunkFlags::NO_CRC32), valid) {
            (true, _) => "none",
            (false, true) => "ok",
            (false, false) => "BAD",
        };
        let header_crc = crc32(chunk.validate_header_checksum());
        let records_crc = crc32(chunk.validate_data_checksum());
        if header_crc == "BAD" || records_crc == "BAD" {
            failing += 1;
        }
        let count = h
            .last_event_record_number
            .checked_sub(h.first_event_record_number);
        records += count.map_or(0, |count| count.saturating_add(1));
        rows.push(Row::Chunk([
            number,
            offset,
            format!(
                "{}-{}",
                h.first_event_record_number, h.last_event_record_number
            ),
            format!("{}-{}", h.first_event_record_id, h.last_event_record_id),
            flag_names(&h.flags),
            header_crc.into(),
            records_crc.into(),
        ]));
    }

    let mut widths = [0; COLUMNS.len()];
    for row in &rows {
        let cells = match row {
            Row::Chunk(cells) => &cells[..],
            Row::Note(number, offset, _) => &[number.clone(), offset.clone()][..],
        };
        for (width, cell) in widths.iter_mut().zip(cells) {
            *width = (*width).max(cell.len());
        }
    }
    writeln!(out)?;
    for row in &rows {
        let mut line = String::new();
        match row {
            Row::Chunk(cells) => {
                for (cell, width) in cells.iter().zip(widths) {
                    line.push_str(&format!("{cell:<width$}  "));
                }
            }
            Row::Note(number, offset, note) => {
                let [number_width, offset_width, ..] = widths;
                line.push_str(&format!(
                    "{number:<number_width$}  {offset:<offset_width$}  {note}"
                ));
            }
        }
        writeln!(out, "{}", line.trim_end())?;
    }

    writeln!(out)?;
    let unreadable = chunks - used - empty;
    writeln!(
        out,
        "{chunks} {}: {used} with records, {empty} empty, {unreadable} unreadable. \
         {records} records by the chunk headers. {failing} {} a CRC32 check.",
        plural(chunks, "chunk", "chunks"),
        plural(failing, "chunk fails", "chunks fail"),
    )?;
    Ok(())
}

fn plural(count: u64, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// The names of the flags set, then any bits evtx doesn't define, in hex.
fn flag_names<F: Flags>(flags: &F) -> String
where
    F::Bits: WriteHex,
{
    let mut names = String::new();
    bitflags::parser::to_writer(flags, &mut names).expect("writing to a String");
    if names.is_empty() {
        names.push_str("none");
    }
    names
}
