from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

from sidecar_protocol import has_bearer_token, resolve_sidecar_port, write_ready

HOST = "127.0.0.1"


class SidecarHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802
        path = urlsplit(self.path).path
        if path == "/health":
            payload = {"engine": "podcast", "status": "ok"}
        elif path == "/api/status":
            if not has_bearer_token(self.headers.get("Authorization"), os.environ.get("IMMERSIVE_SIDECAR_TOKEN", "")):
                self.send_error(401, "Unauthorized")
                return
            payload = {"engine": "podcast", "status": "ready"}
        elif path.startswith("/api/"):
            self.send_error(401, "Unauthorized")
            return
        else:
            self.send_error(404, "Not Found")
            return
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        print(format % args, file=os.sys.stderr, flush=True)


class SidecarServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def start_server(port: int | None = None) -> SidecarServer:
    server = SidecarServer((HOST, resolve_sidecar_port(None if port is None else str(port))), SidecarHandler)
    write_ready("podcast", os.getpid(), server.server_port)
    return server


def main() -> None:
    server = start_server()
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
