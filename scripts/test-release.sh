#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf -- "$test_dir"' EXIT

repo_dir="$test_dir/repo"
bin_dir="$test_dir/bin"
og_calls="$test_dir/og-calls"
mkdir -p -- "$repo_dir" "$bin_dir"

git init -q -b main "$repo_dir"
printf 'release test\n' >"$repo_dir/README.md"
git -C "$repo_dir" add README.md
git -C "$repo_dir" \
    -c user.name='Release Test' \
    -c user.email='release-test@example.com' \
    commit -q -m 'test: initialize repository'

# These variables must expand when the generated fake executable runs.
# shellcheck disable=SC2016
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'case "$1" in' \
    '    push)' \
    '        [[ "$#" -eq 1 ]]' \
    '        ;;' \
    '    tag)' \
    '        [[ "$#" -eq 2 ]]' \
    '        ;;' \
    '    *)' \
    '        echo "unsupported og command: $*" >&2' \
    '        exit 64' \
    '        ;;' \
    'esac' \
    'printf "%s\n" "$*" >>"$OG_CALLS"' \
    >"$bin_dir/og"
chmod +x "$bin_dir/og"

cd -- "$repo_dir"
state_dir="$(git rev-parse --git-path flicknote-release-pending)"
release_head="$(git rev-parse HEAD)"
mkdir -- "$state_dir"
printf 'minor\n' >"$state_dir/level"
printf 'main\n' >"$state_dir/branch"
printf '%s\n' "$release_head" >"$state_dir/start-head"
printf 'publish\n' >"$state_dir/phase"
printf 'v0.4.0\n' >"$state_dir/tag"
printf '%s\n' "$release_head" >"$state_dir/release-head"

export OG_CALLS="$og_calls"
PATH="$bin_dir:$PATH" "$script_dir/release.sh" minor

expected_calls=$'push\ntag v0.4.0'
actual_calls="$(<"$og_calls")"
if [[ "$actual_calls" != "$expected_calls" ]]; then
    echo "unexpected og calls:" >&2
    printf '%s\n' "$actual_calls" >&2
    exit 1
fi

if [[ -e "$state_dir" ]]; then
    echo "release state was not cleared" >&2
    exit 1
fi

echo "release publish command test passed"
