"""Small MkDocs output fixes for Ayame's documentation site."""

from __future__ import annotations

import re

FAVICON_RE = re.compile(r'<link rel="shortcut icon" href="((?:\.\./)*?)img/favicon\.ico">')


def on_post_page(output: str, **_: object) -> str:
    """Use the editor SVG favicon instead of the mkdocs default .ico."""

    def favicon(match: re.Match[str]) -> str:
        prefix = match.group(1)
        href = f"{prefix}assets/favicon.svg"
        return (
            f'<link rel="icon" type="image/svg+xml" href="{href}">\n'
            f'        <link rel="shortcut icon" type="image/svg+xml" href="{href}">'
        )

    return FAVICON_RE.sub(favicon, output)
