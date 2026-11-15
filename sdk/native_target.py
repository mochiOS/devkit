"""The native CPU contract shared by SDK builders, drivers and verifiers."""
import json
from typing import NamedTuple


class Target(NamedTuple):
    triple: str
    arch: str
    clang: str
    machine: int
    emulation: str
    cpu_flags: tuple

    @property
    def builtins(self):
        return 'libclang_rt.builtins-' + self.arch + '.a'

    def check_elf(self, path):
        """Read only the ELF header; never execute a target binary on the host."""
        with path.open('rb') as stream:
            header = stream.read(64)
        if (len(header) != 64 or header[:6] != b'\x7fELF\x02\x01'
                or int.from_bytes(header[18:20], 'little') != self.machine):
            raise ValueError('Expected a native ' + self.arch + ' ELF: ' + str(path))


TARGETS = {
    'x86_64-unknown-mochios': Target('x86_64-unknown-mochios', 'x86_64',
        'x86_64-unknown-none-elf', 62, 'elf_x86_64', ('-mno-red-zone',)),
    # Clang's AArch64 BareMetal toolchain ignores -shared. Generic ELF routes
    # both executable and DSO links through our private LLVM linker adapter.
    'aarch64-unknown-mochios': Target('aarch64-unknown-mochios', 'aarch64',
        'aarch64-unknown-mochios', 183, 'aarch64elf', ('-mno-outline-atomics',)),
}


def resolve(triple):
    try:
        return TARGETS[triple]
    except KeyError:
        raise ValueError('Unsupported native SDK target: ' + str(triple)) from None


def from_sdk(sdk):
    """Require explicit identity and reject disagreeing SDK metadata."""
    marker = sdk / 'share/native-target'
    metadata = sdk / 'sdk.json'
    triple = marker.read_text().strip() if marker.is_file() else None
    if metadata.is_file():
        recorded = json.loads(metadata.read_text())['target']
        if triple is not None and triple != recorded:
            raise ValueError('SDK target marker and metadata disagree')
        triple = recorded
    return resolve(triple)


def initialize(sdk, target):
    """One output tree owns one CPU, including older SDKs without a marker."""
    if (sdk / 'share/native-target').exists() or (sdk / 'sdk.json').exists():
        if from_sdk(sdk) != target:
            raise ValueError('SDK output belongs to another target; use a separate --output')
    if (sdk / 'lib/crt0.o').is_file():
        target.check_elf(sdk / 'lib/crt0.o')
    (sdk / 'share').mkdir(parents=True, exist_ok=True)
    (sdk / 'share/native-target').write_text(target.triple + '\n')
