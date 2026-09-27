"""Build a GitHub project site that redirects old documentation URLs to root."""
import html
import json
from pathlib import Path
import shutil

WEBSITE = Path(__file__).resolve().parents[1]
OUTPUT = WEBSITE / "legacy-dist"
shutil.rmtree(OUTPUT, ignore_errors=True)
OUTPUT.mkdir()

for page in (WEBSITE / "dist").rglob("*.html"):
    relative = page.relative_to(WEBSITE / "dist")
    route = "/" + relative.as_posix().removesuffix("index.html")
    destination = json.dumps(route)
    if relative.as_posix() == "404.html":
        destination = '(location.pathname.replace(/^\\/GraphFusion(?=\\/|$)/, "") || "/")'
    target = OUTPUT / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(f'''<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>GraphFusion documentation has moved</title>
<link rel="canonical" href="https://graphfusion.github.io{html.escape(route, quote=True)}">
<script>const target = new URL(location.href); target.pathname = {destination}; location.replace(target.href);</script>
<meta http-equiv="refresh" content="0;url={html.escape(route, quote=True)}">
<p>Continue to <a href="{html.escape(route, quote=True)}">GraphFusion documentation</a>.</p>
</html>
''')

print(f"Built {len(list(OUTPUT.rglob('*.html')))} legacy page redirects.")
