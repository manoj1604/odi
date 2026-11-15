# Boot a VM, snapshot it running, then restore it in a new Firecracker:
# how long until the guest runs again.
import http.client, json, os, socket, subprocess, time

class UnixConn(http.client.HTTPConnection):
    def __init__(self, path): super().__init__("localhost"); self.path = path
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX); self.sock.connect(self.path)

def api(sock, method, path, body=None):
    c = UnixConn(sock); c.request(method, path, json.dumps(body) if body else None, {"Content-Type": "application/json"})
    r = c.getresponse(); data = r.read()
    if r.status >= 300: raise SystemExit(f"{method} {path}: {r.status} {data}")

def wait_for(p, marker):
    buf = b""
    while marker not in buf:
        c = p.stdout.read1(4096)
        if not c: raise SystemExit("ended: " + buf[-500:].decode(errors="replace"))
        buf += c

def start(sock, *extra):
    if os.path.exists(sock): os.remove(sock)
    p = subprocess.Popen(["./firecracker", "--api-sock", sock, "--level", "Error", *extra], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    while not os.path.exists(sock): time.sleep(0.001)
    return p

p = start("/tmp/fc-a.sock", "--config-file", "boot.json")
wait_for(p, b"ODI_BOOTED"); wait_for(p, b"tick")
t = time.monotonic()
api("/tmp/fc-a.sock", "PATCH", "/vm", {"state": "Paused"})
api("/tmp/fc-a.sock", "PUT", "/snapshot/create", {"snapshot_type": "Full", "snapshot_path": "vm.snap", "mem_file_path": "vm.mem"})
print(f"snapshot taken in {(time.monotonic()-t)*1000:.0f} ms: memory {os.path.getsize(chr(118)+"m.mem")>>20} MB, state {os.path.getsize(chr(118)+"m.snap")>>10} KB")
p.kill(); p.wait()

for run in range(5):
    t = time.monotonic()
    p = start("/tmp/fc-b.sock")
    t_up = time.monotonic()
    api("/tmp/fc-b.sock", "PUT", "/snapshot/load", {"snapshot_path": "vm.snap", "mem_backend": {"backend_type": "File", "backend_path": "vm.mem"}, "resume_vm": True})
    t_loaded = time.monotonic()
    wait_for(p, b"tick")
    t_tick = time.monotonic()
    print(f"restore {run+1}: process up {(t_up-t)*1000:.1f} ms, snapshot loaded and resumed {(t_loaded-t_up)*1000:.1f} ms, guest running (next tick) {(t_tick-t)*1000:.1f} ms total")
    p.kill(); p.wait()
