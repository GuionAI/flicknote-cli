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

state_dir="$(git rev-parse --git-path flicknote-release-pending)"
level_file="$state_dir/level"
start_head_file="$state_dir/start-head"
phase_file="$state_dir/phase"
tag_file="$state_dir/tag"

if [[ -d "$state_dir" ]]; then
    pending_level="$(<"$level_file")"
    if [[ "$pending_level" != "$level" ]]; then
        echo "release $pending_level is pending; retry with: just release $pending_level" >&2
        exit 2
    fi
else
    mkdir "$state_dir"
    printf '%s\n' "$level" >"$level_file"
    git rev-parse HEAD >"$start_head_file"
    printf 'prepare\n' >"$phase_file"
fi

phase="$(<"$phase_file")"
if [[ "$phase" == "publish" ]]; then
    tag="$(<"$tag_file")"
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
    printf '%s\n' "$tag" >"$tag_file"
    printf 'publish\n' >"$phase_file"
else
    echo "invalid release state in $phase_file" >&2
    exit 1
fi

og git push
og git tag "$tag"

rm -- "$level_file" "$start_head_file" "$phase_file" "$tag_file"
rmdir -- "$state_dir"
