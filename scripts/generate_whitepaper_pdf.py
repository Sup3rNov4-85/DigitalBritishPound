#!/usr/bin/env python3
"""Generate docs/DBC_Whitepaper_Public.pdf from docs/DBC_Whitepaper_Public.md."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IN_MD = ROOT / "docs" / "DBC_Whitepaper_Public.md"
OUT_PDF = ROOT / "docs" / "DBC_Whitepaper_Public.pdf"


def md_to_html(md: str) -> str:
    try:
        import markdown
    except ImportError:
        import subprocess

        subprocess.check_call([sys.executable, "-m", "pip", "install", "markdown", "-q"])
        import markdown

    body = markdown.markdown(md, extensions=["tables", "fenced_code", "nl2br"])
    return f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8"/>
<style>
  @page {{ size: A4; margin: 2cm; }}
  body {{ font-family: Helvetica, Arial, sans-serif; font-size: 11pt; line-height: 1.5; color: #111; }}
  h1 {{ font-size: 22pt; border-bottom: 2px solid #333; padding-bottom: 6px; }}
  h2 {{ font-size: 14pt; margin-top: 1.2em; color: #222; }}
  h3 {{ font-size: 12pt; }}
  code, pre {{ font-family: Consolas, monospace; font-size: 9pt; background: #f4f4f4; }}
  pre {{ padding: 10px; border: 1px solid #ddd; white-space: pre-wrap; word-wrap: break-word; }}
  table {{ border-collapse: collapse; width: 100%; margin: 12px 0; font-size: 10pt; }}
  th, td {{ border: 1px solid #ccc; padding: 6px 8px; text-align: left; }}
  th {{ background: #eee; }}
  hr {{ border: none; border-top: 1px solid #ccc; margin: 1.5em 0; }}
</style>
</head>
<body>
{body}
</body>
</html>
"""


def html_to_pdf(html: str, out_path: Path) -> None:
    try:
        from xhtml2pdf import pisa
    except ImportError:
        import subprocess

        subprocess.check_call([sys.executable, "-m", "pip", "install", "xhtml2pdf", "-q"])
        from xhtml2pdf import pisa

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "wb") as pdf_file:
        status = pisa.CreatePDF(html.encode("utf-8"), dest=pdf_file, encoding="utf-8")
    if status.err:
        raise RuntimeError(f"PDF generation failed with {status.err} errors")


def main() -> None:
    if not IN_MD.exists():
        print(f"Missing {IN_MD}", file=sys.stderr)
        sys.exit(1)
    md = IN_MD.read_text(encoding="utf-8")
    html = md_to_html(md)
    html_to_pdf(html, OUT_PDF)
    print(f"Wrote {OUT_PDF}")


if __name__ == "__main__":
    main()

