"""An afri.permissions custom section for a generated test app (item 8, C2). Since C2 an unverified app with no list
gets print and putc only, so a test app that calls more lists what it calls. permlist([]) is b'' (no section)."""


def _uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def _name(s):
    return _uleb(len(s)) + s


def permlist(perms):
    if not perms:
        return b''
    body = _name(b'afri.permissions') + _uleb(len(perms)) + b''.join(_name(p.encode()) for p in perms)
    return b'\x00' + _uleb(len(body)) + body
