#!/bin/bash
# Read version from firmware/VERSION
# Sets version and tag_version in GitHub output

set -e

VERSION=$(cat firmware/VERSION)
# Strip any potential whitespace
VERSION=$(echo "$VERSION" | tr -d '[:space:]')
# For firmware, version and tag_version are the same
TAG_VERSION="$VERSION"

echo "version=$VERSION" >> "$GITHUB_OUTPUT"
echo "tag_version=$TAG_VERSION" >> "$GITHUB_OUTPUT"
echo "Version from firmware/VERSION: $VERSION (tag version: $TAG_VERSION)"
