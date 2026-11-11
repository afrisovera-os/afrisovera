#!/usr/bin/env python3
"""exec-split: move whole arms of `shell::exec`'s `match cmd` out into `#[inline(never)]` functions.

The exec split (ARCHITECT_REVIEWS 11 Nov 2026): at opt-level 0 every arm's locals and format temporaries get their
own stack slots in exec's one frame (590 KiB debug). A moved arm's slots live in its own function's frame, which
exists only while that command runs. The body moves UNCHANGED (same text, re-indented); the arm becomes one call.

  python3 scripts/lib/exec-split.py --list            # every arm: lines, name, first line
  python3 scripts/lib/exec-split.py help net sort ...  # move these arms (by first pattern name)
  python3 scripts/lib/exec-split.py --top 20           # move the 20 longest arms not yet moved

Moved functions go to kernel/src/shell_exec_cmds.rs, pulled into shell.rs with `include!` (the shell_arch.rs
pattern), so they sit in the same module with the same names in scope on both kernels (the PC includes shell.rs by
`#[path]`, where a child `mod` would resolve to a different directory).

Refused (left in place, reported): arms with an `if` guard, arms preceded by an attribute (`#[cfg]`), arms that are
not `pattern => {` ... `}` blocks, and the `_` fallback. Behaviour is unchanged by construction: nothing in exec
follows the match, so a `return` in a moved body ends exec exactly as before.
"""
import re
import sys

SHELL = "kernel/src/shell.rs"
CMDS = "kernel/src/shell_exec_cmds.rs"
INCLUDE = 'include!("shell_exec_cmds.rs");'
STR = re.compile(r'"(\\.|[^"\\])*"')
CHR = re.compile(r"'(\\.|[^'\\])'")
ARM = re.compile(r'^(\s*)((?:"[^"]*"(?:\s*\|\s*"[^"]*")*|_))\s*(if\s.*)?=>\s*(.*)$')


def braces(s):
    s = STR.sub('""', s)
    s = CHR.sub("''", s)
    s = s.split("//")[0]
    return s.count("{") - s.count("}")


def arms(L):
    ex = next(i for i, l in enumerate(L) if l.startswith("pub(crate) fn exec(line: &str)"))
    start = next(i for i in range(ex, ex + 40) if L[i].strip() == "match cmd {")
    out, d, i = [], 0, start
    while True:
        line = L[i]
        if d == 1:
            m = ARM.match(line)
            if m:
                # the arm's own extent: from its pattern line to the line closing its block
                j, dd = i, 0
                while True:
                    dd += braces(L[j])
                    if dd <= 0 and (j > i or not m.group(4).endswith("{")):
                        break
                    j += 1
                names = re.findall(r'"([^"]*)"', m.group(2))
                prev = L[i - 1].strip()
                out.append(dict(name=names[0] if names else "_", a=i, b=j, indent=m.group(1),
                                pat=m.group(2), guard=m.group(3), rhs=m.group(4),
                                attr=prev.startswith("#["), lines=j - i + 1))
        d += braces(line)
        if i > start and d == 0:
            return out, i
        i += 1


def ident(name):
    s = re.sub(r"[^0-9A-Za-z_]", "_", name)
    return "cmd_" + s


def why_not(a, L):
    if a["name"] == "_":
        return "the fallback arm"
    if a["guard"]:
        return "has an if guard"
    if a["attr"]:
        return "preceded by an attribute"
    if a["rhs"].strip() != "{" or L[a["b"]].strip() != "}":
        return "not a `pattern => { ... }` block"
    if re.search(r"=> cmd_[0-9A-Za-z_]+\(line, cmd, rest\),$", L[a["a"]]):
        return "already moved"
    return None


def undo():
    """Put every moved body back in its arm: the exact inverse of a move. Run on a split tree, it must give the
    base shell.rs byte for byte (that is the proof the bodies moved unchanged), and lets one cloud job build the
    base and the branch side by side for a like-for-like md5."""
    if not _exists(CMDS):
        print("exec-split: nothing to undo")
        return 0
    src = open(CMDS, encoding="utf-8").read()
    fns = {}
    for m in re.finditer(r"#\[allow\(unused_variables\)\]\nfn (cmd_[0-9A-Za-z_]+)\(line: &str, cmd: &str, rest: &str\) \{\n(.*?)\n\}\n",
                         src, re.S):
        fns[m.group(1)] = m.group(2).split("\n")
    L = open(SHELL, encoding="utf-8").read().split("\n")
    out = []
    call = re.compile(r"^(\s*)(.*) => (cmd_[0-9A-Za-z_]+)\(line, cmd, rest\)(,)$")
    for l in L:
        m = call.match(l)
        if m and m.group(3) in fns:
            ind = m.group(1)
            out.append("%s%s => {" % (ind, m.group(2)))
            out.extend((ind + b) if b.strip() else b for b in fns.pop(m.group(3)))
            out.append(ind + "}")
        elif l == INCLUDE:
            continue
        else:
            out.append(l)
    if fns:
        print("exec-split: undo found no call for %s" % sorted(fns))
        return 1
    open(SHELL, "w", encoding="utf-8", newline="\n").write("\n".join(out))
    import os
    os.remove(CMDS)
    print("exec-split: undone")
    return 0


def main(argv):
    if argv[:1] == ["--undo"]:
        return undo()
    L = open(SHELL, encoding="utf-8").read().split("\n")
    al, end = arms(L)
    if argv[:1] == ["--list"]:
        for a in sorted(al, key=lambda a: -a["lines"]):
            print("%5d  %-24s %s" % (a["lines"], a["name"], why_not(a, L) or ""))
        print("%d arms, %d lines in the match" % (len(al), end - al[0]["a"]))
        return 0
    if argv[:1] == ["--top"]:
        n = int(argv[1])
        pick = [a for a in sorted(al, key=lambda a: -a["lines"]) if not why_not(a, L)][:n]
    else:
        by = {a["name"]: a for a in al}
        pick = []
        for name in argv:
            a = by.get(name)
            if not a:
                print("exec-split: no arm named %r" % name)
                return 1
            pick.append(a)
    moved, bodies = [], []
    taken = set(re.findall(r"fn (cmd_[0-9A-Za-z_]+)\(", open(CMDS, encoding="utf-8").read()) if _exists(CMDS) else [])
    for a in pick:
        w = why_not(a, L)
        if w:
            print("exec-split: left %-20s (%s)" % (a["name"], w))
            continue
        fn = ident(a["name"])
        if fn in taken:
            print("exec-split: left %-20s (function name %s already used)" % (a["name"], fn))
            continue
        taken.add(fn)
        body = L[a["a"] + 1:a["b"]]
        # arm bodies sit at the arm's indent + 4; a function body at 4
        cut = len(a["indent"])
        body = [(l[cut:] if l[:cut].strip() == "" else l) for l in body]
        bodies.append("/// `%s` (moved out of `exec` unchanged: the exec split)\n#[inline(never)]\n"
                      "#[allow(unused_variables)]\nfn %s(line: &str, cmd: &str, rest: &str) {\n%s\n}\n"
                      % (a["pat"].replace('"', ""), fn, "\n".join(body)))
        moved.append((a, "%s%s => %s(line, cmd, rest)," % (a["indent"], a["pat"], fn)))
    # rewrite bottom-up so indices stay valid
    for a, call in sorted(moved, key=lambda x: -x[0]["a"]):
        L[a["a"]:a["b"] + 1] = [call]
    text = "\n".join(L)
    if INCLUDE not in text:
        text = text.replace('include!("shell_arch.rs");', 'include!("shell_arch.rs");\n' + INCLUDE, 1)
    open(SHELL, "w", encoding="utf-8", newline="\n").write(text)
    head = ("// Arms of `shell::exec` moved into functions of their own (the exec split, ARCHITECT_REVIEWS 11 Nov 2026).\n"
            "// Generated by scripts/lib/exec-split.py: each body is the arm's text unchanged. Included into shell.rs.\n")
    old = open(CMDS, encoding="utf-8").read() if _exists(CMDS) else head
    open(CMDS, "w", encoding="utf-8", newline="\n").write(old + "\n" + "\n".join(bodies))
    print("exec-split: moved %d arm(s), %d line(s)" % (len(moved), sum(a["lines"] for a, _ in moved)))
    return 0


def _exists(p):
    try:
        open(p).close()
        return True
    except OSError:
        return False


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
