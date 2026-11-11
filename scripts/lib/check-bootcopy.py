#!/usr/bin/env python3
"""Checks a disk image after `bootcopy` / `bootcopy remove` (suite pc-bootcopy, PC track step 5).

    check-bootcopy.py copied  ORIG NOW BOOTFILE   # after bootcopy
    check-bootcopy.py removed ORIG MID NOW        # after bootcopy remove (MID: the image after bootcopy)

The EFI system partition is GPT entry 1, where make-disk-image.py's gpt-esp kinds put it (sector 2048; block
256 in gpt-esp-fsinfo-4k). The block size (512 or 4096) is found from where the GPT header sits. Prints
"check: ..." lines and, last, "harness: bootcopy image check PASSED" / "harness: bootcopy removal
check PASSED", or "harness: ... FAILED: <why>". Exit 0 only when it passed.
"""
import os
import shutil
import struct
import subprocess
import sys
import tempfile

S = 512  # the disk's block size; FAT sectors are this size too (set by geometry() from the image)
START = 2048  # the EFI partition's first block (GPT entry 1)
EOC = 0x0FFFFFF8


class Bad(Exception):
    pass


def geometry(img):
    """S and START from the image: the GPT header at block 1 (byte 512 or 4096), entry 1's first block."""
    global S, START
    for bs in (512, 4096):
        if img[bs:bs + 8] == b"EFI PART":
            S = bs
            elba = struct.unpack_from("<Q", img, bs + 72)[0]
            START = struct.unpack_from("<Q", img, elba * bs + 32)[0]
            return
    raise Bad("no GPT header at block 1 (512 or 4096 bytes)")


def geo(img):
    bs = img[START * S:(START + 1) * S]
    _bps, spc, rsvd, nf = struct.unpack_from("<HBHB", bs, 11)
    tot, fsz = struct.unpack_from("<I", bs, 32)[0], struct.unpack_from("<I", bs, 36)[0]
    root, fsi = struct.unpack_from("<I", bs, 44)[0], struct.unpack_from("<H", bs, 48)[0]
    fat0 = START + rsvd
    return {"spc": spc, "nf": nf, "fsz": fsz, "fat0": fat0, "data0": fat0 + nf * fsz, "root": root,
            "cl": (tot - rsvd - nf * fsz) // spc, "end": START + tot, "fsi": START + fsi}


def fat(img, g, k=0):
    return struct.unpack_from("<%dI" % (g["cl"] + 2), img, (g["fat0"] + k * g["fsz"]) * S)


def clus(img, g, c):
    o = (g["data0"] + (c - 2) * g["spc"]) * S
    return img[o:o + g["spc"] * S]


def chain(F, c, g):
    out = []
    while True:
        if c < 2 or c > g["cl"] + 1 or len(out) > g["cl"]:
            raise Bad("a damaged cluster chain at %d" % c)
        out.append(c)
        n = F[c] & 0x0FFFFFFF
        if n >= EOC:
            return out
        if n == 0:
            raise Bad("a chain runs into a free cluster at %d" % c)
        c = n


def entries(img, g, F, c):
    """(name, attr, cluster, size, [(sector, offset) of its slots]) of a folder, long names checked."""
    out, lfn, slots = [], {}, []
    for k in chain(F, c, g):
        base = g["data0"] + (k - 2) * g["spc"]
        for q in range(g["spc"]):
            sec = img[(base + q) * S:(base + q + 1) * S]
            for o in range(0, S, 32):
                e = sec[o:o + 32]
                if e[0] == 0:
                    return out
                if e[0] == 0xE5:
                    lfn, slots = {}, []
                    continue
                if e[11] == 0x0F:
                    seq = e[0] & 0x1F
                    chars = [struct.unpack_from("<H", e, x)[0] for x in (1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30)]
                    lfn[seq] = (e[13], chars)
                    slots.append((base + q, o))
                    continue
                short = e[:11]
                s = 0
                for b in short:
                    s = ((s >> 1) | (s << 7)) + b & 0xFF
                name = None
                if lfn and all(v[0] == s for v in lfn.values()):
                    ch = []
                    for i in sorted(lfn):
                        ch += lfn[i][1]
                    ch = ch[:ch.index(0)] if 0 in ch else [x for x in ch if x != 0xFFFF]
                    name = "".join(chr(x) for x in ch)
                if name is None:
                    b, x = short[:8].decode("latin1").rstrip(), short[8:].decode("latin1").rstrip()
                    name = b + ("." + x if x else "")
                clu = struct.unpack_from("<H", e, 20)[0] << 16 | struct.unpack_from("<H", e, 26)[0]
                out.append((name, e[11], clu, struct.unpack_from("<I", e, 28)[0], slots + [(base + q, o)]))
                lfn, slots = {}, []
    return out


def find(ents, name):
    m = [e for e in ents if e[0].lower() == name.lower()]
    return m[0] if m else None


def fsinfo_free(img, g):
    o = g["fsi"] * S
    if struct.unpack_from("<I", img, o)[0] != 0x41615252 or struct.unpack_from("<I", img, o + 484)[0] != 0x61417272:
        raise Bad("FSInfo's signatures are gone")
    return struct.unpack_from("<I", img, o + 488)[0]


def changed(a, b):
    """Sector numbers where two images differ."""
    out = []
    for n in range(0, len(a), S * 2048):
        if a[n:n + S * 2048] != b[n:n + S * 2048]:
            for k in range(n, min(n + S * 2048, len(a)), S):
                if a[k:k + S] != b[k:k + S]:
                    out.append(k // S)
    return out


def fsck(img, g):
    """'clean' / 'errors' / None (no fsck.fat here) for the EFI partition."""
    tool = shutil.which("fsck.fat") or shutil.which("fsck.vfat")
    if not tool:
        return None
    with tempfile.NamedTemporaryFile(delete=False) as t:
        t.write(img[START * S:g["end"] * S])
    try:
        r = subprocess.run([tool, "-n", t.name], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        return "clean" if r.returncode == 0 else "errors: " + r.stdout.decode("latin1", "replace").strip().replace("\n", " | ")[:300]
    finally:
        os.unlink(t.name)


def efi_folder(img, g, F):
    e = find(entries(img, g, F, g["root"]), "EFI")
    if not e or not e[1] & 0x10:
        raise Bad("no \\EFI folder")
    return e[2]


def in_esp_meta(n, g, efi_secs):
    fat_end = g["fat0"] + g["nf"] * g["fsz"]
    return g["fat0"] <= n < fat_end or n == g["fsi"] or n in efi_secs


def check_copied(orig, now, boot):
    g = geo(orig)
    if len(orig) != len(now):
        raise Bad("the image changed size")
    F0, F = fat(orig, g), fat(now, g, 0)
    if F != fat(now, g, 1):
        raise Bad("the two FAT copies differ")
    print("check: both FAT copies equal")
    for c in range(2, g["cl"] + 2):
        if F0[c] != 0 and F[c] != F0[c]:
            raise Bad("cluster %d's FAT entry was in use and changed" % c)
    new = {c for c in range(2, g["cl"] + 2) if F0[c] == 0 and F[c] != 0}
    print("check: %d clusters taken, all of them free before; no FAT entry in use was changed" % len(new))
    efi = efi_folder(now, g, F)
    efi_secs = {g["data0"] + (k - 2) * g["spc"] + q for k in chain(F, efi, g) for q in range(g["spc"])}
    newsecs = {g["data0"] + (c - 2) * g["spc"] + q for c in new for q in range(g["spc"])}
    for n in changed(orig, now):
        if not (START <= n < g["end"]):
            raise Bad("sector %d outside the EFI partition changed" % n)
        if not (in_esp_meta(n, g, efi_secs) or n in newsecs):
            raise Bad("sector %d changed: not the FAT, FSInfo, \\EFI or a cluster that was free" % n)
    print("check: only the FAT, FSInfo, \\EFI's folder and clusters that were free changed; nothing outside the partition")
    old = entries(orig, g, F0, efi)
    ents = entries(now, g, F, efi)
    for e in old:
        if e not in ents:
            raise Bad("\\EFI's entry %s changed" % e[0])
    d = find(ents, "afrisovera")
    if not d or d[0] != "afrisovera" or not d[1] & 0x10 or len(d[4]) != 2:
        raise Bad("\\EFI\\afrisovera is not there as a folder with its long name")
    inner = entries(now, g, F, d[2])
    names = [e[0] for e in inner]
    if names[:2] != [".", ".."] or inner[0][2] != d[2] or inner[1][2] != efi:
        raise Bad("\\EFI\\afrisovera's . and .. are wrong")
    if names[2:] != ["afrisovera.efi"]:
        raise Bad("\\EFI\\afrisovera holds %r, not just afrisovera.efi" % names[2:])
    f = inner[2]
    ch = chain(F, f[2], g)
    data = b"".join(clus(now, g, c) for c in ch)
    cb = g["spc"] * S
    if f[3] != len(boot) or data[:len(boot)] != boot or len(ch) != -(-len(boot) // cb):
        raise Bad("afrisovera.efi is not the booted kernel byte for byte")
    if set(ch) | set(chain(F, d[2], g)) != new:
        raise Bad("the clusters taken are not exactly the file's and its folder's")
    print("check: \\EFI keeps Microsoft and BOOT unchanged; \\EFI\\afrisovera\\afrisovera.efi (long names) == the booted kernel, %d bytes" % len(boot))
    f0, f1 = fsinfo_free(orig, g), fsinfo_free(now, g)
    if f1 != f0 - len(new):
        raise Bad("FSInfo's free count is %d, not %d" % (f1, f0 - len(new)))
    print("check: FSInfo's free count %d -> %d" % (f0, f1))
    a, b = fsck(orig, g), fsck(now, g)
    if a is None:
        print("check: fsck.fat is not here (skipped)")
    elif a == "clean" and b != "clean":
        raise Bad("fsck.fat: " + b)
    else:
        print("check: fsck.fat on the partition: before %s, after %s" % (a, b))
    print("harness: bootcopy image check PASSED")


def check_removed(orig, mid, now):
    g = geo(orig)
    F0 = fat(orig, g)
    for k in range(g["nf"]):
        if fat(now, g, k) != F0:
            raise Bad("FAT copy %d is not as it was before bootcopy" % (k + 1))
    print("check: both FAT copies are byte for byte as before bootcopy")
    if now[g["fsi"] * S:(g["fsi"] + 1) * S] != orig[g["fsi"] * S:(g["fsi"] + 1) * S]:
        raise Bad("FSInfo is not as it was before bootcopy")
    print("check: FSInfo as before bootcopy")
    Fm = fat(mid, g)
    efi = efi_folder(now, g, F0)
    efi_secs = {g["data0"] + (k - 2) * g["spc"] + q for k in chain(F0, efi, g) for q in range(g["spc"])}
    d = find(entries(mid, g, Fm, efi), "afrisovera")
    if not d:
        raise Bad("the image after bootcopy has no \\EFI\\afrisovera")
    for n in changed(mid, now):
        if not in_esp_meta(n, g, efi_secs):
            raise Bad("sector %d changed by the removal: not the FAT, FSInfo or \\EFI" % n)
    for (n, o) in d[4]:
        if now[n * S + o] != 0xE5 or now[n * S + o + 1:n * S + o + 32] != mid[n * S + o + 1:n * S + o + 32]:
            raise Bad("\\EFI\\afrisovera's slot at sector %d+%d is not just marked deleted" % (n, o))
    for n in efi_secs:
        for o in range(0, S, 32):
            if (n, o) not in d[4] and now[n * S + o:n * S + o + 32] != mid[n * S + o:n * S + o + 32]:
                raise Bad("another \\EFI slot changed (sector %d+%d)" % (n, o))
    if entries(now, g, F0, efi) != entries(orig, g, F0, efi):
        raise Bad("\\EFI does not list what it listed before bootcopy")
    print("check: \\EFI's two slots for afrisovera marked deleted, nothing else changed; \\EFI lists what it did before")
    a, b = fsck(orig, g), fsck(now, g)
    if a is None:
        print("check: fsck.fat is not here (skipped)")
    elif a == "clean" and b != "clean":
        raise Bad("fsck.fat: " + b)
    else:
        print("check: fsck.fat on the partition: before %s, after %s" % (a, b))
    print("harness: bootcopy removal check PASSED")


def main():
    mode = sys.argv[1]
    rd = lambda p: open(p, "rb").read()
    try:
        geometry(rd(sys.argv[2]))
        if mode == "copied":
            check_copied(rd(sys.argv[2]), rd(sys.argv[3]), rd(sys.argv[4]))
        else:
            check_removed(rd(sys.argv[2]), rd(sys.argv[3]), rd(sys.argv[4]))
    except (Bad, struct.error, IndexError, ValueError) as e:
        print("harness: bootcopy %s check FAILED: %s" % ("image" if mode == "copied" else "removal", e))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
