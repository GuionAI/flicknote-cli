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

# An abandoned prepare can outlive an ordinary main update without having
# changed a version. Retrying must prepare a release from the new main HEAD.
cat >Cargo.toml <<'TOML'
[workspace.package]
version = "0.4.0"
TOML
git add Cargo.toml
git -c user.name='Release Test' -c user.email='release-test@example.com' \
    commit -qm 'chore(cli): add test manifest'
start_head="$(git rev-parse HEAD)"
printf 'ordinary main update\n' >>README.md
git add README.md
git -c user.name='Release Test' -c user.email='release-test@example.com' \
    commit -qm 'fix(cli): ordinary main update'
mkdir -- "$state_dir"
printf 'patch\n' >"$state_dir/level"
printf 'main\n' >"$state_dir/branch"
printf '%s\n' "$start_head" >"$state_dir/start-head"
printf 'prepare\n' >"$state_dir/phase"

cat >"$bin_dir/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == 'release patch --execute --no-push' ]]
printf 'prepare\n' >>"$CARGO_CALLS"
sed -i.bak 's/0.4.0/0.4.1/' Cargo.toml
rm -- Cargo.toml.bak
git add Cargo.toml
git -c user.name='Release Test' -c user.email='release-test@example.com' \
    commit -qm 'chore(cli): release 0.4.1'
git tag v0.4.1
SH
chmod +x "$bin_dir/cargo"
export CARGO_CALLS="$test_dir/cargo-calls"
: >"$og_calls"
PATH="$bin_dir:$PATH" "$script_dir/release.sh" patch
[[ "$(<"$CARGO_CALLS")" == prepare ]]
[[ "$(<"$og_calls")" == $'push\ntag v0.4.1' ]]
[[ ! -e "$state_dir" ]]
echo "release retry after main advances test passed"

# A version-changing commit without its tag is a partial release, not a safe
# main advance. Preserve it for inspection rather than bumping or publishing.
start_head="$(git rev-parse HEAD)"
sed -i.bak 's/0.4.1/0.4.2/' Cargo.toml
rm -- Cargo.toml.bak
git add Cargo.toml
git -c user.name='Release Test' -c user.email='release-test@example.com' \
    commit -qm 'chore(cli): partially prepare release'
mkdir -- "$state_dir"
printf 'patch\n' >"$state_dir/level"
printf 'main\n' >"$state_dir/branch"
printf '%s\n' "$start_head" >"$state_dir/start-head"
printf 'prepare\n' >"$state_dir/phase"

assert_prepare_preserved() {
    local before_head
    before_head="$(git rev-parse HEAD)"
    : >"$CARGO_CALLS"
    : >"$og_calls"
    if PATH="$bin_dir:$PATH" "$script_dir/release.sh" patch >"$test_dir/rejected.log" 2>&1; then
        echo 'partial preparation unexpectedly succeeded' >&2
        exit 1
    fi
    [[ ! -s "$CARGO_CALLS" && ! -s "$og_calls" ]]
    [[ "$(<"$state_dir/start-head")" == "$start_head" ]]
    [[ "$(git rev-parse HEAD)" == "$before_head" ]]
    [[ "$(<"$state_dir/phase")" == prepare ]]
}
assert_prepare_preserved
echo "untagged partial release is preserved test passed"

# Even if committed manifests match, unstaged preparation must be preserved.
start_head="$(git rev-parse HEAD)"
printf '%s\n' "$start_head" >"$state_dir/start-head"
printf 'another ordinary update\n' >>README.md
git add README.md
git -c user.name='Release Test' -c user.email='release-test@example.com' \
    commit -qm 'fix(cli): another main update'
sed -i.bak 's/0.4.2/0.4.3/' Cargo.toml
rm -- Cargo.toml.bak
before_diff="$(git diff)"
assert_prepare_preserved
[[ "$(git diff)" == "$before_diff" ]]
echo "dirty partial release is preserved test passed"
