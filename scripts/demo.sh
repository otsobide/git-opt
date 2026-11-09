#!/usr/bin/env bash
# Run git-opt against a fake GitHub API with sample repositories in every
# state (open PR, merged, awaiting approval, fork contribution, error, paused),
# so the UI can be explored without touching github.com.
#
#   scripts/demo.sh                     # serve the demo on http://127.0.0.1:18484
#   scripts/demo.sh --screenshots DIR   # capture the UI in dark mode into DIR and exit
#
# Needs: cargo, git, python3, curl, ImageMagick (`magick`) and jpegtran; npm to
# build the UI if frontend/dist is missing; node + Chromium for --screenshots.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
API_PORT=${API_PORT:-18999}
PORT=${PORT:-18484}
BASE=http://127.0.0.1:$PORT
API=$BASE/api
PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT

SHOTS=""
if [[ ${1:-} == --screenshots ]]; then
  SHOTS=$(mkdir -p "${2:?usage: demo.sh --screenshots DIR}" && cd "$2" && pwd)
fi

log() { printf '== %s\n' "$*"; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
call() { # call <method> <path> [json]: fails loudly with the API's error message
  local out
  out=$(curl -sS -X "$1" -H 'content-type: application/json' -d "${3:-{\}}" -w '\n%{http_code}' "$API$2")
  [[ ${out##*$'\n'} == 2* ]] || { echo "$1 $2 -> ${out##*$'\n'}: ${out%$'\n'*}"; exit 1; }
}
post() { call POST "$@"; }
patch() { call PATCH "$@"; }
repo_id() { curl -fsS "$API/repos" | json "[r['id'] for r in d if r['owner']+'/'+r['name']=='$1'][0]"; }
idle() { # until nothing is queued or running and every repo was checked once
  for _ in $(seq 240); do
    local busy
    busy=$(curl -fsS "$API/status" | json "len(d['jobs']['running']) + len(d['jobs']['queued'])")
    local pending
    pending=$(curl -fsS "$API/repos" | json "sum(r['status'] == 'pending' for r in d)")
    [[ $busy == 0 && $pending == 0 ]] && return
    sleep 0.5
  done
  echo "demo did not settle"; exit 1
}

# ---------------------------------------------------------------- fixtures
commit() { git -c user.name="Repo Owner" -c user.email=owner@example.com commit -q "$@"; }
png() { magick -size "$2" "$3" -define png:compression-level=0 "$1"; }
jpg() { magick -size "$2" plasma:fractal -seed "$3" -quality 92 -define jpeg:optimize-coding=false "$1"; }

# new_repo <owner/name> <builder-function>
new_repo() {
  local dir="$WORK/src/$1"
  mkdir -p "$dir" && (cd "$dir" && git init -q -b main . && echo "# ${1#*/}" >README.md && "$2" && git add -A && commit -m "Initial commit")
  git clone -q --bare "$dir" "$WORK/bare/$1.git"
}
website() {
  mkdir -p public/img public/photos
  png public/img/hero.png 1200x500 gradient:'#1d4ed8'-'#9333ea'
  png public/img/logo.png 256x256 radial-gradient:'#f59e0b'-'#b45309'
  jpg public/photos/team.jpg 900x600 3
  jpg public/photos/office.jpg 900x600 11
}
docs() { mkdir -p assets && png assets/diagram.png 800x450 gradient:'#0f766e'-'#a7f3d0' && jpg assets/cover.jpg 1000x500 5; }
mobile() { mkdir -p res store && png res/icon.png 512x512 radial-gradient:'#22c55e'-'#14532d' && png res/splash.png 720x1280 gradient:'#111827'-'#4b5563' && jpg store/screen-1.jpg 600x1000 8; }
design() { mkdir -p tokens && png tokens/swatches.png 640x160 gradient:'#ef4444'-'#3b82f6'; }
blog() { mkdir -p posts && echo "Hello" >posts/hello.md; }
awesome() { mkdir -p media && png media/banner.png 1280x320 gradient:'#db2777'-'#f97316'; }

log "fixtures"
for tool in cargo git python3 curl magick jpegtran; do command -v "$tool" >/dev/null || { echo "missing $tool"; exit 1; }; done
new_repo acme/website website
new_repo acme/docs docs
new_repo acme/mobile-app mobile
new_repo acme/design-system design
new_repo jdev/blog blog
new_repo someone/awesome-list awesome
git init -q --bare "$WORK/bare/fork/awesome-list.git"

cat >"$WORK/github.json" <<JSON
{ "token": "fake-token", "user": { "id": 4242, "login": "jdev", "name": "Jane Developer" },
  "repos": {
    "acme/website":        { "push": true,  "clone": "$WORK/bare/acme/website.git" },
    "acme/docs":           { "push": true,  "clone": "$WORK/bare/acme/docs.git" },
    "acme/mobile-app":     { "push": true,  "clone": "$WORK/bare/acme/mobile-app.git" },
    "acme/design-system":  { "push": true,  "clone": "$WORK/bare/acme/design-system.git" },
    "acme/old-site":       { "push": true,  "clone": "$WORK/bare/acme/old-site.git" },
    "jdev/blog":           { "push": true,  "clone": "$WORK/bare/jdev/blog.git" },
    "someone/awesome-list":{ "push": false, "clone": "$WORK/bare/someone/awesome-list.git",
                             "fork_clone": "$WORK/bare/fork/awesome-list.git" } } }
JSON

# ------------------------------------------------------------------ start
log "build"
cargo build --quiet --manifest-path "$ROOT/Cargo.toml"
[[ -f $ROOT/frontend/dist/index.html ]] || (cd "$ROOT/frontend" && npm ci --no-audit --no-fund && npm run build)

log "start fake GitHub on :$API_PORT and git-opt on :$PORT"
python3 -I "$ROOT/scripts/mock-github.py" "$API_PORT" "$WORK/github.json" &
PIDS+=($!)
GIT_OPT_BIND=127.0.0.1:$PORT GIT_OPT_DATA_DIR="$WORK/data" GIT_OPT_STATIC_DIR="$ROOT/frontend/dist" \
  GIT_OPT_GITHUB_API=http://127.0.0.1:$API_PORT GITHUB_TOKEN=fake-token \
  "$ROOT/target/debug/git-opt" >"$WORK/server.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 40); do curl -fs "$API/health" >/dev/null 2>&1 && break; sleep 0.25; done

# ------------------------------------------------------------------- seed
log "seed"
curl -fsS "$API/settings" | json "json.dumps({**d['settings'], 'commit_author_name': 'Jane Developer', 'commit_author_email': 'jane@example.com', 'ignore_globs': '**/vendor/**\n*.min.png'})" >"$WORK/settings.json"
curl -fsS -X PUT -H 'content-type: application/json' --data @"$WORK/settings.json" "$API/settings" >/dev/null
post /repos '{"repo":"acme/mobile-app","auto_pr":false}'       # review before opening the PR
post /repos '{"repo":"someone/awesome-list"}'                   # third party: fork mode
post /sources '{"kind":"org","name":"acme"}'                    # imports the rest of acme/*
post /sources '{"kind":"user","name":"jdev"}'
sleep 1 && idle

# acme/docs: merge the bot's PR so the savings are credited.
pr=$(curl -fsS "http://127.0.0.1:$API_PORT/_mock/pulls" | json "[k for k, p in d.items() if p['repo'] == 'acme/docs'][0]")
(cd "$WORK/src/acme/docs" && git fetch -q "$WORK/bare/acme/docs.git" git-opt/optimize-images \
  && git -c user.name="Repo Owner" -c user.email=owner@example.com merge -q --no-ff FETCH_HEAD -m "Merge pull request #$pr" \
  && git push -q "$WORK/bare/acme/docs.git" main)
curl -fsS -X POST "http://127.0.0.1:$API_PORT/_mock/merge/$pr" >/dev/null
post "/repos/$(repo_id acme/docs)/check"

# acme/website: a new image lands while the PR is open, so the PR gets updated.
(cd "$WORK/src/acme/website" && png public/img/pattern.png 600x600 plasma:fractal && git add -A \
  && commit -m "Add background pattern" && git push -q "$WORK/bare/acme/website.git" main)
post "/repos/$(repo_id acme/website)/check"
idle

patch "/repos/$(repo_id acme/design-system)" '{"enabled":false}'
patch "/repos/$(repo_id jdev/blog)" '{"poll_interval_minutes":60}'

if [[ -z $SHOTS ]]; then
  log "demo ready: $BASE  (fake GitHub on :$API_PORT, Ctrl+C to stop)"
  wait
  exit 0
fi

# ------------------------------------------------------------ screenshots
WEBSITE=$(repo_id acme/website)
RUN=$(curl -fsS "$API/repos/$WEBSITE/runs" | json "d[0]['id']")
log "screenshots -> $SHOTS"
node "$ROOT/scripts/screenshots.mjs" "$BASE" "$SHOTS" \
  dashboard=/ repositories=/repos repository="/repos/$WEBSITE" \
  activity=/activity run="/activity/$RUN" sources=/sources settings=/settings
