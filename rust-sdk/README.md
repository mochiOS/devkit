# Standalone Rust application builds

`actions/setup-rust-sdk` prepares a native Rust application build environment on
Ubuntu 24.04. It installs the explicit released C/C++ SDK through komeup, verifies
the reviewed archive SHA-256, and fetches the fixed Rust std, ViewKit, font and
protocol sources from `sources.lock.json`. AppCore comes from the same DevKit
revision as the Action. No OS checkout, image build, SDK release, credential or
organization membership is required.

This initial source SDK prepares one application at a time. It uses the existing
mochiOS Rust std fork and pinned nightly compiler. It is not a newly published
binary Rust SDK. The runner-local `app-build.json` contains local paths and must
not be uploaded or shipped. Immutable source pins and font hashes are public;
the installation does not change the developer's default komeup/rustup choices.

Example caller (replace `staging` with a reviewed full commit SHA):

```yaml
- id: sdk
  uses: mochiOS/devkit/actions/setup-rust-sdk@staging
  with:
    output: ${{ runner.temp }}/mochios-rust-sdk
- name: Test, compile and stage
  env:
    HELPER: ${{ steps.sdk.outputs.helper }}
    SDK: ${{ steps.sdk.outputs.sdk }}
  run: python3 "$HELPER" --source "$GITHUB_WORKSPACE" --sdk "$SDK" --output "$RUNNER_TEMP/app-build" --test
- id: package
  uses: mochiOS/devkit/actions/build-mpkg@20d3fb71b952ae7a4ff583dafe8cc3d00e24d3a3
  with:
    manifest: ${{ runner.temp }}/app-build/stage/manifest.toml
    payload: ${{ runner.temp }}/app-build/stage/payload
    output: dist/Example.mpkg
```

Compilation and native host unit tests run in an isolated staged Cargo project.
The helper maps the existing AppCore/protocol source dependencies into the SDK,
preserves the application's lockfile as input, and exports its actual resulting
resolution as `resolved.Cargo.lock` for review. It never edits or rolls back the
source checkout's Cargo.lock. SDK changes can require adopting this resolution
into the application repository before production release.

The staging helper supports a single-entry Rust application with a completed
MPKG manifest. It preserves document associations and requested capabilities,
copies declared resources, binds real file hashes/modes, removes source-tree
built-in provenance, and changes the binary destination to the normal
`/applications/<name>.app` package path. ELF and host-path checks do not execute
target programs and do not establish native runtime success. The public MPKG
Action separately creates the unsigned container. Signing, approval and release
publication remain caller workflow responsibilities.

## Approval-protected publication

The generic `release.py` helper consumes the unsigned artifact from the same
successful workflow run. It checks the source commit, package ID, actual MPKG
manifest, SDK source lock and archive checksum before accessing signing inputs.
It does not compile or execute application code. `host-tool.py` builds msign from
the same trusted SDK source revision, with developer-CA networking disabled.

The caller must place its publication job behind a protected GitHub Environment.
Configure the repository's chosen reviewer/team and restrict the environment to
its default branch. The public packaging/SDK Actions have no organization or
team requirement of their own.

Official app callers use an `app-release` Environment with
`application-maintainers` approval. Releases stay disabled until the repository
variable `APP_RELEASE_ENABLED` is explicitly set to `true` and these Environment
values have been configured:

* Secret `MSIGN_CERTIFICATE_B64`: base64 of the issued Developer Certificate file.
* Secret `MSIGN_PRIVATE_KEY_B64`: base64 of its corresponding private-key file.
* Variable `MSIGN_ROOT_PUBLIC_KEY_B64`: base64 of the trusted issuer public-key file.

No root private key, fixture key, generated substitute identity or unsigned
publication is accepted. Credentials are decoded into a private temporary
directory, not propagated as subprocess environment values, and removed on
success or failure. The signed package is verified against the trusted root
before publication. Existing release tags/assets are never replaced. The initial
release helper publishes prereleases as `v<version>-r<revision>`; update the
manifest revision before making another release of the same version.
