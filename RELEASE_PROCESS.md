# HALPI2 Firmware Release Process

This document outlines the complete process for creating and publishing new releases of the HALPI2 firmware.

## Prerequisites

- `bumpversion` installed and configured (or manual VERSION file update)
- Push access to the repository
- Hardware available for testing (recommended)

## Release Process Overview

The release process is **fully automated via GitHub Actions**:

1. **Update version** and merge to main
2. **Automated draft release** is created on push to main
3. **Test the draft release** artifacts
4. **Publish the release** to trigger APT repository update

## Detailed Steps

### 1. Prepare for Release

Ensure you're on the main branch with latest changes:

```bash
git checkout main
git pull origin main
```

Verify all changes intended for the release are committed and merged.

### 2. Update Version

#### Option A: Using bumpversion (recommended)

Use `bumpversion` to increment the version number:

```bash
# For patch releases (3.1.0 -> 3.1.1)
bumpversion patch

# For minor releases (3.1.0 -> 3.2.0)
bumpversion minor

# For major releases (3.1.0 -> 4.0.0)
bumpversion major

# For pre-releases (3.1.0 -> 3.1.1-a1)
bumpversion prerelease
```

This automatically:
- Updates `firmware/VERSION`
- Updates version strings in code files
- Creates a commit with the version change
- Creates a git tag

#### Option B: Manual version update

Alternatively, manually update `firmware/VERSION` and commit:

```bash
echo "3.2.0" > firmware/VERSION
git add firmware/VERSION
git commit -m "chore: bump version to 3.2.0"
```

### 3. Push to Main

Push your changes to main to trigger the automated build:

```bash
git push origin main
```

**Note**: You do NOT need to push tags manually. The draft release workflow handles everything.

### 4. Monitor Automated Build

1. **Go to GitHub Actions** tab in the repository
2. **Find the "Build and Draft Release" workflow** that was triggered by your push
3. **Wait for completion** (usually takes 5-10 minutes)
4. **Check for any build failures** and fix if necessary

The workflow will:
- Check if a release for this version already exists
- Skip build if a published release exists
- Delete and recreate if a draft release exists
- Build all artifacts (firmware, bootloader, Debian package)
- Create a draft release with all artifacts

### 5. Review Draft Release

Once the build completes successfully:

1. **Go to the GitHub Releases page**
2. **Find the draft release** (will be titled "HALPI2 Firmware vX.X.X")
3. **Review the automatically generated artifacts**:
   - `halpi2-rs-bootloader_X.X.X.elf/.uf2/.bin` - Bootloader in all formats
   - `halpi2-rs-firmware_X.X.X.elf/.uf2/.bin` - Firmware in all formats
   - `halpi2-firmware_X.X.X.deb` - Debian package
   - `build-info.txt` - Build metadata
4. **Review the auto-generated changelog** in the release notes

### 6. Test the Release (Critical Step)

Before publishing, thoroughly test the release:

#### Hardware Testing
- [ ] Download the `halpi2-rs-firmware_X.X.X.uf2` file
- [ ] Flash to RP2040 hardware using bootsel mode
- [ ] Verify basic power management functionality
- [ ] Test state machine transitions
- [ ] Verify I2C communication with CM5
- [ ] Test LED patterns and indicators
- [ ] If bootloader updated: Test bootloader functionality

#### Package Testing  
- [ ] Download the `halpi2-firmware_X.X.X.deb` package
- [ ] Install on target system: `sudo dpkg -i halpi2-firmware_X.X.X.deb`
- [ ] Verify files are installed in `/usr/share/halpi2-firmware/`
- [ ] Check that `halpid` service can access the firmware files

#### Integration Testing
- [ ] Test firmware update process via I2C DFU commands
- [ ] Verify configuration persistence across reboots
- [ ] Test watchdog functionality if in cooperative mode
- [ ] Verify graceful shutdown sequences

### 7. Publish or Fix

#### If All Tests Pass ✅

1. **Edit the draft release** on GitHub
2. **Update the release notes**:
   - Replace placeholder text in "What's new" section
   - The changelog is already populated from commits
   - Add any special upgrade instructions
3. **Uncheck "Set as pre-release"** (unless it's actually a pre-release)
4. **Click "Publish release"**

This will automatically:
- Make the release public
- Trigger the "Release Published" workflow
- Send notification to APT repository for package update

#### If Tests Fail ❌

1. **Do NOT delete the draft** - it will be automatically replaced
2. **Fix the identified issues** in the codebase
3. **Commit and push fixes to main**:
   ```bash
   git add .
   git commit -m "fix: address release testing issues"
   git push origin main
   ```
4. **The workflow will automatically**:
   - Detect the existing draft release
   - Delete it
   - Build new artifacts
   - Create a new draft release

No manual tag or release management needed!

## Release Types

### Stable Releases
- Use semantic versioning: `3.2.0`
- Thoroughly tested
- Published as normal releases (not pre-releases)

### Pre-releases  
- Use format: `3.2.0-a1`, `3.2.0-beta1`, etc.
- Mark as "Set as pre-release" when publishing
- Use for testing new features before stable release

### Hotfix Releases
- Increment patch version: `3.1.1`
- Cherry-pick critical fixes to release branch if needed
- Fast-track testing for critical security/safety issues

## Post-Release Tasks

After publishing a release:

1. **Monitor APT repository update** - Package should appear within minutes
2. **Update documentation** if needed (in the `halpi2/` docs repo)
3. **Notify stakeholders** about the new release
4. **Monitor for issues** reported by users
5. **Plan next release** based on feedback

## Rollback Process

If a critical issue is discovered after release:

1. **Edit the release** and mark it as "pre-release" to hide from latest
2. **Create hotfix** with the fix
3. **Push hotfix to main** to trigger new draft release
4. **Test and publish** the hotfix release
5. **Consider deleting problematic release** if it's unsafe

## Troubleshooting

### Build Fails in CI
- Check GitHub Actions logs for specific errors
- Common issues: missing dependencies, compilation errors, linting failures
- Fix issues and push to main - workflow will retrigger automatically

### Draft Release Not Created
- Check if a published release already exists for this version
- The workflow skips building if a published release exists
- Increment the version number if you need a new release

### Package Installation Fails
- Download the .deb file and test locally: `dpkg -i halpi2-firmware_*.deb`
- Check debian packaging configuration in `debian/` directory
- Verify files are being installed correctly with `dpkg -L halpi2-firmware`

### Hardware Issues
- Use debug builds for better error messages (build locally with `./run build`)
- Check probe-rs/defmt output for runtime issues
- Test on multiple hardware revisions if available

## CI Workflow Details

### Three Workflows

1. **`build.yml` (PR Validation)**
   - Triggers on pull requests to main
   - Runs quick validation (check, clippy, build)
   - Does not create releases

2. **`draft-release.yml` (Build and Draft Release)**
   - Triggers on push to main
   - Checks if version already has a published release (skips if yes)
   - Deletes and recreates draft if it exists
   - Builds all artifacts and creates draft release with changelog

3. **`release.yml` (Release Published)**
   - Triggers when you publish a draft release
   - Notifies APT repository to pull and publish the package

## Notes

- **Always test before publishing** - firmware bugs can brick hardware
- **Keep detailed release notes** - update the template in draft releases
- **Version in `firmware/VERSION` drives everything** - keep it accurate
- **Draft releases are automatic** - just push to main after version update
- **No manual tag management** - the workflow handles all of that
- **When in doubt, don't publish** - better to delay than release broken firmware