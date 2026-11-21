#!/bin/bash
# cd into ODI files (odi hook), in every shell here (zsh, bash, fish): in
# and out (.., ../.., ../x), paths with spaces, mistyped paths, two shells
# opening one file at once, an old hook with this odi, a shell with odi
# uninstalled, tab completion (bash), and (Linux, FUSE) a mount whose odi
# was killed.
#   tests/cd.sh [ODI]      (default: target/debug/odi)
set -uo pipefail
cd "$(dirname "$0")/.."
ODI=$(cd "$(dirname "${1:-target/debug/odi}")" && pwd)/$(basename "${1:-target/debug/odi}")
W=$(mktemp -d)
trap 'cd /; PATH="$(dirname "$ODI"):$PATH" odi close >/dev/null 2>&1; chmod -R u+w "$W" 2>/dev/null; rm -rf "$W"' EXIT
export ODI_HOME=$W/home ODI_CACHE=$W/cache ODY_HOME=$W/none ODI_REGISTRY=http://127.0.0.1:9
export PATH="$(dirname "$ODI"):$PATH"
fails=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fails=$((fails + 1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (got: $2, wanted: $3)"; fi; }

# A small site in a folder with a space in its name.
mkdir -p "$W/src/assets" "$W/my files"
echo '<h1>hi</h1>' > "$W/src/index.html"; echo a > "$W/src/assets/a.txt"
echo '{"name": "site", "type": "static"}' > "$W/src/odi.json"
(cd "$W/src" && odi build -o "$W/my files/site.odi" >/dev/null 2>&1) || { echo "FAIL: building the test file"; exit 1; }
F="$W/my files"
real() { (builtin cd "$1" && pwd -P); }
FP=$(real "$F")

walk_posix() { # SHELL: the same walk in zsh or bash
  "$@" <<EOF
eval "\$(odi hook ${1%% *})"
builtin cd "$F" || exit 1
cd site.odi                 && echo "in=\$(basename "\$PWD")"
cd static/assets            && echo "down=\$(basename "\$PWD")"
cd ..                       && echo "up=\$(basename "\$PWD")"
cd ../..                    && echo "out=\$(pwd -P)"
cd site.odi/static          && cd ../../..  && echo "out2=\$(pwd -P)"
cd "my files/site.odi/static" && cd ../assets 2>/dev/null; cd ../static && echo "sibling=\$(basename "\$PWD")"
cd /tmp && cd .. && echo "plain=\$PWD"
cd "$F"; cd - >/dev/null && echo "dash=\$PWD"
cd notes:old 2>&1 | head -1
cd nope.odi 2>&1 | head -1
EOF
}

for sh in zsh bash; do
  command -v $sh >/dev/null || { echo "skip $sh (not here)"; continue; }
  case $sh in zsh) run="zsh -f";; bash) run="bash --norc --noprofile";; esac
  out=$(walk_posix $run 2>&1)
  get() { printf '%s\n' "$out" | sed -n "s/^$1=//p" | head -1; }
  check "$sh: cd site.odi" "$(get in)" "site-$(odi inspect "$F/site.odi" | sed -n 's/.*sha256:\([0-9a-f]\{12\}\).*/\1/p' | head -1)"
  check "$sh: cd static/assets" "$(get down)" "assets"
  check "$sh: cd .." "$(get up)" "static"
  check "$sh: cd ../.. out of the file" "$(get out)" "$FP"
  check "$sh: cd ../../.. past it" "$(get out2)" "$(dirname "$FP")"
  check "$sh: cd ../static" "$(get sibling)" "static"
  check "$sh: plain cd unchanged" "$(get plain)" "/"
  check "$sh: cd - (the shell's own)" "$(get dash)" "/"
  printf '%s\n' "$out" | grep -i "notes:old" | grep -qi "no such file" && ok "$sh: a mistyped path with : is the shell's own error" || bad "$sh: a mistyped path with : ($out)"
  printf '%s\n' "$out" | grep -q "no file nope.odi in" && ok "$sh: a missing file says where it looked" || bad "$sh: a missing file"
done

if command -v fish >/dev/null; then
  out=$(fish --no-config -c "odi hook fish | source; builtin cd '$F'; cd site.odi/static; echo down=(basename \$PWD); cd ../..; echo out=(pwd -P); cd notes:old" 2>&1)
  check "fish: cd into a folder in it" "$(printf '%s\n' "$out" | sed -n 's/^down=//p')" "static"
  check "fish: cd ../.. out of it" "$(printf '%s\n' "$out" | sed -n 's/^out=//p')" "$FP"
  printf '%s\n' "$out" | grep -i "notes:old" >/dev/null && ok "fish: a mistyped path is the shell's own error" || bad "fish: a mistyped path ($out)"
else
  echo "skip fish (not here)"
fi

# A mistyped path is said at once, with no network.
t0=$(date +%s); odi open notes:old >/dev/null 2>&1; t1=$(date +%s)
[ $((t1 - t0)) -le 2 ] && ok "a mistyped path: no network wait" || bad "a mistyped path took $((t1 - t0)) s"

# An old hook (it ran odi open FILE) with this odi.
d=$(cd "$F" && odi open site.odi) && [ -d "$d/static" ] && ok "odi open FILE (older hooks)" || bad "odi open FILE"

# Two shells opening one file at once, one of them already in it.
odi close >/dev/null 2>&1
top=$(cd "$F" && odi open site.odi)
( builtin cd "$top/static" && sleep 3 ) & held=$!
( cd "$F" && odi open site.odi >/dev/null ) & a=$!
( cd "$F" && odi open site.odi >/dev/null ) & b=$!
wait $a && wait $b && ok "two opens at once" || bad "two opens at once"
[ -f "$top/static/index.html" ] && ok "the copy a shell is in stays" || bad "the copy a shell was in went"
wait $held 2>/dev/null

# Removing odi doesn't break a new terminal.
for sh in zsh bash; do
  command -v $sh >/dev/null || continue
  out=$(env PATH=/usr/bin:/bin $sh -c 'command -v odi >/dev/null && eval "$(odi hook '$sh')"; echo started' 2>&1)
  check "$sh: starts with odi uninstalled" "$out" "started"
done
{ odi hook 2>&1 || true; } | grep -q 'command -v odi >/dev/null && eval' && ok "odi hook shows the safe startup line" || bad "odi hook's usage"

# Tab completion (bash): folders and .odi files after cd.
if command -v bash >/dev/null; then
  out=$(builtin cd "$F" && bash --norc -c 'eval "$(odi hook bash)"; mkdir -p dir1; COMP_WORDS=(cd ""); COMP_CWORD=1; _odi_cd_complete; printf "%s\n" "${COMPREPLY[@]}"' 2>&1)
  printf '%s\n' "$out" | grep -qx "site.odi" && printf '%s\n' "$out" | grep -qx "dir1" && ok "bash: tab offers .odi files and folders" || bad "bash completion ($out)"
fi

# Linux: a mount whose odi was killed is mounted again.
if [ "$(uname -s)" = Linux ] && [ -e /dev/fuse ] && command -v fusermount3 >/dev/null; then
  odi close >/dev/null 2>&1
  top=$(cd "$F" && odi open site.odi)
  if grep -q " $top " /proc/self/mountinfo; then
    pkill -9 -f "odi __mount .*site.odi" ; sleep 1
    ls "$top" >/dev/null 2>&1 && bad "the killed mount still answered" || ok "a killed mount is broken (as expected)"
    top2=$(cd "$F" && odi open site.odi) && [ -f "$top2/static/index.html" ] && ok "a broken mount is mounted again" || bad "a broken mount wasn't mounted again"
  else
    bad "not mounted on Linux with FUSE"
  fi
fi

out=$(odi close 2>&1); printf '%s\n' "$out" | grep -q "closed" && ok "odi close" || bad "odi close ($out)"
echo
[ $fails -eq 0 ] && echo "cd: ok" || { echo "cd: $fails failed"; exit 1; }
