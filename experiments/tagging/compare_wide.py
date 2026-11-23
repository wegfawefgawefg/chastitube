"""How much does a shrunk model change Laya's answers? reference.json vs shrunk.json from onnx-rs.
Reports score drift, flipped yes/no answers, changed choices, and how close to the line the flips were."""
import json, sys
ref = {r["id"]: r for r in json.load(open(sys.argv[1]))}
new = {r["id"]: r for r in json.load(open(sys.argv[2]))}
drift, flips, near, nouls = [], [], [], 0
ch_total = ch_changed = 0
changed = []
for i, r in new.items():
    a = ref[i]
    for q, p in r["probs"].items():
        pa = a["probs"][q]
        drift.extend(abs(x - y) for x, y in zip(p, pa))
        if q in r["raw"]:
            nouls += 1
            if (r["raw"][q] >= 0.5) != (a["raw"][q] >= 0.5):
                flips.append((q, a["raw"][q], r["raw"][q], r["title"][:50]))
        else:
            ch_total += 1
            if r["choices"][q] != a["choices"][q]:
                ch_changed += 1
                top2 = sorted(pa, reverse=True)[:2]
                changed.append((q, a["choices"][q], r["choices"][q], round(top2[0] - top2[1], 3), r["title"][:50]))
drift.sort()
n = len(drift)
pct = lambda f: drift[min(n - 1, int(n * f))]
print(f"{len(new)} videos, {n} probabilities compared")
print(f"drift: median {pct(.5):.4f}, 99% within {pct(.99):.4f}, 99.9% within {pct(.999):.4f}, max {drift[-1]:.4f}")
print(f"yes/no answers flipped at 0.5: {len(flips)} of {nouls}")
for f in flips: print(f"   {f[0]:14} {f[1]:.3f} -> {f[2]:.3f}  {f[3]}")
print(f"choices changed: {ch_changed} of {ch_total}")
for c in changed: print(f"   {c[0]:10} {c[1]} -> {c[2]} (reference top-2 gap {c[3]})  {c[4]}")
