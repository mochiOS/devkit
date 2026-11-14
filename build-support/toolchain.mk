FONT_ASSETS := $(ROOT)/libraries/fonts/out/fonts
SOURCE_CORE := $(ROOT)/core
SOURCE_SYSCALLS := $(ROOT)/user
SOURCE_SERVICES := $(ROOT)/services
SOURCE_BOOT := $(ROOT)/boot
SOURCE_LIBC := $(ROOT)/libraries/libc
SOURCE_VIEWKIT := $(ROOT)/libraries/viewkit
SOURCE_FONTS := $(ROOT)/libraries/fonts
SOURCE_BOOTUI := $(ROOT)/libraries/bootui
SOURCE_DEVKIT := $(ROOT)/tools/devkit
# Scheduling is supplied by mmake at execution time. Keep it out of recipe
# fingerprints: changing -j must not invalidate otherwise identical builds.
RUST_OVERLAY := $(OUT)/rust-std/sysroot-overlay
RUST_TARGET_DIR := $(OUT)/rust-std/target
RUST_TARGET := $(SOURCE_SYSCALLS)/targets/x86_64-unknown-mochios.json
RUST_SDK := $(OUT)/newlib-port/sdk
RUST_LINK_FLAGS := -C debuginfo=1 --remap-path-prefix ${HOME}=/build --remap-path-prefix $(ROOT)=/src/mochios -C linker=x86_64-elf-gcc -C link-arg=--sysroot=$(RUST_SDK)/sysroot -C link-arg=-L$(RUST_SDK)/sysroot/lib -C link-arg=-static -C link-arg=-nostdlib -C link-arg=-nostartfiles -C link-arg=-Wl,-T,$(RUST_SDK)/lib/linker.ld -C link-arg=-Wl,-no-pie -C link-arg=-Wl,-z,noexecstack -C link-arg=-Wl,--start-group -C link-arg=$(RUST_SDK)/lib/crt0.o -C link-arg=$(RUST_SDK)/lib/libmochi_user_newlib_runtime.a -C link-arg=-lc -C link-arg=-lm -C link-arg=-lgcc -C link-arg=-Wl,--end-group
CARGO_PATCHES := --config "patch.crates-io.libc.path='$(SOURCE_LIBC)'" --config "patch.\"https://github.com/mochiOS/libc\".libc.path='$(SOURCE_LIBC)'" --config "patch.\"https://github.com/mochiOS/mnu\".mnu-abi.path='$(SOURCE_CORE)/crates/abi'" --config "patch.\"https://github.com/mochiOS/mBoot\".mboot-protocol.path='$(SOURCE_SERVICES)/mboot-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochi-user-platform.path='$(SOURCE_SYSCALLS)/crates/platform'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochi-user-syscall.path='$(SOURCE_SYSCALLS)/crates/syscall'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-capability-protocol.path='$(SOURCE_SYSCALLS)/crates/capability-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-user-database.path='$(SOURCE_SYSCALLS)/crates/user-database'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-user-protocol.path='$(SOURCE_SYSCALLS)/crates/user-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-viewkit-gpu-protocol.path='$(SOURCE_SYSCALLS)/crates/viewkit-gpu-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-virtio-gpu-protocol.path='$(SOURCE_SYSCALLS)/crates/virtio-gpu-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-linux-gui-protocol.path='$(SOURCE_SYSCALLS)/crates/linux-gui-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-linux-portal-protocol.path='$(SOURCE_SYSCALLS)/crates/linux-portal-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-signature-protocol.path='$(SOURCE_SYSCALLS)/crates/signature-protocol'" --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-tls-client.path='$(SOURCE_SYSCALLS)/crates/tls-client'" --config "patch.crates-io.mochios-mdriver-protocol.path='$(SOURCE_BOOT)/crates/mdriver-protocol'" --config "patch.crates-io.viewkit.path='$(SOURCE_VIEWKIT)'" --config "patch.\"https://github.com/mochiOS/viewkit\".viewkit.path='$(SOURCE_VIEWKIT)'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-net-device-protocol.path='$(SOURCE_SYSCALLS)/crates/net-device-protocol'"
CARGO_PATCHES += --config "patch.crates-io.mnu-abi.path='$(SOURCE_CORE)/crates/abi'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/mnu\".mnu.path='$(SOURCE_CORE)'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/mnu\".plugkit.path='$(SOURCE_CORE)/crates/PlugKit/plugkit'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochi-user-runtime.path='$(SOURCE_SYSCALLS)/crates/runtime'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-filesystem-protocol.path='$(SOURCE_SYSCALLS)/crates/filesystem-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-workspace-protocol.path='$(SOURCE_SYSCALLS)/crates/workspace-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-posix-protocol.path='$(SOURCE_SYSCALLS)/crates/posix-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-posix-client.path='$(SOURCE_SYSCALLS)/crates/posix-client'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-certificate-database.path='$(SOURCE_SYSCALLS)/crates/certificate-database'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-elf-signature.path='$(SOURCE_SYSCALLS)/crates/elf-signature'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-keychain-protocol.path='$(SOURCE_SYSCALLS)/crates/keychain-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-network-stack.path='$(SOURCE_SYSCALLS)/crates/network-stack'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-package-client.path='$(SOURCE_SYSCALLS)/crates/package-client'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-package-manifest.path='$(SOURCE_SYSCALLS)/crates/package-manifest'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-package-protocol.path='$(SOURCE_SYSCALLS)/crates/package-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-test-support.path='$(SOURCE_SYSCALLS)/crates/test-support'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/fonts\".mochios-fonts.path='$(SOURCE_FONTS)'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/BootUI\".bootui.path='$(SOURCE_BOOTUI)'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/boot\".mochios-boot-selection.path='$(SOURCE_BOOT)/crates/boot-selection'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/boot\".mochios-mdriver-protocol.path='$(SOURCE_BOOT)/crates/mdriver-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/boot\".mochios-system-image.path='$(SOURCE_BOOT)/crates/system-image'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/devkit\".mochios-appcore.path='$(SOURCE_DEVKIT)/crates/appcore'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/devkit\".mochios-device-consent.path='$(SOURCE_DEVKIT)/crates/device-consent'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/devkit\".mochios-file-publication.path='$(SOURCE_DEVKIT)/crates/file-publication'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/devkit\".mochios-linker-driver.path='$(SOURCE_DEVKIT)/crates/linker-driver'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/services\".ext4plus.path='$(SOURCE_SERVICES)/third_party/ext4plus'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-certificate.path='$(SOURCE_SYSCALLS)/crates/certificate'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-driver-control-protocol.path='$(SOURCE_SYSCALLS)/crates/driver-control-protocol'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-http-client.path='$(SOURCE_SYSCALLS)/crates/http-client'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochios-tls-stream-ffi.path='$(SOURCE_SYSCALLS)/crates/tls-stream-ffi'"
CARGO_PATCHES += --config "patch.\"https://github.com/mochiOS/syscalls\".mochi-user-newlib-runtime.path='$(SOURCE_SYSCALLS)/crates/newlib-runtime'"
MOCHIOS_CARGO := env CARGO_HOME="${CARGO_HOME:-${HOME}/.cargo}" CARGO_BUILD_JOBS="${MMAKE_JOBS}" RUSTC=$(RUST_OVERLAY)/bin/rustc RUSTDOC=$(RUST_OVERLAY)/bin/rustdoc VIEWKIT_UI_FONT_PATH=$(FONT_ASSETS)/InterVariable.ttf VIEWKIT_MONOSPACE_FONT_PATH=$(FONT_ASSETS)/UDEVGothic-Regular.ttf VIEWKIT_JAPANESE_FONT_PATH=$(FONT_ASSETS)/IBMPlexSansJP-Regular.ttf MOCHIOS_VERSION="${MOCHIOS_VERSION:-26.0.0}" MNU_VERSION="${MNU_VERSION:-0.1.0}" MBOOT_VERSION="${MBOOT_VERSION:-0.1.0}" MOCHIOS_BUILD_NUMBER="${MOCHIOS_BUILD_NUMBER:-0}" MNU_GIT_REVISION="${MNU_GIT_REVISION:-workspace}" MNU_RUSTC_VERSION="${MNU_RUSTC_VERSION:-nightly-2026-07-16}" MNU_BUILD_PROFILE="${MNU_BUILD_PROFILE:-release}" MNU_BUILD_FEATURES="${MNU_BUILD_FEATURES:-kernel-bin}" "RUSTFLAGS=$(RUST_LINK_FLAGS)" $(RUST_OVERLAY)/bin/cargo build --offline -Z build-std=std,panic_abort,compiler_builtins -Z json-target-spec $(CARGO_PATCHES) --release --target $(RUST_TARGET) --target-dir $(RUST_TARGET_DIR)
