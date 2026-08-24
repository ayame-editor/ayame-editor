//! Bounded character/byte inspection and explicit escape parsing (#247).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::Result;
use axum::extract::State;
use axum::Json;
use ayame_core::{Document, EditSession, Encoding};
use east_asian_width::east_asian_width_type;
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_bidi::bidi_class;
use unicode_general_category::get_general_category;
use unicode_script::UnicodeScript;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{bad_request, internal, ApiError, SharedState};

const MAX_INSPECT_LINE_BYTES: usize = 256 * 1024;
const MAX_SELECTION_LINES: u64 = 16;
const MAX_CLUSTERS: usize = 64;
const MAX_SCALARS: usize = 256;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_EXPRESSION_BYTES: usize = 1024;

#[derive(Clone, Copy, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct InspectPoint {
    line: u64,
    col: usize,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct InspectRequest {
    start: InspectPoint,
    end: InspectPoint,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct InspectSummary {
    grapheme_count: usize,
    scalar_count: usize,
    utf8_bytes: usize,
    utf16_units: usize,
    truncated: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ScalarInfo {
    text: String,
    code_point: String,
    name: String,
    general_category: String,
    script: String,
    bidi_class: String,
    east_asian_width: String,
    utf8_hex: String,
    utf16_hex: String,
    diagnostics: Vec<String>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ClusterInfo {
    line: u64,
    col: usize,
    end_col: usize,
    text: String,
    display: String,
    kind: String,
    scalars: Vec<ScalarInfo>,
    cell_width: usize,
    cell_width_cjk: usize,
    utf8_hex: String,
    utf16_hex: String,
    original_byte_offset: Option<u64>,
    raw_hex: Option<String>,
    original_encoding_hex: Option<String>,
    source: String,
    representable: bool,
    diagnostics: Vec<String>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ColorLiteral {
    line: u64,
    start_col: usize,
    end_col: usize,
    literal: String,
    rgb_hex: String,
    alpha: u8,
    format: String,
    prefix: String,
    uppercase: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct InspectResponse {
    encoding: Encoding,
    bom_bytes: u64,
    bom_hex: String,
    summary: InspectSummary,
    clusters: Vec<ClusterInfo>,
    diagnostics: Vec<String>,
    color: Option<ColorLiteral>,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ParseEscapeRequest {
    expression: String,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ParseEscapeResponse {
    text: String,
    code_points: String,
    original_encoding_hex: Option<String>,
    representable: bool,
    diagnostics: Vec<String>,
}

fn ordered_points(a: InspectPoint, b: InspectPoint) -> (InspectPoint, InspectPoint) {
    if (a.line, a.col) <= (b.line, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn utf16_hex(text: &str) -> String {
    text.encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn code_points(text: &str) -> String {
    text.chars()
        .map(|scalar| format!("U+{:04X}", scalar as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn byte_index_at_scalar(text: &str, col: usize) -> Option<usize> {
    if col == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(col)
        .map(|(offset, _)| offset)
        .or_else(|| (text.chars().count() == col).then_some(text.len()))
}

fn clipped_text(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn scalar_diagnostics(scalar: char) -> Vec<String> {
    let value = scalar as u32;
    let mut out = Vec::new();
    match value {
        0x202A..=0x202E | 0x2066..=0x2069 => out.push("bidi-control".into()),
        0x200B => out.push("zero-width-space".into()),
        0x200C => out.push("zero-width-non-joiner".into()),
        0x200D => out.push("zero-width-joiner".into()),
        0x00A0 => out.push("non-breaking-space".into()),
        0x00AD => out.push("soft-hyphen".into()),
        0xFE00..=0xFE0F | 0xE0100..=0xE01EF => out.push("variation-selector".into()),
        0xFFFD => out.push("replacement-character".into()),
        _ => {}
    }
    if scalar.is_control() {
        out.push("control-character".into());
    }
    out
}

fn visible_scalar(scalar: char) -> String {
    match scalar {
        '\0' => "<NUL>".into(),
        '\t' => "<TAB>".into(),
        '\r' => "<CR>".into(),
        '\n' => "<LF>".into(),
        '\u{00A0}' => "<NBSP>".into(),
        '\u{00AD}' => "<SOFT HYPHEN>".into(),
        '\u{200B}' => "<ZWSP>".into(),
        '\u{200C}' => "<ZWNJ>".into(),
        '\u{200D}' => "<ZWJ>".into(),
        '\u{202A}' => "<LRE>".into(),
        '\u{202B}' => "<RLE>".into(),
        '\u{202C}' => "<PDF>".into(),
        '\u{202D}' => "<LRO>".into(),
        '\u{202E}' => "<RLO>".into(),
        '\u{2066}' => "<LRI>".into(),
        '\u{2067}' => "<RLI>".into(),
        '\u{2068}' => "<FSI>".into(),
        '\u{2069}' => "<PDI>".into(),
        c if c.is_control() => format!("<CONTROL U+{:04X}>", c as u32),
        c => c.to_string(),
    }
}

fn scalar_info(scalar: char) -> ScalarInfo {
    let text = scalar.to_string();
    ScalarInfo {
        text: text.clone(),
        code_point: format!("U+{:04X}", scalar as u32),
        name: unicode_names2::name(scalar)
            .map(|name| name.to_string())
            .unwrap_or_default(),
        general_category: format!("{:?}", get_general_category(scalar)),
        script: format!("{:?}", scalar.script()),
        bidi_class: format!("{:?}", bidi_class(scalar)),
        east_asian_width: east_asian_width_type(scalar as u32).into(),
        utf8_hex: hex(text.as_bytes()),
        utf16_hex: utf16_hex(&text),
        diagnostics: scalar_diagnostics(scalar),
    }
}

fn inspect_cluster(
    doc: &Document,
    edits: &EditSession,
    line: u64,
    col: usize,
    text: &str,
    kind: &str,
) -> ClusterInfo {
    let end_col = col + text.chars().count();
    let encoded = doc.encoding().encode_text(text);
    let (origin, edited) = edits.line_origin(doc, line).unwrap_or((None, true));
    let original_range = if !edited {
        origin.and_then(|original| {
            Some((
                doc.line_col_byte(original, col as u64)?,
                doc.line_col_byte(original, end_col as u64)?,
            ))
        })
    } else {
        None
    };
    let raw = original_range.and_then(|(start, end)| doc.raw_byte_range(start, end));
    let mut diagnostics = text
        .chars()
        .flat_map(scalar_diagnostics)
        .collect::<BTreeSet<_>>();
    if encoded.is_none() {
        diagnostics.insert("unrepresentable".into());
    }
    if doc.encoding() != Encoding::Iso2022Jp
        && raw.is_some_and(|bytes| encoded.as_deref() != Some(bytes))
    {
        diagnostics.insert("decode-mismatch".into());
    }
    let display = text
        .chars()
        .map(visible_scalar)
        .collect::<Vec<_>>()
        .join("");
    ClusterInfo {
        line,
        col,
        end_col,
        text: text.into(),
        display,
        kind: kind.into(),
        scalars: text.chars().map(scalar_info).collect(),
        cell_width: text.width(),
        cell_width_cjk: text.width_cjk(),
        utf8_hex: hex(text.as_bytes()),
        utf16_hex: utf16_hex(text),
        original_byte_offset: original_range.map(|range| range.0),
        raw_hex: raw.map(hex),
        original_encoding_hex: encoded.as_deref().map(hex),
        source: if original_range.is_some() {
            "original".into()
        } else {
            "overlay".into()
        },
        representable: encoded.is_some(),
        diagnostics: diagnostics.into_iter().collect(),
    }
}

fn inspect_eol(doc: &Document, edits: &EditSession, line: u64, col: usize) -> ClusterInfo {
    let (origin, edited) = edits.line_origin(doc, line).unwrap_or((None, true));
    let original = origin.filter(|_| !edited);
    let raw = original.and_then(|source| doc.line_terminator(source));
    let offset = original.and_then(|source| doc.line_col_byte(source, col as u64));
    let has_logical_eol = line.saturating_add(1) < edits.total_lines(doc)
        || raw.is_some_and(|bytes| !bytes.is_empty());
    let label = match raw {
        Some(b"\r\n") | Some(b"\r\0\n\0") | Some(b"\0\r\0\n") => "<EOL CRLF>",
        Some(b"\r") | Some(b"\r\0") | Some(b"\0\r") => "<EOL CR>",
        Some(bytes) if !bytes.is_empty() => "<EOL LF>",
        _ => "<END OF FILE>",
    };
    ClusterInfo {
        line,
        col,
        end_col: col,
        text: if has_logical_eol { "\n" } else { "" }.into(),
        display: label.into(),
        kind: "eol".into(),
        scalars: has_logical_eol
            .then(|| scalar_info('\n'))
            .into_iter()
            .collect(),
        cell_width: 0,
        cell_width_cjk: 0,
        utf8_hex: if has_logical_eol { "0A" } else { "" }.into(),
        utf16_hex: if has_logical_eol { "000A" } else { "" }.into(),
        original_byte_offset: offset,
        raw_hex: raw.map(hex),
        original_encoding_hex: raw.map(hex),
        source: if raw.is_some() { "original" } else { "overlay" }.into(),
        representable: true,
        diagnostics: vec!["line-ending".into()],
    }
}

fn push_cluster(
    doc: &Document,
    edits: &EditSession,
    line: u64,
    col: usize,
    text: &str,
    summary: &mut InspectSummary,
    clusters: &mut Vec<ClusterInfo>,
) -> bool {
    let remaining = MAX_TEXT_BYTES.saturating_sub(summary.utf8_bytes);
    let (text, clipped) = clipped_text(text, remaining);
    let scalars = text.chars().count();
    if text.is_empty()
        || clusters.len() >= MAX_CLUSTERS
        || summary.scalar_count.saturating_add(scalars) > MAX_SCALARS
    {
        summary.truncated = true;
        return false;
    }
    clusters.push(inspect_cluster(doc, edits, line, col, text, "grapheme"));
    summary.grapheme_count += 1;
    summary.scalar_count += scalars;
    summary.utf8_bytes += text.len();
    summary.utf16_units += text.encode_utf16().count();
    if clipped {
        summary.truncated = true;
        return false;
    }
    true
}

fn inspect_caret(
    doc: &Document,
    edits: &EditSession,
    point: InspectPoint,
    summary: &mut InspectSummary,
    clusters: &mut Vec<ClusterInfo>,
) -> Result<(), ApiError> {
    if edits.total_lines(doc) == 0 && point.line == 0 {
        clusters.push(inspect_eol(doc, edits, 0, 0));
        summary.grapheme_count = 1;
        return Ok(());
    }
    let line = edits
        .line_capped(doc, point.line, MAX_INSPECT_LINE_BYTES)
        .ok_or_else(|| bad_request("inspection position is outside the document"))?;
    let line_scalars = line.text.chars().count();
    if line.truncated && point.col >= line_scalars {
        summary.truncated = true;
        return Ok(());
    }
    let col = point.col.min(line_scalars);
    if col == line_scalars {
        clusters.push(inspect_eol(doc, edits, point.line, col));
        summary.grapheme_count = 1;
        let text = &clusters[0].text;
        summary.scalar_count = text.chars().count();
        summary.utf8_bytes = text.len();
        summary.utf16_units = text.encode_utf16().count();
        return Ok(());
    }
    let mut scalar_col = 0;
    for (_, grapheme) in line.text.grapheme_indices(true) {
        let end = scalar_col + grapheme.chars().count();
        if col >= scalar_col && col < end {
            push_cluster(
                doc, edits, point.line, scalar_col, grapheme, summary, clusters,
            );
            summary.truncated |= line.truncated;
            return Ok(());
        }
        scalar_col = end;
    }
    Err(bad_request("inspection column could not be resolved"))
}

fn inspect_selection(
    doc: &Document,
    edits: &EditSession,
    start: InspectPoint,
    end: InspectPoint,
    summary: &mut InspectSummary,
    clusters: &mut Vec<ClusterInfo>,
) -> Result<(), ApiError> {
    let last_line = end
        .line
        .min(start.line.saturating_add(MAX_SELECTION_LINES - 1));
    if last_line < end.line {
        summary.truncated = true;
    }
    for line_number in start.line..=last_line {
        let line = edits
            .line_capped(doc, line_number, MAX_INSPECT_LINE_BYTES)
            .ok_or_else(|| bad_request("inspection selection is outside the document"))?;
        let line_scalars = line.text.chars().count();
        let from = if line_number == start.line {
            start.col.min(line_scalars)
        } else {
            0
        };
        let to = if line_number == end.line {
            end.col.min(line_scalars)
        } else {
            line_scalars
        };
        let from_byte = byte_index_at_scalar(&line.text, from)
            .ok_or_else(|| internal("inspection start column could not be resolved"))?;
        let to_byte = byte_index_at_scalar(&line.text, to)
            .ok_or_else(|| internal("inspection end column could not be resolved"))?;
        let mut col = from;
        for grapheme in line.text[from_byte..to_byte].graphemes(true) {
            if !push_cluster(doc, edits, line_number, col, grapheme, summary, clusters) {
                return Ok(());
            }
            col += grapheme.chars().count();
        }
        summary.truncated |= line.truncated;
        if line.truncated && to == line_scalars {
            return Ok(());
        }
        if line_number < end.line {
            if clusters.len() >= MAX_CLUSTERS
                || summary.scalar_count >= MAX_SCALARS
                || summary.utf8_bytes >= MAX_TEXT_BYTES
            {
                summary.truncated = true;
                return Ok(());
            }
            clusters.push(inspect_eol(doc, edits, line_number, line_scalars));
            summary.grapheme_count += 1;
            summary.scalar_count += 1;
            summary.utf8_bytes += 1;
            summary.utf16_units += 1;
        }
    }
    Ok(())
}

fn color_regex() -> &'static Regex {
    static COLOR: OnceLock<Regex> = OnceLock::new();
    COLOR.get_or_init(|| {
        Regex::new(r"(?i)(#[0-9a-f]{8}|#[0-9a-f]{6}|#[0-9a-f]{4}|#[0-9a-f]{3}|0x[0-9a-f]{8}|0x[0-9a-f]{6})")
            .expect("fixed color literal regex")
    })
}

fn parse_color(line: u64, text: &str, caret_col: usize) -> Option<ColorLiteral> {
    for found in color_regex().find_iter(text) {
        if found.end() < text.len() && text.as_bytes()[found.end()].is_ascii_hexdigit() {
            continue;
        }
        let start_col = text[..found.start()].chars().count();
        let end_col = start_col + found.as_str().chars().count();
        if caret_col < start_col || caret_col > end_col {
            continue;
        }
        let literal = found.as_str();
        let digits = literal
            .strip_prefix('#')
            .or_else(|| literal.strip_prefix("0x"))
            .or_else(|| literal.strip_prefix("0X"))
            .unwrap_or(literal);
        let (format, r, g, b, alpha) = match digits.len() {
            3 => (
                "hex3",
                digits[0..1].repeat(2),
                digits[1..2].repeat(2),
                digits[2..3].repeat(2),
                255,
            ),
            4 => (
                "hex4",
                digits[0..1].repeat(2),
                digits[1..2].repeat(2),
                digits[2..3].repeat(2),
                u8::from_str_radix(&digits[3..4].repeat(2), 16).ok()?,
            ),
            6 => (
                if literal.starts_with('#') {
                    "hex6"
                } else {
                    "0x6"
                },
                digits[0..2].into(),
                digits[2..4].into(),
                digits[4..6].into(),
                255,
            ),
            8 => (
                if literal.starts_with('#') {
                    "hex8"
                } else {
                    "0x8"
                },
                digits[0..2].into(),
                digits[2..4].into(),
                digits[4..6].into(),
                u8::from_str_radix(&digits[6..8], 16).ok()?,
            ),
            _ => continue,
        };
        return Some(ColorLiteral {
            line,
            start_col,
            end_col,
            literal: literal.into(),
            rgb_hex: format!("#{r}{g}{b}").to_ascii_lowercase(),
            alpha,
            format: format.into(),
            prefix: if literal.starts_with('#') {
                "#".into()
            } else {
                literal[..2].into()
            },
            uppercase: literal.chars().any(|c| matches!(c, 'A'..='F')),
        });
    }
    None
}

pub(super) async fn api_inspect(
    State(state): State<SharedState>,
    Json(request): Json<InspectRequest>,
) -> Result<Json<InspectResponse>, ApiError> {
    let snapshot = state.read(|workspace| {
        let (doc, edits) = workspace.doc_and_edits()?;
        Ok::<_, ApiError>((doc.clone(), edits.view_clone()))
    })?;
    let (doc, edits) = snapshot;
    let (start, end) = ordered_points(request.start, request.end);
    let response = tokio::task::spawn_blocking(move || {
        let mut summary = InspectSummary {
            grapheme_count: 0,
            scalar_count: 0,
            utf8_bytes: 0,
            utf16_units: 0,
            truncated: false,
        };
        let mut clusters = Vec::new();
        if (start.line, start.col) == (end.line, end.col) {
            inspect_caret(&doc, &edits, start, &mut summary, &mut clusters)?;
        } else {
            inspect_selection(&doc, &edits, start, end, &mut summary, &mut clusters)?;
        }
        let mut diagnostics = clusters
            .iter()
            .flat_map(|cluster| cluster.diagnostics.iter().cloned())
            .collect::<BTreeSet<_>>();
        let scripts = clusters
            .iter()
            .flat_map(|cluster| cluster.scalars.iter().map(|scalar| scalar.script.as_str()))
            .filter(|script| !matches!(*script, "Common" | "Inherited" | "Unknown"))
            .collect::<BTreeSet<_>>();
        if scripts.len() > 1 {
            diagnostics.insert("mixed-script-possible-confusable".into());
        }
        if summary.truncated {
            diagnostics.insert("inspection-truncated".into());
        }
        let color = edits
            .line_capped(&doc, start.line, MAX_INSPECT_LINE_BYTES)
            .and_then(|line| parse_color(start.line, &line.text, start.col));
        let stat = doc.stat();
        let bom = doc.raw_byte_range(0, stat.bom_bytes).unwrap_or_default();
        Ok::<_, ApiError>(InspectResponse {
            encoding: doc.encoding(),
            bom_bytes: stat.bom_bytes,
            bom_hex: hex(bom),
            summary,
            clusters,
            diagnostics: diagnostics.into_iter().collect(),
            color,
        })
    })
    .await
    .map_err(|error| internal(error.to_string()))??;
    Ok(Json(response))
}

fn parse_scalar(value: &str) -> Result<char, ApiError> {
    let scalar = u32::from_str_radix(value, 16).map_err(|_| bad_request("invalid scalar value"))?;
    char::from_u32(scalar).ok_or_else(|| bad_request("surrogates are not Unicode scalar values"))
}

fn parse_u_plus(expression: &str) -> Result<Option<String>, ApiError> {
    let mut out = String::new();
    for part in expression.split_ascii_whitespace() {
        let Some(value) = part.strip_prefix("U+").or_else(|| part.strip_prefix("u+")) else {
            return Ok(None);
        };
        if !(4..=6).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(bad_request("U+ escapes require 4-6 hexadecimal digits"));
        }
        out.push(parse_scalar(value)?);
    }
    Ok((!out.is_empty()).then_some(out))
}

fn parse_braced(expression: &str) -> Result<Option<String>, ApiError> {
    let compact = expression
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    if !compact.starts_with("\\u{") {
        return Ok(None);
    }
    let mut rest = compact.as_str();
    let mut out = String::new();
    while let Some(value) = rest.strip_prefix("\\u{") {
        let Some(close) = value.find('}') else {
            return Err(bad_request("unterminated \\u{...} escape"));
        };
        let digits = &value[..close];
        if digits.is_empty()
            || digits.len() > 6
            || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(bad_request("\\u{...} requires 1-6 hexadecimal digits"));
        }
        out.push(parse_scalar(digits)?);
        rest = &value[close + 1..];
    }
    if !rest.is_empty() {
        return Err(bad_request("only explicit \\u{...} escapes are accepted"));
    }
    Ok(Some(out))
}

fn parse_hex_bytes(
    expression: &str,
    encoding: Encoding,
) -> Result<Option<(String, bool)>, ApiError> {
    let compact = expression
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    if !compact.starts_with("\\x") {
        return Ok(None);
    }
    let mut rest = compact.as_str();
    let mut bytes = Vec::new();
    while let Some(value) = rest.strip_prefix("\\x") {
        if value.len() < 2 || !value[..2].bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(bad_request(
                "\\x escapes require exactly two hexadecimal digits",
            ));
        }
        bytes.push(
            u8::from_str_radix(&value[..2], 16).map_err(|_| bad_request("invalid byte escape"))?,
        );
        rest = &value[2..];
    }
    if !rest.is_empty() {
        return Err(bad_request("only explicit \\xNN byte escapes are accepted"));
    }
    let text = encoding.decode_line(&bytes);
    let exact =
        !text.contains('\u{FFFD}') && encoding.encode_text(&text).as_deref() == Some(&bytes);
    Ok(Some((text, exact)))
}

pub(super) async fn api_parse_escape(
    State(state): State<SharedState>,
    Json(request): Json<ParseEscapeRequest>,
) -> Result<Json<ParseEscapeResponse>, ApiError> {
    if request.expression.is_empty() || request.expression.len() > MAX_EXPRESSION_BYTES {
        return Err(bad_request("escape expression must contain 1-1024 bytes"));
    }
    let encoding = state
        .read(|workspace| workspace.doc().map(|doc| doc.encoding()))
        .ok_or_else(|| bad_request("no document is open"))?;
    let (text, exact_bytes) = if let Some(text) = parse_u_plus(&request.expression)? {
        (text, true)
    } else if let Some(text) = parse_braced(&request.expression)? {
        (text, true)
    } else if let Some((text, exact)) = parse_hex_bytes(&request.expression, encoding)? {
        (text, exact)
    } else {
        return Err(bad_request(
            "use an explicit U+XXXX, \\u{...}, or \\xNN expression",
        ));
    };
    let encoded = encoding.encode_text(&text);
    let representable = exact_bytes && encoded.is_some();
    let mut diagnostics = Vec::new();
    if !exact_bytes {
        diagnostics.push("decode-mismatch".into());
    }
    if encoded.is_none() {
        diagnostics.push("unrepresentable".into());
    }
    Ok(Json(ParseEscapeResponse {
        code_points: code_points(&text),
        text,
        original_encoding_hex: encoded.as_deref().map(hex),
        representable,
        diagnostics,
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ayame_core::OpenOptions;

    use super::*;

    fn open_bytes(bytes: &[u8], encoding: Encoding) -> (tempfile::NamedTempFile, Document) {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
        let doc = Document::open(
            file.path(),
            &OpenOptions {
                encoding: Some(encoding),
                ..Default::default()
            },
        )
        .unwrap();
        (file, doc)
    }

    #[test]
    fn dangerous_scalars_are_labeled_without_removing_them() {
        assert_eq!(visible_scalar('\u{202E}'), "<RLO>");
        assert!(scalar_diagnostics('\u{200B}').contains(&"zero-width-space".into()));
        assert!(scalar_diagnostics('\u{202E}').contains(&"bidi-control".into()));
    }

    #[test]
    fn color_literals_preserve_shape_and_alpha_position() {
        let color = parse_color(4, "paint = #12Ab34CC;", 12).unwrap();
        assert_eq!(color.format, "hex8");
        assert_eq!(color.rgb_hex, "#12ab34");
        assert_eq!(color.alpha, 0xCC);
        assert_eq!(color.prefix, "#");
        assert!(color.uppercase);

        let prefixed = parse_color(0, "0X123456", 2).unwrap();
        assert_eq!(prefixed.format, "0x6");
        assert_eq!(prefixed.prefix, "0X");
        assert_eq!(prefixed.rgb_hex, "#123456");
    }

    #[test]
    fn explicit_scalar_parser_rejects_surrogates() {
        assert_eq!(parse_u_plus("U+0041 U+1F600").unwrap().unwrap(), "A😀");
        assert!(parse_u_plus("U+D800").is_err());
    }

    #[test]
    fn original_bytes_are_authoritative_across_supported_encodings() {
        for encoding in [
            Encoding::Utf8,
            Encoding::ShiftJis,
            Encoding::EucJp,
            Encoding::Iso2022Jp,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let mut bytes = encoding.bom().to_vec();
            bytes.extend(encoding.encode_text("あ\n").unwrap());
            let (_file, doc) = open_bytes(&bytes, encoding);
            let cluster = inspect_cluster(&doc, &EditSession::default(), 0, 0, "あ", "grapheme");
            let expected = encoding.encode_text("あ").unwrap();
            assert_eq!(
                cluster.original_byte_offset,
                Some(encoding.bom().len() as u64)
            );
            if encoding == Encoding::Iso2022Jp {
                assert!(cluster.raw_hex.as_deref().unwrap().starts_with("1B 24"));
            } else {
                assert_eq!(cluster.raw_hex, Some(hex(&expected)));
            }
            assert_eq!(cluster.source, "original");
            assert!(cluster.representable);
        }
    }

    #[test]
    fn malformed_utf8_reports_the_exact_raw_byte_without_hiding_replacement() {
        let (_file, doc) = open_bytes(&[0xFF, b'\n'], Encoding::Utf8);
        let cluster = inspect_cluster(&doc, &EditSession::default(), 0, 0, "\u{FFFD}", "grapheme");
        assert_eq!(cluster.raw_hex.as_deref(), Some("FF"));
        assert_eq!(cluster.original_encoding_hex.as_deref(), Some("EF BF BD"));
        assert!(cluster.diagnostics.contains(&"decode-mismatch".into()));
        assert!(cluster
            .diagnostics
            .contains(&"replacement-character".into()));
    }

    #[test]
    fn eof_does_not_invent_a_line_ending_and_huge_lines_stop_at_the_cap() {
        let (_empty_file, empty) = open_bytes(b"", Encoding::Utf8);
        let mut summary = InspectSummary {
            grapheme_count: 0,
            scalar_count: 0,
            utf8_bytes: 0,
            utf16_units: 0,
            truncated: false,
        };
        let mut clusters = Vec::new();
        inspect_caret(
            &empty,
            &EditSession::default(),
            InspectPoint { line: 0, col: 0 },
            &mut summary,
            &mut clusters,
        )
        .unwrap();
        assert_eq!(clusters[0].display, "<END OF FILE>");
        assert!(clusters[0].text.is_empty());

        let bytes = vec![b'a'; MAX_INSPECT_LINE_BYTES + 1];
        let (_huge_file, huge) = open_bytes(&bytes, Encoding::Utf8);
        summary.truncated = false;
        clusters.clear();
        inspect_caret(
            &huge,
            &EditSession::default(),
            InspectPoint {
                line: 0,
                col: MAX_INSPECT_LINE_BYTES,
            },
            &mut summary,
            &mut clusters,
        )
        .unwrap();
        assert!(summary.truncated);
        assert!(clusters.is_empty());
    }
}
