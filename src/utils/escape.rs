use memchr::memchr2;

/// Escape UTF-8 bytes for JSON and append into `out`. Returns true if any escape was performed.
#[inline]
pub fn escape_json_ascii(input: &[u8], out: &mut String) -> bool {
	// Quick scan for any special bytes. If none, just push the bytes directly.
	let mut has_escape = false;
	let mut i = 0usize;
	let len = input.len();
	// Reserve optimistic capacity: input + quotes handled by caller; here only payload.
	out.reserve(input.len());

	while i < len {
		// Find next '"' or '\\' from current position.
		let slice = &input[i..];
		let pos = memchr2(b'"', b'\\', slice);
		let next = match pos {
			Some(p) => i + p,
			None => len,
		};
		// Between i..next there are no quotes or backslashes; still may have control bytes.
		let mut j = i;
		while j < next {
			let b = input[j];
			if b < 0x20 {
				// flush preceding safe range
				if i < j {
					// Safety: bytes are valid UTF-8 slice of original str
					out.push_str(unsafe { std::str::from_utf8_unchecked(&input[i..j]) });
				}
				push_control_escape(b, out);
				has_escape = true;
				j += 1;
				i = j;
			} else {
				j += 1;
			}
		}
		// Copy the rest of the safe run (no controls within i..next)
		if i < next {
			out.push_str(unsafe { std::str::from_utf8_unchecked(&input[i..next]) });
		}
		if next >= len {
			break;
		}
		// Handle special at `next`
		match input[next] {
			b'"' => {
				out.push_str("\\\"");
				has_escape = true;
			}
			b'\\' => {
				out.push_str("\\\\");
				has_escape = true;
			}
			_ => {}
		}
		i = next + 1;
	}
	has_escape
}

#[inline]
fn push_control_escape(b: u8, out: &mut String) {
	// Common escapes first
	match b {
		b'\n' => out.push_str("\\n"),
		b'\r' => out.push_str("\\r"),
		b'\t' => out.push_str("\\t"),
		0x00..=0x1F => {
			const HEX: &[u8; 16] = b"0123456789ABCDEF";
			let mut buf: [u8; 6] = [b'\\', b'u', b'0', b'0', 0, 0];
			buf[4] = HEX[(b >> 4) as usize];
			buf[5] = HEX[(b & 0x0F) as usize];
			// SAFETY: buf is valid ASCII
			out.push_str(unsafe { std::str::from_utf8_unchecked(&buf) });
		}
		_ => {}
	}
}

/// Decode UTF-16 units to UTF-8 String and then escape for JSON.
/// Caller should have validated surrogate pairs.
#[inline]
pub fn escape_json_utf16(units: &[u16], out: &mut String) -> std::io::Result<()> {
	// Reuse `utf16_opt` decode to String, then feed to ASCII escaper.
	let decoded = crate::utils::utf16_opt::decode_utf16_trim(units)?;
	let start_len = out.len();
	let had = escape_json_ascii(decoded.as_bytes(), out);
	if !had {
		// If no escapes, we still need to append the raw string
		if out.len() == start_len {
			out.push_str(&decoded);
		}
	}
	Ok(())
}