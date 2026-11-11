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
