"""Serve the root site and legacy project redirects for browser tests."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

WEBSITE = Path(__file__).resolve().parents[1]
class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(WEBSITE / 'dist'), **kwargs)
    def do_GET(self):
        legacy = urlsplit(self.path).path.startswith('/GraphFusion/')
        self.directory = str(WEBSITE / ('legacy-dist' if legacy else 'dist'))
        if legacy:
            self.path = self.path[len('/GraphFusion'):]
        super().do_GET()
    def log_message(self, *args):
        pass
ThreadingHTTPServer(('127.0.0.1', 4321), Handler).serve_forever()
