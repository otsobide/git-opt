#!/usr/bin/env python3
"""Tiny fake of the GitHub REST endpoints git-opt uses (for e2e tests and demos).

usage: mock-github.py <port> <config.json>

config.json:
  {
    "token": "fake-token",
    "user": {"id": 42, "login": "jdev", "name": "Jane Developer"},
    "repos": {
      "octo/demo":        {"push": true,  "clone": "/abs/path/demo.git"},
      "other/thirdparty": {"push": false, "clone": "/abs/path/third.git", "fork_clone": "/abs/path/fork.git"},
      "octo/old":         {"push": true,  "clone": "/abs/path/old.git", "archived": true}
    }
  }

Repos are cloned from local bare repositories (file:// URLs, so git-opt injects
no credentials). Forking creates "<login>/<name>" backed by "fork_clone".
Test helpers (no auth): POST /_mock/merge/<n>, POST /_mock/close/<n>, GET /_mock/pulls.
"""
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = int(sys.argv[1])
with open(sys.argv[2]) as f:
    CONFIG = json.load(f)
TOKEN = CONFIG.get("token", "fake-token")
USER = CONFIG.get("user", {"id": 42, "login": "jdev", "name": "Jane Developer"})
REPOS = CONFIG["repos"]
FORKS = {}
PULLS = {}


def find(full):
    for table in (REPOS, FORKS):
        for key, value in table.items():
            if key.lower() == full.lower():
                return key, value
    return None, None


def repo_json(full):
    full, info = find(full)
    owner, name = full.split("/")
    return {
        "name": name, "full_name": full, "owner": {"login": owner},
        "clone_url": "file://" + info["clone"], "default_branch": info.get("branch", "main"),
        "fork": full in FORKS, "archived": info.get("archived", False), "disabled": False,
        "permissions": {"push": info.get("push", False)},
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        length = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(length) or b"{}")

    def route(self, method):
        url = urlparse(self.path)
        path, query = url.path, parse_qs(url.query)

        if path == "/_mock/pulls":
            return self.send(200, PULLS)
        m = re.fullmatch(r"/_mock/(merge|close)/(\d+)", path)
        if m:
            pr = PULLS[int(m[2])]
            pr["state"] = "closed"
            if m[1] == "merge":
                pr["merged_at"] = "2026-01-01T00:00:00Z"
            return self.send(200, pr)

        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            return self.send(401, {"message": "Bad credentials"})

        if path == "/user":
            return self.send(200, USER)
        if path == "/user/repos":
            mine = [k for k, v in REPOS.items() if v.get("push") or k.split("/")[0] == USER["login"]]
            return self.send(200, [repo_json(k) for k in mine])
        m = re.fullmatch(r"/(?:users|orgs)/([^/]+)/repos", path)
        if m:
            owned = [k for k in REPOS if k.split("/")[0].lower() == m[1].lower()]
            if not owned:
                return self.send(404, {"message": "Not Found"})
            return self.send(200, [repo_json(k) for k in owned])

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)", path)
        if m:
            return self.send(200, repo_json(m[1])) if find(m[1])[0] else self.send(404, {"message": "Not Found"})

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/forks", path)
        if m and method == "POST":
            full, info = find(m[1])
            if not full:
                return self.send(404, {"message": "Not Found"})
            fork = USER["login"] + "/" + full.split("/")[1]
            FORKS[fork] = {"push": True, "clone": info["fork_clone"]}
            return self.send(202, repo_json(fork))

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls", path)
        if m and method == "GET":
            head = query.get("head", [""])[0]
            found = [p for p in PULLS.values() if p["repo"] == m[1] and p["head"] == head and p["state"] == "open"]
            return self.send(200, found)
        if m and method == "POST":
            b = self.body()
            number = len(PULLS) + 1
            head = b["head"] if ":" in b["head"] else m[1].split("/")[0] + ":" + b["head"]
            PULLS[number] = {
                "number": number, "html_url": f"https://github.com/{m[1]}/pull/{number}", "state": "open",
                "merged_at": None, "repo": m[1], "head": head, "base": b["base"], "title": b["title"], "body": b["body"],
            }
            return self.send(201, PULLS[number])

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls/(\d+)", path)
        if m:
            pr = PULLS.get(int(m[2]))
            if not pr:
                return self.send(404, {"message": "Not Found"})
            if method == "PATCH":
                b = self.body()
                pr.update({k: b[k] for k in ("title", "body") if k in b})
            return self.send(200, pr)

        return self.send(404, {"message": "Not Found: " + path})

    def do_GET(self):
        self.route("GET")

    def do_POST(self):
        self.route("POST")

    def do_PATCH(self):
        self.route("PATCH")


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
