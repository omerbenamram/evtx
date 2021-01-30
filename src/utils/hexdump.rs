#![allow(dead_code)]
use crate::evtx_parser::ReadSeek;

use std::cmp;
use std::error::Error;
use std::fmt::Write;
use std::io::SeekFrom;

pub fn dump_stream<T: ReadSeek>(cursor: &mut T, lookbehind: i32) -> Result<String, Box<dyn Error>> {
    let mut s = String::new();

    cursor.seek(SeekFrom::Current(lookbehind.into()))?;

    let mut data = vec![0; 100_usize];
    let _ = cursor.read(&mut data)?;

    writeln!(
        s,
        "\n\n---------------------------------------------------------------------------"
    )?;
    writeln!(s, "Current Value {:02x}", data[0])?;
    writeln!(s, "              --")?;
    write!(s, "{}", hexdump(&data, 0, 'C')?)?;
    writeln!(
        s,
        "\n----------------------------------------------------------------------------"
    )?;

    Ok(s)
}

/// Dumps bytes at data to the screen as hex.
/// Display may be one of:
/// b  One-byte octal display.
///    Display the input offset in hexadecimal, followed by sixteen space-separated, three column, zero-filled, bytes of input data, in octal, per line.
///
/// c  One-byte character display. One-byte character display.
///    Display the input offset in hexadecimal, followed by sixteen space-separated, three column, space-filled, characters of input data per line.
///
/// C  Canonical hex display.
///    Display the input offset in hexadecimal, followed by sixteen space-separated, two column, hexadecimal bytes, followed by the same sixteen bytes in %_p format enclosed in ``|'' characters.
///
/// d  Two-byte decimal display.
/// o  Two-byte octal display.
/// x  Two-byte hexadecimal display.
///    Display the input offset in hexadecimal, followed by eight, space separated, four column, zero-filled, two-byte quantities of input data, in hexadecimal, per line.
pub fn hexdump(
    data: &[u8],
    offset: usize,
    display: char,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut s = String::new();
    let mut address = 0;

    let number_of_bytes = match display {
        'b' => 1,
        'c' => 1,
        'C' => 1,
        'd' => 2,
        'o' => 2,
        _ => 2,
    };

    while address <= data.len() {
        // Read next 16 bytes of until end of data
        let end = cmp::min(address + 16, data.len());

        write!(
            s,
            "{}",
            print_line(
                &data[address..end],
                address + offset,
                display,
                number_of_bytes,
            )?
        )?;
        address += 16;
    }

    Ok(s)
}

fn print_line(
    line: &[u8],
    address: usize,
    display: char,
    bytes: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut s = String::new();
    // print address (ex - 000000d0)
    write!(s, "\n{:08x}:", address)?;

    let words = if (line.len() % bytes) == 0 {
        line.len() / bytes
    } else {
        (line.len() / bytes) + 1
    };

    for b in 0..words {
        let word = match bytes {
            1 => u16::from(line[b]),
            _ => {
                if line.len() == bytes * b + 1 {
                    u16::from_be(u16::from(line[bytes * b]) << 8)
                } else {
                    u16::from_be((u16::from(line[bytes * b]) << 8) + u16::from(line[bytes * b + 1]))
                }
            }
        };
        match display {
            'b' => write!(s, " {:03o}", word)?,
            'c' => {
                if ((word as u8) as char).is_control() {
                    write!(s, " ")?
                } else {
                    write!(s, " {:03}", (word as u8) as char)?
                }
            }
            'C' => write!(s, " {:02x}", word)?,
            'x' => write!(s, " {:04x}", word)?,
            'o' => write!(s, " {:06o} ", word)?,
            'd' => write!(s, "  {:05} ", word)?,
            _ => write!(s, " {:04x}", word)?,
        }
    }

    // print ASCII repr
    if display != 'c' {
        if (line.len() % 16) > 0 {
            // align
            let words_left = (16 - line.len()) / bytes;
            let word_size = match display {
                'b' => 4,
                'c' => 4,
                'C' => 3,
                'x' => 5,
                'o' => 8,
                'd' => 8,
                _ => 5,
            };
            for _ in 0..word_size * words_left {
                write!(s, " ")?;
            }
        }

        write!(s, "  ")?;
        for c in line {
            // replace all control chars with dots
            if (*c as char).is_control() {
                write!(s, ".")?
            } else {
                write!(s, "{}", (*c as char))?
            }
        }
    }

    Ok(s)
}
