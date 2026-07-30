# Development signing fixture

These keys are exclusively for reproducible local mochiOS images and tests.
They are public repository fixtures and provide no production identity.

The development Root private key is intentionally not stored in this repository.
`developer.cert.b64` was issued offline for the development Developer key by the
Issuer in `trust-a.json`. The Trust Snapshot is signed by the development Root
whose public key is in `root.pub`; `revocations-a.json` is signed by that Issuer.
Neither the Root nor Issuer private key is retained.

Production builds must set `MOCHIOS_DEVELOPER_ROOT_PUBLIC_KEYS_HEX` and use certificates
issued outside the normal source and image build environments.
