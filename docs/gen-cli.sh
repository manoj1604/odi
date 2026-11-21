#!/bin/sh
# Makes docs/cli.md from odi's own help, so the reference matches the
# command: sh docs/gen-cli.sh [ODI]   (default: builds target/debug/odi)
set -e
cd "$(dirname "$0")/.."
if [ -n "${1:-}" ]; then ODI=$1; else "${CARGO:-cargo}" build -q -p odi; ODI=target/debug/odi; fi
{
  echo "# Command reference"
  echo
  echo "Made from \`odi help\` by \`docs/gen-cli.sh\`: don't edit by hand."
  echo
  echo '```'
  "$ODI" help | sed "s/^odi [0-9][^:]*:/odi:/"
  echo '```'
  for c in $("$ODI" help --list); do
    echo
    echo "## odi $c"
    echo
    echo '```'
    "$ODI" help "$c"
    echo '```'
  done
  echo
  echo "# Experimental commands"
  echo
  echo "These work, and may change before they join the list above (see"
  echo "[TODO.md](../TODO.md)). \`odi help --all\` lists them."
  for c in $("$ODI" help --list-more); do
    echo
    echo "## odi $c"
    echo
    echo '```'
    "$ODI" help "$c"
    echo '```'
  done
} > docs/cli.md
echo "wrote docs/cli.md"
