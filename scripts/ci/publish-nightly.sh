#!/usr/bin/env bash
# Publish the current main commit's checksummed desktop artifacts as Nightly.
set -euo pipefail
shopt -s nullglob

repo="repos/$GITHUB_REPOSITORY"
prefix="pcbcraft-nightly-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"

current_main() {
    local main_sha
    main_sha="$(gh api "$repo/git/ref/heads/main" --jq '.object.sha')" || exit 1
    if [[ "$main_sha" != "$GITHUB_SHA" ]]; then
        echo "Skipping Nightly publication because main has moved."
        return 1
    fi
}

collect() {
    local platform="$1" directory pattern
    shift
    directory="packages/pcbcraft-nightly-$GITHUB_SHA-$platform"
    (cd "$directory" && sha256sum --check --strict SHA256SUMS.txt)
    local matches=()
    for pattern in "$@"; do
        matches=("$directory"/$pattern)
        if [[ "${#matches[@]}" != 1 ]]; then
            echo "Expected one Nightly package matching $pattern for $platform." >&2
            exit 1
        fi
        cp -- "${matches[0]}" "nightly-assets/$prefix-${matches[0]##*/}"
    done
}

current_main || exit 0
mkdir nightly-assets
collect linux-x86_64 'pcbcraft-*-linux-x86_64.tar.gz'
collect windows-x64 'pcbcraft-*-windows-x64.zip'
collect macos-aarch64 'pcbcraft-*-macos-aarch64.dmg' 'pcbcraft-cli-*-macos-aarch64.zip'
(cd nightly-assets && sha256sum -- * > "$prefix-SHA256SUMS.txt")

release="$(gh api --paginate "$repo/releases?per_page=100" --jq '.[] | select(.tag_name == "nightly")' |
    jq -cs 'if length > 1 then error("Multiple releases use the nightly tag") else .[0] // {} end')"
if [[ "$release" != '{}' ]]; then
    if ! jq -e '.prerelease and (.immutable | not)' <<< "$release" > /dev/null; then
        echo "The nightly tag is already in use by a release that cannot be rolled forward." >&2
        exit 1
    fi
fi

tag_sha="$(gh api "$repo/git/matching-refs/tags/nightly" \
    --jq '.[] | select(.ref == "refs/tags/nightly") | .object.sha')"
if [[ "$release" == '{}' && -n "$tag_sha" && "$tag_sha" != "$GITHUB_SHA" ]]; then
    echo "The nightly tag already points to another commit without a Nightly release." >&2
    exit 1
fi

{
    printf 'Desktop packages for commit `%s` after full CI passed on `main`.\n\n' "$GITHUB_SHA"
    printf '[CI run](%s/%s/actions/runs/%s)\n\n' "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$GITHUB_RUN_ID"
    for asset in nightly-assets/*; do
        name="${asset##*/}"
        printf -- '- [%s](%s/%s/releases/download/nightly/%s)\n' "${name#"$prefix-"}" "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$name"
    done
} > nightly-notes.md

current_main || exit 0
if [[ "$release" == '{}' ]]; then
    target=(--verify-tag)
    if [[ -z "$tag_sha" ]]; then
        target=(--target "$GITHUB_SHA")
    fi
    gh release create nightly --repo "$GITHUB_REPOSITORY" "${target[@]}" --prerelease --latest=false \
        --title "PCBcraft Nightly" --notes-file nightly-notes.md nightly-assets/*
    exit 0
fi

# Keep the previous downloads intact until all replacement files are uploaded.
gh release upload nightly --repo "$GITHUB_REPOSITORY" nightly-assets/*
current_main || exit 0
if [[ -z "$tag_sha" ]]; then
    gh api "$repo/git/refs" -f ref=refs/tags/nightly -f sha="$GITHUB_SHA" > /dev/null
elif [[ "$tag_sha" != "$GITHUB_SHA" ]]; then
    gh api --method PATCH "$repo/git/refs/tags/nightly" -f sha="$GITHUB_SHA" -F force=true > /dev/null
fi

gh release edit nightly --repo "$GITHUB_REPOSITORY" --draft=false --prerelease --latest=false \
    --title "PCBcraft Nightly" --notes-file nightly-notes.md
# Remove only the job's older uploads after the new downloads are linked.
stale_assets="$(jq -r --arg prefix "$prefix-" '.assets[] | select(
    (.name | test("^pcbcraft-nightly-[0-9]+-[0-9]+-")) and
    (.name | startswith($prefix) | not)
) | .id' <<< "$release")"
while read -r asset_id; do
    if [[ -n "$asset_id" ]]; then
        gh api --method DELETE "$repo/releases/assets/$asset_id"
    fi
done <<< "$stale_assets"
