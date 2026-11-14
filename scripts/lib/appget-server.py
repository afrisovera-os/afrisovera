"""Host HTTP server for the appget suite (claude/appget). The guest reaches it as 10.0.2.2:<port> through SLIRP.

  appget-server.py make DIR        write the test modules into DIR and print "name sha256" for each
  appget-server.py serve DIR PORTFILE   serve them (HTTP/1.0); the chosen port goes in PORTFILE

Paths (the file name the guest asks for):
  big.wasm      the 3,000-byte persist-big app (prints 4242, then its last data byte 90)
  v2.wasm       a small app that prints 77
  netapp.wasm   an app that calls net(0) (a capability sideloaded apps do not get), then print
  notwasm.wasm  plain text
  huge.wasm     20,000 bytes (over the 16 KiB a file holds)
  extra.wasm    big.wasm with 300 bytes past its Content-Length
  nolen.wasm    big.wasm with no Content-Length
  swap.wasm     v2.wasm's bytes (asked for with big.wasm's hash: a swapped valid module)
  re.wasm       big.wasm the first time, v2.wasm after (reinstall with new bytes)
  permbad.wasm  v2.wasm with an afri.permissions section asking print and 'teleport' (unknown: refused)
  permnet.wasm  netapp.wasm with an afri.permissions section asking print and net (claude/nia-perms)
  anything else 404
"""
import hashlib
import http.server
import os
import sys


def uleb(n):
    b = bytearray()
    while True:
        x = n & 0x7F
        n >>= 7
        b.append(x | (0x80 if n else 0))
        if not n:
            return bytes(b)


def sec(i, body):
    return bytes([i]) + uleb(len(body)) + body


def module(imports, body):
    """imports: list of (name, type index); types: 0 = (i32)->(), 1 = ()->(), 2 = (i32)->i32."""
    types = sec(1, b"\x03" + b"\x60\x01\x7f\x00" + b"\x60\x00\x00" + b"\x60\x01\x7f\x01\x7f")
    imp = sec(2, uleb(len(imports)) + b"".join(
        b"\x04afri" + uleb(len(n)) + n + b"\x00" + uleb(t) for n, t in imports))
    run = len(imports)
    funcs = sec(3, b"\x01\x01")
    exp = sec(7, b"\x01\x03run\x00" + uleb(run))
    code = sec(10, b"\x01" + uleb(len(body) + 1) + b"\x00" + body)
    return b"\x00asm\x01\x00\x00\x00" + types + imp + funcs + exp + code


def perms(names):
    """An afri.permissions custom section (claude/nia-perms) naming these permissions."""
    tag = b"afri.permissions"
    return sec(0, uleb(len(tag)) + tag + uleb(len(names)) + b"".join(uleb(len(n)) + n for n in names))


def make(d):
    big = open(os.path.join(d, "big.wasm"), "rb").read()  # made by mkvfs-big.py bigwasm
    files = {
        "v2.wasm": module([(b"print", 0)], b"\x41\xcd\x00\x10\x00\x0b"),                      # print(77)
        "netapp.wasm": module([(b"net", 2), (b"print", 0)], b"\x41\x00\x10\x00\x10\x01\x0b"),  # print(net(0))
        "notwasm.wasm": b"this is not a WebAssembly module, just some text\n",
        "permbad.wasm": module([(b"print", 0)], b"\x41\xcd\x00\x10\x00\x0b") + perms([b"print", b"teleport"]),
        "permnet.wasm": module([(b"net", 2), (b"print", 0)], b"\x41\x00\x10\x00\x10\x01\x0b")
        + perms([b"print", b"net"]),
        "huge.wasm": b"\x00asm\x01\x00\x00\x00" + bytes(20000 - 8),
    }
    for n, b in files.items():
        open(os.path.join(d, n), "wb").write(b)
    for n in ["big.wasm"] + list(files):
        b = big if n == "big.wasm" else files[n]
        print(n, hashlib.sha256(b).hexdigest())


def serve(d, portfile):
    rd = lambda n: open(os.path.join(d, n), "rb").read()
    re_count = [0]

    class H(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.0"

        def log_message(self, fmt, *a):
            sys.stderr.write("appget-server: " + (fmt % a) + "\n")

        def send(self, body, length=True, extra=b""):
            self.send_response(200)
            self.send_header("Content-Type", "application/wasm")
            if length:
                self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body + extra)

        def do_GET(self):
            n = self.path.lstrip("/")
            if n in ("big.wasm", "v2.wasm", "netapp.wasm", "notwasm.wasm", "huge.wasm", "permbad.wasm",
                     "permnet.wasm"):
                self.send(rd(n))
            elif n == "extra.wasm":
                self.send(rd("big.wasm"), extra=b"X" * 300)
            elif n == "nolen.wasm":
                self.send(rd("big.wasm"), length=False)
            elif n == "swap.wasm":
                self.send(rd("v2.wasm"))
            elif n == "re.wasm":
                re_count[0] += 1
                self.send(rd("big.wasm") if re_count[0] == 1 else rd("v2.wasm"))
            else:
                self.send_error(404)

    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    open(portfile, "w").write(str(srv.server_address[1]))
    srv.serve_forever()


if __name__ == "__main__":
    if sys.argv[1] == "make":
        make(sys.argv[2])
    else:
        serve(sys.argv[2], sys.argv[3])
