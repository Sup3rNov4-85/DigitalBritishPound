#!/usr/bin/env python3
"""Generate docs/DBC_Node_README.pdf from README.md."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
README = ROOT / "README.md"
OUT_DIR = ROOT / "docs"
OUT_PDF = OUT_DIR / "DBC_Node_README.pdf"


def md_to_html(md: str) -> str:
    try:
        import markdown
    except ImportError:
        print("Installing markdown...", file=sys.stderr)
        import subprocess

        subprocess.check_call([sys.executable, "-m", "pip", "install", "markdown", "-q"])
        import markdown

    body = markdown.markdown(
        md,
        extensions=["tables", "fenced_code", "nl2br"],
    )
    return f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8"/>
<style>
  @page {{ size: A4; margin: 2cm; }}
  body {{ font-family: Helvetica, Arial, sans-serif; font-size: 11pt; line-height: 1.45; color: #111; }}
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
        print("Installing xhtml2pdf...", file=sys.stderr)
        import subprocess

        subprocess.check_call([sys.executable, "-m", "pip", "install", "xhtml2pdf", "-q"])
        from xhtml2pdf import pisa

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "wb") as pdf_file:
        status = pisa.CreatePDF(html.encode("utf-8"), dest=pdf_file, encoding="utf-8")
    if status.err:
        raise RuntimeError(f"PDF generation failed with {status.err} errors")


def main() -> None:
    if not README.exists():
        print(f"Missing {README}", file=sys.stderr)
        sys.exit(1)
    md = README.read_text(encoding="utf-8")
    html = md_to_html(md)
    html_to_pdf(html, OUT_PDF)
    print(f"Wrote {OUT_PDF}")


if __name__ == "__main__":
    main()
