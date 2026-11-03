#!/usr/bin/env python3
"""Private adapter for LLVM's Generic_GCC toolchain on x86_64-unknown-none-elf.

Clang performs every compilation. Its generic ELF driver delegates only the
final link to a program named gcc; this adapter translates those driver flags
to the SDK's LLVM ELF linker. A host GCC runtime is never invoked or injected.
"""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile


def expand(arguments, depth=0):
    """Read compiler-generated response files with a finite nesting bound."""
    if depth > 8: raise ValueError('Link response nesting exceeds 8')
    result = []
    for arg in arguments:
        if arg.startswith('@'): result.extend(expand(shlex.split(Path(arg[1:]).read_text()), depth + 1))
        else: result.append(arg)
    return result


def main():
    """Translate only supported compiler/linker flags and propagate the actual linker status."""
    sdk = Path(__file__).resolve().parents[1]
    args = expand(sys.argv[1:]); link = ['-m', 'elf_x86_64']; index = 0
    while index < len(args):
        arg = args[index]
        if arg.startswith('-Wl,'): link.extend(arg[4:].split(','))
        elif arg == '-Xlinker': index += 1; link.append(args[index])
        elif arg in ['-o','-L','-l','-T','-u','-e','--sysroot']:
            link.extend(args[index:index + 2]); index += 1
        elif arg.startswith(('-L','-l','-T','-u','--sysroot=')): link.append(arg)
        elif arg in ['-shared','-static','-r','-s','-pie','-no-pie','-rdynamic']:
            link.append('--export-dynamic' if arg == '-rdynamic' else arg)
        elif arg in ['-m64','-nostdlib','-nostartfiles','-nodefaultlibs','-pthread'] or arg.startswith(('-f','-O','-g','-B')):
            pass
        elif arg.startswith('-'): raise ValueError('Unsupported compiler link flag: ' + arg)
        else: link.append(arg)
        index += 1
    # LLVM LLD supports response files too, avoiding the host argument-size limit.
    with tempfile.TemporaryDirectory(prefix='mochios-ld-') as temporary:
        response = Path(temporary) / 'args.rsp'
        response.write_text('\n'.join(json.dumps(arg, ensure_ascii=False) for arg in link))
        raise SystemExit(subprocess.call([str(sdk / 'host/bin/ld.lld'), '@' + str(response)]))


if __name__ == '__main__':
    try: main()
    except (ValueError,OSError,IndexError) as error:
        print(f'mochios ELF linker: {error}',file=sys.stderr);raise SystemExit(1)
