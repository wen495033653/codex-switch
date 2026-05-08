# Code Signing Policy

Free code signing provided by SignPath.io, certificate by SignPath Foundation.

This policy applies to official Codex Switch release artifacts published from this repository.

## Signed Artifacts

Official Windows installers are built from the source code and build scripts in this repository by GitHub Actions. Release artifacts must match the project name and version from the release source.

## Team Roles

Committers and reviewers: repository maintainers with write access.

Approvers: repository maintainers with admin access who approve release signing requests.

Current maintainer: [wen495033653](https://github.com/wen495033653).

## Privacy Policy

Codex Switch stores account data, settings, API configuration, session sync state, and proxy settings locally on the user's device.

The app transfers data to network services only when required for a user-requested action or an enabled feature, including OAuth login, token refresh, quota checks, configured OpenAI-compatible API usage, and update checks through GitHub Releases.

Exported account files may contain credentials such as refresh tokens and should be handled as secrets.
