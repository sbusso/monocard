#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["markdown==3.8", "pillow==11.3.0"]
# ///
"""The user guide's pages, from docs/monocard/guide/*.md to site/landing/guide/.

Fails on a link to a chapter or a heading that is not there, on a picture
that is not there, and on one that no chapter shows. The pictures are at
two pixels to a point, so each is shown at half its size.
"""

import html
import re
import shutil
import sys
from pathlib import Path

import markdown
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs/monocard/guide"
OUT = ROOT / "site/landing/guide"
ORDER = """index library import culling develop looks masks crop-and-retouch
presets-and-history merge export settings keys command-line""".split()

PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<meta name="theme-color" content="#0f0f0f">
<meta name="description" content="{about}">
<link rel="canonical" href="https://sbusso.github.io/monocard/guide/{file}">
<title>{title}: Monocard user guide</title>
<link rel="icon" href="../assets/icon.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body class="page">
<a class="skip" href="#main">Skip to the content</a>

<header class="top">
  <div class="wrap top-row">
    <a class="brand" href="../">
      <img src="../assets/icon.svg" width="28" height="28" alt="">
      <span>Monocard</span>
    </a>
    <nav class="top-links" aria-label="Site">
      <ul>
        <li><a href="../#zones">Features</a></li>
        <li><a href="./" aria-current="true">Guide</a></li>
        <li><a href="../changelog.html">Changelog</a></li>
        <li><a href="https://github.com/sbusso/monocard">GitHub</a></li>
      </ul>
      <a class="btn" href="../#downloads">Download</a>
    </nav>
  </div>
</header>

<div class="wrap guide">
  <div class="guide-nav">
    <nav aria-label="User guide">
      <details>
        <summary>User guide</summary>
        <ul>
{chapters}
        </ul>
      </details>
    </nav>
    <nav class="toc" aria-labelledby="on-this-page">
      <h2 id="on-this-page">On this page</h2>
      <ul>
{toc}
      </ul>
    </nav>
  </div>
  <main id="main" class="guide-text">
{body}
    <nav class="guide-next" aria-label="Chapters">{next}</nav>
  </main>
</div>

<footer class="foot">
  <div class="wrap foot-grid">
    <p>Parts of this guide are adapted from <a href="https://github.com/jessolmstead/greycard/blob/master/docs/user-guide.md">Greycard’s user guide</a> by Jess Olmstead, GPL-3.0.</p>
    <p>Something is wrong or missing? <a href="https://github.com/sbusso/monocard/issues">Report it</a>.</p>
  </div>
</footer>
</body>
</html>
"""


def page(name):
    return "./" if name == "index" else f"{name}.html"


def main():
    found = {p.stem for p in SOURCE.glob("*.md")}
    if found != set(ORDER):
        sys.exit(f"chapters and ORDER differ: {sorted(found ^ set(ORDER))}")
    chapters = {}
    for name in ORDER:
        md = markdown.Markdown(extensions=["tables", "toc", "fenced_code", "sane_lists"])
        body = md.convert((SOURCE / f"{name}.md").read_text(encoding="utf-8"))
        (top,) = md.toc_tokens
        chapters[name] = (top, body, set(re.findall(r'id="([^"]+)"', body)))

    wrong = []
    OUT.mkdir(exist_ok=True)
    shown = set()

    def figure(m):
        name, alt, caption = m["src"], m["alt"], m["title"] or ""
        shown.add(name)
        if not (SOURCE / name).is_file():
            wrong.append(f"picture: {name}")
            return m[0]
        with Image.open(SOURCE / name) as im:
            w, h = im.width // 2, im.height // 2
        wide = ' class="wide"' if w > 736 else ""
        return (
            f'<figure{wide}><a href="{name}"><img src="{name}" alt="{alt}" width="{w}" height="{h}"'
            f' loading="lazy" decoding="async"></a>'
            + (f"<figcaption>{caption}</figcaption>" if caption else "")
            + "</figure>"
        )

    for i, name in enumerate(ORDER):
        top, body, _ = chapters[name]

        def link(m):
            to, _, at = m[1].partition("#")
            to = to.removesuffix(".md") or name
            if to not in chapters or (at and at not in chapters[to][2]):
                wrong.append(f"{name}.md: {m[1]}")
                return m[0]
            here = "" if to == name else page(to)
            return f'href="{here}#{at}"' if at else f'href="{page(to)}"'

        body = re.sub(r'href="((?!https?:)[^"]+)"', link, body)
        body = re.sub(
            r'<p><img alt="(?P<alt>[^"]*)" src="(?P<src>[^"]+)"(?: title="(?P<title>[^"]*)")? /></p>',
            figure,
            body,
        )
        body = body.replace("<table>", '<div class="table" tabindex="0"><table>')
        body = body.replace("</table>", "</table></div>")
        first = re.search(r"<p>(.*?)</p>", body, re.S)[1]
        about = " ".join(html.unescape(re.sub(r"<[^>]+>", "", first)).split())

        def item(n):
            here = ' aria-current="page"' if n == name else ""
            return f'          <li><a href="{page(n)}"{here}>{chapters[n][0]["name"]}</a></li>'

        beside = ""
        if i:
            beside += f'<a href="{page(ORDER[i - 1])}" rel="prev">← {chapters[ORDER[i - 1]][0]["name"]}</a>'
        if i + 1 < len(ORDER):
            beside += f'<a href="{page(ORDER[i + 1])}" rel="next">{chapters[ORDER[i + 1]][0]["name"]} →</a>'
        (OUT / f"{name}.html").write_text(
            PAGE.format(
                about=html.escape(about),
                file="" if name == "index" else f"{name}.html",
                title=top["name"],
                chapters="\n".join(map(item, ORDER)),
                toc="\n".join(
                    f'        <li><a href="#{t["id"]}">{t["name"]}</a></li>' for t in top["children"]
                ),
                body=body,
                next=beside,
            ),
            encoding="utf-8",
        )
    wrong += [
        f"no chapter shows images/{p.name}"
        for p in (SOURCE / "images").glob("*")
        if f"images/{p.name}" not in shown
    ]
    shutil.rmtree(OUT / "images", ignore_errors=True)
    shutil.copytree(SOURCE / "images", OUT / "images")
    if wrong:
        sys.exit("links to nothing:\n" + "\n".join(wrong))


main()
