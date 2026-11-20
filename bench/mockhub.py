# A registry in memory, as docs/registry.md describes it, for testing and timing
# odi push, pull and run. Not for real use: no storage, one token ("secret").
#
#   python3 bench/mockhub.py PORT
#   DELAY_MS=50: wait that long before each answer (a network, roughly)
#   NO_BATCH=1: no batch request (POST .../segments/batch), as older registries
#   LOG=FILE: a line per request (time, method, path)
#   NO_EXTRAS=1: tags without their builds (?builds=1), builds without their
#   parts' (?parts=1), as older registries
import hashlib, json, sys, time, re, os, urllib.parse
DELAY = int(os.environ.get("DELAY_MS", "0")) / 1000
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
segs, builds, tags, repos = {}, {}, {}, {}   # hash->bytes; (repo,digest)->build; (repo,tag)->tag; repo->info
def listing(b): return {"size": b["size"], "segments": [{"hash": s["hash"], "length": s["length"]} for s in b["segments"]]}
def parts_of(b):
    # A set's parts' lists, from whichever repository has them (a real
    # registry: only those the caller may read).
    m = b.get("manifest") or {}
    if isinstance(m, str): m = json.loads(m)
    found = {}
    for part in m.get("parts") or []:
        d = part.get("digest", "").split(":")[-1]
        for (rp, dd), pb in builds.items():
            if dd == d and pb["done"]: found[d] = listing(pb); break
    return found
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, code, body=b"", ctype="application/json"):
        if not isinstance(body, bytes): body = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def body(self): return self.rfile.read(int(self.headers.get("Content-Length") or 0))
    def route(self, m):
        time.sleep(DELAY)
        if os.environ.get("LOG"):
            with open(os.environ["LOG"], "a") as f: f.write("%.3f %s %s\n" % (time.time(), m, self.path))
        p, _, query = self.path.partition("?")
        extras = os.environ.get("NO_EXTRAS") != "1"
        if m != "GET" and not p.endswith("/segments/batch") and self.headers.get("Authorization") != "Bearer secret": return self.send(401, {"error": "sign in"})
        r = re.match(r"^/v1/repos/([^/]+/[^/]+)(.*)$", p)
        if p in ("/v1/search", "/v1/mine"):
            q = urllib.parse.parse_qs(query).get("q", [""])[0].lower()
            rows = [{"ns": k.split("/")[0], "name": k.split("/")[1], "public": v["public"], "updated": v.get("updated", 0), "language": "", "pulls": v["pulls"]} for k, v in repos.items()]
            if p == "/v1/search": rows = [x for x in rows if x["public"] and x["updated"] and q in (x["ns"] + "/" + x["name"]).lower()]
            return self.send(200, rows)
        if p == "/v1/usage": return self.send(200, {"bytes": sum(map(len, segs.values())), "namespaces": 1, "repos": 1, "limits": {"bytes": 1<<30, "file": 1<<28, "namespaces": 3, "repos": 10}})
        if not r: return self.send(404, {"error": "no route"})
        repo, rest = r.groups()
        if m == "POST" and rest == "/digests":
            b = json.loads(self.body()); d = b["digest"].split(":")[1]
            repos.setdefault(repo, {"public": False, "pulls": 0})
            if builds.get((repo, d), {}).get("done"): return self.send(200, {"digest": d, "ready": True, "missing": []})  # (the same build)
            builds[(repo, d)] = {"size": b["size"], "segments": b["segments"], "manifest": b["manifest"], "done": False, "created": int(time.time()*1000)}
            missing = sorted({s["hash"] for s in b["segments"] if s["hash"] not in segs})
            return self.send(200, {"digest": d, "ready": False, "missing": missing})
        if m == "POST" and rest == "/segments/batch" and os.environ.get("NO_BATCH") != "1":
            hs = json.loads(self.body())["hashes"]
            if any(h not in segs for h in hs): return self.send(404, {"error": "no piece"})
            return self.send(200, b"".join(segs[h] for h in hs), "application/octet-stream")
        if m == "PUT" and rest.startswith("/segments/"):
            h = rest.split("/")[-1]; data = self.body()
            if hashlib.sha256(data).hexdigest() != h: return self.send(400, {"error": "bad hash"})
            segs[h] = data; return self.send(200, {})
        if m == "POST" and rest.endswith("/commit"):
            d = rest.split("/")[2]; b = builds[(repo, d)]
            if any(s["hash"] not in segs for s in b["segments"]): return self.send(409, {"error": "missing"})
            b["done"] = True; return self.send(200, {})
        if m == "GET" and rest.startswith("/digests/"):
            b = builds.get((repo, rest.split("/")[2]))
            if not b: return self.send(404, {"error": "no such build"})
            out = listing(b); out.update({"digest": rest.split("/")[2], "manifest": b["manifest"], "created": b["created"]})
            if extras and "parts=1" in query: out["parts"] = parts_of(b)
            return self.send(200, out)
        if m == "GET" and rest.startswith("/segments/"):
            h = rest.split("/")[-1]; return self.send(200, segs[h], "application/octet-stream") if h in segs else self.send(404, {"error": "no piece"})
        if rest.startswith("/tags/"):
            t = rest.split("/")[2]
            if m == "PUT":
                b = json.loads(self.body()); cur = tags.get((repo, t))
                if any(not builds.get((repo, x["digest"].split(":")[-1]), {}).get("done") for x in b["platforms"]): return self.send(404, {"error": "no finished build with that digest here"})
                expect = self.headers.get("If-Match")
                if expect is not None and (cur["digest"] if cur else "") != expect.split(":")[-1]: return self.send(412, {"error": "the tag has moved"})
                old = cur.get("platforms", []) if (b.get("add") and cur) else []
                keep = [x for x in old if x["platform"] not in {p["platform"] for p in b["platforms"]}]
                plats = keep + [{"platform": x["platform"], "digest": x["digest"].split(":")[-1]} for x in b["platforms"]]
                tags[(repo, t)] = {"tag": t, "digest": plats[0]["digest"], "updated": int(time.time()*1000), "platforms": plats}
                repos.setdefault(repo, {"public": False, "pulls": 0})["updated"] = int(time.time()*1000); return self.send(200, {})
            if m == "GET":
                if (repo, t) not in tags: return self.send(404, {"error": "no such tag"})
                out = dict(tags[(repo, t)])
                if extras and "builds=1" in query:
                    out["builds"] = {}
                    for x in out["platforms"]:
                        b = builds.get((repo, x["digest"]))
                        if b: out["builds"][x["digest"]] = listing(b); out["builds"].update(parts_of(b))
                return self.send(200, out)
            if m == "DELETE": tags.pop((repo, t), None); return self.send(200, {})
        if m == "GET" and rest == "/tags":
            if repo not in repos: return self.send(404, {"error": "no such repository"})
            return self.send(200, sorted([v for (rp, _), v in tags.items() if rp == repo], key=lambda v: -v["updated"]))
        if m == "PUT" and rest == "":
            repos.setdefault(repo, {"public": False, "pulls": 0})["public"] = bool(json.loads(self.body()).get("public")); return self.send(200, {})
        if m == "GET" and rest == "":
            if repo not in repos: return self.send(404, {"error": "no such repository"})
            ts = sorted([v for (rp, _), v in tags.items() if rp == repo], key=lambda v: -v["updated"])
            ds = sorted([{"digest": d, "size": b["size"], "created": b["created"]} for (rp, d), b in builds.items() if rp == repo and b["done"]], key=lambda x: -x["created"])
            info = repos[repo]
            return self.send(200, {"namespace": repo.split("/")[0], "name": repo.split("/")[1], "public": info["public"], "mine": True, "pulls": info["pulls"], "language": "", "updated": info.get("updated", 0), "tags": ts, "digests": ds})
        return self.send(404, {"error": "no route " + m + " " + p})
    def do_GET(self): self.route("GET")
    def do_POST(self): self.route("POST")
    def do_PUT(self): self.route("PUT")
    def do_DELETE(self): self.route("DELETE")
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
