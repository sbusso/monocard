#!/bin/sh
# Write Monocard's THIRD-PARTY-NOTICES.
#
#     scripts/monocard-notices.sh [--no-crate-texts] [output]
#
# docs/monocard/THIRD-PARTY-NOTICES.md, then Greycard's own notices
# (scripts/notices.sh, with the crates of the `mono-usb` feature, which are
# `mono`'s and the Mac's USB import's). Those need cargo-about.
# --no-crate-texts does without it and lists the crates by name and license
# instead of giving their texts: for a test build, not for a release.

set -eu
cd "$(dirname "$0")/.."

texts=1
out=
for arg; do
    case "$arg" in
    --no-crate-texts) texts= ;;
    -*)
        echo "monocard-notices.sh: $arg is not an option" >&2
        exit 2
        ;;
    *) out=$arg ;;
    esac
done
out=${out:-${CARGO_TARGET_DIR:-target}/MONOCARD-THIRD-PARTY-NOTICES}
mkdir -p "$(dirname "$out")"
rest="$out.rest"
trap 'rm -f "$rest"' EXIT

if [ -n "$texts" ]; then
    NOTICES_FEATURES=mono-usb scripts/notices.sh "$rest" >/dev/null
else
    {
        cat packaging/notices/bundled.txt
        sed 's/^/    /; s/^ *$//' crates/greycard-slint/ui/icons/LICENSE
        cat packaging/notices/ported.txt
        echo "The license texts of these crates are left out of this test build."
        echo
        cargo metadata --locked --format-version 1 | python3 -c '
import json, sys
for p in sorted(json.load(sys.stdin)["packages"], key=lambda p: (p["name"], p["version"])):
    if p["source"]:
        print("  %s %s: %s" % (p["name"], p["version"], p["license"] or "see its source"))'
    } >"$rest"
fi
{
    # A Windows checkout may have CRLF line ends.
    tr -d '\r' <docs/monocard/THIRD-PARTY-NOTICES.md
    printf '\n'
    cat "$rest"
} >"$out"
echo "$out"
