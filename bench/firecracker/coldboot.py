# Times Firecracker from launch to the guest printing ODI_BOOTED.
import subprocess, time, sys, os
for run in range(5):
    t = time.monotonic()
    p = subprocess.Popen(["./firecracker", "--no-api", "--config-file", "boot.json", "--level", "Error"],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    buf = b""
    while b"ODI_BOOTED" not in buf:
        c = p.stdout.read1(4096)
        if not c: print("ended:", buf[-800:].decode(errors="replace")); sys.exit(1)
        buf += c
    print(f"cold boot {run+1}: {(time.monotonic()-t)*1000:.0f} ms to the guest running its first program")
    p.kill(); p.wait()
