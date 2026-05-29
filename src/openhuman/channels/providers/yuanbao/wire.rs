//! Hand-rolled protobuf wire-format primitives.
//!
//! Only varints, length-delimited bytes, and the two fixed-width forms
//! are supported — that's everything the yuanbao protocol uses. Kept
//! separate from `proto.rs` so the latter stays under 500 lines and
//! reads as a "schema" file.

use std::sync::atomic::{AtomicU32, Ordering};

use super::errors::YuanbaoError;

/// Global per-process sequence number for ConnMsg head.seq_no.
static SEQ: AtomicU32 = AtomicU32::new(1);

pub fn next_seq_no() -> u32 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

pub const WT_VARINT: u8 = 0;
pub const WT_LEN: u8 = 2;

// ─── Varint ─────────────────────────────────────────────────────────

pub fn encode_varint(mut value: u64, buf: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

pub fn decode_varint(data: &[u8], pos: usize) -> Result<(u64, usize), YuanbaoError> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    let mut i = pos;
    loop {
        if i >= data.len() {
            return Err(YuanbaoError::ProtoDecode("truncated varint".into()));
        }
        let byte = data[i];
        // On the 10th byte (shift == 63) a valid u64 varint can only have
        // the lowest bit set (values 0 or 1); anything higher overflows.
        if shift == 63 && byte > 1 {
            return Err(YuanbaoError::ProtoDecode(format!(
                "varint overflow: 10th byte is {byte:#04x}, expected 0x00 or 0x01"
            )));
        }
        value |= ((byte & 0x7F) as u64) << shift;
        i += 1;
        if byte & 0x80 == 0 {
            return Ok((value, i - pos));
        }
        shift += 7;
        if shift >= 64 {
            return Err(YuanbaoError::ProtoDecode("varint too long".into()));
        }
    }
}

// ─── Field encoders ────────────────────────────────────────────────

pub fn encode_field_varint(field: u32, value: u64, buf: &mut Vec<u8>) {
    encode_varint(((field as u64) << 3) | WT_VARINT as u64, buf);
    encode_varint(value, buf);
}

pub fn encode_field_bytes(field: u32, data: &[u8], buf: &mut Vec<u8>) {
    encode_varint(((field as u64) << 3) | WT_LEN as u64, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

pub fn encode_field_string(field: u32, s: &str, buf: &mut Vec<u8>) {
    encode_field_bytes(field, s.as_bytes(), buf);
}

// ─── Field parsing ──────────────────────────────────────────────────

#[derive(Debug)]
pub enum FieldValue {
    Varint(u64),
    Bytes(Vec<u8>),
    Fixed32(u32),
    Fixed64(u64),
}

pub fn parse_fields(data: &[u8]) -> Result<Vec<(u32, FieldValue)>, YuanbaoError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (tag, n) = decode_varint(data, pos)?;
        pos += n;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x07) as u8;
        match wire {
            WT_VARINT => {
                let (v, n) = decode_varint(data, pos)?;
                pos += n;
                out.push((field, FieldValue::Varint(v)));
            }
            WT_LEN => {
                let (len, n) = decode_varint(data, pos)?;
                pos += n;
                // Use checked conversions / arithmetic — a crafted oversize
                // varint length would otherwise overflow `usize` on 32-bit
                // targets and panic during slicing.
                let len_usize = usize::try_from(len).map_err(|_| {
                    YuanbaoError::ProtoDecode(format!(
                        "len field {field} too large for platform: {len}"
                    ))
                })?;
                let end = pos.checked_add(len_usize).ok_or_else(|| {
                    YuanbaoError::ProtoDecode(format!(
                        "len field {field} overflows position: pos={pos} len={len}"
                    ))
                })?;
                if end > data.len() {
                    return Err(YuanbaoError::ProtoDecode(format!(
                        "truncated len field {field}: need {len} have {}",
                        data.len() - pos
                    )));
                }
                out.push((field, FieldValue::Bytes(data[pos..end].to_vec())));
                pos = end;
            }
            1 => {
                if pos + 8 > data.len() {
                    return Err(YuanbaoError::ProtoDecode("truncated fixed64".into()));
                }
                let v = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
                pos += 8;
                out.push((field, FieldValue::Fixed64(v)));
            }
            5 => {
                if pos + 4 > data.len() {
                    return Err(YuanbaoError::ProtoDecode("truncated fixed32".into()));
                }
                let v = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
                pos += 4;
                out.push((field, FieldValue::Fixed32(v)));
            }
            other => {
                return Err(YuanbaoError::ProtoDecode(format!(
                    "unsupported wire type {other} at field {field}"
                )));
            }
        }
    }
    Ok(out)
}

pub fn get_string(fields: &[(u32, FieldValue)], num: u32) -> String {
    for (n, v) in fields {
        if *n == num {
            if let FieldValue::Bytes(b) = v {
                return String::from_utf8_lossy(b).into_owned();
            }
        }
    }
    String::new()
}

pub fn get_varint(fields: &[(u32, FieldValue)], num: u32) -> u64 {
    for (n, v) in fields {
        if *n == num {
            if let FieldValue::Varint(x) = v {
                return *x;
            }
        }
    }
    0
}

pub fn get_bytes(fields: &[(u32, FieldValue)], num: u32) -> Vec<u8> {
    for (n, v) in fields {
        if *n == num {
            if let FieldValue::Bytes(b) = v {
                return b.clone();
            }
        }
    }
    Vec::new()
}

pub fn get_repeated_bytes(fields: &[(u32, FieldValue)], num: u32) -> Vec<Vec<u8>> {
    fields
        .iter()
        .filter_map(|(n, v)| match v {
            FieldValue::Bytes(b) if *n == num => Some(b.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for &v in &[0u64, 1, 127, 128, 300, 16384, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            encode_varint(v, &mut buf);
            let (got, n) = decode_varint(&buf, 0).unwrap();
            assert_eq!(got, v, "varint roundtrip failed for {v}");
            assert_eq!(n, buf.len());
        }
    }

    #[test]
    fn varint_truncated_errors() {
        let buf = vec![0x80, 0x80]; // continuation bit set but no end
        assert!(decode_varint(&buf, 0).is_err());
    }

    #[test]
    fn field_roundtrip() {
        let mut buf = Vec::new();
        encode_field_varint(1, 42, &mut buf);
        encode_field_string(2, "hello", &mut buf);
        encode_field_bytes(3, b"\x00\x01\x02", &mut buf);

        let fields = parse_fields(&buf).unwrap();
        assert_eq!(get_varint(&fields, 1), 42);
        assert_eq!(get_string(&fields, 2), "hello");
        assert_eq!(get_bytes(&fields, 3), vec![0, 1, 2]);
    }

    #[test]
    fn unknown_field_skipped_gracefully() {
        let mut buf = Vec::new();
        encode_field_varint(99, 123, &mut buf);
        encode_field_string(1, "wanted", &mut buf);
        let fields = parse_fields(&buf).unwrap();
        assert_eq!(get_string(&fields, 1), "wanted");
        assert_eq!(get_string(&fields, 2), ""); // missing field returns default
    }

    #[test]
    fn seq_numbers_are_monotonic() {
        let a = next_seq_no();
        let b = next_seq_no();
        assert!(b > a);
    }

    #[test]
    fn varint_too_long_errors() {
        // 11 continuation bytes — the 10th byte (0x80) triggers the overflow
        // guard before the loop even reaches the 11th.
        let buf = vec![0x80; 11];
        match decode_varint(&buf, 0).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => {
                assert!(m.contains("too long") || m.contains("overflow"), "got {m}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fields_truncated_bytes_field_errors() {
        // Field 1 (wire type 2) declaring length 5 but only 1 byte of payload.
        let buf = vec![
            (1 << 3) | 2, // tag: field=1, wire=2
            5,            // claimed len
            b'a',
        ];
        match parse_fields(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => assert!(m.contains("truncated"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fields_oversize_len_field_errors_without_panic() {
        // Field 1 (wire type 2) with a varint length encoding `u64::MAX` —
        // previously this would attempt `pos + len as usize`, overflowing
        // on 32-bit and slicing past the buffer on 64-bit. Now it must
        // return a structured decode error.
        let mut buf = Vec::new();
        buf.push((1 << 3) | 2); // tag: field=1, wire=2
        encode_varint(u64::MAX, &mut buf); // adversarial length
        match parse_fields(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => {
                assert!(
                    m.contains("too large") || m.contains("overflows") || m.contains("truncated"),
                    "expected overflow/truncation error, got {m}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fields_reads_fixed64() {
        let mut buf = Vec::new();
        buf.push((1 << 3) | 1); // tag: field=1, wire=1 (fixed64)
        buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        let f = parse_fields(&buf).unwrap();
        match f[0].1 {
            FieldValue::Fixed64(v) => assert_eq!(v, 0x1122_3344_5566_7788),
            ref other => panic!("expected Fixed64 got {other:?}"),
        }
    }

    #[test]
    fn parse_fields_truncated_fixed64_errors() {
        let mut buf = Vec::new();
        buf.push((1 << 3) | 1);
        buf.extend_from_slice(&[0u8; 4]); // only 4/8 bytes
        match parse_fields(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => assert!(m.contains("fixed64"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fields_reads_fixed32() {
        let mut buf = Vec::new();
        buf.push((1 << 3) | 5); // tag: field=1, wire=5 (fixed32)
        buf.extend_from_slice(&0xCAFEBABEu32.to_le_bytes());
        let f = parse_fields(&buf).unwrap();
        match f[0].1 {
            FieldValue::Fixed32(v) => assert_eq!(v, 0xCAFEBABE),
            ref other => panic!("expected Fixed32 got {other:?}"),
        }
    }

    #[test]
    fn parse_fields_truncated_fixed32_errors() {
        let mut buf = Vec::new();
        buf.push((1 << 3) | 5);
        buf.extend_from_slice(&[0u8; 2]); // only 2/4 bytes
        match parse_fields(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => assert!(m.contains("fixed32"), "got {m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_fields_unsupported_wire_type_errors() {
        // wire type 3 (start group) is not supported.
        let buf = vec![(1 << 3) | 3];
        match parse_fields(&buf).unwrap_err() {
            YuanbaoError::ProtoDecode(m) => {
                assert!(m.contains("unsupported wire type 3"), "got {m}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn get_string_returns_empty_when_field_is_varint() {
        // Field 1 exists but encoded as varint, not bytes — get_string must
        // skip past it and return the default.
        let mut buf = Vec::new();
        encode_field_varint(1, 7, &mut buf);
        let fields = parse_fields(&buf).unwrap();
        assert_eq!(get_string(&fields, 1), "");
    }

    #[test]
    fn get_varint_returns_zero_when_field_is_bytes() {
        let mut buf = Vec::new();
        encode_field_string(1, "not a varint", &mut buf);
        let fields = parse_fields(&buf).unwrap();
        assert_eq!(get_varint(&fields, 1), 0);
    }

    #[test]
    fn get_bytes_returns_empty_when_field_is_varint() {
        let mut buf = Vec::new();
        encode_field_varint(1, 7, &mut buf);
        let fields = parse_fields(&buf).unwrap();
        assert!(get_bytes(&fields, 1).is_empty());
    }

    #[test]
    fn get_repeated_bytes_collects_multiple_same_field() {
        let mut buf = Vec::new();
        encode_field_string(1, "a", &mut buf);
        encode_field_string(1, "bb", &mut buf);
        encode_field_string(2, "c", &mut buf); // different field — should be skipped
        encode_field_string(1, "ddd", &mut buf);
        let fields = parse_fields(&buf).unwrap();
        let got = get_repeated_bytes(&fields, 1);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], b"a");
        assert_eq!(got[1], b"bb");
        assert_eq!(got[2], b"ddd");
    }
}
