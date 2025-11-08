#!/usr/bin/env bash
set -euo pipefail

# Generate polished release notes for HALPI2 firmware releases
# Usage: generate-release-notes.sh <version> [last_tag] [repository] [template_path]

VERSION="${1:-}"
LAST_TAG="${2:-}"
REPOSITORY="${3:-hatlabs/HALPI2-firmware}"
TEMPLATE="${4:-$(dirname "$0")/../templates/release-notes.md.template}"

if [ -z "$VERSION" ]; then
    echo "Error: Version required" >&2
    echo "Usage: $0 <version> [last_tag] [repository] [template_path]" >&2
    exit 1
fi

if [ ! -f "$TEMPLATE" ]; then
    echo "Error: Template file not found: $TEMPLATE" >&2
    exit 1
fi

# Determine changelog range
if [ -n "$LAST_TAG" ]; then
    echo "Generating changelog since $LAST_TAG" >&2
    CHANGELOG_RANGE="${LAST_TAG}..HEAD"
else
    echo "No previous release found, using all commits" >&2
    CHANGELOG_RANGE="HEAD"
fi

# Categorize commits by conventional commit type
# Using git log with --grep to filter by commit message patterns
FEATURES=$(git log "$CHANGELOG_RANGE" --pretty=format:"- **%s**" --no-merges --grep="^feat" || true)
FIXES=$(git log "$CHANGELOG_RANGE" --pretty=format:"- **%s**" --no-merges --grep="^fix" || true)
IMPROVEMENTS=$(git log "$CHANGELOG_RANGE" --pretty=format:"- **%s**" --no-merges --grep="^refactor\|^perf\|^chore" || true)
DOCS=$(git log "$CHANGELOG_RANGE" --pretty=format:"- **%s**" --no-merges --grep="^docs" || true)

# Count commits in each category (safely handle empty strings)
FEAT_COUNT=0
if [ -n "$FEATURES" ]; then
    FEAT_COUNT=$(echo "$FEATURES" | grep -c "^- " || echo "0")
fi

FIX_COUNT=0
if [ -n "$FIXES" ]; then
    FIX_COUNT=$(echo "$FIXES" | grep -c "^- " || echo "0")
fi

IMP_COUNT=0
if [ -n "$IMPROVEMENTS" ]; then
    IMP_COUNT=$(echo "$IMPROVEMENTS" | grep -c "^- " || echo "0")
fi

DOC_COUNT=0
if [ -n "$DOCS" ]; then
    DOC_COUNT=$(echo "$DOCS" | grep -c "^- " || echo "0")
fi

# Build section content with headers
FEATURES_SECTION=""
if [ "$FEAT_COUNT" -gt 0 ]; then
    FEATURES_SECTION="## ✨ New Features

$FEATURES

"
fi

FIXES_SECTION=""
if [ "$FIX_COUNT" -gt 0 ]; then
    FIXES_SECTION="## 🐛 Bug Fixes

$FIXES

"
fi

IMPROVEMENTS_SECTION=""
if [ "$IMP_COUNT" -gt 0 ]; then
    IMPROVEMENTS_SECTION="## 🔧 Improvements

$IMPROVEMENTS

"
fi

DOCS_SECTION=""
if [ "$DOC_COUNT" -gt 0 ]; then
    DOCS_SECTION="## 📚 Documentation

$DOCS

"
fi

# Build changelog link
CHANGELOG_LINK=""
if [ -n "$LAST_TAG" ]; then
    CHANGELOG_LINK="---

**Full Changelog**: [$LAST_TAG...v$VERSION](https://github.com/$REPOSITORY/compare/$LAST_TAG...v$VERSION)"
fi

# Read template and perform substitutions
# Use a temporary variable to avoid subshell issues with sed
NOTES=$(cat "$TEMPLATE")
NOTES="${NOTES//\{\{VERSION\}\}/$VERSION}"
NOTES="${NOTES//\{\{FEATURES\}\}/$FEATURES_SECTION}"
NOTES="${NOTES//\{\{FIXES\}\}/$FIXES_SECTION}"
NOTES="${NOTES//\{\{IMPROVEMENTS\}\}/$IMPROVEMENTS_SECTION}"
NOTES="${NOTES//\{\{DOCS\}\}/$DOCS_SECTION}"
NOTES="${NOTES//\{\{CHANGELOG_LINK\}\}/$CHANGELOG_LINK}"

# Output the final release notes
echo "$NOTES"
