#!/usr/bin/env bash
# End-to-end test against a fake GitHub API (scripts/mock-github.py) and local
# bare repositories. Never talks to github.com.
# Needs: cargo, git, python3, curl and ImageMagick (`magick`).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
API_PORT=${API_PORT:-18999}
PORT=${PORT:-18484}
API=http://127.0.0.1:$PORT/api
PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT

for tool in cargo git python3 curl magick; do
  command -v "$tool" >/dev/null || { echo "missing $tool"; exit 1; }
done

step() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*"; echo "--- server log"; tail -30 "$WORK/server.log"; exit 1; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
commit() { git -c user.name=Owner -c user.email=owner@example.com commit -q "$@"; }
wait_idle() { # wait until repo $1 is neither queued nor running
  for _ in $(seq 60); do
    [[ $(curl -fs "$API/repos/$1" | json "d['running'] or d['queued']") == False ]] && return
    sleep 0.5
  done
  fail "repo $1 still busy"
}
last_run() { curl -fs "$API/runs?repo_id=$1&limit=1" | json "d[0]['$2']"; }

step "build"
cargo build --quiet --manifest-path "$ROOT/Cargo.toml"

step "fixtures"
mkdir -p "$WORK/work/img" && cd "$WORK/work" && git init -q -b main .
magick -size 300x200 gradient:red-blue -define png:compression-level=0 img/big.png
magick -size 64x64 xc:white -fill black -draw "circle 32,32 32,10" -define png:compression-level=0 img/icon.png
magick -size 400x300 plasma:fractal -seed 7 -quality 92 -define jpeg:optimize-coding=false img/photo.jpg
git add -A && commit -m "Initial"
git clone -q --bare . ../demo.git && git clone -q --bare . ../third.git && git init -q --bare ../fork.git

step "start mock GitHub and git-opt"
cat >"$WORK/github.json" <<JSON
{ "token": "fake-token", "user": { "id": 42, "login": "jdev", "name": "J Dev" },
  "repos": { "octo/demo": { "push": true, "clone": "$WORK/demo.git" },
             "other/thirdparty": { "push": false, "clone": "$WORK/third.git", "fork_clone": "$WORK/fork.git" } } }
JSON
python3 -I "$ROOT/scripts/mock-github.py" "$API_PORT" "$WORK/github.json" &
PIDS+=($!)
GIT_OPT_BIND=127.0.0.1:$PORT GIT_OPT_DATA_DIR="$WORK/data" GIT_OPT_GITHUB_API=http://127.0.0.1:$API_PORT \
  GITHUB_TOKEN=fake-token GIT_OPT_STATIC_DIR=/nonexistent "$ROOT/target/debug/git-opt" >"$WORK/server.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 40); do curl -fs "$API/health" >/dev/null 2>&1 && break; sleep 0.25; done

step "add octo/demo -> PR with 3 optimized images"
curl -fs -X POST -H 'content-type: application/json' -d '{"repo":"octo/demo"}' "$API/repos" >/dev/null
wait_idle 1
[[ $(last_run 1 status) == pr_opened ]] || fail "expected pr_opened, got $(last_run 1 status): $(last_run 1 message)"
[[ $(last_run 1 files_optimized) == 3 ]] || fail "expected 3 files"

step "pixels are identical (ImageMagick AE metric)"
cd "$WORK" && git clone -q demo.git check && cd check
for f in img/big.png img/icon.png img/photo.jpg; do
  git show "main:$f" >"$WORK/orig"
  ae=$(git show "origin/git-opt/optimize-images:$f" | magick compare -metric AE "$WORK/orig" - null: 2>&1 || true)
  [[ $ae == 0* ]] || fail "$f differs: AE=$ae"
  echo "$f AE=$ae"
done
git log -1 --format=%B origin/git-opt/optimize-images | grep -q "^Git-Opt: lossless-optimization$" || fail "marker missing"

step "merge PR -> nothing left to optimize, savings credited"
cd "$WORK/work" && git fetch -q ../demo.git git-opt/optimize-images
git -c user.name=Owner -c user.email=owner@example.com merge -q --no-ff FETCH_HEAD -m "Merge pull request #1"
git push -q ../demo.git main
curl -fs -X POST "http://127.0.0.1:$API_PORT/_mock/merge/1" >/dev/null
curl -fs -X POST "$API/repos/1/check" && wait_idle 1
[[ $(last_run 1 status) == up_to_date ]] || fail "expected up_to_date after merge"
[[ $(curl -fs "$API/repos/1" | json "d['bytes_saved'] > 0") == True ]] || fail "savings not credited"

step "third-party repo -> fork mode, manual approval, publish"
curl -fs -X POST -H 'content-type: application/json' -d '{"repo":"other/thirdparty","auto_pr":false}' "$API/repos" >/dev/null
wait_idle 2
[[ $(curl -fs "$API/repos/2" | json "d['mode'] + ' ' + d['status']") == "fork awaiting_approval" ]] || fail "expected fork awaiting_approval"
curl -fs -X POST "$API/repos/2/publish" && wait_idle 2
[[ $(last_run 2 status) == pr_opened ]] || fail "publish did not open a PR: $(last_run 2 message)"
git -C "$WORK/fork.git" rev-parse -q --verify git-opt/optimize-images >/dev/null || fail "branch not pushed to fork"

echo; echo "e2e OK"
