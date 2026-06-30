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
}

impl Matcher {
    fn build(opts: &SearchOptions, enc: Encoding) -> Result<Matcher> {
        if opts.query.is_empty() {
            return Err(Error::Search("empty query".into()));
        }
        if !opts.regex && opts.case_sensitive {
            // Fast path: encode the needle into the file's encoding and scan bytes.
            let needle = enc
                .encode_query(&opts.query)
                .ok_or_else(|| Error::Search("query not representable in file encoding".into()))?;
            Ok(Matcher::Literal(Box::new(
                memmem::Finder::new(&needle).into_owned(),
            )))
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
            for pos in finder.find_iter(hay) {
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
            for pos in finder.find_iter(hay) {
                let abs = base as usize + pos;
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
}
