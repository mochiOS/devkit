#!/usr/bin/env python3
"""Build a relocatable, source-pinned mochiOS C/C++ SDK without an OS checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import urllib.request

import path_hygiene

ROOT = Path(__file__).resolve().parent


def digest(path):
    """Hash a source/archive incrementally."""
    with path.open('rb') as stream: return hashlib.file_digest(stream,'sha256').hexdigest()


def copy_stable(source,target):
    """Preserve timestamps for identical SDK inputs so incremental target builds remain useful."""
    source,target=Path(source),Path(target)
    if target.is_file() and digest(source)==digest(target):return str(target)
    return shutil.copy2(source,target)


def command(args, env=None, cwd=None, log=None):
    """Propagate real tool failures and preserve build diagnostics."""
    print('+ ' + ' '.join(map(str,args)),flush=True)
    if log:
        log.parent.mkdir(parents=True,exist_ok=True)
        with log.open('a') as stream: subprocess.run(list(map(str,args)),env=env,cwd=cwd,stdout=stream,stderr=subprocess.STDOUT,check=True)
    else: subprocess.run(list(map(str,args)),env=env,cwd=cwd,check=True)


def download(spec, path):
    """Fetch one fixed HTTPS input and require its locked SHA-256 before use."""
    path.parent.mkdir(parents=True,exist_ok=True)
    if not path.exists():
        temporary=path.with_suffix('.part')
        try:
            with urllib.request.urlopen(spec['url'],timeout=120) as source,temporary.open('wb') as output:
                size=0
                while chunk:=source.read(1024*1024):
                    size+=len(chunk)
                    if size>2*1024**3: raise ValueError('Input download exceeds 2 GiB')
                    output.write(chunk)
            if digest(temporary)!=spec['sha256']: raise ValueError('Downloaded input digest mismatch')
            temporary.replace(path)
        finally:
            if temporary.exists():temporary.unlink()
    if digest(path)!=spec['sha256']:raise ValueError('Cached input digest mismatch: '+str(path))
    return path


def fetch(lock, out):
    """Fetch pinned Git objects, host packages and the LLVM release; never pull a floating branch."""
    sources={}
    for name,spec in lock['sources'].items():
        path=out/'sources'/(name+'-'+spec['commit'])
        if not path.exists():
            command(['git','init','--initial-branch=source',path])
            command(['git','-C',path,'remote','add','origin',spec['url']])
            command(['git','-C',path,'-c','core.hooksPath=/dev/null','fetch','--depth=1','--no-tags','origin',spec['commit']])
            command(['git','-C',path,'-c','core.hooksPath=/dev/null','checkout','--detach',spec['commit']])
        actual=subprocess.check_output(['git','-C',path,'rev-parse','HEAD'],text=True).strip()
        changed=subprocess.check_output(['git','-C',path,'status','--porcelain'],text=True).strip()
        if actual!=spec['commit'] or changed:raise ValueError('Source cache is modified: '+str(path))
        sources[name]=path
    host=out/'host-inputs'
    for package in lock['host_packages']:
        archive=download(package,out/'downloads'/(package['name']+'.deb'))
        command(['dpkg-deb','-x',archive,host])
    archive=download(lock['llvm'],out/'downloads/llvm-project-18.1.8.src.tar.xz')
    llvm=out/'sources/llvm-project-18.1.8.src'
    if not llvm.exists():
        # Only projects needed for target runtimes/builtins and their CMake helpers.
        wanted={'runtimes','libcxx','libcxxabi','libunwind','compiler-rt','cmake'}
        with tarfile.open(archive) as tar:
            members=[]
            for member in tar:
                parts=Path(member.name).parts
                if not parts or parts[0]!='llvm-project-18.1.8.src' or '..' in parts:raise ValueError('Unsafe LLVM archive')
                if len(parts)==1 or parts[1] in wanted or parts[1:3]==('llvm','cmake') or parts[1:3]==('llvm','utils') or parts[1:3]==('llvm','include') or (len(parts)==2 and member.isfile()):members.append(member)
            tar.extractall(out/'sources',members=members,filter='data')
    sources['llvm']=llvm
    return sources,host


def host_tools(host_input, sdk):
    """Install LLVM tools and permissive dependencies; system glibc/GCC runtime remain host requirements."""
    binary=host_input/'usr/lib/llvm-18/bin'
    (sdk/'host/bin').mkdir(parents=True,exist_ok=True);(sdk/'host/lib').mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,LD_LIBRARY_PATH=str(host_input/'usr/lib/x86_64-linux-gnu')+':'+str(host_input/'usr/lib/llvm-18/lib'))
    notices=sdk/'licenses/host';notices.mkdir(parents=True,exist_ok=True)
    provenance=[]
    for name,actual in [('clang','clang'),('ld.lld','lld'),('ar','llvm-ar'),('ranlib','llvm-ranlib'),('nm','llvm-nm'),('readelf','llvm-readelf'),('objcopy','llvm-objcopy')]:
        source=(binary/actual).resolve()
        if not source.is_relative_to(host_input.resolve()):raise ValueError('Host package symlink escaped input tree')
        shutil.copy2(source,sdk/'host/bin'/name)
        deps=subprocess.check_output(['ldd',str(source)],env=env,text=True)
        for library,path in re.findall(r'^\s*(\S+) => (/\S+)',deps,re.M):
            if library in ['libc.so.6','libm.so.6','libdl.so.2','librt.so.1','libpthread.so.0','libresolv.so.2','libutil.so.1','libstdc++.so.6','libgcc_s.so.1']:continue
            target=sdk/'host/lib'/library
            if target.exists() and digest(target)!=digest(Path(path)):raise ValueError('Conflicting host dependency')
            if not Path(path).resolve().is_relative_to(host_input.resolve()):raise ValueError('Unpinned host dependency: '+library)
            if not target.exists():shutil.copy2(path,target)
            provenance.append(dict(name=library,sha256=digest(Path(path))))
            try:
                package=subprocess.check_output(['dpkg-query','-S',str(Path(path))],text=True,stderr=subprocess.DEVNULL).split(': ',1)[0].split(':')[0]
                copyright=Path('/usr/share/doc')/package/'copyright'
                if copyright.is_file():shutil.copy2(copyright,notices/(package+'.copyright'))
            except subprocess.CalledProcessError:pass
    resource=host_input/'usr/lib/llvm-18/lib/clang/18'
    if not (sdk/'host/lib/clang/18').exists():shutil.copytree(resource,sdk/'host/lib/clang/18')
    for copyright in (host_input/'usr/share/doc').glob('*/copyright'):
        shutil.copy2(copyright,notices/(copyright.parent.name+'.copyright'))
    (sdk/'share').mkdir(exist_ok=True)
    (sdk/'share/host-dependencies.json').write_text(json.dumps(provenance,indent=2)+'\n')
    return env


def bootstrap(sdk, out, policy):
    """Create private GCC-compatible command names for the canonical userland SDK source builder."""
    (sdk/'libexec').mkdir(exist_ok=True)
    shutil.copy2(ROOT/'link-adapter.py',sdk/'libexec/gcc');(sdk/'libexec/gcc').chmod(0o755)
    (sdk/'libexec/g++').symlink_to('gcc') if not (sdk/'libexec/g++').exists() else None
    directory=out/'bootstrap-bin';directory.mkdir(exist_ok=True)
    script='''#!/usr/bin/env python3
import os,sys,subprocess
from pathlib import Path
sdk=Path(%s); builtins=Path(%s); mappings=%s
if sys.argv[1:]==['-print-libgcc-file-name']:
    if not builtins.is_file():raise SystemExit('compiler-rt builtins have not been built')
    print(builtins);raise SystemExit(0)
env=dict(os.environ,LD_LIBRARY_PATH=str(sdk/'host/lib'))
args=['--target=x86_64-unknown-none-elf','-nostdlibinc','-ffreestanding','-mno-red-zone','-D__SCHAR_WIDTH__=__CHAR_BIT__','-D__LONG_LONG_WIDTH__=__LLONG_WIDTH__','-B'+str(sdk/'libexec')]
if '++' in Path(sys.argv[0]).name:args+=['--driver-mode=g++']
raise SystemExit(subprocess.call([str(sdk/'host/bin/clang'),*args,*sys.argv[1:],*mappings],env=env))
'''%(repr(str(sdk)),repr(str(out/'builtins/lib/linux/libclang_rt.builtins-x86_64.a')),repr(path_hygiene.compiler_flags(policy)))
    for name in ['x86_64-elf-gcc','bootstrap-cc','bootstrap-c++']:
        (directory/name).write_text(script);(directory/name).chmod(0o755)
    for name,tool in [('x86_64-elf-ar','ar'),('x86_64-elf-ranlib','ranlib')]:
        path=directory/name
        if not path.exists():path.symlink_to(sdk/'host/bin'/tool)
    return directory


def build_builtins(sources, sdk, out, env, jobs, policy):
    """Build target compiler-rt from source instead of copying a Linux/GCC target runtime."""
    build=out/'builtins'
    mappings=shlex.join(path_hygiene.compiler_flags(policy))
    headers=out/'bootstrap-headers';headers.mkdir(exist_ok=True)
    # Private freestanding assert for compiler-rt CPU detection, never installed as a libc header.
    (headers/'assert.h').write_text('#define static_assert _Static_assert\n#ifdef NDEBUG\n#define assert(x) ((void)0)\n#else\n#define assert(x) ((x)?(void)0:__builtin_trap())\n#endif\n')
    command(['cmake','-S',sources['llvm']/'compiler-rt/lib/builtins','-B',build,'-G','Unix Makefiles',
        '-DCMAKE_SYSTEM_NAME=Generic','-DCMAKE_SYSTEM_PROCESSOR=x86_64','-DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY',
        '-DCMAKE_C_COMPILER='+str(sdk/'host/bin/clang'),'-DCMAKE_ASM_COMPILER='+str(sdk/'host/bin/clang'),
        '-DCMAKE_C_COMPILER_TARGET=x86_64-unknown-none-elf','-DCMAKE_ASM_COMPILER_TARGET=x86_64-unknown-none-elf',
        '-DCMAKE_C_FLAGS=-ffreestanding -fPIC -nostdlibinc -isystem'+shlex.quote(str(headers))+' '+mappings, '-DCMAKE_ASM_FLAGS='+mappings,'-DCMAKE_BUILD_TYPE=Release',
        '-DCOMPILER_RT_BAREMETAL_BUILD=ON','-DCOMPILER_RT_DEFAULT_TARGET_ONLY=ON',
        '-DCOMPILER_RT_EXCLUDE_ATOMIC_BUILTIN=ON','-DLLVM_CMAKE_DIR='+str(sources['llvm']/'llvm/cmake/modules')],env,log=out/'logs/builtins.log')
    command(['cmake','--build',build,'--parallel',jobs],env,log=out/'logs/builtins.log')
    candidates=[p for p in build.rglob('libclang_rt.builtins-x86_64.a') if p.parent != build/'lib/linux']
    if len(candidates)!=1:raise ValueError('Expected one target builtins archive')
    stable=build/'lib/linux/libclang_rt.builtins-x86_64.a';stable.parent.mkdir(parents=True,exist_ok=True)
    if candidates[0]!=stable:shutil.copy2(candidates[0],stable)
    return stable


def prepared_sources(lock, sources, out):
    """Patch disposable copies, keeping fetched Git inputs pristine and reviewable."""
    prepared={}
    for name in ['user','newlib','llvm']:
        target=out/'prepared'/name
        if not target.exists():
            shutil.copytree(sources[name],target,ignore=shutil.ignore_patterns('.git','target'))
        prepared[name]=target
    # This recipe owns the disposable builder script; refresh it when patch versions change.
    copy_stable(sources['user']/'scripts/build-newlib.sh',prepared['user']/'scripts/build-newlib.sh')
    for name,expected in lock['patches'].items():
        patch=ROOT/'patches'/name
        if digest(patch)!=expected:raise ValueError('Patch digest mismatch: '+name)
        target=prepared['user' if name.startswith('syscalls') else 'newlib' if name.startswith('newlib') else 'llvm']
        check=['patch','--batch','--dry-run','-p1','-d',str(target),'-i',str(patch)]
        if subprocess.run([*check,'--forward'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0:
            command(['patch','--batch','--forward','-p1','-d',target,'-i',patch])
        elif subprocess.run([*check,'--reverse'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode:
            raise ValueError('Prepared source does not match locked patch: '+name)
    if digest(ROOT/'runtime.Cargo.lock')!=lock['runtime_cargo_lock_sha256']:raise ValueError('Runtime Cargo lock mismatch')
    shutil.copy2(ROOT/'runtime.Cargo.lock',prepared['user']/'Cargo.lock')
    return dict(sources,**prepared)


def build_c_runtime(lock,sources,sdk,out,env,jobs,policy):
    """Build newlib and the real syscall-backed Rust process runtime from pinned sources."""
    directory=bootstrap(sdk,out,policy)
    env=dict(env,PATH=str(directory)+':'+env['PATH'],CARGO_HOME=str(out/'cargo-home'))
    env.update(RUSTC_WRAPPER=str(ROOT/'path_hygiene.py'),
        MOCHIOS_SDK_PATH_POLICY=str(out/'path-policy.json'),
        MOCHIOS_SDK_CFLAGS=shlex.join(path_hygiene.compiler_flags(policy)),
        MOCHIOS_SDK_CARGO_TARGET_DIR=str(out/'rust-targets'/path_hygiene.identity(policy)))
    command(['bash',sources['user']/'scripts/build-newlib.sh','--newlib-source',sources['newlib'],
        '--output',out/'c-runtime','--abi-source',sources['mnu']/'crates/abi',
        '--libc-source',sources['libc'],'--mboot-protocol-source',sources['mboot']/'mboot-protocol',
        '--syscalls-source',sources['user'],'--toolchain',lock['rust_toolchain'],'--jobs',jobs],
        env,log=out/'logs/c-runtime.log')
    base=out/'c-runtime/sdk'
    for name in ['lib','sysroot']:
        shutil.copytree(base/name,sdk/name,dirs_exist_ok=True,copy_function=copy_stable)
    (sdk/'lib/libgcc.a').unlink(missing_ok=True)
    shutil.copy2(out/'builtins/lib/linux/libclang_rt.builtins-x86_64.a',sdk/'lib/libbuiltins.a')
    isolate_math_builtins(sdk,out,env)
    return env


def isolate_math_builtins(sdk,out,env):
    """Keep Rust's hidden weak math intrinsics private so real newlib math can supply the public ABI."""
    names=['cbrt','ceil','copysign','fabs','fdim','floor','fma','fmax','fmin','fmod','rint','round','sqrt','trunc']
    names=sorted(names+[name+'f' for name in names])
    mapping=sdk/'share/runtime-intrinsics.rename'
    mapping.write_text(''.join(name+' __mochios_builtin_'+name+'\n' for name in names))
    command([sdk/'host/bin/objcopy','--redefine-syms='+str(mapping),sdk/'lib/libmochi_user_newlib_runtime.a'],env)


def build_cxx_runtime(sources,sdk,out,env,jobs,policy):
    """Build target libc++, libc++abi and libunwind; bootstrap configure never executes target code."""
    build=out/'cxx-runtime'
    flags='-nostdlibinc -D__SCHAR_WIDTH__=__CHAR_BIT__ -D__LONG_LONG_WIDTH__=__LLONG_WIDTH__ -D__mochios__=1 -D_GNU_SOURCE=1 -mno-red-zone -fno-pie -isystem'+shlex.quote(str(sdk/'sysroot/include'))+' '+shlex.join(path_hygiene.compiler_flags(policy))
    options={
        'CMAKE_SYSTEM_NAME':'Generic','CMAKE_SYSTEM_PROCESSOR':'x86_64','CMAKE_TRY_COMPILE_TARGET_TYPE':'STATIC_LIBRARY',
        'CMAKE_C_COMPILER':sdk/'host/bin/clang','CMAKE_CXX_COMPILER':sdk/'host/bin/clang', 'CMAKE_ASM_COMPILER':sdk/'host/bin/clang',
        'CMAKE_C_COMPILER_TARGET':'x86_64-unknown-none-elf','CMAKE_CXX_COMPILER_TARGET':'x86_64-unknown-none-elf','CMAKE_ASM_COMPILER_TARGET':'x86_64-unknown-none-elf',
        'CMAKE_SYSROOT':sdk/'sysroot','CMAKE_FIND_ROOT_PATH':sdk/'sysroot',
        'CMAKE_FIND_ROOT_PATH_MODE_LIBRARY':'ONLY','CMAKE_FIND_ROOT_PATH_MODE_INCLUDE':'ONLY','CMAKE_FIND_ROOT_PATH_MODE_PACKAGE':'ONLY','CMAKE_FIND_ROOT_PATH_MODE_PROGRAM':'NEVER',
        'CMAKE_C_FLAGS':flags,'CMAKE_ASM_FLAGS':shlex.join(path_hygiene.compiler_flags(policy)),'CMAKE_CXX_FLAGS':flags+' --driver-mode=g++','CMAKE_BUILD_TYPE':'Release','CMAKE_INSTALL_PREFIX':build/'install',
        'LLVM_ENABLE_RUNTIMES':'libcxx;libcxxabi;libunwind','LIBCXX_ENABLE_SHARED':'OFF','LIBCXXABI_ENABLE_SHARED':'OFF','LIBUNWIND_ENABLE_SHARED':'OFF',
        'LIBCXX_HAS_PTHREAD_API':'ON','LIBCXXABI_HAS_PTHREAD_API':'ON','LIBCXX_ENABLE_FILESYSTEM':'ON','LIBCXX_ENABLE_LOCALIZATION':'ON',
        'LIBCXX_INCLUDE_TESTS':'OFF','LIBCXX_INCLUDE_BENCHMARKS':'OFF','LIBCXXABI_INCLUDE_TESTS':'OFF','LIBUNWIND_INCLUDE_TESTS':'OFF','LIBUNWIND_IS_BAREMETAL':'ON',
        'LIBCXX_HAS_PTHREAD_LIB':'OFF','LIBCXX_HAS_RT_LIB':'OFF','LIBCXX_HAS_ATOMIC_LIB':'OFF','LIBCXXABI_HAS_PTHREAD_LIB':'OFF','LIBCXXABI_HAS_DL_LIB':'OFF','LIBCXXABI_HAS_CXA_THREAD_ATEXIT_IMPL':'OFF','LIBUNWIND_HAS_PTHREAD_LIB':'OFF','LIBUNWIND_HAS_DL_LIB':'OFF'}
    command(['cmake','-S',sources['llvm']/'runtimes','-B',build,'-G','Unix Makefiles',*[f'-D{k}={v}' for k,v in options.items()]],env,log=out/'logs/cxx-runtime.log')
    command(['cmake','--build',build,'--parallel',jobs],env,log=out/'logs/cxx-runtime.log')
    for name in ['libc++.a','libc++abi.a','libunwind.a']:
        candidates=[p for p in (build/'lib').rglob(name) if p.is_file()]
        if len(candidates)!=1:raise ValueError('Expected one target runtime archive: '+name)
        shutil.copy2(candidates[0],sdk/'lib'/name)
    shutil.copytree(build/'include/c++/v1',sdk/'sysroot/include/c++/v1',dirs_exist_ok=True,symlinks=False)


UNSUPPORTED_C_API={'rpmatch','cacosl','cacoshl','casinl','casinhl','catanhl'}

def assemble(lock,sources,sdk,out,env,policy):
    """Install relocatable drivers and measure the exported ABI of a real linked process image."""
    (sdk/'bin').mkdir(exist_ok=True)
    shutil.copy2(ROOT/'driver.py',sdk/'bin/driver.py');(sdk/'bin/driver.py').chmod(0o755)
    shutil.copy2(ROOT/'link-adapter.py',sdk/'libexec/gcc');(sdk/'libexec/gcc').chmod(0o755)
    for name in ['mochios-cc','mochios-c++']:
        p=sdk/'bin'/name;p.unlink(missing_ok=True);p.symlink_to('driver.py')
    for name in ['executable.ld','shared-unwind.ld']:shutil.copy2(ROOT/name,sdk/'lib'/name)
    shutil.copytree(ROOT/'cmake',sdk/'share/cmake',dirs_exist_ok=True)
    command([sdk/'bin/mochios-cc',*path_hygiene.compiler_flags(policy),'-c',ROOT/'shared-crt.c','-o',sdk/'lib/shared-crt.o'],env)
    all_symbols=set();c_symbols=set()
    archives=[sdk/'lib'/n for n in ['libmochi_user_newlib_runtime.a','libc++.a','libc++abi.a','libunwind.a','libbuiltins.a']]+[sdk/'sysroot/lib/libc.a',sdk/'sysroot/lib/libm.a']
    for path in archives:
        text=subprocess.check_output([sdk/'host/bin/nm','-g','--defined-only',path],stderr=subprocess.DEVNULL,env=env,text=True)
        for line in text.splitlines():
            parts=line.split()
            if len(parts)!=3 or parts[1].upper() not in ['T','D','B','R','W','V']:continue
            symbol=parts[2]
            if not re.fullmatch('[A-Za-z_][A-Za-z0-9_.$]*',symbol):continue
            if symbol.startswith(('_R','DW.','__mochios_builtin_')) or re.search(r'17h[0-9a-f]{16}E$',symbol):continue
            all_symbols.add(symbol)
            if path.name in ['libc.a','libm.a'] and re.fullmatch('[A-Za-z][A-Za-z0-9_]*',symbol):c_symbols.add(symbol)
    (sdk/'share/runtime.exports').write_text('{\nglobal:\n'+''.join(s+';\n' for s in sorted(all_symbols-UNSUPPORTED_C_API))+'local: *;\n};\n')
    (sdk/'share/runtime-roots.json').write_text(json.dumps(['-Wl,--undefined='+s for s in sorted(c_symbols-UNSUPPORTED_C_API)]))
    (out/'anchor.c').write_text('int main(void) { return 0; }\n')
    command([sdk/'bin/mochios-cc',out/'anchor.c','-o',out/'anchor.elf'],env)
    text=subprocess.check_output([sdk/'host/bin/nm','-D','--defined-only','--format=posix',out/'anchor.elf'],env=env,text=True)
    exports=sorted({line.split()[0] for line in text.splitlines() if len(line.split())>=2})
    missing=c_symbols-UNSUPPORTED_C_API-set(exports)
    if missing:raise ValueError('Public C runtime functions missing from dynamic ABI: '+', '.join(sorted(missing)))
    if len(exports)<1000:raise ValueError('Executable runtime ABI export table is incomplete')
    (sdk/'share/runtime-symbols.json').write_text(json.dumps(exports)+'\n')
    metadata=dict(format=1,version=lock['version'],target=lock['target'],abi=lock['abi'],host=lock['host'],
        features=['pic','shared','cxx23','threads','exceptions'],sysroot='sysroot',cmake_toolchain='share/cmake/mochios.cmake',
        runtime_model='executable-owned-newlib-v1',tls_model='initial-exec',shared_loading='startup-linked-only',runtime_tested=False,
        verification='compile-and-link-only',unsupported_c_api=sorted(UNSUPPORTED_C_API))
    (sdk/'sdk.json').write_text(json.dumps(metadata,indent=2)+'\n')
    shutil.copy2(ROOT/'sources.lock.json',sdk/'share/sources.lock.json')
    shutil.copy2(sources['user']/'Cargo.lock',sdk/'share/runtime.Cargo.lock')
    return metadata


def licenses(sources,sdk,out):
    """Bundle original license notices and exact corresponding source inputs, without inventing legal text."""
    directory=sdk/'licenses/target';directory.mkdir(parents=True,exist_ok=True)
    for project in ['user','mnu','libc','newlib','llvm','mboot']:
        for source in sources[project].iterdir():
            if source.is_file() and (source.name.upper().startswith(('LICENSE','COPYING','NOTICE'))):
                shutil.copy2(source,directory/(project+'-'+source.name))
    for project in ['libcxx','libcxxabi','libunwind','compiler-rt']:
        source=sources['llvm']/project/'LICENSE.TXT'
        if source.is_file():shutil.copy2(source,directory/(project+'-LICENSE.TXT'))
    rustsrc=Path(subprocess.check_output(['rustc','+'+json.loads((ROOT/'sources.lock.json').read_text())['rust_toolchain'],'--print','sysroot'],text=True).strip())/'share/doc/rust'
    if (rustsrc/'licenses').is_dir():shutil.copytree(rustsrc/'licenses',directory/'licenses',dirs_exist_ok=True)
    for source in rustsrc.glob('*'):
        if source.is_file() and source.name.upper().startswith(('LICENSE','COPYRIGHT')):shutil.copy2(source,directory/('rust-'+source.name))
    for source in (out/'cargo-home/registry/src').glob('*/*'):
        for notice in source.iterdir():
            if notice.is_file() and notice.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
                shutil.copy2(notice,directory/(source.name+'-'+notice.name))
    # Newlib's patched corresponding source is included to satisfy LGPL components' source requirements.
    path_hygiene.write_archive(sources['newlib'],sdk/'licenses/newlib-corresponding-source.tar.gz','newlib',exclude_git=True)
    shutil.copytree(ROOT/'patches',sdk/'licenses/patches',dirs_exist_ok=True)


def verify(sdk,out,env,jobs):
    """Run actual target executable and shared-library links; never execute mochiOS binaries on Linux."""
    command(['cmake','-S',ROOT/'probe','-B',out/'verification','-G','Unix Makefiles','-DCMAKE_TOOLCHAIN_FILE='+str(sdk/'share/cmake/mochios.cmake')],env)
    command(['cmake','--build',out/'verification','--parallel',jobs],env)
    command([sys.executable,ROOT/'verify.py',sdk,out/'verification'],env)


def package(lock,sdk,out,policy):
    """Publish local archives only after verification, with inventory and a SHA-256 completion marker."""
    audit=path_hygiene.audit_tree(sdk,policy)
    (out/'logs').mkdir(parents=True,exist_ok=True)
    (out/'logs/path-audit.json').write_text(json.dumps(audit,indent=2)+'\n')
    release=out/'releases';release.mkdir(exist_ok=True)
    name='x86_64-mochios-sdk-'+lock['version']+'.tar.gz';archive=release/name
    temporary=archive.with_suffix('.tmp')
    try:
        path_hygiene.write_archive(sdk,temporary,'sdk')
        with tarfile.open(temporary) as tar:path_hygiene.audit_tar(tar,policy,name)
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    inventory={str(p.relative_to(sdk)):({'symlink':os.readlink(p)} if p.is_symlink() else {'sha256':digest(p),'size':p.stat().st_size}) for p in sorted(sdk.rglob('*')) if p.is_file() or p.is_symlink()}
    manifest=dict(format=1,path_audit=audit,sdk=json.loads((sdk/'sdk.json').read_text()),sources=lock,files=inventory,archive=name,sha256=digest(archive))
    (release/(name+'.json')).write_text(json.dumps(manifest,indent=2)+'\n')
    (release/'SHA256SUMS').write_text(digest(archive)+'  '+name+'\n')
    print('Verified SDK archive: '+str(archive),flush=True)


def main():
    """Fetch/build from this SDK recipe with all outputs confined to a selected work directory."""
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['fetch','builtins','build'])
    parser.add_argument('--output',type=Path,default=ROOT.parent/'out/shared-sdk')
    parser.add_argument('--jobs',type=int,default=4)
    args=parser.parse_args();out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
    lock=json.loads((ROOT/'sources.lock.json').read_text())
    sources,host=fetch(lock,out)
    if args.command=='fetch':return
    sdk=out/'sdk';sdk.mkdir(exist_ok=True)
    host_tools(host,sdk)
    rust_sysroot=Path(subprocess.check_output(['rustc','+'+lock['rust_toolchain'],'--print','sysroot'],text=True).strip())
    policy=path_hygiene.make_policy(ROOT.parent,out,sources,rust_sysroot)
    (out/'path-policy.json').write_text(json.dumps(policy,indent=2)+'\n')
    env=dict(os.environ,LD_LIBRARY_PATH=str(sdk/'host/lib'))
    build_builtins(sources,sdk,out,env,str(args.jobs),policy)
    if args.command=='builtins':return
    sources=prepared_sources(lock,sources,out)
    env=build_c_runtime(lock,sources,sdk,out,env,str(args.jobs),policy)
    build_cxx_runtime(sources,sdk,out,env,str(args.jobs),policy)
    assemble(lock,sources,sdk,out,env,policy)
    licenses(sources,sdk,out)
    verify(sdk,out,env,str(args.jobs))
    package(lock,sdk,out,policy)


if __name__=='__main__':
    try:main()
    except (ValueError,OSError,KeyError,subprocess.CalledProcessError,tarfile.TarError) as error:
        print('SDK build failed: '+str(error),file=sys.stderr);raise SystemExit(1)
