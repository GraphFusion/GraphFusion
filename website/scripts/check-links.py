"""Check the built project site's links, fragments and referenced local assets."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit

ROOT = Path(__file__).resolve().parents[1] / "dist"
ORIGIN = "https://graphfusion.github.io"
BASE = "/GraphFusion/"

class Page(HTMLParser):
    def __init__(self, source):
        super().__init__()
        self.ids, self.urls = set(), []
        self.feed(source)
    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        for key in ("href", "src"):
            if attrs.get(key):
                self.urls.append(attrs[key])
        if attrs.get("srcset") and not attrs["srcset"].startswith("data:"):
            self.urls.extend(item.strip().split()[0] for item in attrs["srcset"].split(","))

pages = {p: Page(p.read_text()) for p in ROOT.rglob("*.html")}
errors = []
links = 0
for source, parsed in pages.items():
    relative = source.relative_to(ROOT).as_posix()
    route = BASE + (relative[:-10] if relative.endswith("index.html") else relative)
    for href in parsed.urls:
        url = urlsplit(urljoin(ORIGIN + route, href))
        if url.scheme not in ("https", "http") or url.netloc != urlsplit(ORIGIN).netloc:
            continue
        links += 1
        if not url.path.startswith(BASE):
            errors.append(f"{relative}: missing project base in {href}")
            continue
        dest = ROOT / unquote(url.path[len(BASE):])
        if dest.is_dir():
            dest /= "index.html"
        if not dest.is_file():
            errors.append(f"{relative}: missing {href}")
        elif url.fragment and dest in pages and unquote(url.fragment) not in pages[dest].ids:
            errors.append(f"{relative}: missing fragment in {href}")
if not (ROOT / "pagefind/pagefind.js").is_file():
    errors.append("Search bundle was not built")
if errors:
    raise SystemExit("\n".join(sorted(set(errors))))
print(f"Verified {links} internal links/assets across {len(pages)} HTML pages, including fragments and project base.")
