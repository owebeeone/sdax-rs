import csv
from pathlib import Path

root = Path(__file__).resolve().parent
paths = {
    "forward_baseline": root / "forward" / "baseline.csv",
    "forward_candidate": root / "forward" / "candidate.csv",
    "reverse_candidate": root / "reverse" / "candidate.csv",
    "reverse_baseline": root / "reverse" / "baseline.csv",
}
rows = {name: list(csv.DictReader(path.open(newline=""))) for name, path in paths.items()}
fields = ["phase", "workload", "nodes", "edges", "sample"]
def key(row):
    return tuple(row[field] for field in fields)
def metric(row):
    return tuple(row[field] for field in ["allocations", "allocated_bytes", "peak_live_bytes", "retained_bytes", "checksum"])
for name, data in rows.items():
    assert len(data) == 82, (name, len(data))
    assert len({key(row) for row in data}) == len(data)
assert [key(r) for r in rows["forward_baseline"]] == [key(r) for r in rows["reverse_baseline"]]
assert [key(r) for r in rows["forward_candidate"]] == [key(r) for r in rows["reverse_candidate"]]
assert [metric(r) for r in rows["forward_baseline"]] == [metric(r) for r in rows["reverse_baseline"]]
assert [metric(r) for r in rows["forward_candidate"]] == [metric(r) for r in rows["reverse_candidate"]]
base = {key(r): r for r in rows["forward_baseline"]}
cand = {key(r): r for r in rows["forward_candidate"]}
assert base.keys() == cand.keys()
print("rows_each=82")
print("within_source_reverse_order=exact")
print("dynamic:")
for k in sorted(base):
    if "dynamic_live_instances" in k[1]:
        print(",".join(k), "baseline="+"/".join(metric(base[k])), "candidate="+"/".join(metric(cand[k])))
diffs = []
for k in sorted(base):
    if metric(base[k]) != metric(cand[k]):
        diffs.append((k, metric(base[k]), metric(cand[k])))
print("all_metric_diff_rows="+str(len(diffs)))
for k, bm, cm in diffs:
    print(",".join(k), "baseline="+"/".join(bm), "candidate="+"/".join(cm))
