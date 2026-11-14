#!/usr/bin/env python3
"""Describe an actual built SDK for independently checked-out consumers."""
import hashlib
import json
import os
from pathlib import Path
import sys

root=Path(os.environ['MDP_CONTEXT']).resolve()
sys.path.insert(0,str(root/'tools/mdp/src'))
from mdp.cli import atomic_json,encoded
from mdp.workspace import source_state
receipts=json.loads((root/'.mdp/builds.json').read_text())
__import__('subprocess').run(['python3',str(Path(__file__).resolve().parent/'materialize-sdk.py'),'shared'],check=True)
ids=['runtime','fonts','viewkit','devkit-host','shared-sdk','system-components']
artifacts=[]
for name in ids:
    value=receipts[name]
    artifacts.append(dict(path=str(root/value['artifact']),sha256=value['sha256'],name=name,package=value['package']))
identity=hashlib.sha256(encoded([{k:v for k,v in item.items() if k!='path'} for item in artifacts])).hexdigest()
sources=[]
for path in ['core','boot','user','services','libraries/newlib','libraries/rust','libraries/fonts','libraries/libc','libraries/bootui','libraries/viewkit','tools/devkit']:
    state=source_state(root/path)
    sources.append(dict(path=str(root/path),revision=state['revision'],working_tree=state['working_tree']))
environment={key:str(root/value) for key,value in {'MDP_CONTEXT':'.','MDP_WORKSPACE':'.','MDP_VARIABLES':'out/mdp/context.json','MDP_PROFILE_FILE':'.config','MDP_VERSION_FILE':'version.toml'}.items()}
bindings=json.loads((root/'out/mdp/context.json').read_text())
for kind in ['native','shared']:bindings.update(json.loads((root/('out/mdp/'+kind+'-bindings.json')).read_text()))
frozen=root/'out/sdk/contexts'/identity
frozen.mkdir(parents=True,exist_ok=True)
atomic_json(frozen/'bindings.json',bindings)
environment['MDP_VARIABLES']=str(frozen/'bindings.json')
environment['MDP_VARIABLE_OVERLAYS']='[]'
value={'format':1,'sdk':dict(id=identity,target='x86_64-mochios',abi='mochios-1',kind='built-source-sdk'),
       'artifacts':artifacts,'sources':sources,'environment':environment}
atomic_json(root/'out/mdp/developer-context.json',value)
