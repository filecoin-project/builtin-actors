# Release Process

This document describes the process for releasing a new version of the `builtin-actors` project.

## Current State

1. Create a pull request which updates the `workspace.package.version` in the [top-level `Cargo.toml` file](https://github.com/filecoin-project/builtin-actors/blob/master/Cargo.toml).
   - Title the PR `chore: release X.Y.Z`
2. On pull request creation, a [Release Checker](.github/workflows/release-check.yml) workflow will run. It will perform the following actions:
    1. Extract the version from the top-level `Cargo.toml` file.
    2. Check if a git tag for the version already exists. Continue only if it does not.
    3. Create a draft GitHub release with the version as the tag. (A git tag with this version string will be created when the release is published.)
    4. Comment on the pull request with a link to the draft release.
3. On pull request merge, a [Releaser](.github/workflows/releaser.yml) workflow will run. It will perform the following actions:
    1. Extract the version from the top-level `Cargo.toml` file.
    2. Check if a git tag for the version already exists. Continue only if it does not.
    3. Check if a draft GitHub release with the version as the tag exists. Otherwise, create it.
    4. Trigger the [Upload Release Assets](.github/workflows/upload-release-assets.yml) workflow to:
        1. Build `builtin-actors.car`s for various networks.
        2. Generate checksums for the built `builtin-actors.car`s.
        3. Upload the built `builtin-actors.car`s and checksums as assets to the draft release.
    5. Publish the draft release. Publishing the release creates the git tag.

## Known Limitations

1. Unless triggered manually, release assets will only be built after merging the release PR.
