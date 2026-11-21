#!/bin/bash
# The hub workflow end to end, against bench/mockhub.py: build a project,
# publish versions, list them, pull by range, compare, roll back.
#   tests/hub.sh [ODI]      (default: target/debug/odi)
set -euo pipefail
cd "$(dirname "$0")/.."
ODI=$(cd "$(dirname "${1:-target/debug/odi}")" && pwd)/$(basename "${1:-target/debug/odi}")
PORT=${PORT:-8395}
W=$(mktemp -d)
python3 bench/mockhub.py "$PORT" & HUB=$!; disown
trap 'kill $HUB 2>/dev/null; rm -rf "$W"' EXIT
export ODI_HOME=$W/home ODY_HOME=$W/none ODI_CACHE=$W/cache ODI_REGISTRY=http://127.0.0.1:$PORT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com
for _ in $(seq 50); do curl -s "$ODI_REGISTRY/v1/search" >/dev/null && break; sleep 0.1; done
step() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
odi() { "$ODI" "$@"; }

odi login --token secret >/dev/null
mkdir "$W/site" && cd "$W/site"
git init -q && echo '<h1>one</h1>' > index.html

step "odi.json: required, written by odi init, checked"
{ odi build -o x.odi 2>&1 || true; } | grep "odi init writes one" >/dev/null || fail "no odi.json: says odi init"
odi init >/dev/null && grep '"type": "static"' odi.json >/dev/null || fail "odi init: a static site"
{ odi init 2>&1 || true; } | grep "has one already" >/dev/null || fail "odi init twice"
echo '{"name": "site", "type": "static", "repo": "acme/site", "includ_runtime": true}' > odi.json
{ odi build -o x.odi 2>&1 || true; } | grep 'did you mean "include_runtime"' >/dev/null || fail "a misspelled key"
echo '{"name": "site", "type": "static", "repo": "acme/site"}' > odi.json
git add . && git commit -qm one && git tag v1.0.0

step "build: the version comes from the git tag"
odi build -o site.odi >/dev/null
odi inspect site.odi | grep "site 1.0.0" >/dev/null || fail "version from git tag"

step "push with no name: odi.json's repo, the file's version, and latest"
odi push site.odi | tee "$W/out"
grep -q "acme/site  1.0.0, latest" "$W/out" || fail "push 1.0.0"

step "the same again: nothing moves"
odi push site.odi | grep "1.0.0 (already), latest (already)" >/dev/null || fail "idempotent push"

step "a change, published as the next patch"
echo '<h1>two</h1>' > index.html && git commit -qam two
odi build -o site.odi >/dev/null
odi push --bump patch site.odi | tee "$W/out"
grep -q "acme/site  1.0.1, latest" "$W/out" || fail "bump"

step "a published version doesn't change"
if odi push acme/site:1.0.0 site.odi 2>"$W/err"; then fail "overwrote 1.0.0"; fi
cat "$W/err"; grep -q "already published" "$W/err" || fail "refusal message"

step "a pre-release doesn't move latest"
echo '<h1>three</h1>' > index.html && git commit -qam three && odi build -o site.odi >/dev/null
odi push acme/site:1.1.0-rc.1 site.odi --tag next | grep "1.1.0-rc.1, next" >/dev/null || fail "pre-release"

step "versions"
odi versions | tee "$W/out"
grep -E "^1\.0\.1 +latest" "$W/out" >/dev/null || fail "latest on 1.0.1"
grep -E "^1\.1\.0-rc\.1 +next" "$W/out" >/dev/null || fail "next on rc"

step "pull by range: 1.0 is the newest 1.0.x"
cd "$W" && odi pull acme/site:1.0 -o got.odi 2>&1 | tee "$W/out"
grep -q "acme/site:1.0 is 1.0.1" "$W/out" || fail "range"
odi inspect got.odi | grep "site" >/dev/null || fail "pulled file"

step "diff two versions"
odi diff acme/site:1.0.0 acme/site:1.0.1 | tee "$W/out"
grep -q "~ static/index.html" "$W/out" || fail "diff file"
grep -q "version   1.0.0 -> " "$W/out" || fail "diff version"

step "roll back: latest to 1.0.0, without pushing"
odi tag acme/site:1.0.0 latest | grep "acme/site:latest -> 1.0.0" >/dev/null || fail "tag"
odi versions acme/site | grep -E "^1\.0\.0 +latest" >/dev/null || fail "latest on 1.0.0"

step "a version can't be moved by tag either, unless forced"
if odi tag acme/site:1.0.1 1.0.0 2>/dev/null; then fail "moved 1.0.0"; fi
odi tag acme/site:1.0.1 1.0.0 --force >/dev/null

step "search and repos"
odi search | grep "no public repositories" >/dev/null || fail "nothing public yet"
cd "$W/site" && odi push acme/site:1.0.2 site.odi --public --no-latest >/dev/null && cd "$W"
odi search site | grep "acme/site" >/dev/null || fail "search"
odi repos | grep "acme/site  public" >/dev/null || fail "repos"

step "step inside: odi shell"
odi shell acme/site:1.0.2 -- cat static/index.html | grep "three" >/dev/null || fail "shell"
odi shell acme/site:1.0.2 static -- ls | grep "index.html" >/dev/null || fail "shell in a folder"

step "cd into it: odi init, odi open, odi close"
PATH="$(dirname "$ODI"):$PATH"   # (the cd it prints runs the odi command itself)
odi hook bash | grep "builtin cd" >/dev/null || fail "hook"
( eval "$(odi hook bash)"; cd acme/site:1.0.2/static && grep three index.html >/dev/null ) || fail "cd a hub name"
( eval "$(odi hook bash)"; cd "$W/site/site.odi" && ls static/index.html >/dev/null ) || fail "cd a file"
odi close | grep "closed" >/dev/null || fail "close"

step "bundle: one executable, no odi needed to run it"
cd "$W" && odi bundle "$W/site/site.odi" -o "$W/site-app" --runner "$ODI" >/dev/null
"$W/site-app" --port 8396 >/dev/null 2>&1 & APP=$!
for _ in $(seq 50); do curl -s localhost:8396/ >/dev/null && break; sleep 0.1; done
curl -s localhost:8396/ | grep "three" >/dev/null || fail "bundle serves"
kill $APP
"$W/site-app" --odi-file "$W/back.odi" 2>/dev/null && cmp -s "$W/back.odi" "$W/site/site.odi" || fail "bundle: the same file back"

echo; echo "hub workflow: ok"
