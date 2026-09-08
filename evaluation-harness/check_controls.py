#!/usr/bin/env python3
"""Retain known-correct and targeted mutated controls; no inference/network."""
import argparse, hashlib, json, os, subprocess, time
from pathlib import Path
import uuid
ROOT=Path(__file__).resolve().parent

def verify():
    manifest=json.loads((ROOT/'task-freeze.json').read_text())
    for name,digest in manifest['files'].items():
        assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest()==digest,name

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--negative',action='store_true');args=parser.parse_args()
    run=ROOT/'evidence'/('controls-'+uuid.uuid4().hex);run.mkdir(parents=True)
    cases=json.loads((ROOT/'cases.json').read_text())['cases'][:8]
    mutations=json.loads((ROOT/'negative-controls.json').read_text()) if args.negative else {}
    source_files=[ROOT.parent/"Cargo.toml",ROOT.parent/"Cargo.lock",ROOT/"Cargo.lock",*sorted((ROOT.parent/"crates/sdax").rglob("*.rs")),*sorted((ROOT.parent/"crates/sdax-tokio").rglob("*.rs")),ROOT.parent/"crates/sdax/Cargo.toml",ROOT.parent/"crates/sdax-tokio/Cargo.toml"]
    source_hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in source_files}
    (run/"source-manifest.json").write_text(json.dumps(source_hashes,indent=2)+"\n")
    def verify_source():
        for name,digest in source_hashes.items():assert hashlib.sha256(Path(name).read_bytes()).hexdigest()==digest,name
    results=[]
    for c in cases:
        variants=mutations.get(c['id'],[]) if args.negative else [{'id':'correct'}]
        for variant in variants:
            verify();verify_source();label=c['id']+'-'+variant['id'];work=run/label;(work/'src').mkdir(parents=True)
            code=(ROOT/c['control']).read_text()
            if args.negative:
                assert code.count(variant['old'])==1,(label,code.count(variant['old']))
                code=code.replace(variant['old'],variant['new'])
            (work/'src/candidate.rs').write_text(code)
            (work/'src/lib.rs').write_text(f'#[path="{ROOT}/support.rs"] mod support;\nmod candidate;\n#[cfg(test)] #[path="{ROOT/c["assertions"]}"] mod assertions;\n')
            (work/'Cargo.toml').write_text((ROOT/'Cargo.template.toml').read_text().replace('{{SDAX_RS}}',str(ROOT.parent)))
            (work/'Cargo.lock').write_bytes((ROOT/'Cargo.lock').read_bytes())
            immutable={p:hashlib.sha256(p.read_bytes()).hexdigest() for p in [work/'Cargo.toml',work/'Cargo.lock',work/'src/lib.rs',ROOT/'support.rs',ROOT/c['assertions']]}
            start=time.monotonic()
            result=subprocess.run(['cargo','test','--locked','--offline','--manifest-path',str(work/'Cargo.toml')],capture_output=True,timeout=120,env=dict(os.environ,CARGO_TARGET_DIR=str(ROOT/'evidence'/'target')))
            (work/'stdout.log').write_bytes(result.stdout);(work/'stderr.log').write_bytes(result.stderr)
            verify();verify_source()
            for p,d in immutable.items():assert hashlib.sha256(p.read_bytes()).hexdigest()==d,p
            stdout=result.stdout.decode();stderr=result.stderr.decode();compiled='could not compile' not in stderr and 'error[E' not in stderr
            row={'case':c['id'],'variant':variant['id'],'compiled':compiled,'exit_code':result.returncode,'seconds':time.monotonic()-start,'expected_rejection':args.negative,'as_expected':compiled and ((result.returncode!=0) if args.negative else result.returncode==0)}
            results.append(row);print(json.dumps(row),flush=True)
    (run/'results.json').write_text(json.dumps(results,indent=2)+'\n');print(run)
    return 0 if all(r['as_expected'] for r in results) else 1
if __name__=='__main__':raise SystemExit(main())
