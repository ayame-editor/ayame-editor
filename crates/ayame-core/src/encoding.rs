//! Encoding and line-ending detection.
//!
//! Big-data text in the wild — especially in Japan, Ayame's heartland — is not
//! always UTF-8. Shift_JIS / CP932 and EUC-JP are everywhere in legacy exports.
//! We lean on `encoding_rs` (Firefox's encoding engine) for decoding and
//! `chardetng` (Firefox's detector) for guessing, then expose a small stable
//! enum so the rest of the engine never touches a charset table.

use serde::Serialize;

/// Encodings Ayame understands for *indexed* viewing.
///
/// All of these are ASCII-transparent for `0x0A`, which is what lets the line
/// index scan raw bytes for newlines. UTF-16/UTF-32 are detected (see
/// [`detect`]) but are intentionally rejected by the document layer for now —
/// their newline units are not single bytes. (Roadmap.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    ShiftJis,
    EucJp,
    /// 7-bit ASCII (a strict, common subset; decoded as UTF-8).
    Ascii,
    /// Detected but not supported for indexing yet.
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::ShiftJis => "Shift_JIS",
            Encoding::EucJp => "EUC-JP",
            Encoding::Ascii => "ASCII",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
        }
    }

    /// True for encodings the indexer cannot yet handle (multi-byte newline unit).
    pub fn is_wide(self) -> bool {
        matches!(self, Encoding::Utf16Le | Encoding::Utf16Be)
    }

    fn rs(self) -> &'static encoding_rs::Encoding {
        match self {
            Encoding::Utf8 | Encoding::Ascii => encoding_rs::UTF_8,
            Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
            Encoding::EucJp => encoding_rs::EUC_JP,
            Encoding::Utf16Le => encoding_rs::UTF_16LE,
            Encoding::Utf16Be => encoding_rs::UTF_16BE,
        }
    }

    /// Decode a single line's bytes to a `String`, replacing malformed sequences.
    /// Operates on small slices (one line), so allocation here is negligible.
    pub fn decode_line(self, bytes: &[u8]) -> String {
        let (cow, _had_errors) = self.rs().decode_without_bom_handling(bytes);
        cow.into_owned()
    }

    /// Encode a query string into this encoding's bytes for raw-byte searching.
    /// Returns `None` if the query is unmappable (e.g. CJK into ASCII).
    pub fn encode_query(self, q: &str) -> Option<Vec<u8>> {
        let (cow, _enc, had_unmappable) = self.rs().encode(q);
        if had_unmappable {
            None
        } else {
            Some(cow.into_owned())
        }
    }

    /// Parse a user-supplied encoding name (CLI / API override).
    pub fn parse(name: &str) -> Option<Encoding> {
        let n = name
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "");
        Some(match n.as_str() {
            "utf8" => Encoding::Utf8,
            "ascii" | "usascii" => Encoding::Ascii,
            "shiftjis" | "sjis" | "cp932" | "windows31j" | "ms932" => Encoding::ShiftJis,
            "eucjp" | "euc" => Encoding::EucJp,
            "utf16" | "utf16le" => Encoding::Utf16Le,
            "utf16be" => Encoding::Utf16Be,
            _ => return None,
        })
    }
}

/// Length in bytes of a leading byte-order mark, if present.
pub fn bom_len(buf: &[u8]) -> (usize, Option<Encoding>) {
    if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (3, Some(Encoding::Utf8))
    } else if buf.starts_with(&[0xFF, 0xFE]) {
        (2, Some(Encoding::Utf16Le))
    } else if buf.starts_with(&[0xFE, 0xFF]) {
        (2, Some(Encoding::Utf16Be))
    } else {
        (0, None)
    }
}

/// Detect `(encoding, bom_byte_length)` for a buffer.
///
/// A BOM is authoritative. Otherwise we sniff a bounded prefix: pure-ASCII is
/// reported as ASCII; valid UTF-8 wins; failing that we let `chardetng` choose
/// between the Japanese legacy candidates. An explicit `override_enc` short-
/// circuits everything except BOM length.
pub fn detect(buf: &[u8], override_enc: Option<Encoding>) -> (Encoding, usize) {
    let (bom, bom_enc) = bom_len(buf);

    if let Some(forced) = override_enc {
        return (forced, bom);
    }
    if let Some(e) = bom_enc {
        return (e, bom);
    }

    // Sniff a bounded prefix so detection is O(1) regardless of file size.
    const SNIFF: usize = 256 * 1024;
    let prefix = &buf[..buf.len().min(SNIFF)];
    if prefix.is_empty() {
        return (Encoding::Utf8, 0);
    }
    if prefix.is_ascii() {
        return (Encoding::Ascii, 0);
    }
    if std::str::from_utf8(prefix).is_ok() {
        return (Encoding::Utf8, 0);
    }

    let mut det = chardetng::EncodingDetector::new();
    det.feed(prefix, buf.len() <= SNIFF);
    let guess = det.guess(None, true);
    let enc = if guess == encoding_rs::SHIFT_JIS {
        Encoding::ShiftJis
    } else if guess == encoding_rs::EUC_JP {
        Encoding::EucJp
    } else if guess == encoding_rs::UTF_16LE {
        Encoding::Utf16Le
    } else if guess == encoding_rs::UTF_16BE {
        Encoding::Utf16Be
    } else {
        // chardetng may suggest western single-byte encodings; for a byte-oriented
        // viewer UTF-8 (lossy) is the safest universal fallback.
        Encoding::Utf8
    };
    (enc, 0)
}

/// Line-ending styles, detected from a bounded prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Eol {
    Lf,
    Crlf,
    Cr,
    Mixed,
    /// No line terminator seen at all.
    None,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
            Eol::Cr => "CR",
            Eol::Mixed => "Mixed",
            Eol::None => "None",
        }
    }
}

/// Detect the dominant line ending from a bounded prefix of `content`.
pub fn detect_eol(content: &[u8]) -> Eol {
    const SNIFF: usize = 64 * 1024;
    let p = &content[..content.len().min(SNIFF)];
    let mut crlf = 0u32;
    let mut lf = 0u32;
    let mut cr = 0u32;
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            b'\n' => {
                lf += 1;
                i += 1;
            }
            b'\r' => {
                if p.get(i + 1) == Some(&b'\n') {
                    crlf += 1;
                    i += 2;
                } else {
                    cr += 1;
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    // CRLF terminators advance `i` by 2 without touching `lf`, so `lf` already
    // counts only lone line feeds.
    let lone_lf = lf;
    let styles = (crlf > 0) as u8 + (lone_lf > 0) as u8 + (cr > 0) as u8;
    match (styles, crlf, lone_lf, cr) {
        (0, _, _, _) => Eol::None,
        (1, c, _, _) if c > 0 => Eol::Crlf,
        (1, _, l, _) if l > 0 => Eol::Lf,
        (1, _, _, _) => Eol::Cr,
        _ => Eol::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ascii_and_utf8() {
        assert_eq!(detect(b"hello world\n", None), (Encoding::Ascii, 0));
        let (e, b) = detect("日本語のテキスト\n".as_bytes(), None);
        assert_eq!((e, b), (Encoding::Utf8, 0));
    }

    #[test]
    fn detects_utf8_bom() {
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice("あ\n".as_bytes());
        assert_eq!(detect(&v, None), (Encoding::Utf8, 3));
    }

    #[test]
    fn detects_shift_jis() {
        // "日本語" in Shift_JIS.
        let (cow, _, err) = encoding_rs::SHIFT_JIS.encode("日本語テキストのサンプルです。");
        assert!(!err);
        let (enc, bom) = detect(&cow, None);
        assert_eq!(bom, 0);
        assert_eq!(enc, Encoding::ShiftJis);
        assert_eq!(enc.decode_line(&cow), "日本語テキストのサンプルです。");
    }

    #[test]
    fn eol_detection() {
        assert_eq!(detect_eol(b"a\nb\n"), Eol::Lf);
        assert_eq!(detect_eol(b"a\r\nb\r\n"), Eol::Crlf);
        assert_eq!(detect_eol(b"a\rb\r"), Eol::Cr);
        assert_eq!(detect_eol(b"a\r\nb\nc"), Eol::Mixed);
        assert_eq!(detect_eol(b"no newline"), Eol::None);
    }

    #[test]
    fn override_wins_but_keeps_bom() {
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice(b"abc\n");
        assert_eq!(
            detect(&v, Some(Encoding::ShiftJis)),
            (Encoding::ShiftJis, 3)
        );
    }
}
