"""Serve the static build at its actual GitHub project base path for browser tests."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

DIST = Path(__file__).resolve().parents[1] / "dist"
class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(DIST), **kwargs)
    def do_GET(self):
        if not urlsplit(self.path).path.startswith('/GraphFusion/'):
            self.send_error(404)
            return
        self.path = self.path[len('/GraphFusion'):]
        super().do_GET()
    def log_message(self, *args):
        pass
ThreadingHTTPServer(('127.0.0.1', 4321), Handler).serve_forever()
