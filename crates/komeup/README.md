# komeup

komeup manages developer toolchains. Kome compiler commands remain under
`komeup install`, `update`, `default`, `status`, and `doctor`. C/C++ SDKs have an
independent `komeup sdk` namespace and do not require an installed Kome compiler.

## C/C++ SDK commands

```sh
komeup sdk install 27.0-dp.1-shared.2
komeup sdk list
komeup sdk default 27.0-dp.1-shared.2
komeup sdk path --cmake
komeup sdk path --version 27.0-dp.1-shared.2 --bin
komeup sdk doctor
```

`install` selects the first SDK automatically. Later installations preserve the
existing default unless `--default` is specified. `komeup sdk update VERSION`
installs that explicit version and selects it while retaining older versions.
There is no implicit “latest” lookup: AppCore and C/C++ SDK releases are distinct,
and SDK prereleases use the `sdk-VERSION` release tag in `mochiOS/devkit`.

For an SDK that has not been published, use the directory containing the three
build artifacts:

```sh
komeup sdk install 27.0-dp.1-shared.2 --from "$HOME/Downloads/shared-sdk"
```

The directory must contain `x86_64-mochios-sdk-VERSION.tar.gz`, the corresponding
`.tar.gz.json` manifest, and `SHA256SUMS`. Installation verifies the archive's
SHA-256, each payload file's hash/size, symlinks, SDK identity, host, target, ABI,
and declared shared-library features. Remote metadata and assets must come from
the explicit mochiOS release over HTTPS. Checksums establish consistency with
that release; they are not a separate code-signing identity.

Downloads/extraction have size limits. Extraction happens in private staging,
rejects traversal, duplicate paths, privileged permissions, hardlinks and
unexpected entries, and creates symlinks only after regular files. A completed
installation becomes visible with one rename. Existing versions cannot be
replaced with different bytes. Failed downloads/validation preserve the selected
SDK. Downloaded SDK programs are not executed by installation or `sdk doctor`.

## Build-system integration

The default compiler shims are `~/.kome/bin/mochios-cc` and `mochios-c++`.
Add `~/.kome/bin` to PATH yourself if it is not already present; komeup never
edits shell profiles.

```sh
cmake -S . -B build --toolchain "$(komeup sdk path --cmake)"
```

`path` prints the immutable version directory, not the mutable default selector.
A project configured this way stays pinned when the global default changes.
Use `path --version VERSION --cmake` to select a project-specific SDK without
changing the global default. Existing build directories must be reconfigured
when intentionally switching their compiler/toolchain. komeup does not rewrite
CMakeLists.txt, build projects, build SDKs, or publish releases.

SDK versions live in `~/.kome/sdks/installed/VERSION`. Their receipts and manifest
copies are local manager metadata. The default selector is a relative symlink,
updated atomically under the SDK manager lock. Kome compiler selection,
`komeup.toml`, AppCore, and existing Kome shims are preserved. `KOMEUP_HOME`
overrides the installation root for isolated environments.

`komeup sdk uninstall VERSION` removes only that managed SDK. Removing the selected
SDK clears the SDK default; another installed version can be selected explicitly.
The pre-existing global `komeup uninstall --yes` command removes the entire Kome
home, including SDKs.

The current SDK requires x86-64 Linux compatible with Ubuntu 24.04 and Python
3.12+. `sdk doctor` checks recorded payload hashes and Python availability; it
does not claim that SDK binaries or generated mochiOS applications were executed.
Building komeup from source requires Rust 1.89 or newer.
