"""Small MkDocs output fixes for Ayame's documentation site."""

from __future__ import annotations

import re

FAVICON_RE = re.compile(r'<link rel="shortcut icon" href="((?:\.\./)*?)img/favicon\.ico">')


def on_post_page(output: str, **_: object) -> str:
    """Apply small document-level details that the stock theme cannot express."""

    page = _.get("page")
    src_uri = getattr(getattr(page, "file", None), "src_uri", "")
    is_japanese = src_uri.startswith("ja/") or src_uri == (
        "adr/0001-diff-extraction-and-deprecation.md"
    )
    language = "ja" if is_japanese else "en"
    output = output.replace("<body", f'<body data-ay-language="{language}"', 1)

    def favicon(match: re.Match[str]) -> str:
        prefix = match.group(1)
        href = f"{prefix}assets/favicon.svg"
        return (
            f'<link rel="icon" type="image/svg+xml" href="{href}">\n'
            f'        <link rel="shortcut icon" type="image/svg+xml" href="{href}">'
        )

    return FAVICON_RE.sub(favicon, output)
