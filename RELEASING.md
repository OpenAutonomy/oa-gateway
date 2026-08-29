# Releasing

Releases are a git tag plus a GitHub Release; the crates are not published to
crates.io. Pushing a `vX.Y.Z` tag triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml), which builds
the artifacts and publishes them. The steps below are the human half.

## 1. Prepare the version on a branch

- Bump `version` in the root `[workspace.package]` table of `Cargo.toml`. Every
  crate inherits it.
- In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, add
  a fresh empty `## [Unreleased]` above it, and update the link references at
  the bottom:
  - `[Unreleased]: …/compare/vX.Y.Z...HEAD`
  - `[X.Y.Z]: …/compare/vPREV...vX.Y.Z`
- `cargo build` so `Cargo.lock` picks up the new version, and commit that too.
- Open a PR, let CI pass, merge it.

## 2. Tag the merge commit

```sh
git checkout main && git pull
git tag -a vX.Y.Z <merge-commit> -m "X.Y.Z"
git push origin vX.Y.Z
```

## 3. What the workflow does

- Builds `oa-gateway` release binary for `x86_64-unknown-linux-gnu`.
- Writes `sha256sums.txt`.
- Generates a CycloneDX SBOM (`oa-gateway-X.Y.Z.cdx.json`).
- Creates the GitHub Release, body taken from the matching `CHANGELOG.md`
  section, with the binary, checksums, and SBOM attached.
- Builds the `Dockerfile` and pushes `ghcr.io/openautonomy/oa-gateway:X.Y.Z`
  and `:latest` (a separate job, so a schema-download hiccup there does not
  block the Release).

## 4. Check

- The Release exists with three attachments.
- `docker pull ghcr.io/openautonomy/oa-gateway:X.Y.Z` works.
- If the `image` job failed on a transient error, re-run just that job from the
  Actions tab; the Release is already in place.

## Dry run

Run the `release` workflow from the Actions tab (`workflow_dispatch`) on a
branch: it builds the binary and SBOM and uploads them as a `release-dist`
workflow artifact, and builds the image without pushing. Nothing is published.

## Not yet automated

- Binaries for other targets (macOS, `aarch64`, musl).
- Sigstore signing / build provenance attestation.
- `latest` moves on every tag, including any future pre-release; retag by hand
  if that is ever not wanted.
