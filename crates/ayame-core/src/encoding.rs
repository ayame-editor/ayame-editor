//! Encoding and line-ending detection.
//!
//! Big-data text in the wild — especially in Japan, Ayame's heartland — is not
//! always UTF-8. Shift_JIS / CP932 and EUC-JP are everywhere in legacy exports.
//! We lean on `encoding_rs` (Firefox's encoding engine) for decoding and
//! `chardetng` (Firefox's detector) for guessing, then expose a small stable
//! enum so the rest of the engine never touches a charset table.

use serde::Serialize;

/// Encodings Ayame understands for indexed display and edit-save round trips.
///
/// UTF-8/Shift_JIS/EUC-JP/ASCII use single-byte LF terminators. UTF-16LE/BE
/// use aligned 16-bit LF code units and are indexed by the document layer with
/// a wide-newline scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    ShiftJis,
    EucJp,
    /// 7-bit ASCII (a strict, common subset; decoded as UTF-8).
    Ascii,
    /// Stateful 7-bit JIS (mail archives, legacy exports). Decoded per line,
    /// which assumes each line starts in ASCII designation — the universal
    /// JIS convention; the rare line inheriting JIS mode across its newline
    /// decodes to replacement characters instead of derailing anything (#196).
    #[serde(rename = "iso-2022-jp")]
    Iso2022Jp,
    /// UTF-16LE input, indexed with aligned 16-bit newline scanning.
    Utf16Le,
    /// UTF-16BE input, indexed with aligned 16-bit newline scanning.
    Utf16Be,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::ShiftJis => "Shift_JIS",
            Encoding::EucJp => "EUC-JP",
            Encoding::Ascii => "ASCII",
            Encoding::Iso2022Jp => "ISO-2022-JP",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
        }
    }

    /// True when newline scanning must use aligned 16-bit code units.
    pub fn is_wide(self) -> bool {
        matches!(self, Encoding::Utf16Le | Encoding::Utf16Be)
    }

    fn rs(self) -> &'static encoding_rs::Encoding {
        match self {
            Encoding::Utf8 | Encoding::Ascii => encoding_rs::UTF_8,
            Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
            Encoding::EucJp => encoding_rs::EUC_JP,
            Encoding::Iso2022Jp => encoding_rs::ISO_2022_JP,
            Encoding::Utf16Le => encoding_rs::UTF_16LE,
            Encoding::Utf16Be => encoding_rs::UTF_16BE,
        }
    }

    /// Decode a single line's bytes to a `String`, replacing malformed sequences.
    ///
    /// Allocates the whole decoded line — for *display* paths prefer
    /// [`Encoding::decode_line_capped`], which bounds the allocation for the
    /// pathological "one multi-gigabyte line" input (#201). This unbounded
    /// form remains for fidelity-critical paths (saving, transforms) where
    /// truncation would corrupt data.
    pub fn decode_line(self, bytes: &[u8]) -> String {
        if matches!(self, Encoding::Utf16Le | Encoding::Utf16Be) {
            return decode_utf16_line(bytes, self == Encoding::Utf16Le);
        }
        let (cow, _had_errors) = self.rs().decode_without_bom_handling(bytes);
        cow.into_owned()
    }

    /// Decode at most `max_bytes` of a line, returning the text and whether it
    /// was cut short. The cut lands on a character boundary for UTF-8 and on a
    /// code-unit boundary for UTF-16; a legacy multi-byte sequence split at the
    /// cap decodes to one trailing replacement character, which the `true`
    /// flag tells the caller to expect. This is what keeps viewport memory
    /// proportional to the screen, not to the longest line in the file.
    pub fn decode_line_capped(self, bytes: &[u8], max_bytes: usize) -> (String, bool) {
        if bytes.len() <= max_bytes {
            return (self.decode_line(bytes), false);
        }
        let mut cut = max_bytes;
        match self {
            Encoding::Utf16Le | Encoding::Utf16Be => cut &= !1,
            Encoding::Utf8 | Encoding::Ascii => {
                // Back off a split UTF-8 sequence (at most 3 continuation bytes).
                while cut > 0 && bytes[cut] & 0xC0 == 0x80 {
                    cut -= 1;
                }
            }
            Encoding::ShiftJis | Encoding::EucJp | Encoding::Iso2022Jp => {}
        }
        (self.decode_line(&bytes[..cut]), true)
    }

    /// Count the decoded characters of `bytes` in constant memory (streaming
    /// decode through a fixed buffer). Used to compute character columns for
    /// matches deep inside one enormous line without materializing the whole
    /// decoded prefix (#201).
    pub fn count_chars(self, bytes: &[u8]) -> u64 {
        // Valid UTF-8 (the overwhelmingly common case) counts without decoding.
        if matches!(self, Encoding::Utf8 | Encoding::Ascii) {
            if let Ok(s) = std::str::from_utf8(bytes) {
                return s.chars().count() as u64;
            }
        }
        let mut decoder = self.rs().new_decoder_without_bom_handling();
        let mut out = [0u8; 8192];
        let mut total = 0u64;
        let mut input = bytes;
        loop {
            let last = input.is_empty();
            let (result, read, written, _had_errors) =
                decoder.decode_to_utf8(input, &mut out, last);
            // The decoder emits valid UTF-8 (malformed input becomes U+FFFD).
            total += std::str::from_utf8(&out[..written])
                .map(|s| s.chars().count() as u64)
                .unwrap_or(0);
            input = &input[read..];
            if last && result == encoding_rs::CoderResult::InputEmpty {
                return total;
            }
        }
    }

    /// Encode a query string into this encoding's bytes for raw-byte searching.
    /// Returns `None` if the query is unmappable (e.g. CJK into ASCII).
    pub fn encode_query(self, q: &str) -> Option<Vec<u8>> {
        if matches!(self, Encoding::Utf16Le | Encoding::Utf16Be) {
            return Some(encode_utf16_text(q, self == Encoding::Utf16Le));
        }
        let (cow, _enc, had_unmappable) = self.rs().encode(q);
        if had_unmappable {
            None
        } else {
            Some(cow.into_owned())
        }
    }

    /// Encode edited text into this encoding's bytes.
    ///
    /// Edits are kept as UTF-8 strings in the UI/session layer. When saving, we
    /// convert only those edited fragments back to the source encoding; untouched
    /// mmap-backed lines are copied as their original bytes.
    pub fn encode_text(self, text: &str) -> Option<Vec<u8>> {
        self.encode_query(text)
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
            "iso2022jp" | "iso2022" | "jis" => Encoding::Iso2022Jp,
            "utf16" | "utf16le" => Encoding::Utf16Le,
            "utf16be" => Encoding::Utf16Be,
            _ => return None,
        })
    }

    pub fn bom(self) -> &'static [u8] {
        match self {
            Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            Encoding::ShiftJis | Encoding::EucJp | Encoding::Ascii | Encoding::Iso2022Jp => &[],
        }
    }
}

fn decode_utf16_line(bytes: &[u8], le: bool) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(if le {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        });
    }
    String::from_utf16_lossy(&units)
}

fn encode_utf16_text(text: &str, le: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&if le {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    out
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
/// A BOM is authoritative. Otherwise we sniff a bounded prefix: valid UTF-8
/// (which includes pure ASCII) wins and is reported as UTF-8; failing that we
/// let `chardetng` choose between the Japanese legacy candidates. An explicit
/// `override_enc` short-circuits everything except BOM length.
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
    // Both of these encodings are valid UTF-8 byte-wise (NUL and ESC are legal
    // UTF-8), so they must be recognized BEFORE the `from_utf8` shortcut or
    // ASCII-heavy UTF-16 and 7-bit JIS text short-circuit to "UTF-8" and
    // render as garbage (#196).
    if let Some(wide) = detect_bomless_utf16(prefix) {
        return (wide, 0);
    }
    if looks_iso_2022_jp(prefix) {
        return (Encoding::Iso2022Jp, 0);
    }
    // Pure ASCII is a subset of UTF-8; report it as UTF-8 (the default users
    // expect) rather than a distinct "ASCII" label. The two are byte-identical
    // for ASCII content, so nothing about saving changes.
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
    } else if guess == encoding_rs::ISO_2022_JP {
        Encoding::Iso2022Jp
    } else if guess == encoding_rs::UTF_16LE {
        Encoding::Utf16Le
    } else if guess == encoding_rs::UTF_16BE {
        Encoding::Utf16Be
    } else {
        // chardetng may suggest western single-byte encodings; for a byte-oriented
        // editor UTF-8 (lossy) is the safest universal fallback.
        Encoding::Utf8
    };
    (enc, 0)
}

/// Sniff BOM-less UTF-16 by NUL-byte parity. ASCII-heavy UTF-16 encodes each
/// character as `byte,0x00` (LE) or `0x00,byte` (BE), putting NULs on one
/// parity almost exclusively — while genuine text in any byte encoding
/// contains no NULs at all, and binary blobs scatter NULs across both
/// parities. Requires strong dominance so it can never fire on either.
/// (CJK-heavy BOM-less UTF-16 has few NULs and stays undetected — the
/// reported failure mode is ASCII-heavy logs/exports.)
fn detect_bomless_utf16(prefix: &[u8]) -> Option<Encoding> {
    if prefix.len() < 16 {
        return None;
    }
    let mut even_nul = 0usize;
    let mut odd_nul = 0usize;
    for (i, &b) in prefix.iter().enumerate() {
        if b == 0 {
            if i % 2 == 0 {
                even_nul += 1;
            } else {
                odd_nul += 1;
            }
        }
    }
    let units = prefix.len() / 2;
    let dominant = (units * 3) / 10; // ≥30% of code units carry a NUL half
    let noise = units / 20; // …while the other parity stays ≤5%
    if odd_nul >= dominant && even_nul <= noise {
        return Some(Encoding::Utf16Le);
    }
    if even_nul >= dominant && odd_nul <= noise {
        return Some(Encoding::Utf16Be);
    }
    None
}

/// Sniff ISO-2022-JP by its JIS X 0208 opening escapes (`ESC $ B` / `ESC $ @`).
/// Deliberately NOT keyed on the ASCII-return sequences (`ESC ( B` …): those
/// double as ordinary charset designations in terminal logs, and misdetecting
/// a colored log as JIS would mangle every CSI sequence in it.
fn looks_iso_2022_jp(prefix: &[u8]) -> bool {
    let mut at = 0usize;
    while let Some(rel) = memchr::memchr(0x1B, &prefix[at..]) {
        let i = at + rel;
        match prefix.get(i + 1..i + 3) {
            Some(seq) if seq == b"$B" || seq == b"$@" => return true,
            _ => at = i + 1,
        }
    }
    false
}

/// Byte offset and byte length of the decoded-character span
/// `[char_start, char_start + char_len)` inside one raw ISO-2022-JP line.
/// Walks the designation escapes the way the decoder segments well-formed
/// text: escapes consume bytes but produce no characters, JIS X 0208 runs are
/// two bytes per character, ASCII/kana runs one. Search and caret mapping use
/// this because a re-encode round trip cannot recover mid-run byte offsets in
/// a stateful encoding.
pub(crate) fn iso2022jp_char_span(
    raw: &[u8],
    char_start: usize,
    char_len: usize,
) -> (usize, usize) {
    let start = iso2022jp_col_offset(raw, char_start as u64).unwrap_or(raw.len());
    let end = iso2022jp_col_offset(raw, (char_start + char_len) as u64).unwrap_or(raw.len());
    (start, end.saturating_sub(start))
}

/// Byte offset where decoded character column `col` starts in one raw
/// ISO-2022-JP line (clamped to the line end like the other legacy walkers).
pub(crate) fn iso2022jp_col_offset(raw: &[u8], col: u64) -> Option<usize> {
    #[derive(PartialEq)]
    enum Mode {
        SingleByte, // ASCII / JIS-Roman / halfwidth kana: one byte per char
        DoubleByte, // JIS X 0208 / 0212: two bytes per char
    }
    let mut mode = Mode::SingleByte;
    let mut off = 0usize;
    let mut produced = 0u64;
    while off < raw.len() {
        if produced >= col {
            return Some(off);
        }
        if raw[off] == 0x1B {
            match raw.get(off + 1..off + 3) {
                Some(seq) if seq == b"$B" || seq == b"$@" => {
                    mode = Mode::DoubleByte;
                    off += 3;
                    continue;
                }
                Some(seq) if seq == b"(B" || seq == b"(J" || seq == b"(I" => {
                    mode = Mode::SingleByte;
                    off += 3;
                    continue;
                }
                // ESC $ ( D — JIS X 0212 (4-byte designation).
                Some(seq) if seq == b"$(" && raw.get(off + 3) == Some(&b'D') => {
                    mode = Mode::DoubleByte;
                    off += 4;
                    continue;
                }
                // Malformed escape: the decoder emits a replacement character.
                _ => {
                    produced += 1;
                    off += 1;
                    continue;
                }
            }
        }
        produced += 1;
        off += if mode == Mode::DoubleByte && off + 1 < raw.len() {
            2
        } else {
            1
        };
    }
    (produced >= col).then_some(off.min(raw.len()))
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

    /// Preferred line terminator for newly inserted lines. Mixed or no-EOL
    /// files use LF for new text while untouched lines keep their original bytes.
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Eol::Crlf => b"\r\n",
            Eol::Cr => b"\r",
            Eol::Lf | Eol::Mixed | Eol::None => b"\n",
        }
    }

    /// Parse a user-supplied line-ending name (API / UI). Only the three
    /// concrete styles are selectable for a converting save.
    pub fn parse(name: &str) -> Option<Eol> {
        match name
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "")
            .as_str()
        {
            "lf" | "n" | "unix" => Some(Eol::Lf),
            "crlf" | "rn" | "dos" | "windows" => Some(Eol::Crlf),
            "cr" | "r" | "mac" => Some(Eol::Cr),
            _ => None,
        }
    }
}

/// Detect the dominant line ending from a bounded prefix of `content`.
pub fn detect_eol(content: &[u8]) -> Eol {
    detect_eol_for(content, Encoding::Utf8)
}

pub fn detect_eol_for(content: &[u8], enc: Encoding) -> Eol {
    if matches!(enc, Encoding::Utf16Le | Encoding::Utf16Be) {
        return detect_utf16_eol(content, enc == Encoding::Utf16Le);
    }
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

fn detect_utf16_eol(content: &[u8], le: bool) -> Eol {
    const SNIFF: usize = 64 * 1024;
    let p = &content[..content.len().min(SNIFF)];
    let mut crlf = 0u32;
    let mut lf = 0u32;
    let mut cr = 0u32;
    let mut prev_cr = false;
    for chunk in p.chunks_exact(2) {
        let unit = if le {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        };
        match unit {
            0x000D => {
                if prev_cr {
                    cr += 1;
                }
                prev_cr = true;
            }
            0x000A => {
                if prev_cr {
                    crlf += 1;
                    prev_cr = false;
                } else {
                    lf += 1;
                }
            }
            _ => {
                if prev_cr {
                    cr += 1;
                    prev_cr = false;
                }
            }
        }
    }
    if prev_cr {
        cr += 1;
    }
    let styles = (crlf > 0) as u8 + (lf > 0) as u8 + (cr > 0) as u8;
    match (styles, crlf, lf, cr) {
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
        // Pure ASCII is reported as UTF-8 (a superset), not a distinct label.
        assert_eq!(detect(b"hello world\n", None), (Encoding::Utf8, 0));
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
    fn decode_line_capped_cuts_on_a_utf8_char_boundary() {
        let s = "あ".repeat(10); // 30 bytes, 3 per char
        let (text, truncated) = Encoding::Utf8.decode_line_capped(s.as_bytes(), 10);
        assert!(truncated);
        assert_eq!(text, "あああ", "the cut backs off a split character");
        let (full, truncated) = Encoding::Utf8.decode_line_capped(s.as_bytes(), 30);
        assert!(!truncated);
        assert_eq!(full, s);
    }

    #[test]
    fn count_chars_matches_a_full_decode() {
        let utf8 = "abcあいうえおxyz日本語";
        assert_eq!(
            Encoding::Utf8.count_chars(utf8.as_bytes()),
            utf8.chars().count() as u64
        );
        // Well past the streaming buffer, in a legacy encoding.
        let big = format!("{}終", "字".repeat(20_000));
        let (sjis, _, err) = encoding_rs::SHIFT_JIS.encode(&big);
        assert!(!err);
        assert_eq!(
            Encoding::ShiftJis.count_chars(&sjis),
            Encoding::ShiftJis.decode_line(&sjis).chars().count() as u64
        );
    }

    #[test]
    fn detects_bomless_utf16_by_nul_parity() {
        let text = "hello utf-16 world without any byte order mark here\n".repeat(4);
        let le: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(detect(&le, None), (Encoding::Utf16Le, 0));
        let be: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(detect(&be, None), (Encoding::Utf16Be, 0));
        // An all-zero blob has NULs on both parities: not UTF-16.
        assert_eq!(detect(&[0u8; 64], None).0, Encoding::Utf8);
    }

    #[test]
    fn detects_iso_2022_jp_but_not_terminal_escapes() {
        let (jis, _, err) = encoding_rs::ISO_2022_JP.encode("日本語のテスト\nplain ascii line\n");
        assert!(!err);
        assert_eq!(detect(&jis, None), (Encoding::Iso2022Jp, 0));
        let first = &jis[..jis.iter().position(|&b| b == b'\n').unwrap()];
        assert_eq!(Encoding::Iso2022Jp.decode_line(first), "日本語のテスト");
        // Colored terminal logs use ESC[ (CSI) and the ESC(B designation;
        // neither may flip a log file into ISO-2022-JP.
        let log = b"\x1b[31merror\x1b[0m done \x1b(B still ascii\n";
        assert_eq!(detect(log, None), (Encoding::Utf8, 0));
    }

    #[test]
    fn iso2022jp_col_offset_walks_designation_escapes() {
        // A(0) B(1) ESC$B(3 bytes) 日(2) 本(2) ESC(B(3 bytes) C — decoded "AB日本C".
        let (bytes, _, err) = encoding_rs::ISO_2022_JP.encode("AB日本C");
        assert!(!err);
        assert_eq!(Encoding::Iso2022Jp.decode_line(&bytes), "AB日本C");
        assert_eq!(iso2022jp_col_offset(&bytes, 0), Some(0));
        assert_eq!(iso2022jp_col_offset(&bytes, 1), Some(1));
        assert_eq!(iso2022jp_col_offset(&bytes, 2), Some(2)); // before the opening escape
        assert_eq!(iso2022jp_col_offset(&bytes, 4), Some(9)); // before the closing escape
        assert_eq!(iso2022jp_col_offset(&bytes, 5), Some(bytes.len()));
        assert_eq!(iso2022jp_col_offset(&bytes, 6), None);
        // The span of "日本" (chars 2..4) decodes back to exactly that text.
        let (off, len) = iso2022jp_char_span(&bytes, 2, 2);
        assert_eq!(
            Encoding::Iso2022Jp.decode_line(&bytes[off..off + len]),
            "日本"
        );
    }

    #[test]
    fn labels_round_trip_through_cli_parse() {
        for enc in [
            Encoding::Utf8,
            Encoding::ShiftJis,
            Encoding::EucJp,
            Encoding::Ascii,
        ] {
            assert_eq!(Encoding::parse(enc.label()), Some(enc));
        }
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
