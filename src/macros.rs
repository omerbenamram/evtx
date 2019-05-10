/// Tries to read X bytes from the cursor, if reading fails, captures position nicely.
macro_rules! try_read {
    ($cursor: ident, u8) => {
        $cursor.read_u8().context(err::IO)?;
    };

    ($cursor: ident, i8) => {
        $cursor.read_i8().context(err::IO)?;
    };

    ($cursor: ident, u16) => {
        $cursor
            .read_u16::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, i16) => {
        $cursor
            .read_i16::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, i32) => {
        $cursor
            .read_i32::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, u32) => {
        $cursor
            .read_u32::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, f32) => {
        $cursor
            .read_f32::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, i64) => {
        $cursor
            .read_i64::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, u64) => {
        $cursor
            .read_u64::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, f64) => {
        $cursor
            .read_f64::<byteorder::LittleEndian>()
            .context(err::IO)?;
    };

    ($cursor: ident, bool) => {{
        let bool_value = try_read!($cursor, i32);
        match bool_value {
            0 => false,
            1 => true,
            _ => {
                log::warn!(
                    "{:?} is an unknown value for bool, coercing to `true`",
                    bool_value
                );
                true
            }
        }
    }};

    ($cursor: ident, guid) => {
        Guid::from_stream($cursor).context(err::FailedToReadGUID {
            offset: $cursor.position(),
        })?
    };

    ($cursor: ident, utf_16_str) => {{
        let s = read_len_prefixed_utf16_string($cursor, false)
            .context(err::FailedToDecodeUTF16String {
                offset: $cursor.position(),
            })?
            .unwrap_or_else(|| "".to_owned());

        Cow::Owned(s)
    }};

    ($cursor: ident, null_terminated_utf_16_str) => {{
        let s =
            read_null_terminated_utf16_string($cursor).context(err::FailedToDecodeUTF16String {
                offset: $cursor.position(),
            })?;

        Cow::Owned(s)
    }};

    ($cursor: ident, sid) => {
        Sid::from_stream($cursor).context(err::FailedToReadNTSID {
            offset: $cursor.position(),
        })?
    };

    ($cursor: ident, hex32) => {
        Cow::Owned(format!("0x{:x}", try_read!($cursor, i32)))
    };

    ($cursor: ident, hex64) => {
        Cow::Owned(format!("0x{:x}", try_read!($cursor, i64)))
    };

    ($cursor: ident, filetime) => {
        datetime_from_filetime(try_read!($cursor, u64))
    };

    ($cursor: ident, systime) => {
        read_systemtime($cursor)?
    };
}

macro_rules! try_read_sized_array {
    ($cursor: ident, $unit: ident, $size: ident) => {{
        let mut array = vec![];
        let start_pos = $cursor.position();

        loop {
            if ($cursor.position() - start_pos) >= u64::from($size) {
                break;
            }

            let val = try_read!($cursor, $unit);
            array.push(val);
        }

        array
    }};
}
