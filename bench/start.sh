#!/bin/bash
# How long an ODI file takes to answer its first request when run from a
# registry, read as needed (Linux: FUSE), and what it fetched.
#
#   bench/start.sh FILE.odi [DELAY_MS] [PATH]
#
# Starts bench/mockhub.py with DELAY_MS before each answer (default 50),
# pushes FILE, runs it with odi ($ODI, default: odi on PATH), and prints when
# each step happened, then the first request to PATH (default /). Every run
# starts with an empty cache; FILL=1: odi fill first (the host filled ahead
# of the first request), FILL=all: odi fill --all.
set -u
FILE=$1; DELAY=${2:-50}; URLPATH=${3:-/}; ODI=${ODI:-odi}; PORT=8390; WEB=8500
HERE=$(cd "$(dirname "$0")" && pwd); LOG=$(mktemp); HOME_DIR=$(mktemp -d)
DELAY_MS=$DELAY python3 "$HERE/mockhub.py" $PORT >/dev/null 2>&1 & HUB=$!
trap 'kill $HUB 2>/dev/null; rm -rf "$LOG" "$HOME_DIR"' EXIT
sleep 1
export ODI_CACHE=$HOME_DIR/cache ODI_HOME=$HOME_DIR ODY_HOME=$HOME_DIR/none ODI_REGISTRY=http://127.0.0.1:$PORT
"$ODI" login --token secret >/dev/null && "$ODI" push bench/app:1 "$FILE" >/dev/null || exit 1
case "${FILL:-}" in
  1) "$ODI" fill bench/app:1 || exit 1 ;;
  all) "$ODI" fill --all bench/app:1 || exit 1 ;;
esac
start=$(date +%s%N); ms() { echo $(( ($(date +%s%N) - start) / 1000000 )); }
"$ODI" run bench/app:1 --port $WEB > "$LOG" 2>&1 & RUN=$!
seen=""
for _ in $(seq 1 6000); do
  for pat in "start list" "web started"; do
    if [[ "$seen" != *"$pat"* ]] && grep -q "$pat" "$LOG"; then
      seen="$seen|$pat"; echo "$(ms) ms  $(grep -m1 "$pat" "$LOG")"
    fi
  done
  if curl -sf -o /dev/null "localhost:$WEB$URLPATH"; then echo "$(ms) ms  $URLPATH answered"; break; fi
  sleep 0.01
done
kill -TERM $RUN; wait $RUN 2>/dev/null
grep "^odi: fetched" "$LOG"
