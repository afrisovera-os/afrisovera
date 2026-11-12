"""Drive AfriSh over the serial socket and answer `[fuzz] keys` markers (N1 stop key).

Usage: nia-keys.py <serial_port> <cmd1> [cmd2] ...

Like shell-demo.py it types each command and waits for the prompt, writing everything the guest says to
stdout. While a command runs, every line `[fuzz] keys (wait N ms): 03 61 ...` makes it wait N ms and then
send those bytes (hex) raw, 50 ms apart: that is how `wasmfuzz` gets its Ctrl+C and test keys at the
moment the kernel asks for them. No screenshot.
"""
import importlib.util
import os
import re
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("shell_demo", os.path.join(HERE, "..", "shell-demo.py"))
shell_demo = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shell_demo)

PROMPT = shell_demo.PROMPT
MARKER = re.compile(rb"\[fuzz\] keys \(wait (\d+) ms\): ([0-9A-Fa-f ]+)\r?\n")
BOOT_CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "120"))
CMD_QUIET = float(os.environ.get("AFRISH_CMD_QUIET", "60"))
CMD_CEILING = float(os.environ.get("AFRISH_CMD_CEILING", "300"))


def main():
    shell_demo.stdout_utf8()
    port = int(sys.argv[1])
    cmds = sys.argv[2:]
    ser = shell_demo.connect_serial(port, BOOT_CEILING)
    ser.settimeout(0.2)
    log = bytearray()

    def read():
        try:
            b = ser.recv(4096)
        except OSError:  # socket.timeout is an OSError
            return b""
        log.extend(b)
        return b

    # Boot: wait for the first prompt (or the ceiling: the assertions judge what came).
    stop = time.time() + BOOT_CEILING
    while time.time() < stop and PROMPT not in log:
        read()
    if PROMPT not in log:
        print("nia-keys: no prompt after %.0fs, typing anyway" % BOOT_CEILING, file=sys.stderr)
    time.sleep(0.5)
    read()

    for cmd in cmds:
        start = len(log)
        t_send = time.time()
        ser.sendall((cmd + "\r").encode())
        seen = 0  # markers already answered in this command's output
        hard = time.time() + CMD_CEILING
        idle = time.time() + CMD_QUIET
        why = "ceiling"
        while time.time() < min(hard, idle):
            if read():
                idle = time.time() + CMD_QUIET
            marks = list(MARKER.finditer(bytes(log[start:])))
            for m in marks[seen:]:
                time.sleep(int(m.group(1)) / 1000.0)
                for h in m.group(2).split():
                    ser.sendall(bytes([int(h, 16)]))
                    time.sleep(0.05)
                print("nia-keys: sent %s" % m.group(2).decode().strip(), file=sys.stderr)
                idle = time.time() + CMD_QUIET
            seen = len(marks)
            tail = bytes(log[start:]).rstrip()
            if cmd.encode() in tail and tail.endswith(PROMPT):
                why = "prompt"
                break
        else:
            why = "idle" if time.time() >= idle else "ceiling"
        print("nia-keys: '%s' ended on %s after %.2f s" % (cmd, why, time.time() - t_send), file=sys.stderr)
    time.sleep(1.0)
    read()
    sys.stdout.write(log.decode("utf-8", errors="replace"))


if __name__ == "__main__":
    main()
