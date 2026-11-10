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
      "octo/old":         {"push": true,  "clone": "/abs/path/old.git", "archived": true},
      "octo/gone":        {"push": true,  "clone": "/abs/path/gone.git", "deleted": true}
    }
  }

Repos are cloned from local bare repositories (file:// URLs, so git-opt injects
no credentials). "deleted" repos are still listed for their owner but answer
404, like a repository removed after it was imported. Forking creates "<login>/<name>" backed by "fork_clone";
only forks can be deleted. Branches, pull request heads and comparisons are read from the bare repositories.
Test helpers (no auth): POST /_mock/merge/<n>, POST /_mock/close/<n>, GET /_mock/pulls, GET /_mock/deleted.
"""
import json
import re
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

PORT = int(sys.argv[1])
with open(sys.argv[2]) as f:
    CONFIG = json.load(f)
TOKEN = CONFIG.get("token", "fake-token")
USER = CONFIG.get("user", {"id": 42, "login": "jdev", "name": "Jane Developer"})
REPOS = CONFIG["repos"]
FORKS = {}
PULLS = {}
DELETED = []


def git(clone, *args):
    if not clone:
        return None
    out = subprocess.run(["git", "--git-dir", clone, *args], capture_output=True, text=True)
    return out.stdout.strip() if out.returncode == 0 else None


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
        "parent": {"full_name": info["parent"]} if "parent" in info else None,
    }


def head_clone(pr):
    """The bare repository the pull request's branch lives in."""
    owner = pr["head_label"].split(":")[0]
    full, info = find(owner + "/" + pr["repo"].split("/")[1])
    return info["clone"] if full else None


def pull_json(pr):
    out = {k: v for k, v in pr.items() if k not in ("head_label", "head_sha")}
    branch = pr["head_label"].split(":", 1)[1]
    sha = pr.get("head_sha") or git(head_clone(pr), "rev-parse", "-q", "--verify", "refs/heads/" + branch)
    out["head"] = {"label": pr["head_label"], "ref": branch, "sha": sha}
    return out


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, code, body):
        data = b"" if code == 204 else json.dumps(body).encode()
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
            return self.send(200, {n: pull_json(p) for n, p in PULLS.items()})
        if path == "/_mock/deleted":
            return self.send(200, DELETED)
        m = re.fullmatch(r"/_mock/(merge|close)/(\d+)", path)
        if m:
            pr = PULLS[int(m[2])]
            pr["head_sha"] = pull_json(pr)["head"]["sha"]  # a closed pull request stops following its branch
            pr["state"] = "closed"
            if m[1] == "merge":
                pr["merged_at"] = "2026-01-01T00:00:00Z"
            return self.send(200, pull_json(pr))

        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            return self.send(401, {"message": "Bad credentials"})

        if path == "/user":
            return self.send(200, USER)
        if path == "/user/repos":
            owned_only = query.get("affiliation", [""])[0] == "owner"
            mine = [k for k, v in REPOS.items()
                    if k.split("/")[0] == USER["login"] or (v.get("push") and not owned_only)]
            return self.send(200, [repo_json(k) for k in mine])
        m = re.fullmatch(r"/(?:users|orgs)/([^/]+)/repos", path)
        if m:
            owned = [k for k in REPOS if k.split("/")[0].lower() == m[1].lower()]
            if not owned:
                return self.send(404, {"message": "Not Found"})
            return self.send(200, [repo_json(k) for k in owned])

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)", path)
        if m:
            full, info = find(m[1])
            if not full or info.get("deleted"):
                return self.send(404, {"message": "Not Found"})
            if method == "DELETE":
                if full not in FORKS:
                    return self.send(403, {"message": "Must have admin rights to Repository."})
                del FORKS[full]
                DELETED.append(full)
                return self.send(204, None)
            return self.send(200, repo_json(full))

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/branches", path)
        if m:
            full, info = find(m[1])
            if not full:
                return self.send(404, {"message": "Not Found"})
            refs = git(info["clone"], "for-each-ref", "--format=%(refname:short) %(objectname)", "refs/heads") or ""
            return self.send(200, [{"name": n, "commit": {"sha": s}} for n, s in (l.split(" ") for l in refs.splitlines())])

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/compare/(.+)\.\.\.([^:]+):(.+)", unquote(path))
        if m:
            base_full, base = find(m[1])
            head_full, head = find(m[3] + "/" + m[1].split("/")[1])
            if not base_full or not head_full:
                return self.send(404, {"message": "Not Found"})
            git(head["clone"], "fetch", "-q", base["clone"], f"+refs/heads/{m[2]}:refs/mock/base")
            ahead = git(head["clone"], "rev-list", "--count", f"refs/mock/base..refs/heads/{m[4]}")
            if ahead is None:
                return self.send(404, {"message": "Not Found"})
            return self.send(200, {"ahead_by": int(ahead), "status": "ahead" if int(ahead) else "behind"})

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/forks", path)
        if m and method == "POST":
            full, info = find(m[1])
            if not full:
                return self.send(404, {"message": "Not Found"})
            fork = USER["login"] + "/" + full.split("/")[1]
            FORKS[fork] = {"push": True, "clone": info["fork_clone"], "parent": full}
            return self.send(202, repo_json(fork))

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls", path)
        if m and method == "GET":
            head = query.get("head", [""])[0]
            state = query.get("state", ["open"])[0]
            found = [pull_json(p) for p in PULLS.values() if p["repo"] == m[1] and p["head_label"] == head
                     and state in ("all", p["state"])]
            return self.send(200, found)
        if m and method == "POST":
            b = self.body()
            number = len(PULLS) + 1
            head = b["head"] if ":" in b["head"] else m[1].split("/")[0] + ":" + b["head"]
            PULLS[number] = {
                "number": number, "html_url": f"https://github.com/{m[1]}/pull/{number}", "state": "open",
                "merged_at": None, "repo": m[1], "head_label": head, "base": b["base"], "title": b["title"],
                "body": b["body"],
            }
            return self.send(201, pull_json(PULLS[number]))

        m = re.fullmatch(r"/repos/([^/]+/[^/]+)/pulls/(\d+)", path)
        if m:
            pr = PULLS.get(int(m[2]))
            if not pr:
                return self.send(404, {"message": "Not Found"})
            if method == "PATCH":
                b = self.body()
                pr.update({k: b[k] for k in ("title", "body") if k in b})
            return self.send(200, pull_json(pr))

        return self.send(404, {"message": "Not Found: " + path})

    def do_GET(self):
        self.route("GET")

    def do_POST(self):
        self.route("POST")

    def do_PATCH(self):
        self.route("PATCH")

    def do_DELETE(self):
        self.route("DELETE")


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
