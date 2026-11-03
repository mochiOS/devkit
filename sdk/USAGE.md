# Shared C/C++ SDK

This SDK targets `x86_64-unknown-mochios` (`mochios-1`). Its host is Ubuntu 24.04 x86-64, with Python 3.12, glibc, libstdc++ and libgcc_s supplied by the host. The archive contains Clang/LLD, LLVM archive and inspection tools, target headers, compiler-rt builtins, newlib, the syscall-backed mochiOS runtime, libc++, libc++abi, libunwind, CMake files, original notices and pinned source provenance.

## Build the SDK

Install CMake 3.28+, Make, Git, Python 3.12+, Perl, patch, dpkg-deb, a host C/C++ compiler and Rust `nightly-2026-05-14` with rust-src. Then run:

```sh
python3 sdk/build.py build --jobs 4
```

All downloaded inputs and outputs go to `out/shared-sdk`; an OS checkout is neither read nor written. Git commits, LLVM and host package checksums, and local patch checksums are fixed in `sdk/sources.lock.json`. Newlib's corresponding patched source accompanies the binary archive. No credentials are required for public source downloads. Target binaries are inspected and linked, never run on Linux. A failed build does not produce a new successful release.

## Compile applications and shared libraries

After verifying `SHA256SUMS`, extract the archive and use:

```sh
sdk/bin/mochios-cc -shared library.c -Wl,-soname,libexample.so -o libexample.so
sdk/bin/mochios-cc main.c -L. -lexample -o Example.app
sdk/bin/mochios-c++ -std=c++23 main.cpp -L. -lexample -o Example.app
cmake -S . -B build -DCMAKE_TOOLCHAIN_FILE="$PWD/sdk/share/cmake/mochios.cmake"
cmake --build build
```

Executables own one process runtime, including FD, errno, allocator and thread state. Shared objects import the executable's measured C/C++ ABI; they never embed another copy of that runtime. Shared links reject unknown strong imports outside that ABI and declared shared dependencies. Use a SONAME rather than embedding an absolute dependency path. CMake root searches cannot fall back to host headers or libraries. Executables are ELF EXEC at/above 0x400000, with no Linux interpreter, W^X, RELRO and a non-executable stack.

Only startup-linked shared objects are supported. TLS uses initial-exec. This SDK does not promise `dlopen`, PIE applications, complete POSIX conformance, or a ready-to-run WebKit port. The runtime API list records known unsupported C functions. C++23, threads and exceptions are compile/link checks; actual execution on mochiOS remains a separate validation gate. `sdk.json` explicitly records `runtime_tested: false` until that gate is met.

## Releases

Shared SDK releases use a distinct prerelease tag, `sdk-<sdk.json version>`, without replacing AppCore releases. The manually dispatched SDK workflow rebuilds from the fixed inputs, runs structural and linker tests, uploads the SDK artifacts and build logs. It never creates or publishes a GitHub release; release creation and asset upload are performed manually by the maintainer. Existing release assets are never overwritten. The SDK archive, manifest and SHA-256 completion marker form one release. `mochiOS/libs` pins the archive URL, SHA-256 and builder image digest before building target libraries.

## Build path hygiene

C, C++, preprocessed assembly and Rust runtime builds remap source, checkout,
cache and toolchain paths to stable virtual prefixes. `__FILE__` and debug
records retain useful source locations without exposing the build machine.
Rust compiler wrappers and existing compiler flags are preserved. A changed
path policy gets a separate Cargo cache so old unsanitized objects are not
reused. Newlib records its effective mapping flags in its rebuild stamp.

Before producing an upload candidate, the builder scans every distributed
file, symlink and bundled source archive for real build roots. The final tar
is checked again. Archive ownership and timestamps are normalized, and gzip
does not record a build-machine filename. Original source and license text is
not rewritten. A failed audit stops archive and checksum publication; an old
artifact is never silently replaced. The JSON manifest contains a path-audit
summary, while the policy containing real local paths stays outside the SDK.

Standard target paths and virtual `/src`, `/build`, `/sdk` and `/toolchain`
locations are intentional. This is not a promise to remove every absolute
path used by the target operating system. CI only produces downloadable
artifacts; creating a GitHub release remains a manual maintainer action.
