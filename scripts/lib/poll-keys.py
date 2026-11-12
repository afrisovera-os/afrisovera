"""Drive AfriSh over the serial socket with timed raw keys (claude/nia-poll, the wasm-poll suite).

Usage: poll-keys.py <serial_port> STEP ...

  CMD      type CMD and wait for the prompt; report how long it took
  &CMD     type CMD and do not wait (the app keeps running)
  ~SECS    wait SECS seconds, still reading the console
  :HEX     send these bytes raw (hex, e.g. :61 or :03), no CR
  =        wait for the prompt that ends the last &CMD; report the time since it was typed and since the last key
  ?TEXT    record whether TEXT is already in what the guest said since the last &CMD (`poll-keys: saw 'TEXT': yes`)

Everything the guest says goes to stdout; one line per timed step goes to stdout too, as
`poll-keys: 'CMD' took 1.53 s` or `poll-keys: '&CMD' took 4.10 s, 0.62 s after the last key`.
"""
import importlib.util
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("shell_demo", os.path.join(HERE, "..", "shell-demo.py"))
shell_demo = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shell_demo)

PROMPT = shell_demo.PROMPT
BOOT_CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "120"))
CMD_CEILING = float(os.environ.get("AFRISH_CMD_CEILING", "60"))


def main():
    shell_demo.stdout_utf8()
    ser = shell_demo.connect_serial(int(sys.argv[1]), BOOT_CEILING)
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
    t_key = None
    for step in sys.argv[2:]:
        if step.startswith("&"):
            cmd = step[1:]
            start = len(log)
            ser.sendall((cmd + "\r").encode())
            bg, t_key = (cmd, start, time.time()), None
        elif step.startswith("~"):
            stop = time.time() + float(step[1:])
            while time.time() < stop:
                read()
        elif step.startswith(":"):
            ser.sendall(bytes.fromhex(step[1:]))
            t_key = time.time()
        elif step.startswith("?"):
            read()
            seen = step[1:].encode() in bytes(log[bg[1] if bg else 0:])
            notes.append("poll-keys: saw '%s': %s" % (step[1:], "yes" if seen else "no"))
        elif step == "=":
            cmd, start, t0 = bg
            ok = wait_prompt(start + len(cmd), CMD_CEILING)
            t = time.time()
            after = ", %.2f s after the last key" % (t - t_key) if t_key else ""
            notes.append("poll-keys: '&%s' took %.2f s%s%s" % (cmd, t - t0, after, "" if ok else " (NO PROMPT)"))
        else:
            start = len(log)
            t0 = time.time()
            ser.sendall((step + "\r").encode())
            ok = wait_prompt(start + len(step), CMD_CEILING)
            notes.append("poll-keys: '%s' took %.2f s%s" % (step, time.time() - t0, "" if ok else " (NO PROMPT)"))
    time.sleep(0.5)
    read()
    sys.stdout.write(log.decode("utf-8", errors="replace"))
    sys.stdout.write("\n" + "\n".join(notes) + "\n")


if __name__ == "__main__":
    main()
