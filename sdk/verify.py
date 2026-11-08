#!/usr/bin/env python3
"""Verify linked target ELF structure and reject host contamination without executing target code."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def inspect(sdk,path):
    """Require x86-64, W^X, non-executable stack and capability-loader-compatible dependencies."""
    text=subprocess.check_output([sdk/'host/bin/readelf','-W','-h','-l','-d','--version-info',path],text=True)
    if not re.search(r'Machine:\s+(Advanced Micro Devices X86-64|AMD x86-64)',text):raise ValueError('Unexpected ELF machine')
    if re.search(r'\bINTERP\b|GLIBC_|libc.so.6|\(RPATH\)|\(RUNPATH\)',text):raise ValueError('Host loader or runtime dependency')
    for line in text.splitlines():
        if line.lstrip().startswith(('LOAD','GNU_STACK')) and 'W' in line and 'E' in line:raise ValueError('Writable executable mapping')
        if line.lstrip().startswith('LOAD') and re.search(r'Type:\s+EXEC',text):
            if int(line.split()[2],16)<0x400000:raise ValueError('Executable below mochiOS user address floor')
    if 'GNU_RELRO' not in text:raise ValueError('Missing RELRO')
    if any('/' in name for name in re.findall(r'\(NEEDED\).*?\[([^]]+)\]',text)):raise ValueError('Absolute dependency name')
    return text


def main():
    """Validate real outputs, a failed transactional link, dependency imports and a relocated SDK path."""
    sdk=Path(sys.argv[1]).resolve();build=Path(sys.argv[2]).resolve()
    os.environ['LD_LIBRARY_PATH']=str(sdk/'host/lib')
    objects=[p for p in build.rglob('*') if p.is_file() and not p.is_symlink() and (p.name in {'sdk_probe', 'sdk_resolver_probe', 'sdk_loader_probe', 'sdk_stack_probe', 'sdk_memory_probe'} or p.name.startswith(('libsdk_probe_shared.so', 'libsdk_resolver_probe_shared.so', 'libsdk_loader_probe', 'libsdk_stack_probe_shared.so', 'libsdk_memory_probe_shared.so')))]
    if len(objects)<2:raise ValueError('Missing C++ executable/shared probe outputs')
    resolver_available=(sdk/'sysroot/include/resolv.h').is_file()
    if resolver_available:
        if not any(p.name=='sdk_resolver_probe' for p in objects) or not any(p.name.startswith('libsdk_resolver_probe_shared.so') for p in objects):
            raise ValueError('Resolver headers require executable/shared ABI probes')
    loader_available=(sdk/'sysroot/include/dlfcn.h').is_file()
    if loader_available:
        names={p.name for p in objects}
        if not {'sdk_loader_probe','libsdk_loader_probe_shared.so','libsdk_loader_probe_private.so'}<=names:
            raise ValueError('Loader headers require executable/shared/private DSO ABI probes')
        exports=set(json.loads((sdk/'share/runtime-symbols.json').read_text()))
        if not {'dlopen','dlsym','dlclose','dlerror'}<=exports:
            raise ValueError('Advertised private loader functions missing from executable ABI')
    names={p.name for p in objects}
    if not {'sdk_stack_probe','libsdk_stack_probe_shared.so'} <= names:
        raise ValueError('Missing actual pthread stack executable/shared ABI probes')
    exports=set(json.loads((sdk/'share/runtime-symbols.json').read_text()))
    if 'pthread_getattr_np' not in exports:
        raise ValueError('Observed pthread stack API missing from executable ABI')
    if not {'sdk_memory_probe', 'libsdk_memory_probe_shared.so'} <= names:
        raise ValueError('Missing current resident memory executable/shared ABI probes')
    if 'mochios_process_memory_read' not in exports:
        raise ValueError('Current resident memory API missing from executable ABI')
    for path in objects:inspect(sdk,path)
    with tempfile.TemporaryDirectory(prefix='sdk checks ',dir=build) as tmp:
        tmp=Path(tmp);source=tmp/'source.c';output=tmp/'libcheck.so'
        source.write_text('extern int missing_sdk_symbol_239(void); int check(void){return missing_sdk_symbol_239();}\n')
        output.write_bytes(b'previous-output')
        result=subprocess.run([sdk/'bin/mochios-cc','-shared',source,'-Wl,-soname,libcheck.so','-o',output],capture_output=True,text=True)
        if result.returncode==0 or 'Unresolved non-runtime imports' not in result.stderr:raise ValueError('Unknown strong import was not rejected')
        if output.read_bytes()!=b'previous-output':raise ValueError('Failed link replaced prior output')
        source.write_text('#include <stdio.h>\n#include <math.h>\ndouble check_math(double x){return floor(x); }\nint check(void){return puts("probe");}\n')
        subprocess.run([sdk/'bin/mochios-cc','-shared',source,'-fno-builtin-floor','-lm','-Wl,-soname,libcheck.so','-o',output],check=True)
        inspect(sdk,output)
        source.write_text('extern int check(void); int main(void){return check();}\n')
        subprocess.run([sdk/'bin/mochios-cc',source,'-L'+str(tmp),'-lcheck','-o',tmp/'check'],check=True)
        inspect(sdk,tmp/'check')
        # A real relocated copy, with no symlink back to the build SDK.
        alias=tmp/'relocated SDK';shutil.copytree(sdk,alias,symlinks=True)
        relocated_env=dict(os.environ,LD_LIBRARY_PATH=str(alias/'host/lib'))
        subprocess.run([alias/'bin/mochios-cc','-c',source,'-o',tmp/'space name.o'],env=relocated_env,check=True)
        subprocess.run(['cmake','-S',Path(__file__).resolve().parent/'probe','-B',tmp/'relocated build','-DCMAKE_TOOLCHAIN_FILE='+str(alias/'share/cmake/mochios.cmake')],env=relocated_env,check=True)
        subprocess.run(['cmake','--build',tmp/'relocated build','--parallel','2'],env=relocated_env,check=True)
    report={'format':1,'compile_link_passed':True,'runtime_tested':False,'checks':['cxx23','thread-and-exception-symbols','elf-wx-relro','no-host-runtime','unknown-import-rejection','atomic-link-output','runtime-imports','declared-dso-imports','paths-with-spaces','pthread-stack-c-abi-compile-link','current-resident-memory-c-abi-compile-link']}
    if resolver_available:report['checks'].append('native-resolver-c-abi')
    if loader_available:report['checks'].append('private-loader-c-abi-compile-link')
    (build/'verification.json').write_text(json.dumps(report,indent=2)+'\n')
    print('SDK compile/link/ELF checks passed; mochiOS runtime execution not performed.')


if __name__=='__main__':main()
