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

#[cfg(not(test))]
const FIND_PREV_CHUNK: usize = 64 * 1024 * 1024;
#[cfg(test)]
const FIND_PREV_CHUNK: usize = 32;

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

pub(crate) enum MatchPlan {
    Literal {
        finder: Box<memmem::Finder<'static>>,
        validate_legacy_boundary: bool,
    },
    Regex {
        bytes: regex::bytes::Regex,
        text: regex::Regex,
    },
    /// Legacy multi-byte encodings (Shift_JIS / EUC-JP) with a case-insensitive
    /// or regex query: decode each line to UTF-8 and match on the text.
    DecodeLine(regex::Regex),
}

impl MatchPlan {
    fn build(
        enc: Encoding,
        query: &str,
        regex: bool,
        case_sensitive: bool,
        error: impl Fn(String) -> Error,
    ) -> Result<MatchPlan> {
        if query.is_empty() {
            return Err(error("empty query".into()));
        }
        if !regex && case_sensitive {
            // Fast path: encode the needle into the file's encoding and scan bytes.
            // For legacy multi-byte encodings the call sites boundary-validate
            // every raw hit so a match can't start inside a double-byte
            // character (the classic 0x5C trail-byte problem).
            let needle = enc
                .encode_query(query)
                .ok_or_else(|| error("query not representable in file encoding".into()))?;
            Ok(MatchPlan::Literal {
                finder: Box::new(memmem::Finder::new(&needle).into_owned()),
                validate_legacy_boundary: is_legacy_multibyte(enc),
            })
        } else if is_legacy_multibyte(enc) || enc.is_wide() {
            // A byte regex assumes a UTF-8 haystack, so applying it to raw
            // Shift_JIS / EUC-JP / UTF-16 bytes silently misses the text (a
            // case-insensitive "abc" never matches the interleaved-NUL UTF-16
            // bytes). Decode line-by-line and match against the decoded text.
            // (Patterns spanning a line boundary will not match on this path.)
            let pat = if regex {
                query.to_string()
            } else {
                regex::escape(query)
            };
            let re = regex::RegexBuilder::new(&pat)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|e| error(format!("invalid regex: {e}")))?;
            Ok(MatchPlan::DecodeLine(re))
        } else {
            // Case-insensitive or regex -> compile a byte regex. A non-regex query
            // is escaped so it stays literal but gains the `i` flag.
            let pat = if regex {
                query.to_string()
            } else {
                regex::escape(query)
            };
            let bytes = regex::bytes::RegexBuilder::new(&pat)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|e| error(format!("invalid regex: {e}")))?;
            let text = regex::RegexBuilder::new(&pat)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|e| error(format!("invalid regex: {e}")))?;
            Ok(MatchPlan::Regex { bytes, text })
        }
    }

    pub(crate) fn for_search(opts: &SearchOptions, enc: Encoding) -> Result<MatchPlan> {
        Self::build(
            enc,
            &opts.query,
            opts.regex,
            opts.case_sensitive,
            Error::Search,
        )
    }

    pub(crate) fn for_replace(
        enc: Encoding,
        query: &str,
        regex: bool,
        case_sensitive: bool,
    ) -> Result<MatchPlan> {
        Self::build(enc, query, regex, case_sensitive, Error::InvalidInput)
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
    let matcher = MatchPlan::for_search(opts, enc)?;
    let start = opts.start_byte.clamp(base, len) as usize;
    let hay = &buf[start..len as usize];
    let mut hits = Vec::new();
    let mut truncated = false;

    match matcher {
        MatchPlan::Literal {
            finder,
            validate_legacy_boundary,
        } => {
            for pos in finder.find_iter(hay) {
                if !utf16_hit_aligned(enc, start + pos) {
                    continue;
                }
                if validate_legacy_boundary
                    && !is_legacy_char_boundary(enc, buf, index, start + pos)
                {
                    continue;
                }
                if opts.whole_word
                    && !is_whole_word_match_encoded(
                        buf,
                        index,
                        enc,
                        start + pos,
                        finder.needle().len(),
                    )
                {
                    continue;
                }
                if hits.len() >= opts.max_hits {
                    truncated = true;
                    break;
                }
                hits.push(hit_at(buf, index, enc, start + pos, finder.needle().len()));
            }
        }
        MatchPlan::Regex { bytes: re, .. } => {
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
                hits.push(hit_at(
                    buf,
                    index,
                    enc,
                    start + m.start(),
                    m.end() - m.start(),
                ));
            }
        }
        MatchPlan::DecodeLine(re) => {
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
    let matcher = MatchPlan::for_search(&search_opts, enc)?;
    let before_byte = opts.byte.clamp(base, buf.len() as u64);
    if before_byte <= base {
        return Ok(None);
    }
    match matcher {
        MatchPlan::Literal {
            finder,
            validate_legacy_boundary,
        } => {
            let needle = finder.needle();
            let overlap = needle.len().saturating_sub(1);
            let base = base as usize;
            let mut cursor = before_byte as usize;
            while cursor > base {
                let accept_start = cursor.saturating_sub(FIND_PREV_CHUNK).max(base);
                let scan_end = cursor.saturating_add(overlap).min(before_byte as usize);
                let hay = &buf[accept_start..scan_end];
                for pos in memmem::rfind_iter(hay, needle) {
                    let abs = accept_start + pos;
                    if abs >= cursor {
                        continue;
                    }
                    if !utf16_hit_aligned(enc, abs) {
                        continue;
                    }
                    if validate_legacy_boundary && !is_legacy_char_boundary(enc, buf, index, abs) {
                        continue;
                    }
                    if opts.whole_word
                        && !is_whole_word_match_encoded(buf, index, enc, abs, needle.len())
                    {
                        continue;
                    }
                    return Ok(Some(hit_at(buf, index, enc, abs, needle.len())));
                }
                cursor = accept_start;
            }
            Ok(None)
        }
        MatchPlan::Regex { bytes: re, .. } => {
            // Byte regexes may span arbitrary distances, so keep the exact
            // whole-prefix scan. Literal and decoded-line searches above/below
            // are the latency-sensitive interactive cases.
            let hay = &buf[base as usize..before_byte as usize];
            let mut last: Option<(usize, usize)> = None;
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
            Ok(last.map(|(abs, mlen)| hit_at(buf, index, enc, abs, mlen)))
        }
        MatchPlan::DecodeLine(re) => Ok(find_prev_decoded_line(
            buf,
            index,
            enc,
            &re,
            opts.whole_word,
            base,
            before_byte,
        )),
    }
}

/// Build a [`SearchHit`] for a raw byte match at `abs` spanning `mlen` bytes,
/// resolving its line and decoded character column via the sparse index.
fn hit_at(buf: &[u8], index: &LineIndex, enc: Encoding, abs: usize, mlen: usize) -> SearchHit {
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
}

#[inline]
fn is_whole_word_match(buf: &[u8], start: usize, len: usize) -> bool {
    let before = start.checked_sub(1).and_then(|i| buf.get(i).copied());
    let after = buf.get(start.saturating_add(len)).copied();
    !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
}

fn is_whole_word_match_encoded(
    buf: &[u8],
    index: &LineIndex,
    enc: Encoding,
    start: usize,
    len: usize,
) -> bool {
    if !is_legacy_multibyte(enc) {
        return is_whole_word_match(buf, start, len);
    }
    let before = legacy_neighbor_byte(enc, buf, index, start, false);
    let after = legacy_neighbor_byte(enc, buf, index, start.saturating_add(len), true);
    !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
}

#[inline]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True if one line's raw bytes contain at least one match under `plan` —
/// the same acceptance rules [`search`] applies (legacy boundary validation,
/// whole-word, zero-width skipping), restricted to a single line. Lets the
/// grep-to-file transform extract exactly the lines the search bar would
/// report, without building a whole-buffer index.
pub(crate) fn line_has_match(
    plan: &MatchPlan,
    enc: Encoding,
    raw: &[u8],
    whole_word: bool,
) -> bool {
    match plan {
        MatchPlan::Literal {
            finder,
            validate_legacy_boundary,
        } => {
            for pos in finder.find_iter(raw) {
                if !utf16_hit_aligned(enc, pos) {
                    continue;
                }
                if *validate_legacy_boundary && !is_legacy_char_boundary_in_line(enc, raw, pos) {
                    continue;
                }
                if whole_word
                    && !is_whole_word_match_encoded_in_line(enc, raw, pos, finder.needle().len())
                {
                    continue;
                }
                return true;
            }
            false
        }
        MatchPlan::Regex { bytes, .. } => bytes.find_iter(raw).any(|m| {
            m.end() > m.start()
                && (!whole_word || is_whole_word_match(raw, m.start(), m.end() - m.start()))
        }),
        MatchPlan::DecodeLine(re) => {
            let text = enc.decode_line(raw);
            re.find_iter(&text).any(|m| {
                m.end() > m.start()
                    && (!whole_word || is_whole_word_text(&text, m.start(), m.end()))
            })
        }
    }
}

/// Line-local mirror of [`is_whole_word_match_encoded`]: neighbor bytes are
/// resolved by walking the legacy lead/trail structure from the line start
/// (always a character boundary) instead of through a whole-buffer index.
fn is_whole_word_match_encoded_in_line(
    enc: Encoding,
    raw: &[u8],
    start: usize,
    len: usize,
) -> bool {
    if !is_legacy_multibyte(enc) {
        return is_whole_word_match(raw, start, len);
    }
    let mut p = 0usize;
    let mut prev: Option<usize> = None;
    while p < start && p < raw.len() {
        prev = Some(p);
        p += legacy_step(enc, raw, p);
    }
    let before = (p == start).then(|| prev.map(|i| raw[i])).flatten();
    let end = start.saturating_add(len);
    let mut q = start;
    while q < end && q < raw.len() {
        q += legacy_step(enc, raw, q);
    }
    let after = (q == end).then(|| raw.get(end).copied()).flatten();
    !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
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
pub(crate) fn is_legacy_multibyte(enc: Encoding) -> bool {
    matches!(enc, Encoding::ShiftJis | Encoding::EucJp)
}

/// True when a raw literal hit at absolute offset `abs` starts on a UTF-16
/// code-unit boundary. UTF-16 content begins either at 0 or just after a 2-byte
/// BOM, so every code unit lands on an even absolute offset; a `memmem` hit at
/// an odd offset straddles two characters and must be rejected (issue #68).
#[inline]
fn utf16_hit_aligned(enc: Encoding, abs: usize) -> bool {
    !enc.is_wide() || abs.is_multiple_of(2)
}

/// Byte offset and length inside a decoded line of the span starting at decoded
/// char `char_start` and running `char_len` chars, for UTF-16. Each scalar is
/// one or two 16-bit code units, i.e. 2 or 4 bytes (endianness does not change
/// the byte count), so the raw span is the re-encoded UTF-16 length.
fn utf16_char_span(text: &str, char_start: usize, char_len: usize) -> (usize, usize) {
    let boff = text
        .chars()
        .take(char_start)
        .map(|c| c.len_utf16() * 2)
        .sum();
    let blen = text
        .chars()
        .skip(char_start)
        .take(char_len)
        .map(|c| c.len_utf16() * 2)
        .sum();
    (boff, blen)
}

/// Dispatch the decoded char -> raw byte span mapping by encoding family:
/// UTF-16 re-encodes code units, legacy encodings walk lead/trail bytes.
fn decoded_char_span(
    enc: Encoding,
    raw: &[u8],
    text: &str,
    char_start: usize,
    char_len: usize,
) -> (usize, usize) {
    if enc.is_wide() {
        utf16_char_span(text, char_start, char_len)
    } else {
        legacy_char_span(enc, raw, char_start, char_len)
    }
}

/// Bytes consumed by the character starting at `raw[i]` under a legacy
/// multi-byte encoding: a lead byte followed by a valid trail forms one 2-
/// (or, for EUC-JP 0x8F, 3-) byte character; anything else is a single byte.
/// This mirrors how the decoder segments well-formed text.
fn legacy_step(enc: Encoding, raw: &[u8], i: usize) -> usize {
    let b = raw[i];
    match enc {
        Encoding::ShiftJis
            if matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
                && matches!(raw.get(i + 1).copied(), Some(0x40..=0x7E | 0x80..=0xFC)) =>
        {
            2
        }
        Encoding::ShiftJis => 1,
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

pub(crate) fn is_legacy_char_boundary_in_line(enc: Encoding, raw: &[u8], abs: usize) -> bool {
    let mut p = 0usize;
    while p < abs && p < raw.len() {
        p += legacy_step(enc, raw, p);
    }
    p == abs
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

fn legacy_neighbor_byte(
    enc: Encoding,
    buf: &[u8],
    index: &LineIndex,
    abs: usize,
    after: bool,
) -> Option<u8> {
    let line = index.line_of_byte(buf, abs as u64);
    let (ls, le) = index.line_range(buf, line)?;
    let mut p = ls as usize;
    let mut previous = None;
    let end = le as usize;
    while p < abs && p < end {
        previous = Some(p);
        p += legacy_step(enc, buf, p);
    }
    if p != abs {
        return None;
    }
    if after {
        (p < end).then(|| buf[p])
    } else {
        previous.map(|i| buf[i])
    }
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
                let (boff, blen) = decoded_char_span(enc, raw, &text, cs, cl);
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

fn find_prev_decoded_line(
    buf: &[u8],
    index: &LineIndex,
    enc: Encoding,
    re: &regex::Regex,
    whole_word: bool,
    base: u64,
    before_byte: u64,
) -> Option<SearchHit> {
    const LINE_BATCH: u64 = 1024;
    let total = index.line_count();
    if total == 0 {
        return None;
    }
    let mut line = index.line_of_byte(buf, before_byte).min(total - 1);
    loop {
        let batch_start = line.saturating_sub(LINE_BATCH - 1);
        let ranges = index.line_ranges(buf, batch_start, line - batch_start + 1);
        for &(ln, ls, le) in ranges.iter().rev() {
            let raw = &buf[ls as usize..le as usize];
            let text = enc.decode_line(raw);
            let matches: Vec<_> = re.find_iter(&text).collect();
            for m in matches.into_iter().rev() {
                if m.end() == m.start() {
                    continue;
                }
                if whole_word && !is_whole_word_text(&text, m.start(), m.end()) {
                    continue;
                }
                let cs = text[..m.start()].chars().count();
                let cl = text[m.start()..m.end()].chars().count();
                let (boff, blen) = decoded_char_span(enc, raw, &text, cs, cl);
                if blen == 0 {
                    continue;
                }
                let byte = ls + boff as u64;
                let byte_len = blen as u64;
                if byte >= base && byte + byte_len <= before_byte {
                    return Some(SearchHit {
                        line: ln,
                        column: cs as u64,
                        byte,
                        byte_len,
                    });
                }
            }
        }
        if batch_start == 0 {
            return None;
        }
        line = batch_start - 1;
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
    fn prev_literal_search_walks_backward_by_chunks() {
        let mut buf = b"needle first\n".to_vec();
        for _ in 0..12 {
            buf.extend_from_slice(b"padding padding padding\n");
        }
        let second_start = buf.len() as u64;
        buf.extend_from_slice(b"needle second\n");
        let idx = LineIndex::build(&buf, 0, 4);

        let q = |byte| FindOptions {
            query: "needle".into(),
            regex: false,
            case_sensitive: true,
            whole_word: false,
            byte,
        };

        let last = find_prev(&buf, 0, &idx, Encoding::Ascii, &q(buf.len() as u64))
            .unwrap()
            .unwrap();
        assert_eq!(last.byte, second_start);

        let first = find_prev(&buf, 0, &idx, Encoding::Ascii, &q(last.byte))
            .unwrap()
            .unwrap();
        assert_eq!(first.line, 0);
    }

    #[test]
    fn prev_literal_search_finds_match_crossing_chunk_boundary() {
        let needle = b"ABCDE";
        let mut buf = vec![b'x'; 67];
        let match_start = FIND_PREV_CHUNK - 1;
        buf[match_start..match_start + needle.len()].copy_from_slice(needle);
        let match_start = match_start as u64;
        let idx = LineIndex::build(&buf, 0, 4);

        let hit = find_prev(
            &buf,
            0,
            &idx,
            Encoding::Ascii,
            &FindOptions {
                query: "ABCDE".into(),
                regex: false,
                case_sensitive: true,
                whole_word: false,
                byte: buf.len() as u64,
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(hit.byte, match_start);
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

    fn euc_jp(text: &str) -> Vec<u8> {
        let (cow, _, err) = encoding_rs::EUC_JP.encode(text);
        assert!(!err, "fixture text must be representable in EUC-JP");
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
    fn shift_jis_whole_word_uses_character_not_trail_byte_boundary() {
        // "ア" is 0x83 0x41 in Shift_JIS. The trail byte is ASCII 'A', but it
        // must not make the following ASCII word look like it has a word-byte
        // prefix.
        let buf = sjis("アword\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let res = search(
            &buf,
            0,
            len,
            &idx,
            Encoding::ShiftJis,
            &SearchOptions {
                query: "word".into(),
                whole_word: true,
                max_hits: 10,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].column, 1);
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
    fn euc_jp_literal_and_decoded_search_paths_find_japanese_text() {
        let buf = euc_jp("first line\n検索テスト abc\nエラー: 42\nlast\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;

        let literal = SearchOptions {
            query: "検索".into(),
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::EucJp, &literal).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 1);
        assert_eq!(res.hits[0].byte, "first line\n".len() as u64);
        assert_eq!(res.hits[0].byte_len, euc_jp("検索").len() as u64);

        let ci = SearchOptions {
            query: "ABC".into(),
            case_sensitive: false,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::EucJp, &ci).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 1);

        let rx = SearchOptions {
            query: r"エラー: \d+".into(),
            regex: true,
            max_hits: 10,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::EucJp, &rx).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].line, 2);
        assert_eq!(
            res.hits[0].byte,
            ("first line\n".len() + euc_jp("検索テスト abc\n").len()) as u64
        );
        assert_eq!(res.hits[0].byte_len, euc_jp("エラー: 42").len() as u64);
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

    #[test]
    fn euc_jp_find_prev_decoded_path_walks_backward_by_lines() {
        let buf = euc_jp("テスト\nほかの行\nテスト再び\n");
        let idx = LineIndex::build(&buf, 0, 16);
        let len = buf.len() as u64;
        let q = |byte: u64| FindOptions {
            query: "テスト".into(),
            regex: false,
            case_sensitive: false,
            whole_word: false,
            byte,
        };
        let last = find_prev(&buf, 0, &idx, Encoding::EucJp, &q(len))
            .unwrap()
            .unwrap();
        assert_eq!(last.line, 2);
        let prev = find_prev(&buf, 0, &idx, Encoding::EucJp, &q(last.byte))
            .unwrap()
            .unwrap();
        assert_eq!(prev.line, 0);
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn utf16_literal_search_rejects_misaligned_hits() {
        // Issue #68: searching "A" ([41,00] LE) in "䅂　" (U+4142 U+3000 =
        // bytes 42 41 00 30) used to report a false hit at odd offset 1.
        let buf = utf16le("\u{4142}\u{3000}\n");
        let idx = LineIndex::build_utf16_le(&buf, 0, 16);
        let len = buf.len() as u64;
        let opts = SearchOptions {
            query: "A".into(),
            max_hits: 100,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Utf16Le, &opts).unwrap();
        assert_eq!(res.hits.len(), 0, "misaligned UTF-16 hit must be rejected");

        // A genuinely code-unit-aligned "A" is still found.
        let buf2 = utf16le("A\u{3000}\n");
        let idx2 = LineIndex::build_utf16_le(&buf2, 0, 16);
        let res2 = search(&buf2, 0, buf2.len() as u64, &idx2, Encoding::Utf16Le, &opts).unwrap();
        assert_eq!(res2.hits.len(), 1);
        assert_eq!((res2.hits[0].line, res2.hits[0].column), (0, 0));
    }

    #[test]
    fn utf16_case_insensitive_and_regex_match_via_decode() {
        // Issue #68: a byte regex over interleaved-NUL UTF-16 bytes matched
        // nothing; CI/regex now decode each line first.
        let buf = utf16le("abc\nX12Y\n");
        let idx = LineIndex::build_utf16_le(&buf, 0, 16);
        let len = buf.len() as u64;

        let ci = SearchOptions {
            query: "ABC".into(),
            case_sensitive: false,
            max_hits: 100,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Utf16Le, &ci).unwrap();
        assert_eq!(res.hits.len(), 1);
        let h = res.hits[0];
        assert_eq!((h.line, h.column, h.byte, h.byte_len), (0, 0, 0, 6));

        let rx = SearchOptions {
            query: r"\d+".into(),
            regex: true,
            max_hits: 100,
            ..Default::default()
        };
        let res = search(&buf, 0, len, &idx, Encoding::Utf16Le, &rx).unwrap();
        assert_eq!(res.hits.len(), 1);
        let h = res.hits[0];
        // Line 1 "X12Y": "12" starts at char 1 -> byte 2 within the line,
        // spans 2 chars -> 4 bytes.
        assert_eq!((h.line, h.column, h.byte_len), (1, 1, 4));
    }
}
