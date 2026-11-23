"""Compare two results files score by score (e.g. Python vs Rust/ONNX, fp32 vs int8)."""
import json, sys
a = {r["id"]: r for r in json.load(open(sys.argv[1]))}
b = {r["id"]: r for r in json.load(open(sys.argv[2]))}
diffs, fmt = [], 0
for i, r in b.items():
    for t, v in r["yes"].items():
        diffs.append(abs(v - a[i]["yes"][t]))
    fmt += r["format"] == a[i]["format"]
diffs.sort()
print(f"{len(b)} videos, {len(diffs)} scores: biggest diff {diffs[-1]:.2f}, "
      f"99% within {diffs[int(len(diffs) * .99)]:.2f}, same kind-of-video {fmt}/{len(b)}")
