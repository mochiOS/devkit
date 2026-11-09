#!/usr/bin/env python3
"""mochiOS C/C++ compiler driver: isolated headers, ELF shared libraries and one process runtime."""
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile


def response_arguments(arguments, depth=0):
    """Inspect GNU/Clang response files with recursion/size bounds."""
    if depth > 8: raise ValueError('Response-file nesting exceeds 8')
    result = []
    for value in arguments:
        if value.startswith('@'):
            path = Path(value[1:])
            if path.stat().st_size > 8 * 1024**2: raise ValueError('Response file is too large')
            result.extend(response_arguments(shlex.split(path.read_text()), depth + 1))
        else: result.append(value)
    return result


def symbols(tool, path, defined):
    """Read dynamic symbol names without executing target ELF code."""
    args = [str(tool), '-D', '--format=posix', '--defined-only' if defined else '--undefined-only', str(path)]
    text = subprocess.check_output(args, text=True, stderr=subprocess.PIPE)
    result = set()
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and (defined or parts[1] not in ['w', 'v']): result.add(parts[0].split('@', 1)[0])
    return result


def dependencies(sdk, arguments):
    """Collect declared DSO providers for import validation; no host library search is performed."""
    paths = [sdk / 'sysroot/lib', sdk / 'sysroot/usr/lib']
    files, names = [], []
    index = 0
    while index < len(arguments):
        arg = arguments[index]
        if arg == '-L' and index + 1 < len(arguments):
            index += 1; paths.append(Path(arguments[index]))
        elif arg.startswith('-L'): paths.append(Path(arg[2:]))
        elif arg == '-l' and index + 1 < len(arguments):
            index += 1; names.append(arguments[index])
        elif arg.startswith('-l'): names.append(arg[2:])
        elif '.so' in Path(arg).name and Path(arg).is_file(): files.append(Path(arg))
        index += 1
    for name in names:
        for directory in paths:
            path = directory / (name[1:] if name.startswith(':') else 'lib' + name + '.so')
            if path.is_file(): files.append(path); break
    # Explicit DSOs can live in different Meson build subdirectories. Resolve
    # their SONAMEs to those exact inputs before searching a sibling's directory;
    # this adds no ambient host path and cannot hide a missing dependency.
    declared, dynamic_cache = {}, {}
    for path in files:
        path = path.resolve()
        if path in dynamic_cache: continue
        if len(dynamic_cache) >= 256: raise ValueError('Dependency graph exceeds 256 objects')
        dynamic = subprocess.check_output([str(sdk / 'host/bin/readelf'), '-d', str(path)], text=True)
        dynamic_cache[path] = dynamic
        sonames = re.findall(r'\(SONAME\).*?\[([^]]+)\]', dynamic)
        if len(sonames) > 1: raise ValueError('Multiple SONAME entries')
        name = sonames[0] if sonames else path.name
        if '/' in name: raise ValueError('Absolute SONAME paths are forbidden')
        if name in declared and declared[name] != path:
            raise ValueError('Ambiguous explicitly declared SONAME: ' + name)
        declared[name] = path
    providers, visited = set(), set()
    while files:
        path = files.pop().resolve()
        if path in visited: continue
        if len(visited) >= 256: raise ValueError('Dependency graph exceeds 256 objects')
        visited.add(path); providers.update(symbols(sdk / 'host/bin/nm', path, True))
        dynamic = dynamic_cache.get(path)
        if dynamic is None:
            dynamic = subprocess.check_output([str(sdk / 'host/bin/readelf'), '-d', str(path)], text=True)
        for name in re.findall(r'\(NEEDED\).*?\[([^]]+)\]', dynamic):
            if '/' in name: raise ValueError('Absolute DT_NEEDED paths are forbidden')
            found = declared.get(name)
            if found is None:
                found = next((directory / name for directory in [path.parent, *paths] if (directory / name).is_file()), None)
            if found is None: raise ValueError('Missing dependency for import validation: ' + name)
            files.append(found)
    return providers



def shared_runtime_arguments(sdk, arguments):
    """Resolve standard runtime libraries through the process ABI, never embed duplicate runtime state."""
    names={'c','m','c++','c++abi','unwind','pthread','mochi_user_newlib_runtime','builtins','gcc'}
    archives={str((directory/('lib'+name+'.a')).resolve()) for directory in [sdk/'lib',sdk/'sysroot/lib'] for name in names}
    result=[];index=0
    while index<len(arguments):
        arg=arguments[index]
        if arg=='-l' and index+1<len(arguments) and arguments[index+1] in names:index+=2;continue
        if arg.startswith('-l') and arg[2:] in names:index+=1;continue
        if not arg.startswith('-') and str(Path(arg).resolve()) in archives:index+=1;continue
        result.append(arg);index+=1
    return result


def main():
    """Compile or transactionally link, validating shared imports before publishing output."""
    sdk = Path(__file__).resolve().parents[1]
    cxx = '++' in Path(sys.argv[0]).name or Path(sys.argv[0]).name.endswith('c++')
    args = response_arguments(sys.argv[1:])
    env = os.environ.copy()
    env['LD_LIBRARY_PATH'] = str(sdk / 'host/lib')
    os.environ['LD_LIBRARY_PATH'] = env['LD_LIBRARY_PATH']
    compiler = sdk / 'host/bin/clang'
    flags = ['--target=x86_64-unknown-none-elf', '--sysroot=' + str(sdk / 'sysroot'),
             '-nostdlibinc', '-isystem', str(sdk / 'sysroot/include'), '-D__SCHAR_WIDTH__=__CHAR_BIT__', '-D__LONG_LONG_WIDTH__=__LLONG_WIDTH__', '-D__mochios__=1', '-D__unix__=1', '-D_GNU_SOURCE=1', '-ffile-prefix-map=' + str(sdk) + '=/sdk', '-mno-red-zone', '-ftls-model=initial-exec', '-fPIC', '-funwind-tables']
    if cxx: flags[2:2] = ['--driver-mode=g++', '-nostdinc++', '-isystem', str(sdk / 'sysroot/include/c++/v1')]
    query = not args or args in [['-v'], ['--version'], ['--help'], ['-dumpmachine']] or any(arg.startswith('-print-') for arg in args)
    compile_only = query or any(arg in args for arg in ['-c', '-S', '-E', '-M', '-MM', '-fsyntax-only'])
    if compile_only:
        raise SystemExit(subprocess.call([str(compiler), *flags, *args], env=env))
    if '-pie' in args: raise ValueError('PIE executables are not part of this SDK ABI')
    shared = '-shared' in args
    output = Path('a.out'); filtered = []; index = 0
    while index < len(args):
        value = args[index]
        if value == '-o':
            index += 1; output = Path(args[index])
        elif value.startswith('-o') and len(value) > 2: output = Path(value[2:])
        else: filtered.append(value)
        index += 1
    if not output.parent.is_dir(): raise ValueError('Output directory does not exist')
    link = [*flags, '-B' + str(sdk / 'libexec'), '-nostdlib', '-nostartfiles',
            '-Wl,-z,noexecstack', '-Wl,-z,relro,-z,now', '-Wl,-z,max-page-size=4096', '-L' + str(sdk / 'lib'), '-L' + str(sdk / 'sysroot/lib')]
    if shared:
        link += [str(sdk / 'lib/shared-crt.o'), '-Wl,-T,' + str(sdk / 'lib/shared-unwind.ld')]
    else:
        link += ['-Wl,-no-pie', '-Wl,--image-base=0x400000', '-Wl,--no-dynamic-linker', '-Wl,--eh-frame-hdr', '-Wl,-T,' + str(sdk / 'lib/executable.ld'),
                 '-Wl,--export-dynamic', '-Wl,--undefined-version', '-Wl,--version-script=' + str(sdk / 'share/runtime.exports'), str(sdk / 'lib/crt0.o')]
        link += ['-Wl,--whole-archive', *[str(sdk / 'lib' / name) for name in ['libmochi_user_newlib_runtime.a','libc++.a','libc++abi.a','libunwind.a']], '-Wl,--no-whole-archive']
        link += json.loads((sdk / 'share/runtime-roots.json').read_text())
    if shared: filtered = shared_runtime_arguments(sdk, filtered)
    link += filtered
    if shared:
        # Compiler-generated arithmetic calls are implementation details, not
        # process runtime state. Extract only needed target compiler-rt objects
        # and keep their definitions private to this DSO.
        link += ['-x', 'none', '-Wl,--exclude-libs=libbuiltins.a', str(sdk / 'lib/libbuiltins.a')]
    else: link += ['-Wl,--start-group', '-lc', '-lm', str(sdk / 'lib/libbuiltins.a'), '-Wl,--end-group']
    # Runtime imports are intentionally bound by the executable's exported ABI.
    # The post-link check below rejects imports outside that ABI/dependency graph.
    if shared: link += ['-Wl,-z,undefs']
    with tempfile.TemporaryDirectory(dir=output.parent, prefix='.mochios-link-') as temporary:
        temporary = Path(temporary); target = temporary / 'linked.elf'
        response = temporary / 'link.rsp'
        response.write_text('\n'.join(json.dumps(arg, ensure_ascii=False) for arg in [*link, '-o', str(target)]))
        subprocess.run([str(compiler), '@' + str(response)], env=env, check=True)
        if shared:
            allowed = set(json.loads((sdk / 'share/runtime-symbols.json').read_text())) | dependencies(sdk, filtered)
            missing = symbols(sdk / 'host/bin/nm', target, False) - allowed
            if missing: raise ValueError('Unresolved non-runtime imports: ' + ', '.join(sorted(missing)))
        with target.open('rb') as stream: header = stream.read(64)
        if len(header) < 64 or header[:6] != b'\x7fELF\x02\x01' or int.from_bytes(header[18:20], 'little') != 62:
            raise ValueError('Linker did not produce an x86-64 ELF')
        dynamic = subprocess.check_output([str(sdk / 'host/bin/readelf'), '-d', str(target)], text=True)
        versions = subprocess.check_output([str(sdk / 'host/bin/readelf'), '--version-info', str(target)], text=True)
        if 'GLIBC_' in versions or 'libc.so.6' in dynamic or re.search(r'\((?:RUNPATH|RPATH)\)', dynamic): raise ValueError('Host glibc dependency is forbidden')
        if any('/' in value for value in re.findall(r'\(NEEDED\).*?\[([^]]+)\]', dynamic)):
            raise ValueError('Absolute DT_NEEDED path is forbidden; give the DSO a SONAME')
        target.replace(output)


if __name__ == '__main__':
    try: main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(f'mochios compiler: {error}', file=sys.stderr); raise SystemExit(1)
