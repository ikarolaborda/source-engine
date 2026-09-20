#!/bin/sh
set -eu

# Detach known writable paths inherited from older content-link installers.
# Preserve each original symlink in a temporary backup instead of deleting it.
[ "$#" -eq 2 ] || { echo "usage: $0 <runtime> <game>" >&2; exit 2; }
RUNTIME=$(CDPATH= cd -- "$1" && pwd)
GAME=$2
case "$GAME" in ''|*[!a-zA-Z0-9_-]*) exit 2 ;; esac
BACKUP=
detach()
{
	PATH_TO_DETACH=$1
	[ -L "$PATH_TO_DETACH" ] || return 0
	if [ -z "$BACKUP" ]; then BACKUP=$(mktemp -d /tmp/source-rust-writable-links.XXXXXX); fi
	NAME=$(basename "$PATH_TO_DETACH")
	if [ -e "$PATH_TO_DETACH" ]; then
		cp -RL "$PATH_TO_DETACH" "$BACKUP/$NAME.private"
	fi
	mv "$PATH_TO_DETACH" "$BACKUP/$NAME.original-link"
	if [ -e "$BACKUP/$NAME.private" ]; then
		mv "$BACKUP/$NAME.private" "$PATH_TO_DETACH"
	fi
}

detach "$RUNTIME/platform"
detach "$RUNTIME/$GAME/glshaders.cfg"
for CACHE in "$RUNTIME/$GAME"/*.sound.cache; do detach "$CACHE"; done
if [ -n "$BACKUP" ]; then
	echo "runtime writable paths isolated; original links preserved in $BACKUP"
fi
