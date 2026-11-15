#!/bin/bash
# A set run from a registry, its parts each in their own repository: how
# long until it answers, and what it fetched.
#
#   bench/set.sh APP.odi RUNTIME.odi [DELAY_MS] [PROFILE] [RUNTIME_HERE]
#
# Starts bench/mockhub.py with DELAY_MS before each answer (default 50);
# pushes the runtime part (bench/node) and the app (bench/app) to their own
# repositories; makes a set of the two named from the registry; with
# PROFILE=1, profiles the set (a start list across its parts); pushes the
# set (bench/set); then runs it with an empty store and cache, so every part
# comes from the registry, read as needed (Linux: FUSE). RUNTIME_HERE=1: the
# runtime part is in the store already (another app on this host uses it).
set -u
APP=$1; RT=$2; DELAY=${3:-50}; PROFILE=${4:-0}; RT_HERE=${5:-0}; ODI=${ODI:-odi}; PORT=8391; WEB=8501
HERE=$(cd "$(dirname "$0")" && pwd); LOG=$(mktemp); HOME_DIR=$(mktemp -d)
DELAY_MS=$DELAY LOG=${HUBLOG:-} python3 "$HERE/mockhub.py" $PORT >/dev/null 2>&1 & HUB=$!
trap 'kill $HUB 2>/dev/null; rm -rf "$LOG" "$HOME_DIR"' EXIT
sleep 1
export ODI_HOME=$HOME_DIR ODY_HOME=$HOME_DIR/none ODI_REGISTRY=http://127.0.0.1:$PORT
"$ODI" login --token secret >/dev/null || exit 1
"$ODI" push bench/node:1 "$RT" >/dev/null && "$ODI" push bench/app:1 "$APP" >/dev/null || exit 1
ODI_CACHE=$HOME_DIR/make "$ODI" set bench/app:1 --runtime bench/node:1 -o "$HOME_DIR/set.odi" || exit 1
if [ "$PROFILE" = 1 ]; then
  ODI_CACHE=$HOME_DIR/make "$ODI" profile "$HOME_DIR/set.odi" --port $WEB 2>&1 | grep -v "^odi: \(web\|main\|runtime\)" || exit 1
fi
"$ODI" push bench/set:1 "$HOME_DIR/set.odi" >/dev/null || exit 1
[ "$RT_HERE" = 1 ] && { ODI_CACHE=$HOME_DIR/run "$ODI" part add "$RT" >/dev/null || exit 1; }
start=$(date +%s%N); ms() { echo $(( ($(date +%s%N) - start) / 1000000 )); }
ODI_CACHE=$HOME_DIR/run "$ODI" run bench/set:1 --port $WEB > "$LOG" 2>&1 & RUN=$!
seen=""
for _ in $(seq 1 6000); do
  for pat in "runtime" "start list" "web started"; do
    if [[ "$seen" != *"$pat"* ]] && grep -q "$pat" "$LOG"; then
      seen="$seen|$pat"; echo "$(ms) ms  $(grep -m1 "$pat" "$LOG")"
    fi
  done
  if curl -sf -o /dev/null "localhost:$WEB/"; then echo "$(ms) ms  / answered"; break; fi
  sleep 0.01
done
kill -TERM $RUN; wait $RUN 2>/dev/null
grep "^odi: fetched" "$LOG"
