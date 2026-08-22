# Development signing fixture

These keys are exclusively for reproducible local mochiOS development images and
tests. They are public repository fixtures and provide no production identity or
security boundary. Never configure a production image to trust `root.pub`.

The development Root and Issuer private keys are retained here deliberately so
`development-pki refresh` can issue a fresh, protocol-compliant revocation
snapshot during a development build. Revocation snapshots have a hard seven-day
maximum lifetime; the generator uses stable six-day time buckets so cached builds
remain reproducible within a bucket. Rotating this fixture also requires rebuilding
and re-signing every bundled development mpkg.

Production builds must set `MOCHIOS_DEVELOPER_ROOT_PUBLIC_KEYS_HEX` and use certificates
issued outside the normal source and image build environments.
