#!/bin/sh
# Usage: attach-release-assets.sh <tag> <target-sha> <asset-dir>
# 同じタグ/SHA の draft Release にだけ asset を添付する。公開済み Release は書き換えない。
set -eu

[ "$#" -eq 3 ] || {
    echo 'Usage: attach-release-assets.sh <tag> <target-sha> <asset-dir>' >&2
    exit 2
}
tag=$1
target_sha=$2
dir=$3

release=$(gh release view "$tag" --json isDraft,targetCommitish --jq '"\(.isDraft) \(.targetCommitish)"')
[ "$release" = "true $target_sha" ] || {
    echo "Release $tag must be a draft targeting $target_sha (actual: $release)" >&2
    exit 1
}

# 再実行では同じ SHA から再ビルドした asset で置き換える。ZIP と checksum は同じ run で揃って更新される。
gh release upload "$tag" "$dir"/* --clobber
