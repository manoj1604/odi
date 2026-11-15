#!/bin/bash
# A page as a browser loads it, once the app is up from a registry: the HTML,
# then every asset it names (src, href), 6 at a time. How long, and how many
# requests to the registry it costs.
#
#   bench/page.sh FILE.odi PAGE [DELAY_MS]
set -u
FILE=$1; PAGE=$2; DELAY=${3:-50}; ODI=${ODI:-odi}; PORT=8390; WEB=8500
HERE=$(cd "$(dirname "$0")" && pwd); LOG=$(mktemp); HOME_DIR=$(mktemp -d)
DELAY_MS=$DELAY python3 "$HERE/mockhub.py" $PORT >/dev/null 2>&1 & HUB=$!
trap 'kill $HUB 2>/dev/null; rm -rf "$LOG" "$HOME_DIR"' EXIT
sleep 1
export ODI_CACHE=$HOME_DIR/cache ODI_HOME=$HOME_DIR ODY_HOME=$HOME_DIR/none ODI_REGISTRY=http://127.0.0.1:$PORT
"$ODI" login --token secret >/dev/null && "$ODI" push bench/app:1 "$FILE" >/dev/null || exit 1
"$ODI" run bench/app:1 --port $WEB > "$LOG" 2>&1 & RUN=$!
# (Up: anything but no answer or the runner's 502 while the app starts.)
until code=$(curl -s -o /dev/null -w '%{http_code}' "localhost:$WEB/favicon.ico") && [ "$code" != 000 ] && [ "$code" != 502 ]; do sleep 0.01; done
before=$(grep -c . "$LOG")
t=$(date +%s%N)
html=$(curl -s "localhost:$WEB$PAGE")
assets=$(echo "$html" | grep -oE '(src|href)="[^"#]+' | sed -E 's/^(src|href)="//; s/&amp;/\&/g' | grep -vE '^(https?:|//|data:|mailto:)' | sort -u)
base=${PAGE%/*}/
for a in $assets; do case $a in /*) echo "$a";; *) echo "$base$a";; esac; done \
  | xargs -P 6 -I{} curl -s -o /dev/null -H 'accept: image/webp' "localhost:$WEB{}"
echo "$PAGE with $(echo "$assets" | grep -c .) assets: $(( ($(date +%s%N) - t) / 1000000 )) ms"
kill -TERM $RUN; wait $RUN 2>/dev/null
grep "^odi: fetched" "$LOG"
