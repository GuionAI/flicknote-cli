#!/usr/bin/env bash
set -euo pipefail

level="${1:-}"
case "$level" in
    major | minor | patch) ;;
    *)
        echo "level must be major, minor, or patch" >&2
        exit 2
        ;;
esac

current_branch="$(git branch --show-current)"
if [[ "$current_branch" != "main" ]]; then
    echo "releases must run from main; current branch is ${current_branch:-detached HEAD}" >&2
    exit 2
fi

state_dir="$(git rev-parse --git-path flicknote-release-pending)"
level_file="$state_dir/level"
branch_file="$state_dir/branch"
start_head_file="$state_dir/start-head"
phase_file="$state_dir/phase"
tag_file="$state_dir/tag"
release_head_file="$state_dir/release-head"

if [[ -d "$state_dir" ]]; then
    pending_level="$(<"$level_file")"
    if [[ "$pending_level" != "$level" ]]; then
        echo "release $pending_level is pending; retry with: just release $pending_level" >&2
        exit 2
    fi
    release_branch="$(<"$branch_file")"
else
    mkdir "$state_dir"
    printf '%s\n' "$level" >"$level_file"
    printf '%s\n' "$current_branch" >"$branch_file"
    git rev-parse HEAD >"$start_head_file"
    printf 'prepare\n' >"$phase_file"
    release_branch="$current_branch"
fi

phase="$(<"$phase_file")"
if [[ "$phase" == "publish" ]]; then
    tag="$(<"$tag_file")"
    release_head="$(<"$release_head_file")"
elif [[ "$phase" == "prepare" ]]; then
    start_head="$(<"$start_head_file")"
    current_head="$(git rev-parse HEAD)"

    if [[ "$current_head" == "$start_head" ]]; then
        cargo release "$level" --execute --no-push
        current_head="$(git rev-parse HEAD)"
    fi

    release_tags="$(git tag --points-at "$current_head" --list 'v[0-9]*')"
    if [[ "$current_head" == "$start_head" || -z "$release_tags" || "$release_tags" == *$'\n'* ]]; then
        echo "release preparation did not create exactly one tagged commit; inspect $state_dir" >&2
        exit 1
    fi

    tag="$release_tags"
    release_head="$current_head"
    printf '%s\n' "$tag" >"$tag_file"
    printf '%s\n' "$release_head" >"$release_head_file"
    printf 'publish\n' >"$phase_file"
else
    echo "invalid release state in $phase_file" >&2
    exit 1
fi

current_branch="$(git branch --show-current)"
current_head="$(git rev-parse HEAD)"
if [[ "$current_branch" != "$release_branch" || "$current_head" != "$release_head" ]]; then
    echo "pending release $tag belongs to $release_branch at $release_head" >&2
    echo "restore that branch and HEAD before retrying" >&2
    exit 1
fi

og git push
og git tag "$tag"

rm -- \
    "$level_file" \
    "$branch_file" \
    "$start_head_file" \
    "$phase_file" \
    "$tag_file" \
    "$release_head_file"
rmdir -- "$state_dir"
