# A microVM serving Express: cold boot to its first HTTP answer, then a
# snapshot of it warmed up, restored in a new Firecracker, to its first answer.
import http.client, json, os, socket, subprocess, time
exec(open("snap.py").read().split("p = start(")[0])  # (api, start, wait_for)

def first_answer(t0, path="/"):
    while True:
        try:
            c = http.client.HTTPConnection("172.16.0.2", 8080, timeout=1); c.request("GET", path)
            r = c.getresponse(); body = r.read()
            if r.status == 200: return (time.monotonic() - t0) * 1000, body[:40]
        except OSError: time.sleep(0.001)

for run in range(2):
    t = time.monotonic()
    p = start("/tmp/fc-a.sock", "--config-file", "boot-node.json")
    ms, body = first_answer(t)
    print(f"cold boot {run+1}: first HTTP answer after {ms:.0f} ms  {body}")
    if run == 1:
        first_answer(time.monotonic(), "/api/hello")  # (warm: both routes served once)
        t = time.monotonic()
        api("/tmp/fc-a.sock", "PATCH", "/vm", {"state": "Paused"})
        api("/tmp/fc-a.sock", "PUT", "/snapshot/create", {"snapshot_type": "Full", "snapshot_path": "app.snap", "mem_file_path": "app.mem"})
        print(f"snapshot of the running app: {(time.monotonic()-t)*1000:.0f} ms, memory file {os.path.getsize(chr(97)+chr(112)+chr(112)+chr(46)+chr(109)+chr(101)+chr(109))>>20} MB")
    p.kill(); p.wait()

for run in range(5):
    t = time.monotonic()
    p = start("/tmp/fc-b.sock")
    api("/tmp/fc-b.sock", "PUT", "/snapshot/load", {"snapshot_path": "app.snap", "mem_backend": {"backend_type": "File", "backend_path": "app.mem"}, "resume_vm": True})
    t_res = (time.monotonic() - t) * 1000
    ms, body = first_answer(t)
    t2 = time.monotonic(); first_answer(t2); again = (time.monotonic() - t2) * 1000
    print(f"restore {run+1}: resumed after {t_res:.1f} ms, first HTTP answer after {ms:.1f} ms, next {again:.1f} ms  {body}")
    p.kill(); p.wait()
