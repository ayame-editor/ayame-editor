//! Field extraction and operation key construction.
//!
//! Data ops share the same field model: fast raw delimiter splitting by default,
//! RFC-4180 field parsing when requested, and decoded text keys only when an op
//! actually needs text ordering or display.

use unicode_normalization::UnicodeNormalization;

use crate::encoding::Encoding;

/// How a key/value field is located within a line.
#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    /// Field delimiter (e.g. `,` or `\t`).
    pub delimiter: u8,
    /// Quote character for RFC-4180 parsing (only used when `csv` is true).
    pub quote: u8,
    /// When true, split with a real CSV parser (quoted fields may contain the
    /// delimiter; `""` is an escaped quote). When false, split on raw delimiter
    /// bytes (faster; correct for clean TSV/CSV without quoting).
    pub csv: bool,
}

impl Default for FieldSpec {
    fn default() -> Self {
        FieldSpec {
            delimiter: b',',
            quote: b'"',
            csv: false,
        }
    }
}

/// Build a byte key whose `Ord` matches the desired sort order: an
/// order-preserving 8-byte encoding for numeric keys, else the field decoded to
/// NFC-normalized UTF-8 (byte order == code-point order). Shared by sort and
/// top-n.
pub(crate) fn comparable_key(
    raw: &[u8],
    enc: Encoding,
    col: Option<usize>,
    spec: &FieldSpec,
    numeric: bool,
    scratch: &mut Vec<u8>,
) -> Vec<u8> {
    let field = field_bytes(raw, col, spec, scratch);
    if numeric {
        let v = enc
            .decode_line(field)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| !v.is_nan())
            .unwrap_or(f64::NEG_INFINITY);
        f64_order_key(v).to_vec()
    } else {
        normalized_text_key(field, enc)
    }
}

pub(crate) fn comparable_key_into(
    raw: &[u8],
    enc: Encoding,
    col: Option<usize>,
    spec: &FieldSpec,
    numeric: bool,
    field_scratch: &mut Vec<u8>,
    out: &mut Vec<u8>,
) {
    out.clear();
    let field = field_bytes(raw, col, spec, field_scratch);
    if numeric {
        let v = enc
            .decode_line(field)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| !v.is_nan())
            .unwrap_or(f64::NEG_INFINITY);
        out.extend_from_slice(&f64_order_key(v));
    } else {
        let decoded = enc.decode_line(field);
        if decoded.is_ascii() {
            out.extend_from_slice(decoded.as_bytes());
        } else {
            out.extend_from_slice(decoded.nfc().collect::<String>().as_bytes());
        }
    }
}

pub(crate) fn decoded_text_key_into(
    raw: &[u8],
    enc: Encoding,
    col: Option<usize>,
    spec: &FieldSpec,
    field_scratch: &mut Vec<u8>,
    out: &mut Vec<u8>,
) {
    out.clear();
    let field = field_bytes(raw, col, spec, field_scratch);
    out.extend_from_slice(enc.decode_line(field).as_bytes());
}

fn normalized_text_key(field: &[u8], enc: Encoding) -> Vec<u8> {
    let decoded = enc.decode_line(field);
    if decoded.is_ascii() {
        decoded.into_bytes()
    } else {
        decoded.nfc().collect::<String>().into_bytes()
    }
}

/// Extract the key/value field from a line. Returns a borrow of `raw` for the
/// fast (whole-line or raw-split) paths, or of `scratch` after CSV parsing.
pub(crate) fn field_bytes<'a>(
    raw: &'a [u8],
    col: Option<usize>,
    spec: &FieldSpec,
    scratch: &'a mut Vec<u8>,
) -> &'a [u8] {
    match col {
        None => raw,
        Some(c) if spec.csv => {
            csv_nth_field(raw, spec.delimiter, spec.quote, c, scratch);
            &scratch[..]
        }
        Some(c) => nth_field(raw, spec.delimiter, c),
    }
}

/// 1-based field by raw delimiter byte (no quote handling); empty if out of range.
fn nth_field(raw: &[u8], delim: u8, col: usize) -> &[u8] {
    if col == 0 {
        return raw;
    }
    let mut idx = 1;
    let mut field_start = 0usize;
    for (i, &b) in raw.iter().enumerate() {
        if b == delim {
            if idx == col {
                return &raw[field_start..i];
            }
            idx += 1;
            field_start = i + 1;
        }
    }
    if idx == col {
        &raw[field_start..]
    } else {
        &[]
    }
}

/// RFC-4180-aware extraction of the 1-based `col` field of a single record into
/// `out` (cleared first), unescaping quotes. Uses `csv-core` (allocation-free).
///
/// NOTE: one physical line == one record. A quoted field with an *embedded
/// newline* would have already been split by the line index, so embedded
/// newlines in quoted fields are not supported (see DESIGN / ROADMAP). Quoted
/// delimiters and `""` escapes within a line are handled correctly.
fn csv_nth_field(raw: &[u8], delim: u8, quote: u8, col: usize, out: &mut Vec<u8>) {
    out.clear();
    if col == 0 {
        out.extend_from_slice(raw);
        return;
    }
    let mut rdr = csv_core::ReaderBuilder::new()
        .delimiter(delim)
        .quote(quote)
        .build();
    let mut input = raw;
    let mut buf = [0u8; 512];
    let mut idx = 1usize;
    let mut flushed = false;
    loop {
        let (res, nin, nout) = rdr.read_field(input, &mut buf);
        input = &input[nin..];
        if idx == col {
            out.extend_from_slice(&buf[..nout]);
        }
        match res {
            csv_core::ReadFieldResult::InputEmpty => {
                if input.is_empty() {
                    if flushed {
                        break;
                    }
                    flushed = true; // one more call with empty input flushes the final field
                }
            }
            csv_core::ReadFieldResult::OutputFull => {} // same field continues into buf again
            csv_core::ReadFieldResult::Field { record_end } => {
                if idx == col {
                    break;
                }
                idx += 1;
                if record_end {
                    break;
                }
            }
            csv_core::ReadFieldResult::End => break,
        }
    }
}

/// Map an f64 to an 8-byte big-endian key whose unsigned byte order equals the
/// numeric order of the original value (handles negatives and -0.0).
fn f64_order_key(x: f64) -> [u8; 8] {
    let bits = x.to_bits();
    let ord = if bits & 0x8000_0000_0000_0000 != 0 {
        !bits // negative: flip all bits
    } else {
        bits ^ 0x8000_0000_0000_0000 // non-negative: flip sign bit
    };
    ord.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_handles_quotes_and_escapes() {
        let line = br#"a,"x,y ""z""",c"#; // field 2 = x,y "z"
        let mut out = Vec::new();
        csv_nth_field(line, b',', b'"', 1, &mut out);
        assert_eq!(out, b"a");
        csv_nth_field(line, b',', b'"', 2, &mut out);
        assert_eq!(out, &b"x,y \"z\""[..]);
        csv_nth_field(line, b',', b'"', 3, &mut out);
        assert_eq!(out, b"c");
        csv_nth_field(line, b',', b'"', 4, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn csv_field_streams_quoted_fields_larger_than_internal_buffer() {
        let long = "x".repeat(900);
        let line = format!("a,\"{long}\",z");
        let mut out = Vec::new();
        csv_nth_field(line.as_bytes(), b',', b'"', 2, &mut out);
        assert_eq!(out, long.as_bytes());
    }
}
