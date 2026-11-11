# Build an MPKG in GitHub Actions

This public composite Action creates an **unsigned MPKG v1** from an existing
manifest and prebuilt payload. It accepts packages from any publisher: native
applications, command-line tools, libraries, and resources supported by MPKG.
It requires no mochiOS organization membership, GitHub token, signing identity,
AppStore account, or particular repository layout.

Compilation remains a previous workflow step, using the language and SDK chosen
by the developer. The Action does not invoke the mochiOS image build, generate
publisher identities, alter capabilities, sign packages, or publish releases.

```yaml
permissions:
  contents: read

jobs:
  package:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
        with:
          persist-credentials: false
      # Build your software and prepare manifest.toml and payload/ here.
      - name: Create the unsigned package
        id: mpkg
        uses: mochiOS/devkit/actions/build-mpkg@staging
        with:
          manifest: build/package/manifest.toml
          payload: build/package/payload
          output: dist/Example.mpkg
      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02
        with:
          name: unsigned-mpkg
          path: |
            ${{ steps.mpkg.outputs.package }}
            ${{ steps.mpkg.outputs.checksum-file }}
          if-no-files-found: error
```

`staging` above is the development example. For production, pin the Action to a
reviewed full commit SHA. No stable Action version is published by this change.

## Inputs and outputs

| Input | Meaning |
| --- | --- |
| `manifest` | Required, completed UTF-8 MPKG manifest, `format = 1`. |
| `payload` | Required, directory with the exact prebuilt payload. |
| `output` | New MPKG path, default `dist/package.mpkg`. |
| `mpack` | Optional trusted executable path; otherwise build the bundled mpack. |

The output directory must be fresh: an existing package or `SHA256SUMS` is never
overwritten. Use a separate output directory for each package. Symlink and
hard-linked payload files are rejected by mpack. Payload contents follow the ordinary MPKG layout: `bundle/` for application
files and `root/` for filesystem-root payloads. Manifest file bindings, resource
paths and capabilities are preserved exactly; this Action does not derive or
repair missing bindings. It performs packaging and container checks, not signer,
installation-policy or runtime validation.

Outputs are `package` (absolute MPKG path), `sha256` (whole-file SHA-256), and
`checksum-file` (absolute `SHA256SUMS` path). A checksum is not a code signature.
Artifact upload, signing, release approval and publication belong to the caller's
workflow. Official mochiOS apps use the same Action as third-party developers.

## Runner requirements

The initial supported runner is `ubuntu-24.04` (Python 3.12+, Bash). Either provide
`mpack`, or have Cargo/Rust and network access to fetch locked crates. The bundled
tool is built as a standalone host project from this Action's own source and
Cargo.lock; the OS/AppCore workspace is not required. Native mochiOS programs are
never executed on the host by this Action.

Local invocation, useful for debugging the same operation:

```sh
MPKG_MANIFEST=build/package/manifest.toml \
MPKG_PAYLOAD=build/package/payload \
MPKG_OUTPUT=dist/Example.mpkg \
MPKG_TOOL=/path/to/mpack \
python3 /path/to/devkit/actions/build-mpkg/package.py
```
