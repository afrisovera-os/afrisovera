"""Host web server for the dira suite (BROWSER_PLAN.md, milestone M1). The guest reaches it as 10.0.2.2:<port>
through QEMU's user network. HTTP/1.1, one answer per connection, written by hand so each case is exact.

  dira-server.py PORT PAGES_DIR

Paths:
  /index.html /about.html   from PAGES_DIR, with Content-Length
  /chunked.html             Transfer-Encoding: chunked, in uneven pieces sent apart
  /plain.txt                text/plain (tags in it stay as written)
  /latin1.html              charset=iso-8859-1, no Content-Length (read to close)
  /redirect                 302 to /about.html
  /loop                     302 to /loop (Dira gives up after 5)
  /missing (and any other)  404 with a small HTML page
  /pic.png                  image/png (not a page)
  /gzip                     Content-Encoding: gzip (not read in M1)
  /big.html                 about 300 KB of paragraphs, read to close (Dira keeps the first 48,000 letters)
  /slow                     accepts, says nothing for 40 s (Dira stops at 30 s)
  /tometa                   302 to http://169.254.169.254/latest/ (link-local: Dira does not follow it)
  /links.html               300 links (Dira numbers 256 and says the rest are plain text)
  /badchunk                 a chunked page whose second size line is 70 bytes long (the first part stays)
"""
import os
import socket
import socketserver
import sys
import threading
import time

PAGES = sys.argv[2] if len(sys.argv) > 2 else "."


def page(name):
    with open(os.path.join(PAGES, name), "rb") as f:
        return f.read()


def answer(status, headers, body=b""):
    head = "HTTP/1.1 %s\r\n" % status
    for k, v in headers:
        head += "%s: %s\r\n" % (k, v)
    return head.encode() + b"\r\n" + body


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        s.settimeout(10)
        req = b""
        try:
            while b"\r\n\r\n" not in req and len(req) < 8192:
                d = s.recv(2048)
                if not d:
                    return
                req += d
        except OSError:
            return
        line = req.split(b"\r\n", 1)[0].decode("latin1")
        parts = line.split(" ")
        path = parts[1] if len(parts) > 1 else "/"
        host = [l for l in req.split(b"\r\n") if l.lower().startswith(b"host:")]
        sys.stderr.write("dira-server: %s (%s)\n" % (line, host[0].decode("latin1") if host else "no Host"))
        html = [("Content-Type", "text/html; charset=utf-8")]
        try:
            if path in ("/index.html", "/about.html"):
                b = page(path[1:])
                s.sendall(answer("200 OK", html + [("Content-Length", len(b)), ("Connection", "close")], b))
            elif path == "/chunked.html":
                s.sendall(answer("200 OK", html + [("Transfer-Encoding", "chunked")]))
                body = b"<h3>Chunked</h3><p>This page came in pieces,</p><p>and Dira joined them.</p>"
                k = 0
                for n in (3, 7, 1, 11, 5, 64):
                    piece = body[k:k + n]
                    k += n
                    if piece:
                        s.sendall(b"%x;note=1\r\n%s\r\n" % (len(piece), piece))
                        time.sleep(0.05)
                s.sendall(b"0\r\n\r\n")
            elif path == "/plain.txt":
                b = b"Plain text\n  stays as it is: <b>not bold</b> &amp;\n"
                s.sendall(answer("200 OK", [("Content-Type", "text/plain"), ("Content-Length", len(b))], b))
            elif path == "/latin1.html":
                s.sendall(answer("200 OK", [("Content-Type", "text/html; charset=ISO-8859-1")], b"<p>caf\xe9 \x93quoted\x94</p>"))
            elif path == "/redirect":
                s.sendall(answer("302 Found", [("Location", "/about.html"), ("Content-Length", 0)]))
            elif path == "/loop":
                s.sendall(answer("302 Found", [("Location", "/loop"), ("Content-Length", 0)]))
            elif path == "/pic.png":
                b = b"\x89PNG\r\n\x1a\n" + b"\0" * 24
                s.sendall(answer("200 OK", [("Content-Type", "image/png"), ("Content-Length", len(b))], b))
            elif path == "/gzip":
                s.sendall(answer("200 OK", html + [("Content-Encoding", "gzip"), ("Content-Length", 4)], b"\x1f\x8b\x08\x00"))
            elif path == "/big.html":
                s.sendall(answer("200 OK", html))
                para = b"<p>" + b"word " * 40 + b"</p>\n"
                for _ in range(1500):
                    s.sendall(para)
            elif path == "/tometa":
                s.sendall(answer("302 Found", [("Location", "http://169.254.169.254/latest/"), ("Content-Length", 0)]))
            elif path == "/links.html":
                b = b"".join(b'<a href="/p%d">l%d</a> ' % (k, k) for k in range(300))
                s.sendall(answer("200 OK", html + [("Content-Length", len(b))], b))
            elif path == "/badchunk":
                s.sendall(answer("200 OK", html + [("Transfer-Encoding", "chunked")]))
                part = b"<p>The first part arrived.</p>"
                s.sendall(b"%x\r\n%s\r\n" % (len(part), part))
                s.sendall(b"5;" + b"x" * 70 + b"\r\nhello\r\n0\r\n\r\n")
            elif path.startswith("/slow"):
                time.sleep(40)
            else:
                b = b"<h1>Not here</h1><p>No page at this address.</p>"
                s.sendall(answer("404 Not Found", html + [("Content-Length", len(b))], b))
        except OSError:
            pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    port = int(sys.argv[1])
    with Server(("127.0.0.1", port), Handler) as srv:
        sys.stderr.write("dira-server: on port %d\n" % port)
        srv.serve_forever()
