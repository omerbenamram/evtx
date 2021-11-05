macro_rules! capture_context {
    ($cursor: ident, $e: ident, $name: expr) => {{
        let inner = $crate::err::WrappedIoError::capture_hexdump(Box::new($e), $cursor);
        $crate::err::DeserializationError::from(inner)
    }};
    ($cursor: ident, $e: ident, $token: expr, $name: expr) => {{
        let inner = $crate::err::WrappedIoError::capture_hexdump(Box::new($e), $cursor);
        $crate::err::DeserializationError::FailedToReadToken {
            t: $token.to_owned(),
            token_name: $name,
            source: inner,
        }
    }};
}

macro_rules! try_seek {
    ($cursor: ident, $offset: expr, $name: expr) => {
        $cursor
            .seek(SeekFrom::Start(u64::from($offset.clone())))
            .map_err(|e| capture_context!($cursor, e, $name))
    };
}

/// Tries to read X bytes from the cursor, if reading fails, captures position nicely.
macro_rules! try_read {
    ($cursor: ident, u8, $name: expr) => {
        $cursor
            .read_u8()
            .map_err(|e| capture_context!($cursor, e, "u8", $name))
    };

    ($cursor: ident, u8) => {
        try_read!($cursor, u8, "<Unknown>")
    };

    ($cursor: ident, i8, $name: expr) => {
        $cursor
            .read_i8()
            .map_err(|e| capture_context!($cursor, e, "i8", $name))
    };

    ($cursor: ident, i8) => {
        try_read!($cursor, i8, "<Unknown>")
    };

    ($cursor: ident, u16, $name: expr) => {
        $cursor
            .read_u16::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "u16", $name))
    };

    ($cursor: ident, u16) => {
        try_read!($cursor, u16, "<Unknown>")
    };

    ($cursor: ident, i16, $name: expr) => {
        $cursor
            .read_i16::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "i16", $name))
    };

    ($cursor: ident, i16) => {
        try_read!($cursor, i16, "<Unknown>")
    };

    ($cursor: ident, i32, $name: expr) => {
        $cursor
            .read_i32::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "i32", $name))
    };

    ($cursor: ident, i32) => {
        try_read!($cursor, i32, "<Unknown>")
    };

    ($cursor: ident, u32, $name: expr) => {
        $cursor
            .read_u32::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "u32", $name))
    };

    ($cursor: ident, u32) => {
        try_read!($cursor, u32, "<Unknown>")
    };

    ($cursor: ident, f32, $name: expr) => {
        $cursor
            .read_f32::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "f32", $name))
    };

    ($cursor: ident, f32) => {
        try_read!($cursor, f32, "<Unknown>")
    };

    ($cursor: ident, i64, $name: expr) => {
        $cursor
            .read_i64::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "i64", $name))
    };

    ($cursor: ident, i64) => {
        try_read!($cursor, i64, "<Unknown>")
    };

    ($cursor: ident, u64, $name: expr) => {
        $cursor
            .read_u64::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "u64", $name))
    };

    ($cursor: ident, u64) => {
        try_read!($cursor, u64, "<Unknown>")
    };

    ($cursor: ident, f64, $name: expr) => {
        $cursor
            .read_f64::<byteorder::LittleEndian>()
            .map_err(|e| capture_context!($cursor, e, "f64", $name))
    };

    ($cursor: ident, f64) => {
        try_read!($cursor, f64, "<Unknown>")
    };

    ($cursor: ident, bool) => {{
        let bool_value = try_read!($cursor, i32);
        match bool_value {
            Ok(0) => Ok(false),
            Ok(1) => Ok(true),
            Ok(number) => {
                log::warn!(
                    "{:} is an unknown value for bool, coercing to `true`",
                    number
                );
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }};

    ($cursor: ident, guid) => {
        try_read!($cursor, guid, "<Unknown>")
    };

    ($cursor: ident, guid, $name: expr) => {
        Guid::from_reader($cursor).map_err(|e| capture_context!($cursor, e, "guid", $name))
    };

    ($cursor: ident, len_prefixed_utf_16_str) => {{
        try_read!($cursor, len_prefixed_utf_16_str, "<Unknown>")
    }};

    ($cursor: ident, len_prefixed_utf_16_str, $name: expr) => {
        read_len_prefixed_utf16_string($cursor, false)
            .map_err(|e| capture_context!($cursor, e, "len_prefixed_utf_16_str", $name))
    };

    ($cursor: ident, len_prefixed_utf_16_str_nul_terminated) => {{
        try_read!($cursor, len_prefixed_utf_16_str_nul_terminated, "<Unknown>")
    }};

    ($cursor: ident, len_prefixed_utf_16_str_nul_terminated, $name: expr) => {
        read_len_prefixed_utf16_string($cursor, true).map_err(|e| {
            capture_context!($cursor, e, "len_prefixed_utf_16_str_nul_terminated", $name)
        })
    };

    ($cursor: ident, null_terminated_utf_16_str) => {{
        try_read!($cursor, null_terminated_utf_16_str, "<Unknown>")
    }};

    ($cursor: ident, null_terminated_utf_16_str, $name: expr) => {
        read_null_terminated_utf16_string($cursor)
            .map_err(|e| capture_context!($cursor, e, "null_terminated_utf_16_str", $name))
    };

    ($cursor: ident, sid, $name: expr) => {
        Sid::from_reader($cursor).map_err(|e| capture_context!($cursor, e, "ntsid", $name))
    };

    ($cursor: ident, sid) => {
        try_read!($cursor, sid, "<Unknown>")
    };

    ($cursor: ident, hex32) => {{
        try_read!($cursor, i32).map(|value| Cow::Owned(format!("0x{:x}", value)))
    }};

    ($cursor: ident, hex64) => {
        try_read!($cursor, i64).map(|value| Cow::Owned(format!("0x{:x}", value)))
    };

    ($cursor: ident, filetime) => {
        try_read!($cursor, filetime, "<Unknown>")
    };

    ($cursor: ident, filetime, $name: expr) => {
        winstructs::timestamp::WinTimestamp::from_reader($cursor)
            .map_err(|e| capture_context!($cursor, e, "filetime", $name))
            .map(|t| t.to_datetime())
    };

    ($cursor: ident, systime) => {
        read_systemtime($cursor)
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

            let val = try_read!($cursor, $unit)?;
            array.push(val);
        }

        array
    }};
}
