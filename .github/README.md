# GitHub Workflows & Scripts

This directory contains GitHub Actions workflows and supporting scripts for the HALPI2 firmware repository.

## Workflows

### `build.yml` - Pull Request Validation
Runs on every pull request to validate code quality:
- Cargo check
- Clippy lints
- Build verification

### `draft-release.yml` - Automated Draft Releases
Automatically creates draft releases when code is pushed to `main`:
- Triggers on push to main branch
- Builds all release artifacts (firmware, bootloader, Debian package)
- Generates polished release notes from commit history
- Creates draft release with all artifacts

**Smart Release Logic:**
- Skips if a published release already exists for the version
- Deletes and recreates if a draft exists

### `release.yml` - APT Repository Notification
Triggers when a draft release is published:
- Notifies the APT repository to pull new packages
- Simple webhook-based notification

## Scripts

### `generate-release-notes.sh`

Generates polished release notes from git commit history using conventional commit format.

**Requirements:**
- `git` - For commit history analysis
- `bash` 4.0+ - For string manipulation features (parameter expansion)
- `gh` CLI - For release API access (used in workflows only)

**Usage:**
```bash
.github/scripts/generate-release-notes.sh <version> [last_tag] [repository] [template_path]
```

**Arguments:**
- `version` (required): Version number (e.g., "3.2.0")
- `last_tag` (optional): Previous release tag for changelog range (e.g., "v3.1.1")
- `repository` (optional): GitHub repository in owner/repo format (default: "hatlabs/HALPI2-firmware")
- `template_path` (optional): Path to release notes template (default: auto-detected)

**Example:**
```bash
# Generate notes for v3.2.0 since v3.1.1
.github/scripts/generate-release-notes.sh "3.2.0" "v3.1.1"

# Generate notes for first release (all commits)
.github/scripts/generate-release-notes.sh "1.0.0"
```

**How it works:**
1. Analyzes git commit history using conventional commit patterns
2. Categorizes commits by type (feat, fix, refactor, docs, etc.)
3. Populates the template with categorized commits
4. Outputs markdown-formatted release notes

**Conventional Commit Types:**
- `feat:` → ✨ New Features
- `fix:` → 🐛 Bug Fixes
- `refactor:`, `perf:`, `chore:` → 🔧 Improvements
- `docs:` → 📚 Documentation

## Templates

### `templates/release-notes.md.template`

Template for generating release notes. Uses placeholders that are replaced by the script:

- `{{VERSION}}` - Version number
- `{{FEATURES}}` - New features section (with header)
- `{{FIXES}}` - Bug fixes section (with header)
- `{{IMPROVEMENTS}}` - Improvements section (with header)
- `{{DOCS}}` - Documentation section (with header)
- `{{CHANGELOG_LINK}}` - Link to GitHub compare view

**Customizing the template:**

You can edit the template to change the structure, wording, or add new sections. The script will automatically use your changes.

Example additions:
- Add upgrade warnings section
- Include breaking changes notice
- Add testing instructions
- Customize installation methods

## Release Process

1. **Update version**: Update `firmware/VERSION` and commit
2. **Push to main**: `git push origin main`
3. **Draft created**: Workflow automatically creates draft release with:
   - All build artifacts
   - Auto-generated release notes from commits
4. **Review & edit**:
   - Open the draft release on GitHub
   - Edit the "What's New" section to highlight key changes
   - Review auto-generated changelog sections
5. **Publish**: Click "Publish release" when ready
6. **APT update**: Release workflow triggers APT repository update

## Development

### Testing release notes locally

```bash
# Test with current commits since last release
VERSION=$(cat firmware/VERSION)
LAST_TAG=$(git describe --tags --abbrev=0)
.github/scripts/generate-release-notes.sh "$VERSION" "$LAST_TAG"
```

### Modifying the template

1. Edit `.github/templates/release-notes.md.template`
2. Test locally with the script
3. Commit and push - next release will use updated template

### Workflow debugging

View workflow runs in the **Actions** tab of your repository.

Common issues:
- **Draft not created**: Check if published release already exists for version
- **Build failures**: Check cargo/rust toolchain in build step logs
- **Missing artifacts**: Verify build-info.txt shows expected files
