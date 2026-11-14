"""Drive AfriSh with timed raw keys and screenshots while a Nia app owns a cell window (afriui.md section 25,
the desk-cell-app suite). The steps are poll-keys.py's, plus waiting for text and a screenshot.

Usage: cell-keys.py <serial_port> <qmp_port> <shot dir> STEP ...

  CMD      type CMD and wait for the prompt
  &CMD     type CMD and do not wait (the app keeps running)
  ~SECS    wait SECS seconds, still reading the console
  :HEX     send these bytes raw, all at once (hex, e.g. :61 or :1c1c)
  @TEXT    wait (up to 60 s) until TEXT is in what the guest said since the last &CMD
  !NAME    save a screenshot of the screen as <shot dir>/NAME.png (QMP screendump)
  =        wait for the prompt that ends the last &CMD
  %KEYS    press keys on the machine's own keyboard through QMP send-key (e.g. %shift-pgup, %ret): PS/2 on the PC
  ^N       N notches of the mouse wheel through QMP (negative: away from the person, back through text)

Everything the guest says goes to stdout, then one `cell-keys:` line per timed step.
"""
import importlib.util
import json
import os
import socket
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("shell_demo", os.path.join(HERE, "..", "shell-demo.py"))
shell_demo = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shell_demo)

PROMPT = shell_demo.PROMPT
BOOT_CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "120"))
CMD_CEILING = float(os.environ.get("AFRISH_CMD_CEILING", "90"))
NL = b"\n"


def qmp(qmp_port, cmd):
    """One QMP command (a dict), bounded by 10 s in all; returns its reply line."""
    deadline = time.time() + 10
    q = socket.create_connection(("127.0.0.1", qmp_port), timeout=10)
    pending = bytearray()

    def line():
        while pending.find(NL) < 0:
            left = deadline - time.time()
            if left <= 0:
                raise TimeoutError("qmp deadline exceeded")
            q.settimeout(left)
            chunk = q.recv(4096)
            if not chunk:
                raise ConnectionError("qmp peer closed")
            pending.extend(chunk)
        i = pending.find(NL)
        out = bytes(pending[:i + 1])
        del pending[:i + 1]
        return out

    try:
        line()
        q.sendall(b'{"execute":"qmp_capabilities"}' + NL)
        line()
        q.sendall(json.dumps(cmd).encode() + NL)
        while True:
            r = line().decode(errors="replace").strip()
            if '"return"' in r or '"error"' in r:
                return r
    finally:
        q.close()


def screendump(qmp_port, path):
    """One QMP screendump, bounded by 10 s in all."""
    deadline = time.time() + 10
    q = socket.create_connection(("127.0.0.1", qmp_port), timeout=10)
    pending = bytearray()

    def line():
        while pending.find(NL) < 0:
            left = deadline - time.time()
            if left <= 0:
                raise TimeoutError("qmp deadline exceeded")
            q.settimeout(left)
            chunk = q.recv(4096)
            if not chunk:
                raise ConnectionError("qmp peer closed")
            pending.extend(chunk)
        i = pending.find(NL)
        out = bytes(pending[:i + 1])
        del pending[:i + 1]
        return out

    try:
        line()
        q.sendall(b'{"execute":"qmp_capabilities"}' + NL)
        line()
        q.sendall(json.dumps({"execute": "screendump", "arguments": {"filename": path, "format": "png"}}).encode() + NL)
        return line().decode(errors="replace").strip()
    finally:
        q.close()


def main():
    shell_demo.stdout_utf8()
    ser = shell_demo.connect_serial(int(sys.argv[1]), BOOT_CEILING)
    qmp_port, shots = int(sys.argv[2]), sys.argv[3]
    ser.settimeout(0.05)
    log = bytearray()
    notes = []

    def read():
        try:
            b = ser.recv(4096)
        except OSError:
            return b""
        log.extend(b)
        return b

    def wait_prompt(start, ceiling):
        stop = time.time() + ceiling
        while time.time() < stop:
            read()
            if bytes(log[start:]).rstrip().endswith(PROMPT):
                return True
        return False

    stop = time.time() + BOOT_CEILING
    while time.time() < stop and PROMPT not in log:
        read()
    time.sleep(0.5)
    read()

    bg = None  # (cmd, start offset, time typed) of the last &CMD
    for step in sys.argv[4:]:
        if step.startswith("&"):
            cmd = step[1:]
            start = len(log)
            ser.sendall((cmd + "\r").encode())
            bg = (cmd, start, time.time())
        elif step.startswith("~"):
            stop = time.time() + float(step[1:])
            while time.time() < stop:
                read()
        elif step.startswith(":"):
            ser.sendall(bytes.fromhex(step[1:]))
        elif step.startswith("@"):
            want = step[1:].encode()
            t0 = time.time()
            stop = t0 + 60
            while time.time() < stop and want not in bytes(log[bg[1] if bg else 0:]):
                read()
            seen = want in bytes(log[bg[1] if bg else 0:])
            notes.append("cell-keys: waited %.2f s for '%s': %s" % (time.time() - t0, step[1:], "seen" if seen else "NOT SEEN"))
        elif step.startswith("!"):
            path = os.path.join(shots, step[1:] + ".png")
            try:
                notes.append("cell-keys: screenshot %s: %s" % (step[1:], screendump(qmp_port, path)))
            except Exception as e:  # a missing picture is graded by the suite, not fatal here
                notes.append("cell-keys: screenshot %s skipped (%s: %s)" % (step[1:], type(e).__name__, e))
        elif step.startswith("%"):
            keys = [{"type": "qcode", "data": k} for k in step[1:].split("-")]
            try:
                notes.append("cell-keys: keys %s: %s" % (step[1:], qmp(qmp_port, {"execute": "send-key", "arguments": {"keys": keys}})))
            except Exception as e:
                notes.append("cell-keys: keys %s failed (%s: %s)" % (step[1:], type(e).__name__, e))
            time.sleep(0.3)
        elif step.startswith("^"):
            n = int(step[1:])
            button = "wheel-up" if n < 0 else "wheel-down"
            for _ in range(abs(n)):
                ev = [{"type": "btn", "data": {"down": True, "button": button}}, {"type": "btn", "data": {"down": False, "button": button}}]
                try:
                    qmp(qmp_port, {"execute": "input-send-event", "arguments": {"events": ev}})
                except Exception as e:
                    notes.append("cell-keys: wheel failed (%s: %s)" % (type(e).__name__, e))
                    break
                time.sleep(0.1)
            notes.append("cell-keys: wheel %d notch(es)" % n)
            time.sleep(0.3)
        elif step == "=":
            cmd, start, t0 = bg
            ok = wait_prompt(start + len(cmd), CMD_CEILING)
            notes.append("cell-keys: '&%s' took %.2f s%s" % (cmd, time.time() - t0, "" if ok else " (NO PROMPT)"))
        else:
            start = len(log)
            t0 = time.time()
            ser.sendall((step + "\r").encode())
            ok = wait_prompt(start + len(step), CMD_CEILING)
            notes.append("cell-keys: '%s' took %.2f s%s" % (step, time.time() - t0, "" if ok else " (NO PROMPT)"))
    time.sleep(0.5)
    read()
    sys.stdout.write(log.decode("utf-8", errors="replace"))
    sys.stdout.write("\n" + "\n".join(notes) + "\n")


if __name__ == "__main__":
    main()
