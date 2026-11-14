#!/usr/bin/env python3
"""Selftest for tools/report-receiver.py: the --from-stick copy and the --out git guard.

Runs the real receiver as a subprocess against temporary folders (never inside a git tree,
apart from the small git repos this test makes itself). Standard library only; works with
python on Windows and python3 on Linux.

    python scripts/lib/report-receiver-selftest.py

Each case prints "  arm <name>: ok" or "  arm <name>: FAIL <why>"; the last line is
"receiver selftest: <ok> ok / <fail> FAIL" and the exit status is 1 on any FAIL.
"""
import datetime
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
RECEIVER = os.path.join(os.path.dirname(os.path.dirname(HERE)), "tools", "report-receiver.py")
WHEN = datetime.datetime(2026, 1, 2, 3, 4, 5).timestamp()
STAMP = datetime.datetime.fromtimestamp(WHEN).strftime("%Y%m%d-%H%M%S")

results = {"ok": 0, "fail": 0}


def arm(name, problem):
    if problem:
        results["fail"] += 1
        print("  arm %s: FAIL %s" % (name, problem))
    else:
        results["ok"] += 1
        print("  arm %s: ok" % name)


def test_env():
    """On Windows the temp folder sits on the system drive, which the receiver rightly refuses
    as a source. For the copy arms, tell the child another letter is the system drive."""
    env = dict(os.environ)
    if os.name == "nt":
        temp_drive = os.path.splitdrive(tempfile.gettempdir())[0].upper()
        other = "Q:" if temp_drive != "Q:" else "R:"
        env["SystemDrive"] = other
        env["WINDIR"] = other + "\\Windows"
    return env


def run(args, env=None):
    p = subprocess.run(
        [sys.executable, RECEIVER] + args,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        env=env if env is not None else test_env(), timeout=120,
    )
    return p.returncode, p.stdout.decode("utf-8", "replace")


def put(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    os.utime(path, (WHEN, WHEN))


def snapshot(top):
    """Every entry under top with its bytes and modification time."""
    seen = []
    for d, dirs, files in os.walk(top):
        dirs.sort()
        for n in sorted(dirs + files):
            p = os.path.join(d, n)
            st = os.lstat(p)
            data = None
            if os.path.isfile(p) and not os.path.islink(p):
                with open(p, "rb") as f:
                    data = f.read()
            seen.append((os.path.relpath(p, top), st.st_mtime, data))
    return seen


def listing(folder):
    return sorted(os.listdir(folder)) if os.path.isdir(folder) else []


def git_ok():
    try:
        return subprocess.run(["git", "--version"], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=60).returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


def git_init(folder):
    subprocess.run(["git", "init", "-q", folder], check=True, timeout=60,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    print("report-receiver selftest (%s)" % RECEIVER)
    work = tempfile.mkdtemp(prefix="afri-receiver-")
    try:
        cases(work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print("receiver selftest: %d ok / %d FAIL" % (results["ok"], results["fail"]))
    return 1 if results["fail"] else 0


def cases(work):
    # good reports, copied once
    stick = os.path.join(work, "stick1")
    out = os.path.join(work, "out1")
    one = b"== AfriSovera field report ==\nline two\n"
    two = b"second report\r\nmore\n"
    put(os.path.join(stick, "AFREPORT", "1.TXT"), one)
    put(os.path.join(stick, "AFREPORT", "2.TXT"), two)
    before = snapshot(stick)
    rc, log = run(["--from-stick", stick, "--out", out])
    want = ["%s-stick-1.txt" % STAMP, "%s-stick-2.txt" % STAMP]
    p = None
    if rc != 0:
        p = "exit %d: %s" % (rc, log)
    elif listing(out) != want:
        p = "out has %s, wanted %s" % (listing(out), want)
    else:
        for name, data in zip(want, (one, two)):
            with open(os.path.join(out, name), "rb") as f:
                if f.read() != data:
                    p = "%s bytes differ" % name
        if "Copied 2 report(s)" not in log or "first line: == AfriSovera field report ==" not in log:
            p = "summary or first line missing: %s" % log
    arm("two-good", p)

    # second run copies nothing new
    rc, log = run(["--from-stick", stick, "--out", out])
    p = None
    if rc != 0:
        p = "exit %d: %s" % (rc, log)
    elif "already copied" not in log or "Copied 0 report(s)" not in log or "2 already copied" not in log:
        p = "wrong output: %s" % log
    elif listing(out) != want:
        p = "out changed: %s" % listing(out)
    arm("second-run", p)

    # the stick is unchanged
    arm("stick-unchanged", None if snapshot(stick) == before else "the stick changed")

    # lowercase folder and name
    stick = os.path.join(work, "stick2")
    out = os.path.join(work, "out2")
    put(os.path.join(stick, "afreport", "3.txt"), b"lower\n")
    rc, log = run(["--from-stick", stick, "--out", out])
    want = ["%s-stick-3.txt" % STAMP]
    arm("lowercase", None if rc == 0 and listing(out) == want else "exit %d, out %s: %s" % (rc, listing(out), log))

    # acpisave's saved tables: 4.BIN copied as stick-4.bin, byte for byte; 1.BIN.TXT and 4.BINX are not names it takes
    stick = os.path.join(work, "stick2b")
    out = os.path.join(work, "out2b")
    blob = b"AFRISOVERA ACPI TABLES 1\n" + bytes(range(256))
    put(os.path.join(stick, "AFREPORT", "4.BIN"), blob)
    put(os.path.join(stick, "AFREPORT", "1.BIN.TXT"), b"no\n")
    put(os.path.join(stick, "AFREPORT", "4.BINX"), b"no\n")
    rc, log = run(["--from-stick", stick, "--out", out])
    want = ["%s-stick-4.bin" % STAMP]
    same = listing(out) == want and open(os.path.join(out, want[0]), "rb").read() == blob
    arm("tables-bin", None if rc == 0 and same and "Ignored 2 other name(s)" in log else "exit %d, out %s: %s" % (rc, listing(out), log))

    # names that are not reports
    stick = os.path.join(work, "stick3")
    out = os.path.join(work, "out3")
    for n in ("A.TXT", "1.TXT.EXE", "123456.TXT", "1.TX"):
        put(os.path.join(stick, "AFREPORT", n), b"not a report\n")
    os.makedirs(os.path.join(stick, "AFREPORT", "9.TXT"))
    rc, log = run(["--from-stick", stick, "--out", out])
    p = None
    if rc != 1:
        p = "exit %d (wanted 1): %s" % (rc, log)
    elif listing(out):
        p = "copied %s" % listing(out)
    elif "Ignored 5 other name(s)" not in log or "no reports in" not in log:
        p = "wrong output: %s" % log
    arm("ignored-names", p)

    # a file over --max-bytes is skipped, not cut
    stick = os.path.join(work, "stick4")
    out = os.path.join(work, "out4")
    put(os.path.join(stick, "AFREPORT", "4.TXT"), b"x" * 2000)
    put(os.path.join(stick, "AFREPORT", "5.TXT"), b"small\n")
    rc, log = run(["--from-stick", stick, "--out", out, "--max-bytes", "1000"])
    want = ["%s-stick-5.txt" % STAMP]
    p = None
    if rc != 0 or listing(out) != want:
        p = "exit %d, out %s: %s" % (rc, listing(out), log)
    elif "Skipped 4.TXT" not in log or "1 skipped" not in log:
        p = "no skip line: %s" % log
    arm("too-big", p)

    # a link inside AFREPORT is skipped
    stick = os.path.join(work, "stick5")
    out = os.path.join(work, "out5")
    secret = os.path.join(work, "secret.txt")
    put(secret, b"not on the stick\n")
    put(os.path.join(stick, "AFREPORT", "7.TXT"), b"real\n")
    try:
        os.symlink(secret, os.path.join(stick, "AFREPORT", "6.TXT"))
        linked = True
    except (OSError, NotImplementedError, AttributeError):
        linked = False
    if not linked:
        print("  arm symlink: skipped (no symlink rights)")
    else:
        rc, log = run(["--from-stick", stick, "--out", out])
        want = ["%s-stick-7.txt" % STAMP]
        p = None
        if rc != 0 or listing(out) != want:
            p = "exit %d, out %s: %s" % (rc, listing(out), log)
        elif "Skipped 6.TXT" not in log:
            p = "no skip line: %s" % log
        arm("symlink", p)

    # no AFREPORT folder
    stick = os.path.join(work, "stick6")
    os.makedirs(os.path.join(stick, "OTHER"))
    rc, log = run(["--from-stick", stick, "--out", os.path.join(work, "out6")])
    arm("no-folder", None if rc == 1 and "no AFREPORT folder on" in log else "exit %d: %s" % (rc, log))

    # --out inside a git tree
    good = os.path.join(work, "stick1")
    if not git_ok():
        print("  arm git-not-ignored: skipped (git is not installed)")
        print("  arm git-ignored: skipped (git is not installed)")
    else:
        repo = os.path.join(work, "repo1")
        git_init(repo)
        out = os.path.join(repo, "reports")
        rc, log = run(["--from-stick", good, "--out", out])
        p = None
        if rc != 1 or "not ignored; reports must never be committed" not in log:
            p = "exit %d: %s" % (rc, log)
        elif os.path.exists(out):
            p = "it made %s" % out
        arm("git-not-ignored", p)

        repo = os.path.join(work, "repo2")
        git_init(repo)
        with open(os.path.join(repo, ".gitignore"), "w") as f:
            f.write("reports/\n")
        out = os.path.join(repo, "reports")
        rc, log = run(["--from-stick", good, "--out", out])
        arm("git-ignored", None if rc == 0 and len(listing(out)) == 2 else "exit %d: %s" % (rc, log))

    # the system drive (Windows) or / (elsewhere) is refused as a source
    if os.name == "nt":
        src = os.environ.get("SystemDrive", "C:")
    else:
        src = "/"
    rc, log = run(["--from-stick", src, "--out", os.path.join(work, "out7")], env=dict(os.environ))
    p = None
    if rc != 1 or "Refused" not in log:
        p = "exit %d: %s" % (rc, log)
    elif listing(os.path.join(work, "out7")):
        p = "it copied something"
    arm("system-source", p)


if __name__ == "__main__":
    sys.exit(main())
