#!/bin/sh
# Try Monocard on an upstream release: rebase in a scratch worktree,
# build with and without `mono`, run the golden renders, and say how it
# went. This checkout, its branch, and the remotes are left as they are.
#
#     scripts/monocard-rebase.sh [--remote NAME] [--keep] [TAG]
#
# TAG is an upstream tag (v0.13.0), the newest one when none is given.
# --remote is the remote that has it (upstream). --keep leaves the
# worktree, for the conflicts of a rebase that stopped. The steps' output
# is in a log the script names. Nothing is pushed.

set -eu
cd "$(dirname "$0")/.."

fail() {
    echo "monocard-rebase.sh: $*" >&2
    exit 2
}

remote=upstream
keep=
tag=
while [ $# -gt 0 ]; do
    case "$1" in
    --remote)
        [ $# -ge 2 ] || fail "--remote wants a name"
        remote=$2
        shift
        ;;
    --keep) keep=1 ;;
    -*) fail "$1 is not an option" ;;
    *)
        [ -z "$tag" ] || fail "one tag at a time"
        tag=$1
        ;;
    esac
    shift
done
case "$tag" in
'' | v[0-9]*.[0-9]*.[0-9]*) ;;
*) fail "$tag is not a release tag (v1.2.3)" ;;
esac
git remote get-url "$remote" >/dev/null 2>&1 || fail "there is no remote $remote"
[ -z "$(git status --porcelain)" ] || fail "the tree is dirty; commit or stash first"

git fetch --quiet --tags "$remote"
if [ -z "$tag" ]; then
    tag=$(git ls-remote --tags --refs "$remote" 'v*' | sed 's|.*refs/tags/||' |
        grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n 1)
    [ -n "$tag" ] || fail "$remote has no release tag"
fi
git rev-parse -q --verify "refs/tags/$tag^{commit}" >/dev/null || fail "there is no tag $tag"

base=$(git merge-base HEAD "$tag") || fail "$tag and this branch have nothing in common"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/monocard-rebase.XXXXXX")
tree="$scratch/tree"
log="$scratch/log"
: >"$log"
cleanup() {
    [ -n "$keep" ] || {
        git worktree remove --force "$tree" 2>/dev/null || :
        rm -rf "$scratch/target"
    }
}
trap cleanup EXIT
git worktree add --quiet --detach "$tree" HEAD
export CARGO_TARGET_DIR="$scratch/target"

result=0
step() {
    what=$1
    shift
    if (cd "$tree" && "$@") >>"$log" 2>&1; then
        echo "ok      $what"
    else
        echo "FAILED  $what"
        result=1
        return 1
    fi
}

echo "rebasing $(git rev-parse --short HEAD) (from $(git rev-parse --short "$base")) onto $tag; log: $log"
if step "rebase onto $tag" git rebase --onto "$tag" "$base"; then
    step "stock build" cargo build --locked -p greycard-ui -p greycard-cli || :
    step "mono build" cargo build --locked -p greycard-ui -p greycard-cli --features mono || :
    # By name, so the goldens run wherever they are kept; none is not a pass.
    if step "golden renders" cargo test --locked -p greycard-engine --features mono mono_golden; then
        if ! grep -Eq '^test .*mono_golden.* \.\.\. ok' "$log"; then
            echo "NONE    golden renders: no test named mono_golden ran"
            result=1
        fi
    fi
    step "license scan" scripts/monocard-licenses.sh || :
else
    (cd "$tree" && git status --short | grep -E '^(UU|AA|DU|UD)' || :)
fi
[ "$result" = 0 ] && echo "$tag: ready to rebase" || echo "$tag: needs work"
exit "$result"
