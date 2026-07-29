install:
	cargo install --path crates/kome
	cargo install --path crates/mpack
	cargo install --path crates/msign
	cargo install --path crates/komeup

test-e2e-mpkg:
	bash tests/e2e-mpkg-flow.sh

test-e2e-appstore:
	test -n "$(APPSTORE_REVIEWER_DIR)"
	APPSTORE_REVIEWER_DIR="$(APPSTORE_REVIEWER_DIR)" bash tests/e2e-mpkg-flow.sh

.PHONY: install test-e2e-mpkg test-e2e-appstore
