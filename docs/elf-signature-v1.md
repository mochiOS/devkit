# ELF Signature v1 wire format

This document fixes the byte-level format emitted and accepted by `msign elf`.
It is the concrete v1 profile of the design in `docs/elf-signature-memo.md`.

## Supported ELF profile

V1 accepts ELF64, little-endian, ELF version 1, System V ABI `ET_EXEC` and
`ET_DYN` files. A valid Section Header Table and section-name string table are
required. Extended section numbering is not accepted.

Requiring the Section Header Table is a deliberate v1 compatibility boundary,
not an assumption that executable content is section-based. The verifier also
recognizes `PT_NOTE` references and treats a section/program-header pair that
references the exact same NOTE area as one physical NOTE. A future format
revision may permit PT_NOTE-only files without changing the descriptor's
cryptographic meaning.

`msign` writes a non-loaded `SHT_NOTE` section named
`.note.mochios.signature`. The NOTE has:

| Field | Value |
| --- | --- |
| `namesz` | 8, little-endian `u32` |
| `descsz` | 146, little-endian `u32` |
| `type` | `0x4d534947`, little-endian `u32` |
| `name` | the 8 bytes `mochiOS\0` |

All NOTE padding emitted for this NOTE is zero. The verifier discovers the
signature by NOTE name and type, not by section name alone. More than one
physical matching NOTE, duplicate references from the same header table, or
otherwise ambiguous references are rejected. A single SHT_NOTE/PT_NOTE pair
may reference the same byte-identical NOTE area.

## Descriptor

The descriptor is exactly 146 bytes. All integer fields are little-endian.

| Offset | Width | Field | V1 value |
| ---: | ---: | --- | --- |
| 0 | 8 | magic | ASCII `MOELFSIG` |
| 8 | 2 | version | `1` (`u16`) |
| 10 | 1 | kind | `1` AdHoc, `2` Ed25519 |
| 11 | 1 | digest algorithm | `1` SHA-256 |
| 12 | 1 | signature algorithm | `0` none, `1` Ed25519 |
| 13 | 1 | reserved | zero |
| 14 | 4 | flags | zero (`u32`) |
| 18 | 32 | key ID | described below |
| 50 | 32 | ELF digest | SHA-256 |
| 82 | 64 | signature | described below |

Unknown versions, kinds, algorithms or flags and non-zero reserved bytes are
rejected. AdHoc requires a zero key ID and zero signature. Ed25519 requires
signature algorithm 1 and a valid signature; it never falls back to AdHoc.

For Ed25519, `key_id` is exactly:

```text
SHA-256(raw 32-byte Ed25519 public key)
```

It is not a hash of PEM, DER, base64 text, a certificate, or a private key.

## ELF digest

The SHA-256 input is the complete final ELF byte sequence. Only descriptor
bytes `[50, 82)` (the stored digest) and `[82, 146)` (the stored signature) are
replaced with zero bytes while hashing. No bytes are removed and no offsets are
rewritten for hashing.

The verifier derives those ranges from a structurally validated NOTE. It
rejects exclusion ranges that overlap the ELF header, either header table, a
`PT_LOAD` file range, or another section. This prevents attacker-controlled ELF
metadata from turning executable or control bytes into unhashed holes.

## Ed25519 message

This is ordinary Ed25519 over the following exact 96-byte message; it is not
Ed25519ph. Concatenation has no separators other than the fixed terminal NUL in
the context string.

```text
offset  width  bytes
0       25     ASCII "mochios-elf-signature-v1" followed by 0x00
25      1      kind (`u8`)
26      1      digest algorithm (`u8`)
27      1      signature algorithm (`u8`)
28      4      flags (`u32`, little-endian)
32      32     key_id
64      32     ELF digest
```

The keyed verifier asks a `PublicKeyResolver` for the key ID, independently
recomputes the key ID from the returned raw public key, and uses strict
Ed25519 verification. No key, an incorrect key, or an invalid signature is a
hard failure. Certificate validity and trust policy remain separate from this
cryptographic verification and are intentionally outside this implementation.

## CLI and file replacement

```text
msign elf sign FILE --adhoc [--output OUTPUT]
msign elf sign FILE --key PRIVATE_KEY [--output OUTPUT]
msign elf verify FILE [--public-key PUBLIC_KEY]
```

Signing refuses an already signed or ambiguously signed ELF. Output is written
inside a private staging directory in the destination directory. The common
`mochios-file-publication` helper checks that the held descriptor and named
regular file have the same device/inode, syncs the descriptor, atomically
renames the file, and syncs the destination directory on Unix. Input permissions
are preserved without reopening the file after applying those permissions.
The linker driver and MPKG writer use the same publication helper. A directory
sync error can be reported after publication; ordinary failure cleanup removes
only the owned staging directory. Forced-termination recovery is not provided.
Symbolic-link inputs and outputs are rejected, and an output that aliases the
private key is rejected.

AdHoc proves only self-consistency. It does not identify a signer or establish
trust, provenance, execution permission, or capabilities.

## Native build profile

The default `developer-ca` feature includes the HTTP `certificate obtain`
command. `cargo build -p msign --no-default-features` omits that command and
reqwest, while retaining the same ELF signing/verifying, key generation,
certificate issue/inspect, package, and identity implementations. Unsupported
commands are rejected by the CLI. This profile does not implement the missing
mochiOS mio/socket2 port required by the default network-enabled build.

On mochiOS the binary registers posix-client's getrandom 0.2 adapter, using the
same native randomness primitive as getentropy. Partial transfers and EINTR
are handled without replacing entropy or hiding permission/readiness errors.
The production dependency graph uses std file publication and in-memory tar
processing; tempfile remains a host test dependency, and unused tar xattrs are
disabled. No cryptographic format or trust policy changes with the profile.

Regression tests reside under `services/posix/tests/`. The native signer test
uses explicitly staged tools and SDK inputs; it is separate from ordinary
clang defaults and the execution-time Capability consent path.

## Shared userspace codec

`user/crates/elf-signature` provides `mochios-elf-signature`, a `no_std + alloc`
library containing the v1 encoder, parser, normalized hashing, and strict
verification. msign uses this library and continues to own file/key access and
publication. The library operates on supplied bytes and keys; consumers own
trust, revocation, execution policy, and allocation/work budgets.

`decode` returns unverified record metadata. `verify` establishes cryptographic
integrity. Its result's `digest` is the normalized v1 signature digest above;
launch authorization must separately measure the full captured source bytes
and bind the actual prepared image to its commit-time measurement.

The eight existing codec tests moved to `services/posix/tests/elf_signature.rs`.
They include the production codec directly to check internal wire/bounds cases.
msign's CLI tests additionally exercise the library through its public API.
The wire format and signing policy are unchanged. Services can now consume the
same verifier; execution-gate integration is still pending.

`verify_with_limits` bounds image bytes, header counts, and cumulative NOTE/name
scans, including repeated references to overlapping file ranges. `verify` uses
the default budgets. These are consumer resource limits, not a new wire format
or permission to fall back to unsigned execution. Header/reference allocation
is fallible, and the signing message uses a fixed 96-byte array.

`verify_image` additionally returns a borrowed `VerifiedImage` with the SHA-256
of all original file bytes, including the signature fields. Its `source_digest`
is suitable for binding that snapshot to native execution; the normalized
`signature().digest` is not interchangeable with it. `SuppliedPublicKey` only
resolves a caller-provided verification key and confers no developer trust.
The codec suite now has 18 cases covering these measurements, scan bounds and
the embedded declaration format below.

## Embedded manifest declarations

`msign elf sign program --adhoc --manifest manifest.toml` embeds the exact input
declaration before computing the existing v1 signature. Keyed signing accepts
the same option. The input must be a bounded regular file containing valid UTF-8
TOML satisfying `mochios-package-manifest`, the shared userspace schema also
re-exported by platform::package. This rejects duplicate keys, malformed escapes
and invalid declaration types before signing. Its maximum text length is 65536
bytes; empty text and NUL bytes are rejected.
The signer refuses symlink inputs and output aliases of the manifest or key.
No manifest is implicitly discovered in the working directory.

The optional, unique `.mochios.manifest` section is SHT_PROGBITS, with zero flags,
address, link, info and entry size, and four-byte alignment. It is outside all
PT_LOAD and PT_NOTE regions, both header tables and all other file-backed sections.
Its payload starts with this little-endian envelope, followed immediately by
the exact manifest.toml text, without a terminator:

| Offset | Size | Value |
| --- | --- | --- |
| 0 | 8 | ASCII `MOELFMAN` |
| 8 | 2 | Envelope version 1 |
| 10 | 2 | UTF-8 manifest.toml encoding 1 |
| 12 | 4 | Reserved zero |
| 16 | 1–65536 | Manifest text |

`embed_manifest` operates only on unsigned bytes and refuses existing manifests
or signatures. It and the signature encoder share the section-table builder;
neither changes program mappings. `verify` and `verify_image` reject malformed,
duplicate, overlapping or unsupported manifest sections. `VerifiedImage::manifest`
borrows the declaration from the same verified snapshot as `source_digest`.
TOML schema interpretation, capability classification, signer trust and consent
remain userspace policy outside the ELF codec. The declaration grants nothing.

This optional metadata is covered by the existing full-file v1 digest. It does
not change the signature NOTE format or normalized digest algorithm. Without
`--manifest`, the AdHoc output was compared byte-for-byte with the prior signer.
Tests are in services/posix/tests/elf_signature.rs, embedded_manifest_tools.rs
and linker_driver.rs. Enforcing declarations throughout ordinary exec remains
a separate, incomplete integration step.
