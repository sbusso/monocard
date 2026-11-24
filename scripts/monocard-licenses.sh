#!/bin/sh
# Monocard's license and trademark scan. Every failure is listed; the
# exit status is 1 when there is any.
#
#     scripts/monocard-licenses.sh
#
#   1. No crate in the dependency tree is under the AGPL alone.
#   2. No code file names RapidRAW, which is AGPL.
#   3. Every mono file that took something from darktable names the
#      file and the commit it came from.
#   4. Ricoh's marks are in no product name, icon, file name of either,
#      or bundle id.

set -eu
cd "$(dirname "$0")/.."

failed=0
say() {
    echo "monocard-licenses.sh: $1" >&2
    printf '%s\n' "$2" | sed 's/^/  /' >&2
    failed=1
}
# Tracked files and new ones not ignored, so a scan before a commit sees
# what the commit will hold.
files() { git ls-files -co --exclude-standard -- "$@"; }

# 1. A choice that has another license in it is not the AGPL's alone.
agpl=$(cargo metadata --locked --format-version 1 | python3 -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    alone = [a for a in (p["license"] or "").split(" OR ") if "AGPL" not in a]
    if "AGPL" in (p["license"] or "") and not alone:
        print(p["name"], p["version"], p["license"])')
[ -z "$agpl" ] || say "crates under the AGPL:" "$agpl"

# 2. This script has to name it.
rapid=$(files '*.rs' '*.wgsl' '*.slint' '*.toml' '*.py' '*.sh' '*.swift' |
    grep -v '^scripts/monocard-licenses.sh$' |
    tr '\n' '\0' | xargs -0 grep -il 'rapidraw' /dev/null || :)
[ -z "$rapid" ] || say "code files that name RapidRAW:" "$rapid"

# 3. Greycard's own ports, from before the fork, keep its convention (no
# commit), so only the mono crates and files are held to this one.
unnamed=$(files 'crates/mono-*/*.rs' 'crates/mono-*/*.wgsl' \
    'crates/greycard-*/*mono*.rs' 'crates/greycard-*/*mono*.wgsl' |
    while read -r f; do
        [ -f "$f" ] || continue
        # Comment marks and line ends off, so a credit may wrap.
        text=$(sed 's|^[[:space:]]*//[/!]*||' "$f" | tr -s '[:space:]' ' ')
        printf '%s' "$text" | grep -Eqi '(ported|taken|adapted|copied) from darktable|from darktable.s ' ||
            continue
        printf '%s' "$text" | grep -Eq '[0-9a-f]{40}' &&
            printf '%s' "$text" | grep -Eq 'src/[a-z_/]+\.(c|h|cl)' ||
            echo "$f"
    done)
[ -z "$unnamed" ] || say "ports from darktable with no source file or no 40 digit commit:" "$unnamed"

# 4. Plain text that names the supported camera is allowed; these places
# are not plain text.
marks='ricoh|(^|[^[:alnum:]])gr([^[:alnum:]]|$)|gr[ _-]?iv'
names=$(
    {
        grep -hE '^pub const (NAME|APP_DIR|SIDECAR_EXT|SIDECAR_DIR)' -A4 crates/greycard-core/src/brand.rs |
            grep -o '"[^"]*"'
        grep -hE '^name = ' crates/greycard-ui/Cargo.toml crates/greycard-cli/Cargo.toml crates/mono-*/Cargo.toml
        for key in CFBundleName CFBundleDisplayName CFBundleIdentifier CFBundleExecutable CFBundleIconFile; do
            grep -A1 "<key>$key</key>" packaging/monocard/Info.plist | tail -1
        done
        grep -hE '^(Name|Exec|Icon|StartupWMClass)=' packaging/monocard/*.desktop
        grep -hoE '<title>.*</title>' site/monocard/*.html
        files assets/monocard packaging/monocard site/monocard
        cat assets/monocard/*.svg
    } | grep -iE "$marks" || :
)
[ -z "$names" ] || say "Ricoh's marks in a name, an icon, or a bundle id:" "$names"

[ "$failed" = 0 ] && echo "licenses and marks: clean"
exit "$failed"
