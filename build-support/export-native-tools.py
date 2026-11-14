#!/usr/bin/env python3
"""Export native compiler/editor files through the producer's declared image areas."""
import importlib.util
import os
from pathlib import Path
import shutil

root=Path(os.environ['MDP_CONTEXT']).resolve()
if 'USER_INSTALL_DEVELOPER_TOOLS=y' not in (root/'.config').read_text().splitlines():
    raise SystemExit(0)
kit=Path(__file__).resolve().parent
out=kit.parent/'out/mdp/native'
if out.exists():shutil.rmtree(out)
out.mkdir(parents=True)
spec=importlib.util.spec_from_file_location('native_staging',kit/'stage-development-tools.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
module.stage_tools(out,root/'out/newlib-port/sdk',root/'out/llvm-port/build-target',root/'out/rust-std/target/x86_64-unknown-mochios/release')
manifest=out/'manifests';manifest.mkdir()
shutil.copyfile(out/'system/packages/development-tools/manifest.toml',manifest/'development-tools.toml')
shutil.rmtree(out/'system/packages')
vim=root/'out/vim-port/release/stage/system'
shutil.copytree(vim,out/'system',dirs_exist_ok=True,symlinks=True)
shutil.copyfile(kit/'ports/vim/manifest.toml',manifest/'vim.toml')
