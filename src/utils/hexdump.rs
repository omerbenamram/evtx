use std::cmp;

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
pub fn print_hexdump(data: &[u8], offset: usize, display: char) {
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

        print_line(
            &data[address..end],
            address + offset,
            display,
            number_of_bytes,
        );
        address += 16;
    }
}

fn print_line(line: &[u8], address: usize, display: char, bytes: usize) {
    // print address (ex - 000000d0)
    print!("\n{:08x}:", address);

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
            'b' => print!(" {:03o}", word),
            'c' => {
                if ((word as u8) as char).is_control() {
                    print!(" ")
                } else {
                    print!(" {:03}", (word as u8) as char)
                }
            }
            'C' => print!(" {:02x}", word),
            'x' => print!(" {:04x}", word),
            'o' => print!(" {:06o} ", word),
            'd' => print!("  {:05} ", word),
            _ => print!(" {:04x}", word),
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
                print!(" ");
            }
        }

        print!("  ");
        for c in line {
            // replace all control chars with dots
            if (*c as char).is_control() {
                print!(".")
            } else {
                print!("{}", (*c as char))
            }
        }
    }
}
