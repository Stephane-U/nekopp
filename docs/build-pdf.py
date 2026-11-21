#!/usr/bin/env python3
"""Render docs/nekopp-features.md to HTML and then to a PDF.

No third-party packages: this is a converter for the subset of markdown the
feature reference actually uses -- headings, paragraphs, lists, tables, fenced
code, block quotes, rules, and inline code, bold, italic and links. That subset
is all a reference manual needs, and a real markdown library would be one more
thing to install on the machine that rebuilds the PDF.

Usage:

    python3 docs/build-pdf.py                 # PDF next to the markdown
    python3 docs/build-pdf.py --html          # keep the intermediate HTML too
    python3 docs/build-pdf.py --out /tmp/x.pdf

The renderer is WeasyPrint. It is looked up on PATH first and at its usual
per-user location second, because that is where a `pip install --user` puts it
on a machine with no root.
"""

import argparse
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "docs", "nekopp-features.md")
STYLESHEET = os.path.join(ROOT, "docs", "nekopp-features.css")
DEFAULT_OUT = os.path.join(ROOT, "docs", "nekopp-features.pdf")

WEASYPRINT_CANDIDATES = (
    "weasyprint",
    os.path.expanduser("~/.local/bin/weasyprint"),
    "/usr/local/bin/weasyprint",
)


def escape(text):
    return html.escape(text, quote=False)


def slugify(text):
    """A stable anchor for a heading.

    Lowercase ASCII, digits, and hyphens. Everything else is dropped, which
    keeps a heading like "Encodings and Line Endings" addressable as
    `encodings-and-line-endings` from outside this script -- the MCP server
    links sections by exactly this rule.
    """
    slug = text.strip().lower()
    slug = re.sub(r"[`*_]", "", slug)
    slug = re.sub(r"[^a-z0-9]+", "-", slug)
    return slug.strip("-") or "section"


INLINE_CODE = re.compile(r"`([^`]+)`")


def inline(text):
    """Escapes and then decorates one run of inline markdown.

    Code spans are protected first so that emphasis inside them -- `align_items
    *ptr` in a signature, say -- survives as written rather than turning into
    italics.
    """
    spans = []

    def stash(match):
        spans.append(match.group(1))
        return "\x00%d\x00" % (len(spans) - 1)

    protected = INLINE_CODE.sub(stash, text)
    out = escape(protected)

    out = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', out)
    out = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", out)
    out = re.sub(r"(?<!\*)\*([^*\n]+)\*(?!\*)", r"<em>\1</em>", out)
    out = re.sub(r"__([^_]+)__", r"<strong>\1</strong>", out)

    def restore(match):
        return "<code>%s</code>" % escape(spans[int(match.group(1))])

    return re.sub(r"\x00(\d+)\x00", restore, out)


def split_row(line):
    """Cells of a pipe-table row, honouring escaped pipes and code spans."""
    line = line.strip()
    if line.startswith("|"):
        line = line[1:]
    if line.endswith("|"):
        line = line[:-1]
    cells, current, in_code = [], [], False
    for char in line:
        if char == "`":
            in_code = not in_code
        if char == "|" and not in_code:
            cells.append("".join(current).strip())
            current = []
        else:
            current.append(char)
    cells.append("".join(current).strip())
    return cells


def is_table_rule(line):
    stripped = line.strip()
    if not stripped.startswith("|"):
        return False
    return bool(re.fullmatch(r"\|[\s:|-]+\|?", stripped)) and "-" in stripped


class Renderer:
    def __init__(self):
        self.headings = []  # (level, text, slug)
        self.seen = {}

    def anchor(self, text):
        base = slugify(text)
        slug = base
        count = self.seen.get(base, 0)
        while slug in self.seen:
            count += 1
            slug = "%s-%d" % (base, count)
        self.seen[base] = count
        self.seen[slug] = 0
        return slug

    def render(self, markdown):
        lines = markdown.split("\n")
        out = []
        i = 0
        paragraph = []

        def flush():
            if paragraph:
                out.append("<p>%s</p>" % inline(" ".join(paragraph)))
                paragraph.clear()

        while i < len(lines):
            line = lines[i]
            stripped = line.strip()

            if stripped.startswith("```"):
                flush()
                fence = stripped[:3]
                body = []
                i += 1
                while i < len(lines) and not lines[i].strip().startswith(fence):
                    body.append(lines[i])
                    i += 1
                i += 1
                out.append("<pre><code>%s</code></pre>" % escape("\n".join(body)))
                continue

            if not stripped:
                flush()
                i += 1
                continue

            heading = re.match(r"^(#{1,6})\s+(.*)$", stripped)
            if heading:
                flush()
                level = len(heading.group(1))
                text = heading.group(2).strip()
                slug = self.anchor(text)
                self.headings.append((level, text, slug))
                out.append(
                    '<h%d id="%s">%s</h%d>' % (level, slug, inline(text), level)
                )
                i += 1
                continue

            if re.fullmatch(r"(-{3,}|\*{3,}|_{3,})", stripped):
                flush()
                out.append("<hr/>")
                i += 1
                continue

            if stripped.startswith(">"):
                flush()
                quoted = []
                while i < len(lines) and lines[i].strip().startswith(">"):
                    quoted.append(lines[i].strip()[1:].lstrip())
                    i += 1
                body = "<br/>".join(inline(q) for q in quoted if q)
                out.append("<blockquote><p>%s</p></blockquote>" % body)
                continue

            if (
                stripped.startswith("|")
                and i + 1 < len(lines)
                and is_table_rule(lines[i + 1])
            ):
                flush()
                header = split_row(lines[i])
                i += 2
                rows = []
                while i < len(lines) and lines[i].strip().startswith("|"):
                    rows.append(split_row(lines[i]))
                    i += 1
                out.append(self.render_table(header, rows))
                continue

            if re.match(r"^\s*([-*+]|\d+\.)\s+", line):
                flush()
                block, i = self.collect_list(lines, i)
                out.append(block)
                continue

            paragraph.append(stripped)
            i += 1

        flush()
        return "\n".join(out)

    def render_table(self, header, rows):
        parts = ['<table class="grid">', "<thead><tr>"]
        for cell in header:
            parts.append("<th>%s</th>" % inline(cell))
        parts.append("</tr></thead><tbody>")
        for row in rows:
            parts.append("<tr>")
            for cell in row:
                parts.append("<td>%s</td>" % inline(cell))
            parts.append("</tr>")
        parts.append("</tbody></table>")
        return "".join(parts)

    def collect_list(self, lines, start):
        """Nested lists, two spaces per level.

        The items are flattened first and then rebuilt as a tree, because a
        list is a tree and a single pass over indentation is where every bug in
        a markdown converter lives. Two spaces are one level.
        """
        items = []  # (level, ordered, text)
        i = start
        while i < len(lines):
            line = lines[i]
            if not line.strip():
                # A blank line continues a list only if another item follows.
                look = i + 1
                while look < len(lines) and not lines[look].strip():
                    look += 1
                if look < len(lines) and re.match(r"^\s*([-*+]|\d+\.)\s+", lines[look]):
                    i = look
                    continue
                break
            match = re.match(r"^(\s*)([-*+]|\d+\.)\s+(.*)$", line)
            if not match:
                # A plain line under an item is that item's text continued on
                # the next line of the source, not a new paragraph: the
                # markdown is wrapped at 80 columns, so most items wrap.
                if items and line.strip() and not re.match(
                    r"^(#|```|~~~|>|\|)", line.strip()
                ):
                    level, ordered, text = items[-1]
                    items[-1] = (level, ordered, text + " " + line.strip())
                    i += 1
                    continue
                break
            indent = len(match.group(1).replace("\t", "  "))
            ordered = match.group(2)[0].isdigit()
            items.append((indent // 2, ordered, match.group(3).strip()))
            i += 1

        root = []
        stack = [(-1, root)]  # (level, children of the node at that level)
        for level, ordered, text in items:
            while stack and level <= stack[-1][0]:
                stack.pop()
            node = {"text": text, "ordered": ordered, "children": []}
            stack[-1][1].append(node)
            stack.append((level, node["children"]))

        def render(nodes):
            out = []
            index = 0
            while index < len(nodes):
                ordered = nodes[index]["ordered"]
                tag = "ol" if ordered else "ul"
                out.append("<%s>" % tag)
                while index < len(nodes) and nodes[index]["ordered"] == ordered:
                    node = nodes[index]
                    out.append(
                        "<li>%s%s</li>"
                        % (inline(node["text"]), render(node["children"]))
                    )
                    index += 1
                out.append("</%s>" % tag)
            return "".join(out)

        return render(root), i


def split_intro(markdown):
    """Split the abstract off the front of the document.

    The paragraphs between the title and the first "## " are the abstract, not
    a chapter: left in the body flow they print on a page of their own between
    the contents and the first chapter, which reads as a mistake.
    """
    lines = markdown.split("\n")
    start = 0
    for index, line in enumerate(lines):
        if line.startswith("# "):
            start = index + 1
            break
    for index in range(start, len(lines)):
        if lines[index].startswith("## "):
            return "\n".join(lines[start:index]).strip(), "\n".join(lines[index:])
    return "\n".join(lines[start:]).strip(), ""


def build_html(markdown, title):
    intro, rest = split_intro(markdown)
    renderer = Renderer()
    body = renderer.render(rest)
    abstract = Renderer().render(intro) if intro else ""

    toc_rows = []
    for level, text, slug in renderer.headings:
        if level == 1:
            continue
        if level <= 3:
            toc_rows.append(
                '<li class="toc-level-%d"><a href="#%s">%s</a></li>'
                % (level, slug, inline(text))
            )

    return """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>%s</title>
<link rel="stylesheet" href="%s"/>
</head>
<body>
<section class="title-page">
  <p class="title-kicker">Neko++</p>
  <h1 class="title">%s</h1>
  <p class="title-subtitle">Every feature, menu row and key binding, read from the source.</p>
  <div class="title-abstract">%s</div>
  <p class="title-meta">npp 0.1.0 &middot; generated from docs/nekopp-features.md</p>
</section>
<section class="toc">
  <h2 class="toc-title">Contents</h2>
  <ul class="toc-list">
%s
  </ul>
</section>
%s
</body>
</html>
""" % (
        escape(title),
        os.path.basename(STYLESHEET),
        escape(title),
        abstract,
        "\n".join("    " + row for row in toc_rows),
        body,
    )


def find_weasyprint():
    for candidate in WEASYPRINT_CANDIDATES:
        path = candidate if os.path.sep in candidate else shutil.which(candidate)
        if path and os.path.exists(path) and os.access(path, os.X_OK):
            return path
    return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", default=SOURCE)
    parser.add_argument("--out", default=DEFAULT_OUT)
    parser.add_argument(
        "--html", action="store_true", help="keep the intermediate HTML beside the PDF"
    )
    parser.add_argument("--weasyprint", default=None, help="path to the renderer")
    args = parser.parse_args(argv)

    with open(args.source, encoding="utf-8") as handle:
        markdown = handle.read()

    title = "Neko++ Feature Reference"
    for line in markdown.split("\n"):
        if line.startswith("# "):
            title = line[2:].strip()
            break

    document = build_html(markdown, title)

    renderer = args.weasyprint or find_weasyprint()
    if renderer is None:
        sys.stderr.write("build-pdf: weasyprint not found; HTML only\n")
        html_path = os.path.splitext(args.out)[0] + ".html"
        with open(html_path, "w", encoding="utf-8") as handle:
            handle.write(document)
        return 1

    if args.html:
        html_path = os.path.splitext(args.out)[0] + ".html"
        with open(html_path, "w", encoding="utf-8") as handle:
            handle.write(document)
    else:
        handle = tempfile.NamedTemporaryFile(
            "w", suffix=".html", delete=False, encoding="utf-8"
        )
        handle.write(document)
        handle.close()
        html_path = handle.name

    # The stylesheet is linked relatively, so the HTML has to sit beside it for
    # the link to resolve; a temp file elsewhere gets an absolute file: URL.
    # Comparing against the output path instead would style a build into docs/
    # and silently drop the stylesheet for a build anywhere else.
    if os.path.dirname(os.path.abspath(html_path)) != os.path.dirname(STYLESHEET):
        document = document.replace(
            'href="%s"' % os.path.basename(STYLESHEET),
            'href="file://%s"' % STYLESHEET,
        )
        with open(html_path, "w", encoding="utf-8") as again:
            again.write(document)

    result = subprocess.run([renderer, html_path, args.out])
    if not args.html:
        os.unlink(html_path)
    if result.returncode != 0:
        sys.stderr.write("build-pdf: weasyprint failed\n")
        return result.returncode
    print("wrote %s" % args.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
