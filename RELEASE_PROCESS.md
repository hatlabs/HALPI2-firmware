# HALPI2 Firmware Release Process

This document outlines the complete process for creating and publishing new releases of the HALPI2 firmware.

## Prerequisites

- `bumpversion` installed and configured
- Push access to the repository
- Hardware available for testing (recommended)

## Release Process

### 1. Prepare for Release

Ensure you're on the main branch with latest changes:

```bash
git checkout main
git pull origin main
```

Verify all changes intended for the release are committed and merged.

### 2. Update Version with bumpversion

Use `bumpversion` to increment the version number. This will update version numbers across all relevant files:

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

### 3. Push Changes and Tag

Push both the commit and the tag to trigger the automated build:

```bash
git push origin main
git push origin --tags
```

### 4. Monitor Automated Build

1. **Go to GitHub Actions** tab in the repository
2. **Find the "Build Firmware Draft" workflow** that was triggered by your tag
3. **Wait for completion** (usually takes 5-10 minutes)
4. **Check for any build failures** and fix if necessary

### 5. Review Draft Release

Once the build completes successfully:

1. **Go to the GitHub Releases page**
2. **Find the draft release** (will be titled with your version tag)
3. **Review the automatically generated artifacts**:
   - `halpi2-rs-bootloader_X.X.X.elf/.uf2/.bin` - Bootloader in all formats
   - `halpi2-rs-firmware_X.X.X.elf/.uf2/.bin` - Firmware in all formats  
   - `halpi2-firmware_X.X.X.deb` - Debian package
   - `build-info.txt` - Build metadata

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

1. **Edit the draft release**
2. **Add comprehensive release notes**:
   ```markdown
   ## What's New in v3.2.0
   
   ### Features
   - New feature descriptions
   
   ### Improvements  
   - Performance improvements
   - Bug fixes
   
   ### Breaking Changes
   - Any compatibility notes
   
   ### Upgrade Instructions
   - Special instructions if needed
   ```
3. **Uncheck "Set as pre-release"** (unless it's actually a pre-release)
4. **Click "Publish release"**

#### If Tests Fail ❌

1. **Delete the draft release** from GitHub
2. **Fix the identified issues** in the codebase
3. **Force-update the tag** to the new commit:
   ```bash
   git tag -f v3.2.0  # Replace with your version
   git push -f origin v3.2.0
   ```
4. **Repeat from Step 4** (build will retrigger automatically)

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

1. **Update documentation** if needed
2. **Notify stakeholders** about the new release
3. **Monitor for issues** reported by users
4. **Plan next release** based on feedback

## Rollback Process

If a critical issue is discovered after release:

1. **Immediately mark the release as "pre-release"** to hide it from latest
2. **Create hotfix** with the fix
3. **Follow release process** for hotfix version
4. **Consider removing problematic release** entirely if unsafe

## Troubleshooting

### Build Fails
- Check GitHub Actions logs for specific errors
- Common issues: missing dependencies, test failures, linting errors
- Fix issues and force-update the tag

### Package Installation Fails
- Verify all files are included in the package
- Check debian packaging configuration
- Test package installation on clean system

### Hardware Issues
- Use debug builds for better error messages
- Check probe-rs/defmt output for runtime issues
- Test on multiple hardware revisions if available

## Notes

- **Always test before publishing** - firmware bugs can brick hardware
- **Keep detailed release notes** - helps users understand changes  
- **Tag naming must match** the version in `firmware/VERSION`
- **Draft releases are your safety net** - use them!
- **When in doubt, don't publish** - better to delay than release broken firmware