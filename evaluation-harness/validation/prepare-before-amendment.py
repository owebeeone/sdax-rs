#!/usr/bin/env python3
"""Create a retained, independent protocol/context directory before candidate freeze."""
import argparse,json,re
from pathlib import Path
ROOT=Path(__file__).resolve().parent

def main():
    p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=ROOT.parent);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    source=a.source.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    text=(source/'docs/AI-Authoring.md').read_text()
    sections=re.split(r'(?=^## )',text,flags=re.M)
    reduced='\n\n'.join(s.rstrip() for s in sections if s.startswith('## Components\n') or s.startswith('## Services\n'))+'\n'
    assert sum(s.startswith('## Components\n') or s.startswith('## Services\n') for s in sections)==2
    (out/'compact.md').write_text(text);(out/'reduced.md').write_text(reduced)
    # Labels are included by runner.prompt_for and therefore count toward the caps.
    for name,cap in [('compact.md',16000),('reduced.md',8000)]:
        assert len(('FILE: '+name+'\n'+(out/name).read_text()).encode())<=cap,(name,'context budget exceeded')
    protocol=json.loads((ROOT/'protocol.json').read_text())
    for key in ['support_file','cargo_template_file','cargo_lock_file','ledger_file','run_root']:
        protocol[key]=str((ROOT/protocol[key]).resolve())
    protocol['sdax_rs']=str(source)
    protocol['context_files']=['compact.md'];protocol['reduced_context_files']=['reduced.md']
    protocol['snapshot_paths']=[str((ROOT/x).resolve()) for x in protocol['snapshot_paths'] if not x.startswith('../')]+[str(source/x) for x in ['Cargo.toml','Cargo.lock','crates/sdax','crates/sdax-tokio']]
    cases=json.loads((ROOT/'cases.json').read_text())
    for case in cases['cases']:
        for k in ['task','assertions','control']:case[k]=str((ROOT/case[k]).resolve())
    (out/'cases.json').write_text(json.dumps(cases,indent=2)+'\n')
    (out/'protocol.json').write_text(json.dumps(protocol,indent=2)+'\n')
    print(out/'protocol.json')
if __name__=='__main__':main()
