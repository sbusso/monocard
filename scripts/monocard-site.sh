#!/bin/sh
# Publish the website and the user guide. It builds the guide and pushes
# site/landing as the branch gh-pages, which holds nothing else. No test
# and no release is part of it.
#
#     scripts/monocard-site.sh [--dry-run]
#
# --dry-run builds it and says what would change.

set -eu
cd "$(dirname "$0")/.."
site=$PWD/site/landing

uv run -q scripts/monocard-guide.py
git fetch -q origin gh-pages
before=$(git rev-parse FETCH_HEAD)

# An index of its own: the checkout and its index are left as they are.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export GIT_INDEX_FILE="$tmp/index"
(cd "$site" && git --git-dir="$(git rev-parse --absolute-git-dir)" --work-tree="$site" \
    add -A -- . ':!*.DS_Store')
tree=$(git write-tree)

git diff --stat=100 --summary "$before" "$tree" | tail -n 25
if git diff --quiet "$before" "$tree"; then
    echo "The website is up to date."
    exit 0
fi
[ "${1:-}" != --dry-run ] || exit 0

commit=$(git commit-tree "$tree" -p "$before" -m "Website: $(git rev-parse --short HEAD)")
git push -q origin "$commit:refs/heads/gh-pages"
echo "Published. GitHub shows it in a minute or two: https://sbusso.github.io/monocard/"
