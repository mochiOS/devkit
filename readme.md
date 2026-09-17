# mochiOS developer kit

mochiOS applications are distributed as deterministic MPKG v1 archives. The
supported development path uses `mpack` for package construction and `msign`
for local identity management, signing, and verification. Kome remains an
unfinished language/tooling experiment and is not required by this workflow.

## Quick start

Import a Developer Certificate and its matching Ed25519 private key once:

```sh
msign identity import local-dev \
  --certificate developer.cert \
  --key application.key \
  --default
```

Build an unsigned package from a canonical runtime manifest and payload tree,
then sign it using the automatically selected local identity:

```sh
mpack create \
  --manifest manifest.toml \
  --payload payload \
  --output dist/Example.mpkg
msign package sign-auto dist/Example.mpkg
```

Inside the mochiOS source tree the same flow is available through mmake:

```sh
mmake package-and-sign \
  PACKAGE_MANIFEST=/workspace/Example/manifest.toml \
  PACKAGE_PAYLOAD=/workspace/Example/payload \
  PACKAGE=/workspace/Example/dist/Example.mpkg
```

The build graph never contains a private-key path. `msign` selects an eligible
identity by Package ID scope, certificate validity, and requested Capability
allowance. Ambiguous selection fails rather than choosing unpredictably.

## Tools

- `mpack create`: deterministic unsigned MPKG v1 construction
- `msign identity`: local signing identity import and inspection
- `msign package sign-auto`: automatic local identity selection and signing
- `msign package verify`: low-level package verification
- `development-pki`: reproducible development trust fixtures

`mpack pack` and the Kome commands are not part of the supported application
development path while Kome is unfinished.

## Local identity storage

The default store is `$XDG_CONFIG_HOME/mochios/signing`, or
`$HOME/.config/mochios/signing` when `XDG_CONFIG_HOME` is unset.
`MOCHIOS_SIGNING_HOME` may select an isolated store. Store directories are mode
`0700`; private keys and the default selector are mode `0600`.

Identity selector names are workstation-local labels. They do not replace the
certificate developer identity or require central Package ID registration.

## Package and security model

MPKG v1 contains a fixed 32-byte header and a deterministic uncompressed ustar
stream. The signed canonical manifest commits to every payload size and SHA-256
digest. Package signatures use Ed25519 and carry a Developer Certificate.

Signing establishes identity and integrity only. Capability authorization is
computed independently at install, spawn, and exec from requested Capability,
certificate allowance, system policy, user grant, and caller/delegation
ceiling. The kernel remains the final enforcement boundary.

Install provenance is assigned by a trusted verifier:

- BuiltIn records are generated while constructing the boot image.
- VerifiedPackage records come from an ordinary trusted signing issuer.
- Development records require a trusted issuer with the
  `development-package-signing` usage.

A package manifest or signer cannot self-assert trusted provenance.

## Certificate operations

Generate a standalone application key when provisioning a new identity:

```sh
msign key generate \
  --private-key application.key \
  --public-key application.pub
```

Obtain a certificate without uploading the private key, MPKG payload, source,
or build output:

```sh
msign certificate obtain \
  --developer 019f9e5ac6687902b0e72fe53abfbef1 \
  --public-key application.pub \
  --package dist/Example.mpkg \
  --output developer.cert
```

`msign certificate issue` is reserved for operators and reproducible fixtures.

## Guides

- [Package signing](docs/package-signing.md)
- [Developer key management](docs/developer-key-management.md)
- [Certificate obtain](docs/certificate-obtain.md)
- [Package ID rules](docs/package-id.md)
- [MPKG v1](docs/mpkg-v1.md)
- [AppStore publishing](docs/appstore-publish.md)
- [Legacy package migration](docs/legacy-pkg-migration.md)

## Security rules

- Private keys stay local and are never placed in MPKG files.
- Certificate scope and Capability allowance are checked before signing and
  again during installation.
- Runtime trust uses current signed trust and revocation snapshots and fails
  closed when they are unavailable or expired.
- Development fixture keys are public test material and are never production
  identities or production trust roots.
