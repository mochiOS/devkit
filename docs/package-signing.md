# Package signing

mochiOS package signing establishes the developer identity and protects the
MPKG manifest and payload integrity. Capability authorization remains a
separate policy decision made from the package request, certificate allowance,
system policy, user grants, and delegation ceiling.

The normal development workflow uses a local signing identity store. Private
key paths are not stored in a Makefile, package manifest, or repository.

## Identity store

By default, `msign` stores local identities below:

```text
$XDG_CONFIG_HOME/mochios/signing
```

If `XDG_CONFIG_HOME` is unset, it uses:

```text
$HOME/.config/mochios/signing
```

`MOCHIOS_SIGNING_HOME` may override the root for isolated build environments.
The root and identity directories are private to the current user. Private
keys and the default-identity selector are created with mode `0600`.

Import an existing Developer Certificate and matching Ed25519 private key:

```bash
msign identity import local-dev \
  --certificate developer.cert \
  --key developer.key \
  --default
```

The import verifies the canonical certificate encoding and confirms that the
private key matches its subject public key before storing either file.

List identities:

```bash
msign identity list
```

Identity names are selectors local to the workstation. They are not Package
IDs, developer identities, or a central registration mechanism.

## Automatic selection

For an unsigned MPKG, automatic selection reads the Package ID and requested
binary capabilities from the package manifest. A candidate identity is usable
only when all of the following are true:

* its certificate is currently valid;
* its package scope permits the Package ID;
* its capability allowance covers every requested capability;
* its private key matches the certificate.

The default identity is preferred when it is eligible. If no default is
eligible, exactly one eligible identity must exist; ambiguity is reported as an
error rather than selecting a signer unpredictably.

Sign directly:

```bash
msign package sign-auto app.mpkg
```

Select an identity explicitly without exposing a key path:

```bash
msign package sign-auto app.mpkg --identity local-dev
```

Use `--output signed.mpkg` to preserve the unsigned input, and
`--replace-signature` only when intentionally replacing an existing signature.

## mmake integration

The repository provides an incremental host-tool target and an automatic
package-signing target:

```bash
mmake msign-tool
mmake sign-package PACKAGE=/absolute/or/workspace/path/app.mpkg
mmake package-and-sign \
  PACKAGE_MANIFEST=/workspace/app/manifest.toml \
  PACKAGE_PAYLOAD=/workspace/app/payload \
  PACKAGE=/workspace/app/dist/app.mpkg
```

`sign-package` invokes `msign package sign-auto`; it never accepts or records a
private-key path. Package-producing component targets should depend on
`package-and-sign`, or depend on `msign-tool` and invoke the same command after
creating their MPKG. `package-and-sign` uses the existing canonical
`mpack create` implementation and then signs the resulting MPKG using the local
identity store. The boot
image target does not depend on a personal developer identity: files baked into
the system image use explicit BuiltIn provenance and are not silently treated
as locally signed packages.

When verification uses an issuer carrying the trusted
`development-package-signing` usage, signature.service marks the install record
as Development. A package or signer cannot self-assert that provenance.

## Trust and authorization boundaries

The existing format and cryptography remain unchanged:

* Ed25519 signatures;
* SHA-256 payload digests;
* canonical MPKG manifest signing;
* Developer Certificate package scope and capability allowance.

Successful signing or verification does not grant capabilities by itself. At
install and exec time, mochiOS independently resolves the application identity
and computes effective capabilities under system policy and user/delegation
limits. An unverified package must not be treated as a trusted built-in merely
because verification metadata is absent.

Development fixture keys are for tests and local fixtures only. They are not
production trust roots and must not be copied into a user identity store for
real software distribution.
