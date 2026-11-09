#!/usr/bin/env bash
# Run git-opt against a fake GitHub API with sample repositories in every
# state (open PR, merged, awaiting approval, fork contribution, error, paused),
# so the UI can be explored without touching github.com.
#
#   scripts/demo.sh                     # serve the demo on http://127.0.0.1:18484
#   scripts/demo.sh --screenshots DIR   # capture the UI in dark mode into DIR and exit
#
# Needs: cargo, git, python3, curl and ImageMagick (`magick`); npm to
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
  magick -size 480x320 plasma:fractal -seed 5 -define webp:lossless=true -define webp:method=0 public/photos/map.webp
  ico public/favicon.ico 128
  inkscape_svg public/img/mark.svg
}
# ico <path> <size>: an icon holding one uncompressed PNG (ImageMagick writes BMP entries).
ico() {
  png "$WORK/ico.png" "$2x$2" radial-gradient:'#22d3ee'-'#0e7490'
  python3 -I - "$WORK/ico.png" "$1" "$2" <<'PY'
import struct, sys
png = open(sys.argv[1], "rb").read()
size = int(sys.argv[3]) % 256
open(sys.argv[2], "wb").write(struct.pack("<HHHBBBBHHII", 0, 1, 1, size, size, 0, 0, 1, 32, len(png), 22) + png)
PY
}
# An SVG saved by Inkscape, with its editor metadata.
inkscape_svg() {
  cat >"$1" <<'SVG'
<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->
<svg
   width="96"
   height="96"
   viewBox="0 0 96 96"
   version="1.1"
   id="svg1"
   inkscape:version="1.3 (0e150ed6c4, 2023-07-21)"
   sodipodi:docname="mark.svg"
   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
   xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
   xmlns="http://www.w3.org/2000/svg"
   xmlns:svg="http://www.w3.org/2000/svg">
  <sodipodi:namedview
     id="namedview1"
     pagecolor="#ffffff"
     bordercolor="#000000"
     borderopacity="0.25"
     inkscape:showpageshadow="2"
     inkscape:pageopacity="0.0"
     inkscape:pagecheckerboard="0"
     inkscape:deskcolor="#d1d1d1"
     inkscape:document-units="px"
     inkscape:zoom="5.6"
     inkscape:cx="48"
     inkscape:cy="48"
     inkscape:window-width="1920"
     inkscape:window-height="1011"
     inkscape:current-layer="layer1" />
  <defs
     id="defs1" />
  <g
     inkscape:label="Layer 1"
     inkscape:groupmode="layer"
     id="layer1">
    <rect
       style="fill:#0e7490;stroke:none"
       id="rect1"
       width="72"
       height="72"
       x="12"
       y="12"
       rx="14" />
    <path
       style="fill:#f59e0b"
       d="M 30,66 48,28 66,66 Z"
       id="path1"
       sodipodi:nodetypes="cccc" />
  </g>
</svg>
SVG
}
docs() { mkdir -p assets && png assets/diagram.png 800x450 gradient:'#0f766e'-'#a7f3d0' && jpg assets/cover.jpg 1000x500 5; }
mobile() { mkdir -p res store && png res/icon.png 512x512 radial-gradient:'#22c55e'-'#14532d' && png res/splash.png 720x1280 gradient:'#111827'-'#4b5563' && jpg store/screen-1.jpg 600x1000 8; }
design() { mkdir -p tokens && png tokens/swatches.png 640x160 gradient:'#ef4444'-'#3b82f6'; }
blog() { mkdir -p posts && echo "Hello" >posts/hello.md; }
awesome() { mkdir -p media && png media/banner.png 1280x320 gradient:'#db2777'-'#f97316'; }

log "fixtures"
for tool in cargo git python3 curl magick; do command -v "$tool" >/dev/null || { echo "missing $tool"; exit 1; }; done
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
    "acme/old-site":       { "push": true,  "clone": "$WORK/bare/acme/old-site.git", "deleted": true },
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
GIF=$(command -v gifsicle >/dev/null && echo True || echo False)   # hide the "missing tool" warning
curl -fsS "$API/settings" | json "json.dumps({**d['settings'], 'commit_author_name': 'Jane Developer', 'commit_author_email': 'jane@example.com', 'ignore_globs': '**/vendor/**\n*.min.png', 'optimizers': {'gifsicle': $GIF}})" >"$WORK/settings.json"
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
