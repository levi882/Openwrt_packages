#!/usr/bin/env python3
import html
import sys
from pathlib import Path


def tree_lines(root):
    files = sorted(
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path.name != "index.html"
    )
    lines = []

    for index, file_path in enumerate(files):
        parts = file_path.split("/")
        is_last_file = index == len(files) - 1

        for depth, part in enumerate(parts):
            is_leaf = depth == len(parts) - 1
            prefix = ""

            if depth:
                prefix += "|   " * depth

            branch = "`--- " if is_last_file and is_leaf else "|--- "
            label = html.escape(part)

            if is_leaf:
                href = "/" + html.escape(file_path, quote=True)
                lines.append(f'{prefix}{branch}<a href="{href}">{label}</a>')
            else:
                parent = "/".join(parts[: depth + 1])
                previous_parent = "/".join(files[index - 1].split("/")[: depth + 1]) if index else None
                if parent != previous_parent:
                    href = "/" + html.escape(parent, quote=True) + "/"
                    lines.append(f'{prefix}{branch}<a href="{href}">{label}</a>')

    return "\n".join(lines)


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: generate-index.py <public-dir> <title>")

    root = Path(sys.argv[1]).resolve()
    title = sys.argv[2]
    body = tree_lines(root)

    (root / "index.html").write_text(
        f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{html.escape(title)}</title>
  <style>
    body {{
      background: white;
      color: black;
      font-family: "Times New Roman", Times, serif;
      margin: 12px;
    }}
    h1 {{
      font-size: 34px;
      font-weight: 400;
      margin: 0 0 36px;
    }}
    pre {{
      font-family: "Courier New", Courier, monospace;
      font-size: 16px;
      line-height: 1.16;
      margin: 0;
      white-space: pre;
    }}
    a {{
      color: #0000ee;
      text-decoration: none;
    }}
    a:hover {{
      text-decoration: underline;
    }}
  </style>
</head>
<body>
  <h1>{html.escape(title)}</h1>
  <pre>{body}</pre>
</body>
</html>
""",
        encoding="utf-8",
        newline="\n",
    )


if __name__ == "__main__":
    main()
