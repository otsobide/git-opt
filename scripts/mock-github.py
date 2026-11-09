"""Tiny fake of the GitHub REST endpoints git-opt uses, for scripts/e2e.sh.

usage: mock-github.py <port> <demo.git> <third.git> <fork.git>

- octo/demo        -> push access (direct mode), cloned from <demo.git>
- other/thirdparty -> read only (fork mode), cloned from <third.git>; forks push to <fork.git>
- token "fake-token", authenticated user "jdev"
- POST /_mock/merge/<n> and /_mock/close/<n> flip PR states; GET /_mock/pulls dumps them.
"""
import json, sys, re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

BARE = sys.argv[2]
REPOS = {
    "octo/demo": {"push": True, "clone": "file://" + BARE},
    "other/thirdparty": {"push": False, "clone": "file://" + sys.argv[3]},
}
FORKS = {}
PULLS = {}
LOG = []

def repo_json(full):
    owner, name = full.split("/")
    info = REPOS.get(full) or FORKS.get(full)
    return {"name": name, "full_name": full, "owner": {"login": owner},
            "clone_url": info["clone"], "default_branch": "main", "fork": full in FORKS,
            "archived": False, "disabled": False, "permissions": {"push": info["push"]}}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}")
    def handle_any(self, method):
        u = urlparse(self.path); p = u.path; q = parse_qs(u.query)
        LOG.append(f"{method} {self.path}")
        if p == "/_mock/log": return self.send(200, LOG)
        if p == "/_mock/pulls": return self.send(200, PULLS)
        m = re.fullmatch(r"/_mock/(merge|close)/(\d+)", p)
        if m:
            pr = PULLS[int(m[2])]; pr["state"] = "closed"
            if m[1] == "merge": pr["merged_at"] = "2026-11-09T00:00:00Z"
            return self.send(200, pr)
        if self.headers.get("Authorization") != "Bearer fake-token":
            return self.send(401, {"message": "Bad credentials"})
        if p == "/user": return self.send(200, {"id": 42, "login": "jdev", "name": "J Dev"})
        if p == "/users/octo/repos": return self.send(200, [repo_json("octo/demo")])
        m = re.fullmatch(r"/repos/([^/]+/[^/]+)", p)
        if m:
            full = m[1]
            if full in REPOS or full in FORKS: return self.send(200, repo_json(full))
            return self.send(404, {"message": "Not Found"})
        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/forks", p)
        if m and method == "POST":
            full = "jdev/" + m[1].split("/")[1]
            FORKS[full] = {"push": True, "clone": "file://" + sys.argv[4]}
            return self.send(202, repo_json(full))
        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls", p)
        if m and method == "GET":
            head = q.get("head", [""])[0]
            res = [pr for pr in PULLS.values() if pr["repo"] == m[1] and pr["head"] == head and pr["state"] == "open"]
            return self.send(200, res)
        if m and method == "POST":
            b = self.body(); n = len(PULLS) + 1
            head = b["head"] if ":" in b["head"] else m[1].split("/")[0] + ":" + b["head"]
            PULLS[n] = {"number": n, "html_url": f"https://github.com/{m[1]}/pull/{n}", "state": "open",
                        "merged_at": None, "repo": m[1], "head": head, "title": b["title"], "body": b["body"], "base": b["base"]}
            return self.send(201, PULLS[n])
        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls/(\d+)", p)
        if m:
            pr = PULLS.get(int(m[2]))
            if not pr: return self.send(404, {"message": "Not Found"})
            if method == "PATCH":
                b = self.body(); pr.update({k: b[k] for k in ("title", "body") if k in b})
            return self.send(200, pr)
        return self.send(404, {"message": "Not Found: " + p})
    def do_GET(self): self.handle_any("GET")
    def do_POST(self): self.handle_any("POST")
    def do_PATCH(self): self.handle_any("PATCH")

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
