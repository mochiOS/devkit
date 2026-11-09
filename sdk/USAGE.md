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

Executables own one process runtime, including FD, errno, allocator and thread state. Shared objects import the executable's measured C/C++ ABI; they never embed another copy of that runtime. Compiler-generated arithmetic helpers (such as 128-bit division) are extracted on demand from target compiler-rt and hidden within each shared object; they do not duplicate process runtime state. Shared links reject unknown strong imports outside that ABI and declared shared dependencies. Use a SONAME rather than embedding an absolute dependency path. CMake root searches cannot fall back to host headers or libraries. Executables are ELF EXEC at/above 0x400000, with no Linux interpreter, W^X, RELRO and a non-executable stack.

The unreleased shared.5 recipe targets startup-linked shared objects and authenticated private `dlopen` groups. TLS uses initial-exec. Private loading accepts `RTLD_LAZY` or `RTLD_NOW`, with `RTLD_LOCAL`; it requires matching OS loader and capability-broker support. `dlopen(NULL)`, `RTLD_NOLOAD`, global/default namespace lookup, `RTLD_DEEPBIND` and `RTLD_NODELETE` remain unsupported. This SDK does not promise PIE applications, complete POSIX conformance, or a ready-to-run WebKit port. The runtime API list records known unsupported C functions. C++23, threads and exceptions are compile/link checks; actual execution on mochiOS remains a separate validation gate. `sdk.json` explicitly records `runtime_tested: false` until that gate is met.

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

## Native TLS record adapter (shared.4 build candidate)

The SDK builds `lib/libmochios_tls_pic.a` and `sysroot/include/mochios/tls.h`
from the separately pinned `tls` source. The process platform is selected by
the `user` and `mnu` source pins. Only `tls-client` and `tls-stream-ffi`
are overlaid onto that platform workspace; unrelated userland changes are not
part of this SDK build.

This is a **private PIC archive for a shared-library adapter**, such as
`libgiomochios.so`. Link it with `-lmochios_tls_pic` and a version script that
hides all Rust internals. Do not link the archive directly into a C executable:
the executable already owns its Rust process runtime and compiler builtins.
The SDK checks both an adapter DSO and a final executable consuming that DSO.

The adapter uses the existing TLS 1.3 verifier, production roots, OS entropy and
clock. It performs no socket I/O and provides no certificate-verification
bypass. The host-only test CA feature is rejected when targeting mochiOS.
Native runtime interoperability remains a separate OS validation step; a cross
link is not proof of a successful HTTPS request on mochiOS.

The normal build only creates local upload candidates. Release publication
remains manual.

The shared.5 candidate also provides `secure_getenv`. It uses the secure state
captured at authenticated image start plus the current POSIX broker credentials;
missing state, failed queries and unrepresentable IDs deny access. It does not
re-read user-writable auxv to authorize an environment lookup. This does not
replace ELF/package authentication or Capability checks.

### Observing pthread stacks

With `_GNU_SOURCE` enabled, `pthread_getattr_np(thread, &attributes)` observes a live process-local thread. Use `pthread_attr_getstack`, `pthread_attr_getguardsize` and `pthread_attr_destroy` on the returned attribute. The usable range excludes runtime-owned guard pages. A caller-supplied stack reports its supplied range and no runtime-owned guard.

For the initial thread, shared.5 requires a kernel exposing `ProcessInitialStackRead` (syscall 706). The kernel reports the committed stack and the downward extent currently admitted by its existing growth rules. This is an observation, not a reservation: later mappings can restrict growth. Unknown foreign stacks and unsupported kernels return errors; the runtime never invents a default range. Ordinary pthread stacks use runtime-owned metadata and do not require the initial-stack query. The SDK includes DSO and executable compile/link probes; native execution is a separate gate.

### Native Unix APIs in shared.7

The shared.7 recipe exposes `flock`, `mincore`, `madvise`, `dladdr`,
`dl_iterate_phdr`, `pthread_getschedparam` and `pthread_setschedparam` as actual
process-runtime exports. It also corrects the native `ssize_t` I/O declarations.
The source-pinned Unix extension patch is part of the recipe and its digest is
checked before preparation; no uncommitted OS checkout is a build input.

Deployment requires the matching filesystem whole-file lock provider, native
loader metadata and kernel memory observation/discard mechanisms (707/708).
Unsupported kernels and mapping kinds fail explicitly. Realtime FIFO/RR
scheduling, dynamic TLS and RTLD_GLOBAL namespaces are not provided. Local
DSOs can import the initial executable's pinned default ordinary exports;
hidden/nondefault symbol versions and TLS/IFUNC imports do not enter that
fallback. The initial provider is not mapped again or finalized by `dlclose`.

The runtime source has passed the dedicated native Unix VM fixture, including
constructor inspection and unload during enumeration. A distribution build
still needs the complete SDK compiler/archive/path audit and artifact checks.
`sdk.yml` builds upload candidates only; it does not publish a release.
