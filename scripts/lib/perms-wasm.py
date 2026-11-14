"""Write the wasm-perms apps (claude/nia-perms, OS_REQUESTS item 8 step B): an app lists the permissions it needs
in an `afri.permissions` custom section, and gets only those, within what its kind allows.

  perms-wasm.py DIR     writes DIR/NAME.wasm for each app below, one name per line on stdout

The section: id 0, the name "afri.permissions", a uleb count, then each permission name (uleb length, bytes).
Every app imports print (0), time (1) and net (2) from `afri`. As a disk app (unverified) its kind allows
print, putc, draw, time, input and random; never file, net, AI or voice.
  p1   section FIRST, asks print; prints 1, then calls time            DENIED time, TRAP
  p2   asks print and time; prints 2, calls time, prints 3               runs
  p3   asks print and net; prints 4, calls net                            net withheld (unverified), DENIED net
  p4   asks print and 'teleport'                                          refused at load: unknown permission
  p5   two afri.permissions sections                                      refused at load
  p6   a section whose count says 2 but holds one name                    refused at load
  p7   no section; prints 7, calls time                                   runs on the v0 grant, with its line
  p8   asks print, imports nothing it calls but print; prints 8           runs (asking is not using)
  p9   a `name` section, then the code, then afri.permissions [print] LAST; prints 9, calls time
                                                                          DENIED time (the grant is made after
                                                                          every section, wherever it is)
  p10  asks 17 permissions                                                refused at load (Nia holds 16)
  p11  asks print and putc with trailing junk inside the section          refused at load
On the old kernel every app loads: the section is skipped, p1 and p9 call time freely and p3's line is missing.
"""
import os
import sys


def uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def section(sid, body):
    return bytes([sid]) + uleb(len(body)) + body


def name(bs):
    return uleb(len(bs)) + bs


def perms(names, count=None, extra=b''):
    n = len(names) if count is None else count
    return section(0, name(b'afri.permissions') + uleb(n) + b''.join(name(x) for x in names) + extra)


MAGIC = b'\x00asm\x01\x00\x00\x00'
TYPES = section(1, b'\x04' + b'\x60\x01\x7F\x00' + b'\x60\x00\x01\x7F' + b'\x60\x01\x7F\x01\x7F' + b'\x60\x00\x00')
IMPORTS = section(2, b'\x03' + name(b'afri') + name(b'print') + b'\x00\x00'
                  + name(b'afri') + name(b'time') + b'\x00\x01'
                  + name(b'afri') + name(b'net') + b'\x00\x02')
FUNCS = section(3, b'\x01\x03')                            # run: () -> (), type 3
EXPORT = section(7, b'\x01' + name(b'run') + b'\x00\x03')   # 3 imports, so run is function 3
C = lambda v: b'\x41' + bytes([v])                          # i32.const v (v < 64)
PRINT, TIME, NET = b'\x10\x00', b'\x10\x01', b'\x10\x02'
DROP = b'\x1A'


def code(body):
    f = b'\x00' + body + b'\x0B'                             # no locals
    return section(10, b'\x01' + uleb(len(f)) + f)


def app(body, before=b'', after=b''):
    return MAGIC + before + TYPES + IMPORTS + FUNCS + EXPORT + code(body) + after


NAME_SECTION = section(0, name(b'name') + b'\x00\x05' + b'\x04run!')   # a `name` section (module name), skipped

APPS = {
    'p1': app(C(1) + PRINT + TIME + DROP, before=perms([b'print'])),
    'p2': app(C(2) + PRINT + TIME + DROP + C(3) + PRINT, after=perms([b'print', b'time'])),
    'p3': app(C(4) + PRINT + C(0) + NET + DROP, after=perms([b'print', b'net'])),
    'p4': app(C(5) + PRINT, after=perms([b'print', b'teleport'])),
    'p5': app(C(5) + PRINT, after=perms([b'print']) + perms([b'print'])),
    'p6': app(C(5) + PRINT, after=perms([b'print'], count=2)),
    'p7': app(C(7) + PRINT + TIME + DROP),
    'p8': app(C(8) + PRINT, after=perms([b'print'])),
    'p9': app(C(9) + PRINT + TIME + DROP, before=NAME_SECTION, after=perms([b'print'])),
    'p10': app(C(5) + PRINT, after=perms([b'print'] * 17)),
    'p11': app(C(5) + PRINT, after=perms([b'print', b'putc'], extra=b'\x00\x00')),
}

if __name__ == '__main__':
    d = sys.argv[1]
    os.makedirs(d, exist_ok=True)
    for n, body in APPS.items():
        open(os.path.join(d, n + '.wasm'), 'wb').write(body)
        print(n)
