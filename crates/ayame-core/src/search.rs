//! Streaming search over the memory-mapped buffer.
//!
//! Literal queries take a SIMD `memmem` fast path; everything case-insensitive
//! or regex-y is compiled to a byte regex. Either way we scan the mmap directly
//! and translate match offsets back into `(line, column)` via the sparse index,
//! so peak memory stays at "a handful of hits", not "the file".

use crate::encoding::Encoding;
use crate::index::LineIndex;
use crate::{Error, Result};
use memchr::memmem;
use serde::{Deserialize, Serialize};

/// What and how to search for.
#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub query: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Byte offset to begin scanning from (whole-buffer coordinates).
    pub start_byte: u64,
    /// Upper bound on hits returned in one call.
    pub max_hits: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            query: String::new(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            start_byte: 0,
            max_hits: 1000,
        }
    }
}

/// One match: location in both byte and (line, column) space.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SearchHit {
    pub line: u64,
    /// 0-based character column (decoded), not byte column.
    pub column: u64,
    pub byte: u64,
    pub byte_len: u64,
}

/// Result of a bounded search pass.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// True if `max_hits` was reached and more matches may exist past the last hit.
    pub truncated: bool,
}

/// Options for interactive next/previous search.
#[derive(Clone, Debug)]
pub struct FindOptions {
    pub query: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Anchor byte: next searches at/after it, previous searches strictly before it.
    pub byte: u64,
}

enum Matcher {
    Literal(Box<memmem::Finder<'static>>),
    Regex(regex::bytes::Regex),
    /// Legacy multi-byte encodings (Shift_JIS / EUC-JP) with a case-insensitive
    /// or regex query: decode each line to UTF-8 and match on the text.
    DecodeLine(regex::Regex),
}

impl Matcher {
    fn build(opts: &SearchOptions, enc: Encoding) -> Result<Matcher> {
        if opts.query.is_empty() {
            return Err(Error::Search("empty query".into()));
        }
        if !opts.regex && opts.case_sensitive {
            // Fast path: encode the needle into the file's encoding and scan bytes.
            // For legacy multi-byte encodings the call sites boundary-validate
            // every raw hit so a match can't start inside a double-byte
            // character (the classic 0x5C trail-byte problem).
            let needle = enc
                .encode_query(&opts.query)
                .ok_or_else(|| Error::Search("query not representable in file encoding".into()))?;
            Ok(Matcher::Literal(Box::new(
                memmem::Finder::new(&needle).into_owned(),
            )))
        } else if is_legacy_multibyte(enc) {
            // A byte regex assumes a UTF-8 haystack, so applying it to raw
            // Shift_JIS / EUC-JP bytes silently misses Japanese text. Decode
            // line-by-line and match against the decoded text instead.
            // (Patterns spanning a line boundary will not match on this path.)
            let pat = if opts.regex {
                opts.query.clone()
            } else {
                regex::escape(&opts.query)
            };
            let re = regex::RegexBuilder::new(&pat)
                .case_insensitive(!opts.case_sensitive)
                .build()
                .map_err(|e| Error::Search(format!("invalid regex: {e}")))?;
            Ok(Matcher::DecodeLine(re))
        } else {
            // Case-insensitive or regex -> compile a byte regex. A non-regex query
            // is escaped so it stays literal but gains the `i` flag.
            let pat = if opts.regex {
                opts.query.clone()
            } else {
                regex::escape(&opts.query)
            };
            let re = regex::bytes::RegexBuilder::new(&pat)
                .case_insensitive(!opts.case_sensitive)
                .build()
                .map_err(|e| Error::Search(format!("invalid regex: {e}")))?;
            Ok(Matcher::Regex(re))
        }
    }
}

/// Scan forward from `opts.start_byte`, returning up to `max_hits` matches.
pub fn search(
    buf: &[u8],
    base: u64,
    len: u64,
    index: &LineIndex,
    enc: Encoding,
    opts: &SearchOptions,
) -> Result<SearchResult> {
    let matcher = Matcher::build(opts, enc)?;
    let start = opts.start_byte.clamp(base, len) as usize;
    let hay = &buf[start..len as usize];
    let mut hits = Vec::new();
    let mut truncated = false;

    let push = |abs: usize, mlen: usize, hits: &mut Vec<SearchHit>| {
        let byte = abs as u64;
        let line = index.line_of_byte(buf, byte);
        let (ls, _le) = index.line_range(buf, line).unwrap_or((byte, byte));
        let column = enc
            .decode_line(&buf[ls as usize..byte as usize])
            .chars()
            .count() as u64;
        hits.push(SearchHit {
            line,
            column,
            byte,
            byte_len: mlen as u64,
        });
    };

    match matcher {
        Matcher::Literal(finder) => {
            let check_boundary = is_legacy_multibyte(enc);
            for pos in finder.find_iter(hay) {
                if check_boundary && !is_legacy_char_boundary(enc, buf, index, start + pos) {
                    continue;
                }
                if opts.whole_word && !is_whole_word_match(buf, start + pos, finder.needle().len())
                {
                    continue;
                }
                if hits.len() >= opts.max_hits {
                    truncated = true;
                    break;
                }
                push(start + pos, finder.needle().len(), &mut hits);
            }
        }
        Matcher::Regex(re) => {
            for m in re.find_iter(hay) {
                // Skip zero-width matches so we always make progress.
                if m.end() == m.start() {
                    continue;
                }
                if opts.whole_word
                    && !is_whole_word_match(buf, start + m.start(), m.end() - m.start())
                {
                    continue;
                }
                if hits.len() >= opts.max_hits {
                    truncated = true;
                    break;
                }
                push(start + m.start(), m.end() - m.start(), &mut hits);
            }
        }
        Matcher::DecodeLine(re) => {
            let from_line = index.line_of_byte(buf, start as u64);
            scan_decoded_lines(
                buf,
                index,
                enc,
                &re,
                opts.whole_word,
                from_line,
                |line, column, byte, byte_len| {
                    if byte < start as u64 {
                        // Honor `start_byte` inside the first scanned line.
                        return true;
                    }
                    if hits.len() >= opts.max_hits {
                        truncated = true;
                        return false;
                    }
                    hits.push(SearchHit {
                        line,
                        column,
                        byte,
                        byte_len,
                    });
                    true
                },
            );
        }
    }

    Ok(SearchResult { hits, truncated })
}

/// First match at or after `from_byte`.
pub fn find_next(
    buf: &[u8],
    base: u64,
    len: u64,
    index: &LineIndex,
    enc: Encoding,
    opts: &FindOptions,
) -> Result<Option<SearchHit>> {
    let search_opts = SearchOptions {
        query: opts.query.clone(),
        regex: opts.regex,
        case_sensitive: opts.case_sensitive,
        whole_word: opts.whole_word,
        start_byte: opts.byte,
        max_hits: 1,
    };
    Ok(search(buf, base, len, index, enc, &search_opts)?
        .hits
        .into_iter()
        .next())
}

/// Last match strictly before `before_byte`.
///
/// Implemented as a forward scan over `[base, before_byte)` keeping the final
/// hit, so its cost is O(before_byte - base) — i.e. worst-case O(file) when the
/// cursor is near EOF. This is acceptable for interactive "previous" in v1; a
/// chunk-backward scan (fixed windows from `before_byte` downward, stopping at
/// the first window that contains a hit) is the planned improvement to make
/// reverse search genuinely viewport-bounded.
pub fn find_prev(
    buf: &[u8],
    base: u64,
    index: &LineIndex,
    enc: Encoding,
    opts: &FindOptions,
) -> Result<Option<SearchHit>> {
    let search_opts = SearchOptions {
        query: opts.query.clone(),
        regex: opts.regex,
        case_sensitive: opts.case_sensitive,
        whole_word: opts.whole_word,
        start_byte: base,
        max_hits: usize::MAX,
    };
    let matcher = Matcher::build(&search_opts, enc)?;
    let before_byte = opts.byte.clamp(base, buf.len() as u64);
    let hay = &buf[base as usize..before_byte as usize];
    let mut last: Option<(usize, usize)> = None;
    match matcher {
        Matcher::Literal(finder) => {
            let check_boundary = is_legacy_multibyte(enc);
            for pos in finder.find_iter(hay) {
                let abs = base as usize + pos;
                if check_boundary && !is_legacy_char_boundary(enc, buf, index, abs) {
                    continue;
                }
                if opts.whole_word && !is_whole_word_match(buf, abs, finder.needle().len()) {
                    continue;
                }
                last = Some((abs, finder.needle().len()));
            }
        }
        Matcher::Regex(re) => {
            for m in re.find_iter(hay) {
                if m.end() != m.start() {
                    let abs = base as usize + m.start();
                    let len = m.end() - m.start();
                    if opts.whole_word && !is_whole_word_match(buf, abs, len) {
                        continue;
                    }
                    last = Some((abs, len));
                }
            }
        }
        Matcher::DecodeLine(re) => {
            let mut best: Option<SearchHit> = None;
            let end_line = index.line_of_byte(buf, before_byte);
            scan_decoded_lines(
                buf,
                index,
                enc,
                &re,
                opts.whole_word,
                0,
                |line, column, byte, byte_len| {
                    if line > end_line {
                        return false;
                    }
                    // Mirror the byte path: the match must lie entirely in
                    // `[base, before_byte)`.
                    if byte >= base && byte + byte_len <= before_byte {
                        best = Some(SearchHit {
                            line,
                            column,
                            byte,
                            byte_len,
                        });
                    }
                    true
                },
            );
            return Ok(best);
        }
    }
    Ok(last.map(|(abs, mlen)| {
        let byte = abs as u64;
        let line = index.line_of_byte(buf, byte);
        let (ls, _le) = index.line_range(buf, line).unwrap_or((byte, byte));
        let column = enc
            .decode_line(&buf[ls as usize..byte as usize])
            .chars()
            .count() as u64;
        SearchHit {
            line,
            column,
            byte,
            byte_len: mlen as u64,
        }
    }))
}

#[inline]
fn is_whole_word_match(buf: &[u8], start: usize, len: usize) -> bool {
    let before = start.checked_sub(1).and_then(|i| buf.get(i).copied());
    let after = buf.get(start.saturating_add(len)).copied();
    !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
}

#[inline]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whole-word check on decoded text (mirrors [`is_word_byte`]'s definition).
#[inline]
fn is_whole_word_text(text: &str, start: usize, end: usize) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    !text[..start].chars().next_back().is_some_and(word)
        && !text[end..].chars().next().is_some_and(word)
}

/// Encodings whose double-byte characters have trail bytes overlapping ASCII,
/// so raw byte matches must be validated against character boundaries.
#[inline]
fn is_legacy_multibyte(enc: Encoding) -> bool {
    matches!(enc, Encoding::ShiftJis | Encoding::EucJp)
}

/// Bytes consumed by the character starting at `raw[i]` under a legacy
/// multi-byte encoding: a lead byte followed by a valid trail forms one 2-
/// (or, for EUC-JP 0x8F, 3-) byte character; anything else is a single byte.
/// This mirrors how the decoder segments well-formed text.
fn legacy_step(enc: Encoding, raw: &[u8], i: usize) -> usize {
    let b = raw[i];
    match enc {
        Encoding::ShiftJis => {
            if matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
                && matches!(raw.get(i + 1).copied(), Some(0x40..=0x7E | 0x80..=0xFC))
            {
                2
            } else {
                1
            }
        }
        Encoding::EucJp => match b {
            0x8E if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xDF)) => 2,
            0x8F if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xFE))
                && matches!(raw.get(i + 2).copied(), Some(0xA1..=0xFE)) =>
            {
                3
            }
            0xA1..=0xFE if matches!(raw.get(i + 1).copied(), Some(0xA1..=0xFE)) => 2,
            _ => 1,
        },
        _ => 1,
    }
}

/// True if `abs` lies on a character boundary of the legacy-encoded line
/// containing it. Walks the lead/trail structure from the line start (line
/// starts are always boundaries: `0x0A` is never a trail byte in these
/// encodings), so a raw `memmem` hit landing on the *trail* byte of a
/// double-byte character (e.g. the `0x5C` in "ソ") is rejected.
fn is_legacy_char_boundary(enc: Encoding, buf: &[u8], index: &LineIndex, abs: usize) -> bool {
    let line = index.line_of_byte(buf, abs as u64);
    let Some((ls, _le)) = index.line_range(buf, line) else {
        return true;
    };
    let mut p = ls as usize;
    while p < abs {
        p += legacy_step(enc, buf, p);
    }
    p == abs
}

/// Byte offset and length inside `raw` (one legacy-encoded line) of the span
/// starting at decoded char `char_start` and spanning `char_len` chars.
fn legacy_char_span(
    enc: Encoding,
    raw: &[u8],
    char_start: usize,
    char_len: usize,
) -> (usize, usize) {
    let mut off = 0usize;
    for _ in 0..char_start {
        if off >= raw.len() {
            break;
        }
        off += legacy_step(enc, raw, off);
    }
    let mut end = off;
    for _ in 0..char_len {
        if end >= raw.len() {
            break;
        }
        end += legacy_step(enc, raw, end);
    }
    (off, end - off)
}

/// Scan decoded lines from `from_line` on, invoking `on_hit` with
/// `(line, column, byte, byte_len)` for every match. `on_hit` returns `false`
/// to stop the scan. Lines are fetched in batches so the index walks the
/// buffer sequentially instead of re-resolving every line from a checkpoint.
fn scan_decoded_lines<F>(
    buf: &[u8],
    index: &LineIndex,
    enc: Encoding,
    re: &regex::Regex,
    whole_word: bool,
    from_line: u64,
    mut on_hit: F,
) where
    F: FnMut(u64, u64, u64, u64) -> bool,
{
    const LINE_BATCH: u64 = 1024;
    let total = index.line_count();
    let mut line = from_line;
    while line < total {
        let ranges = index.line_ranges(buf, line, LINE_BATCH);
        if ranges.is_empty() {
            return;
        }
        for &(ln, ls, le) in &ranges {
            let raw = &buf[ls as usize..le as usize];
            let text = enc.decode_line(raw);
            for m in re.find_iter(&text) {
                // Skip zero-width matches so we always make progress.
                if m.end() == m.start() {
                    continue;
                }
                if whole_word && !is_whole_word_text(&text, m.start(), m.end()) {
                    continue;
                }
                let cs = text[..m.start()].chars().count();
                let cl = text[m.start()..m.end()].chars().count();
                let (boff, blen) = legacy_char_span(enc, raw, cs, cl);
                if blen == 0 {
                    continue;
                }
                if !on_hit(ln, cs as u64, ls + boff as u64, blen as u64) {
                    return;
                }
            }
        }
        line += ranges.len() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::LineIndex;

    fn fixture() -> (Vec<u8>, LineIndex) {
        let mut buf = Vec::new();
        for i in 0..200u64 {
            buf.extend_from_slice(format!("row {i}: value alpha beta\n").as_bytes());
        }
        let idx = LineIndex::build(&buf, 0, 16);
        (buf, idx)
    }

    #[test]
    fn literal_search_maps_line_and_column() {
        let (buf, idx) = fixture();
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: "alpha".into(),
            max_hits: 1000,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Ascii, &opts).unwrap();
        assert_eq!(res.hits.len(), 200);
        // "row 0: value " is 13 chars, so column 13 on line 0.
        assert_eq!(res.hits[0].line, 0);
        assert_eq!(res.hits[0].column, 13);
    }

    #[test]
    fn case_insensitive_and_regex() {
        let (buf, idx) = fixture();
        let len = buf.len() as u64;
        let ci = SearchOptions {
            query: "ALPHA".into(),
            case_sensitive: false,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Ascii, &ci).unwrap();
        assert_eq!(res.hits.len(), 10);
        assert!(res.truncated);

        let rx = SearchOptions {
            query: r"row \d+:".into(),
            regex: true,
            max_hits: 1000,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Ascii, &rx).unwrap();
        assert_eq!(res.hits.len(), 200);
    }

    #[test]
    fn next_and_prev() {
        let (buf, idx) = fixture();
        let len = buf.len() as u64;
        let first = find_next(
            &buf,
            0,
            len,
            &idx,
            Encoding::Ascii,
            &FindOptions {
                query: "beta".into(),
                regex: false,
                case_sensitive: true,
                whole_word: false,
                byte: 0,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.line, 0);
        let next = find_next(
            &buf,
            0,
            len,
            &idx,
            Encoding::Ascii,
            &FindOptions {
                query: "beta".into(),
                regex: false,
                case_sensitive: true,
                whole_word: false,
                byte: first.byte + 1,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(next.line, 1);
        let prev = find_prev(
            &buf,
            0,
            &idx,
            Encoding::Ascii,
            &FindOptions {
                query: "beta".into(),
                regex: false,
                case_sensitive: true,
                whole_word: false,
                byte: next.byte,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(prev.line, 0);
    }

    #[test]
    fn whole_word_filters_substrings() {
        let buf = b"error\nterror\nerror_code\nerror!\n".to_vec();
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: "error".into(),
            whole_word: true,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Ascii, &opts).unwrap();
        assert_eq!(
            res.hits.iter().map(|h| h.line).collect::<Vec<_>>(),
            vec![0, 3]
        );
    }

    fn sjis(text: &str) -> Vec<u8> {
        let (cow, _, err) = encoding_rs::SHIFT_JIS.encode(text);
        assert!(!err, "fixture text must be representable in Shift_JIS");
        cow.into_owned()
    }

    #[test]
    fn shift_jis_literal_rejects_mid_character_match() {
        // "ソ" is 0x83 0x5C in Shift_JIS; its trail byte is ASCII '\'. A raw
        // byte scan for "\" must not report a hit inside the character.
        let mut buf = sjis("ソースコード\n");
        buf.extend_from_slice(b"path\\file\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: "\\".into(),
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::ShiftJis, &opts).unwrap();
        assert_eq!(res.hits.len(), 1, "only the real backslash matches");
        assert_eq!(res.hits[0].line, 1);
        assert_eq!(res.hits[0].column, 4);
        assert_eq!(res.hits[0].byte, sjis("ソースコード\npath").len() as u64);
        assert_eq!(res.hits[0].byte_len, 1);

        // find_prev applies the same boundary validation.
        let prev = find_prev(
            &buf,
            0,
            &idx,
            Encoding::ShiftJis,
            &FindOptions {
                query: "\\".into(),
                regex: false,
                case_sensitive: true,
                whole_word: false,
                byte: len,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(prev.line, 1);
    }

    #[test]
    fn shift_jis_case_insensitive_finds_japanese_text() {
        let buf = sjis("first line\n検索テスト abc\nlast\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: "テスト".into(),
            case_sensitive: false,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::ShiftJis, &opts).unwrap();
        assert_eq!(res.hits.len(), 1);
        let hit = res.hits[0];
        assert_eq!(hit.line, 1);
        assert_eq!(hit.column, 2); // decoded chars before the match: 検索
        assert_eq!(hit.byte, ("first line\n".len() + sjis("検索").len()) as u64);
        assert_eq!(hit.byte_len, sjis("テスト").len() as u64);

        // ASCII case folding still applies on the decoded path.
        let ci = SearchOptions {
            query: "ABC".into(),
            case_sensitive: false,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::ShiftJis, &ci).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 1);
    }

    #[test]
    fn shift_jis_regex_matches_decoded_text() {
        let buf = sjis("あいう\nエラー: 42\nエラー: xyz\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: r"エラー: \d+".into(),
            regex: true,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::ShiftJis, &opts).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 1);
        assert_eq!(res.hits[0].column, 0);
        assert_eq!(res.hits[0].byte, sjis("あいう\n").len() as u64);
        assert_eq!(res.hits[0].byte_len, sjis("エラー: 42").len() as u64);
    }

    #[test]
    fn shift_jis_find_next_and_prev_step_between_decoded_hits() {
        let buf = sjis("テスト\nほかの行\nテスト再び\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let q = |byte: u64| FindOptions {
            query: "テスト".into(),
            regex: false,
            case_sensitive: false,
            whole_word: false,
            byte,
        };
        let first = find_next(&buf, 0, len, &idx, Encoding::ShiftJis, &q(0))
            .unwrap()
            .unwrap();
        assert_eq!(first.line, 0);
        let second = find_next(&buf, 0, len, &idx, Encoding::ShiftJis, &q(first.byte + 1))
            .unwrap()
            .unwrap();
        assert_eq!(second.line, 2);
        let prev = find_prev(&buf, 0, &idx, Encoding::ShiftJis, &q(second.byte))
            .unwrap()
            .unwrap();
        assert_eq!(prev.line, 0);
    }
}
