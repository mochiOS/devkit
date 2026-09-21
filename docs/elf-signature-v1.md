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
to a temporary file in the destination directory, flushed, atomically renamed,
and the directory is flushed on Unix. Input permissions are preserved.
Symbolic-link inputs and outputs are rejected, and an output that aliases the
private key is rejected.

AdHoc proves only self-consistency. It does not identify a signer or establish
trust, provenance, execution permission, or capabilities.
