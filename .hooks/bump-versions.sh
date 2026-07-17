#!/bin/sh
# cog.toml pre_bump_hooks entry — keeps Cargo.toml's [workspace.package]
# version and the Helm chart's Chart.yaml version/appVersion in lockstep
# with the tag cocogitto is about to create, instead of duplicating
# version-bump logic in the release/publish GitHub Actions workflows.
# cog invokes this as `.hooks/bump-versions.sh {{version}}` and stages
# whatever it edits, so the changes land inside cog's own bump commit.
set -e

VERSION="$1"
if [ -z "$VERSION" ]; then
    echo "usage: bump-versions.sh <new-version>" >&2
    exit 1
fi

# Only the version line inside [workspace.package] — dependency entries
# elsewhere in Cargo.toml (e.g. `serde = { version = "1", ... }`) must be
# left untouched.
awk -v ver="$VERSION" '
    /^\[workspace\.package\]/ { in_pkg = 1 }
    /^\[/ && !/^\[workspace\.package\]/ { in_pkg = 0 }
    in_pkg && /^version = / { print "version = \"" ver "\""; next }
    { print }
' Cargo.toml > Cargo.toml.tmp
mv Cargo.toml.tmp Cargo.toml

sed -i \
    -e "s/^version: .*/version: ${VERSION}/" \
    -e "s/^appVersion: .*/appVersion: \"${VERSION}\"/" \
    deploy/charts/weebo-authentik/Chart.yaml

git add Cargo.toml deploy/charts/weebo-authentik/Chart.yaml
