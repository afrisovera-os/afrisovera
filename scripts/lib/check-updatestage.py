#!/usr/bin/env python3
"""Checks a disk image after `updatestage` (suite pc-updatestage, safe updates U3, AFR-0010 section 8).

    check-updatestage.py staged   FRESH NOW KERNEL SPARE BOOTFILE   # after a stage: SPARE holds BOOTFILE
    check-updatestage.py unstaged FRESH NOW KERNEL SPARE            # after updatestage remove: SPARE is gone
    check-updatestage.py same     A B                               # nothing changed (a refusal)

FRESH is the disk before AfriSovera came; NOW the disk to check; KERNEL the boot file bootcopy put in
\\EFI\\afrisovera\\afrisovera.efi. Every check:
- the disk changed only inside its EFI partition, and every file that was there before (Windows' and the others')
  reads byte for byte as it did;
- afrisovera.efi is the kernel, byte for byte;
- the whole partition is consistent: both FAT copies equal, every chain as long as its file, no two chains sharing a
  cluster, no cluster taken that nothing names, every long name's checksum and sequence right, FSInfo's free count
  exact;
- `fsck.fat -n` on the partition reports no errors (REQUIRED: the suite fails without it, the architect's rule).
Prints "check: ..." lines and, last, "harness: updatestage image check PASSED" or "... FAILED: <why>".
"""
import importlib.util
import os
import struct
import sys

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cb", os.path.join(here, "check-bootcopy.py"))
cb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cb)
Bad = cb.Bad


def walk(img, g, F):
    """Every live file and folder: {path: (attr, first cluster, size)}; long-name pieces checked on the way."""
    out = {}

    def folder(c, path, depth):
        if depth > 16:
            raise Bad("folders nested too deep")
        for name, attr, clu, size, slots in cb.entries(img, g, F, c):
            if attr & 0x08 or name in (".", ".."):
                continue
            p = path + "\\" + name
            out[p.lower()] = (attr, clu, size)
            if attr & 0x10 and clu >= 2:
                folder(clu, p, depth + 1)
    folder(g["root"], "", 0)
    return out


def lfn_ok(img, g, F, c, depth=0):
    """Every long name in the folder at `c` (and below): pieces numbered down to 1, the last one flagged 0x40,
    each carrying its 8.3 entry's checksum."""
    run = []
    for k in cb.chain(F, c, g):
        base = g["data0"] + (k - 2) * g["spc"]
        for q in range(g["spc"]):
            sec = img[(base + q) * cb.S:(base + q + 1) * cb.S]
            for o in range(0, cb.S, 32):
                e = sec[o:o + 32]
                if e[0] == 0:
                    return
                if e[0] == 0xE5:
                    run = []
                    continue
                if e[11] == 0x0F:
                    run.append((e[0], e[13]))
                    continue
                if run:
                    s = 0
                    for b in e[:11]:
                        s = ((s >> 1) | (s << 7)) + b & 0xFF
                    n = len(run)
                    want = [(n - i) | (0x40 if i == 0 else 0) for i in range(n)]
                    if [r[0] for r in run] != want or any(r[1] != s for r in run):
                        raise Bad("a long name's pieces are wrong before %r" % bytes(e[:11]))
                run = []
                if e[11] & 0x10 and e[0] != ord(".") and not e[11] & 0x08:
                    sub = struct.unpack_from("<H", e, 20)[0] << 16 | struct.unpack_from("<H", e, 26)[0]
                    if sub >= 2 and depth < 16:
                        lfn_ok(img, g, F, sub, depth + 1)


def consistent(img, g, cut=False):
    """cut: after a power cut, clusters nothing names (an orphan) and a stale FSInfo are allowed; their count is
    returned as the third value."""
    F = cb.fat(img, g)
    if g["nf"] == 2 and cb.fat(img, g, 1) != F:
        raise Bad("the two FAT copies differ")
    files = walk(img, g, F)
    owner = {}
    for c in cb.chain(F, g["root"], g):
        owner[c] = "\\"
    for p, (attr, clu, size) in files.items():
        if clu < 2:
            if size and not attr & 0x10:
                raise Bad("%s has a size but no cluster" % p)
            continue
        ch = cb.chain(F, clu, g)
        if not attr & 0x10:
            want = max(1, -(-size // (g["spc"] * cb.S)))
            if len(ch) != want:
                raise Bad("%s: chain of %d clusters for %d bytes" % (p, len(ch), size))
        for c in ch:
            if c in owner:
                raise Bad("%s and %s share cluster %d" % (owner[c], p, c))
            owner[c] = p
    lost = [c for c in range(2, g["cl"] + 2) if F[c] & 0x0FFFFFFF and c not in owner]
    if lost and not cut:
        raise Bad("%d cluster(s) taken that nothing names (first %d)" % (len(lost), lost[0]))
    lfn_ok(img, g, F, g["root"])
    free = sum(1 for c in range(2, g["cl"] + 2) if F[c] & 0x0FFFFFFF == 0)
    fi = cb.fsinfo_free(img, g)
    if fi != 0xFFFFFFFF and fi != free and not cut:
        raise Bad("FSInfo says %d free clusters, the FAT %d" % (fi, free))
    if cut:
        print("check: after the cut: FATs equal, %d files and folders, no shared clusters, long names right, %d cluster(s) nothing names" % (len(files), len(lost)))
    else:
        print("check: the partition is consistent (FATs equal, %d files and folders, no shared or lost clusters, long names right, %d clusters free)" % (len(files), free))
    return files, F, len(lost)


def read(img, g, F, clu, size):
    return b"".join(cb.clus(img, g, c) for c in cb.chain(F, clu, g))[:size]


def fsck_required(img, g):
    r = cb.fsck(img, g)
    if r is None:
        raise Bad("fsck.fat is not on this machine (the suite needs dosfstools)")
    if r != "clean":
        raise Bad("fsck.fat: " + r)
    print("check: fsck.fat -n on the partition: no errors")


def outside_same(fresh, now, g):
    lo, hi = cb.START * cb.S, g["end"] * cb.S
    if len(fresh) != len(now) or fresh[:lo] != now[:lo] or fresh[hi:] != now[hi:]:
        raise Bad("the disk changed OUTSIDE its EFI partition")
    print("check: nothing outside the EFI partition changed")


def others_same(fresh, now, g, files):
    Ff = cb.fat(fresh, g)
    before = walk(fresh, g, Ff)
    n = 0
    for p, (attr, clu, size) in before.items():
        if attr & 0x10:
            continue
        if p not in files:
            raise Bad("%s is gone" % p)
        a2, c2, s2 = files[p]
        if (c2, s2) != (clu, size) or read(now, g, cb.fat(now, g), c2, s2) != read(fresh, g, Ff, clu, size):
            raise Bad("%s CHANGED" % p)
        n += 1
    print("check: the %d files that were there before AfriSovera read byte for byte as they did" % n)


def set_dirty(path, dirty):
    """The EFI partition's "closed cleanly" bit (bit 27 of FAT entry 1) cleared or set, in every FAT copy."""
    img = bytearray(open(path, "rb").read())
    cb.geometry(img)
    g = cb.geo(img)
    for k in range(g["nf"]):
        o = (g["fat0"] + k * g["fsz"]) * cb.S + 4
        v = struct.unpack_from("<I", img, o)[0]
        v = v & ~0x08000000 if dirty else v | 0x08000000
        struct.pack_into("<I", img, o, v)
    open(path, "wb").write(img)
    print("harness: the EFI partition marked %s" % ("NOT closed cleanly" if dirty else "closed cleanly again"))


def fill(path, keep_kib):
    """A file \\FILLER.BIN in the EFI partition's top folder taking every free cluster but `keep_kib` KiB, written
    as a FAT driver would (data left as it is, both FATs, FSInfo, the entry), so the partition stays consistent."""
    img = bytearray(open(path, "rb").read())
    cb.geometry(img)
    g = cb.geo(img)
    F = list(cb.fat(img, g))
    cbytes = g["spc"] * cb.S
    free = [c for c in range(2, g["cl"] + 2) if F[c] & 0x0FFFFFFF == 0]
    n = len(free) - -(-keep_kib * 1024 // cbytes)
    if n < 1:
        raise Bad("not enough free room to fill")
    take = free[:n]
    for i, c in enumerate(take):
        F[c] = (F[c] & 0xF0000000) | (take[i + 1] if i + 1 < n else 0x0FFFFFFF)
    for k in range(g["nf"]):
        struct.pack_into("<%dI" % len(F), img, (g["fat0"] + k * g["fsz"]) * cb.S, *F)
    o = g["fsi"] * cb.S + 488
    if struct.unpack_from("<I", img, o)[0] != 0xFFFFFFFF:
        struct.pack_into("<I", img, o, len(free) - n)
    root = g["data0"] + (g["root"] - 2) * g["spc"]
    for q in range(g["spc"] * cb.S // 32):
        e = (root * cb.S) + q * 32
        if img[e] in (0, 0xE5):
            d = bytearray(32)
            d[:11] = b"FILLER  BIN"
            d[11] = 0x20
            struct.pack_into("<H", d, 26, take[0] & 0xFFFF)
            struct.pack_into("<H", d, 20, take[0] >> 16)
            struct.pack_into("<I", d, 28, n * cbytes)
            img[e:e + 32] = d
            break
    else:
        raise Bad("no free slot in the top folder")
    open(path, "wb").write(img)
    print("harness: \\FILLER.BIN takes %d KiB; %d KiB left free in the EFI partition" % (n * cbytes // 1024, (len(free) - n) * cbytes // 1024))


def plant_cut(path, n):
    """The power cut the suite found (14 Nov 2026), made exactly: in \\EFI\\afrisovera a deleted entry of ours
    (0xE5 + "FRISO~7EFI", a file) names a chain of `n` allocated clusters whose last one links to a FREE cluster, as
    when take() linked one FAT sector and the cut came before the next was written. FSInfo is left as it was (stale,
    as after a real cut). The named case pc-updatestage-poweroff's reclaim must free."""
    img = bytearray(open(path, "rb").read())
    cb.geometry(img)
    g = cb.geo(img)
    F = list(cb.fat(img, g))
    free = [c for c in range(2, g["cl"] + 2) if F[c] & 0x0FFFFFFF == 0]
    if len(free) < n + 1:
        raise Bad("not enough free clusters to plant the cut")
    chain = free[:n + 1]  # the last one stays free: the link runs into it
    for i in range(n):
        F[chain[i]] = (F[chain[i]] & 0xF0000000) | chain[i + 1]
    for k in range(g["nf"]):
        struct.pack_into("<%dI" % len(F), img, (g["fat0"] + k * g["fsz"]) * cb.S, *F)
    efi = cb.find(cb.entries(img, g, F, g["root"]), "EFI")
    ours = cb.find(cb.entries(img, g, F, efi[2]), "afrisovera")
    if not ours:
        raise Bad("no \\EFI\\afrisovera to plant in")
    base = g["data0"] + (ours[2] - 2) * g["spc"]
    for q in range(g["spc"] * cb.S // 32):
        e = base * cb.S + q * 32
        if img[e] == 0:  # past the folder's end: the entry and the end mark after it stay valid
            d = bytearray(32)
            d[:11] = b"\xe5FRISO~7EFI"
            d[11] = 0x20
            struct.pack_into("<H", d, 20, chain[0] >> 16)
            struct.pack_into("<H", d, 26, chain[0] & 0xFFFF)
            struct.pack_into("<I", d, 28, (n + 4) * g["spc"] * cb.S)
            img[e:e + 32] = d
            break
    else:
        raise Bad("no free slot in \\EFI\\afrisovera")
    open(path, "wb").write(img)
    print("harness: planted the free-cluster cut: %d clusters from %d, the last linking to free cluster %d" % (n, chain[0], chain[n]))


def main():
    a = sys.argv[1:]
    try:
        if a and a[0] == "plant-cut" and len(a) == 3:
            plant_cut(a[1], int(a[2]))
            return 0
        if a and a[0] == "dirty" and len(a) == 3:
            set_dirty(a[1], a[2] == "on")
            return 0
        if a and a[0] == "fill" and len(a) == 3:
            fill(a[1], int(a[2]))
            return 0
        if a and a[0] == "same" and len(a) == 3:
            x, y = open(a[1], "rb").read(), open(a[2], "rb").read()
            if x != y:
                raise Bad("the disk CHANGED")
            print("check: the disk is byte for byte as before")
            print("harness: updatestage image check PASSED")
            return 0
        if a and a[0] == "cut" and len(a) == 5:
            fresh, now, kernel = open(a[1], "rb").read(), open(a[2], "rb").read(), open(a[3], "rb").read()
            cb.geometry(now)
            g = cb.geo(now)
            outside_same(fresh, now, g)
            if g["nf"] == 2 and cb.fat(now, g, 1) != cb.fat(now, g):
                print("harness: cut state: FATs differ")
                return 0
            files, F, lost = consistent(now, g, cut=True)
            others_same(fresh, now, g, files)
            m = files.get("\\efi\\afrisovera\\afrisovera.efi")
            if not m or read(now, g, F, m[1], m[2]) != kernel:
                raise Bad("afrisovera.efi is not the kernel any more")
            print("check: \\EFI\\afrisovera\\afrisovera.efi is the running kernel, byte for byte")
            if ("\\efi\\afrisovera\\" + a[4]).lower() in files:
                raise Bad("%s is live after the cut (its entry is written last, so it must not be)" % a[4])
            print("check: no live %s: nothing can boot a half-written file" % a[4])
            print("harness: cut state: %d lost clusters" % lost)
            return 0
        if not a or a[0] not in ("staged", "unstaged") or len(a) != (6 if a[0] == "staged" else 5):
            print(__doc__)
            return 2
        fresh, now, kernel = open(a[1], "rb").read(), open(a[2], "rb").read(), open(a[3], "rb").read()
        spare = a[4]
        cb.geometry(now)
        g = cb.geo(now)
        outside_same(fresh, now, g)
        files, F, _ = consistent(now, g)
        others_same(fresh, now, g, files)
        main_f = files.get("\\efi\\afrisovera\\afrisovera.efi")
        if not main_f or read(now, g, F, main_f[1], main_f[2]) != kernel:
            raise Bad("afrisovera.efi is not the kernel bootcopy put there")
        print("check: \\EFI\\afrisovera\\afrisovera.efi is the running kernel, byte for byte")
        sp = files.get(("\\efi\\afrisovera\\" + spare).lower())
        if a[0] == "staged":
            want = open(a[5], "rb").read()
            if not sp:
                raise Bad("%s is not there" % spare)
            if read(now, g, F, sp[1], sp[2]) != want:
                raise Bad("%s is not the update's boot file" % spare)
            print("check: \\EFI\\afrisovera\\%s is the update's boot file, byte for byte (%d bytes)" % (spare, len(want)))
        else:
            if sp:
                raise Bad("%s is still there" % spare)
            print("check: \\EFI\\afrisovera\\%s is gone" % spare)
        mine = sorted(p for p in files if p.startswith("\\efi\\afrisovera\\"))
        print("check: \\EFI\\afrisovera holds: %s" % ", ".join(p.rsplit("\\", 1)[1] for p in mine))
        fsck_required(now, g)
        print("harness: updatestage image check PASSED")
        return 0
    except Bad as e:
        print("harness: updatestage image check FAILED: %s" % e)
        return 1


if __name__ == "__main__":
    sys.exit(main())
