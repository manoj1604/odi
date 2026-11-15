#!/bin/bash
# The first request to a route once an app is up from a registry: how long it
# takes and how many requests it costs. With a route map (meta/odi/routes.json)
# its files come in one batch.
#
#   bench/route.sh FILE.odi ROUTE [DELAY_MS]
set -u
FILE=$1; ROUTE=$2; DELAY=${3:-50}; ODI=${ODI:-odi}; PORT=8390; WEB=8500
HERE=$(cd "$(dirname "$0")" && pwd); LOG=$(mktemp); HOME_DIR=$(mktemp -d)
DELAY_MS=$DELAY python3 "$HERE/mockhub.py" $PORT >/dev/null 2>&1 & HUB=$!
trap 'kill $HUB 2>/dev/null; rm -rf "$LOG" "$HOME_DIR"' EXIT
sleep 1
export ODI_CACHE=$HOME_DIR/cache ODI_HOME=$HOME_DIR ODY_HOME=$HOME_DIR/none ODI_REGISTRY=http://127.0.0.1:$PORT ODI_TRACE=$HOME_DIR/trace
"$ODI" login --token secret >/dev/null && "$ODI" push bench/app:1 "$FILE" >/dev/null || exit 1
"$ODI" run bench/app:1 --port $WEB > "$LOG" 2>&1 & RUN=$!
until curl -sf -o /dev/null "localhost:$WEB/"; do sleep 0.01; done
before=$(wc -l < "$HOME_DIR/trace")
t=$(date +%s%N); curl -s -o /dev/null "localhost:$WEB$ROUTE"
echo "first $ROUTE: $(( ($(date +%s%N) - t) / 1000000 )) ms, $(( $(wc -l < "$HOME_DIR/trace") - before )) pieces fetched for it"
t=$(date +%s%N); curl -s -o /dev/null "localhost:$WEB$ROUTE"
echo "again: $(( ($(date +%s%N) - t) / 1000000 )) ms"
kill -TERM $RUN; wait $RUN 2>/dev/null
grep "^odi: fetched" "$LOG"
