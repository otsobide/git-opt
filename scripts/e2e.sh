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
magick -size 160x120 plasma:fractal -seed 3 -define webp:lossless=true -define webp:method=0 img/photo.webp
magick -delay 10 -size 64x64 plasma:fractal -seed 1 plasma:fractal -seed 2 plasma:fractal -seed 3 apng:img/anim.png
magick -size 128x128 radial-gradient:'#f59e0b'-'#1d4ed8' -define png:compression-level=0 "$WORK/fav.png"
python3 -I - "$WORK/fav.png" img/favicon.ico <<'PY'   # ICO with one PNG entry (ImageMagick writes BMP)
import struct, sys
png = open(sys.argv[1], "rb").read()
open(sys.argv[2], "wb").write(struct.pack("<HHHBBBBHHII", 0, 1, 1, 128, 128, 0, 0, 1, 32, len(png), 22) + png)
PY
cat >img/vector.svg <<'SVG'
<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->
<svg
   width="64"
   height="64"
   viewBox="0 0 64 64"
   inkscape:version="1.3"
   sodipodi:docname="vector.svg"
   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
   xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
   xmlns="http://www.w3.org/2000/svg">
  <sodipodi:namedview
     id="namedview1"
     pagecolor="#ffffff"
     inkscape:zoom="8" />
  <g
     inkscape:label="Layer 1"
     inkscape:groupmode="layer">
    <rect
       style="fill:#d97706"
       width="40"
       height="30"
       x="12"
       y="17" />
    <circle cx="32" cy="32" r="10" fill="#1d4ed8"/>
  </g>
</svg>
SVG
python3 -I - >"$WORK/drawing.svg" <<'PY'   # a bigger Inkscape drawing for the .svgz
print('<?xml version="1.0" encoding="UTF-8" standalone="no"?>')
print('<svg width="600" height="600" viewBox="0 0 600 600" inkscape:version="1.3"')
print('   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns="http://www.w3.org/2000/svg">')
for i in range(60):
    print(f'  <rect\n     inkscape:label="tile {i}"\n     inkscape:transform-center-x="{i}"\n     id="rect{i}"\n'
          f'     style="fill:#{i * 4:02x}{255 - i * 4:02x}80"\n     x="{i % 10 * 60}"\n     y="{i // 10 * 60}"\n     width="50"\n     height="50" />')
print('</svg>')
PY
(cd "$WORK" && gzip -1 -c drawing.svg) >img/vector.svgz   # the gzip header keeps the file name
magick -size 64x64 gradient:'#16a34a'-'#facc15' -define png:compression-level=0 "$WORK/badge.png"
printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><image width="64" height="64" href="data:image/png;base64,%s"/></svg>\n' \
  "$(base64 -w0 "$WORK/badge.png")" >img/badge.svg
magick -size 128x128 radial-gradient:'#a855f7'-'#1e1b4b' -define png:compression-level=0 "$WORK/app.png"
python3 -I - "$WORK/app.png" img/app.icns <<'PY'   # ICNS: TOC + PNG element + opaque RLE element
import struct, sys
png = open(sys.argv[1], "rb").read()
items = [(b"ic07", png), (b"is32", bytes(range(200)))]
toc = b"".join(k + struct.pack(">I", len(d) + 8) for k, d in items)
body = b"".join(k + struct.pack(">I", len(d) + 8) + d for k, d in [(b"TOC ", toc)] + items)
open(sys.argv[2], "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)
PY
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

step "add octo/demo -> PR with every format optimized"
curl -fs -X POST -H 'content-type: application/json' -d '{"repo":"octo/demo"}' "$API/repos" >/dev/null
wait_idle 1
[[ $(last_run 1 status) == pr_opened ]] || fail "expected pr_opened, got $(last_run 1 status): $(last_run 1 message)"
optimized=$(curl -fs "$API/runs/$(last_run 1 id)" | json "' '.join(sorted(f['path'] for f in d['files']))")
expected="img/anim.png img/app.icns img/badge.svg img/big.png img/favicon.ico img/icon.png img/photo.jpg img/photo.webp img/vector.svg img/vector.svgz"
[[ $optimized == "$expected" ]] || fail "optimized files: $optimized"

step "pixels are identical (ImageMagick AE metric; SVG rendered by librsvg)"
cd "$WORK" && git clone -q demo.git check && cd check
for f in img/big.png img/icon.png img/photo.jpg img/photo.webp img/anim.png img/favicon.ico img/vector.svg \
  img/vector.svgz img/badge.svg img/app.icns; do
  ext=${f##*.}
  git show "main:$f" >"$WORK/orig.$ext"
  git show "origin/git-opt/optimize-images:$f" >"$WORK/new.$ext"
  if [[ $ext == icns ]]; then
    # ImageMagick can't read ICNS: compare the PNG element, require the rest byte-identical.
    python3 -I - "$WORK/orig.icns" "$WORK/new.icns" "$WORK" <<'PY' || fail "ICNS elements differ"
import struct, sys
def elements(path):
    d = open(path, "rb").read(); pos, out = 8, {}
    while pos < len(d):
        kind, n = d[pos:pos + 4], struct.unpack(">I", d[pos + 4:pos + 8])[0]
        out[kind] = d[pos + 8:pos + n]; pos += n
    return out
a, b = elements(sys.argv[1]), elements(sys.argv[2])
assert list(a) == list(b) and a[b"is32"] == b[b"is32"]
open(sys.argv[3] + "/orig-ic07.png", "wb").write(a[b"ic07"]); open(sys.argv[3] + "/new-ic07.png", "wb").write(b[b"ic07"])
PY
    ae=$(magick compare -metric AE "$WORK/orig-ic07.png" "$WORK/new-ic07.png" null: 2>&1 || true)
  else
    ae=$(magick compare -metric AE "$WORK/orig.$ext" "$WORK/new.$ext" null: 2>&1 || true)
  fi
  [[ $ae == 0* ]] || fail "$f differs: AE=$ae"
  printf "%-17s %7s -> %7s bytes  AE=%s\n" "$f" "$(stat -c %s "$WORK/orig.$ext")" "$(stat -c %s "$WORK/new.$ext")" "$ae"
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
