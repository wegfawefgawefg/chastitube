"""Pick the tag cutoff: which topics count as tagged, given each topic's median on ordinary videos."""
import json, statistics, sys
from score import V, truth
def load(*paths):
    out = {}
    for p in paths:
        for r in json.load(open(p)):
            out.setdefault(r["id"], {}).update(r["yes"])
    return out
test = load("results-typed-decisions.json", "results-extra-test.json")
ref = load("results-reference.json", "results-extra-reference.json")
topics = list(next(iter(ref.values())))
base = {t: statistics.median(r[t] for r in ref.values()) for t in topics}
def tags(y, best_min, above=0.3, use=topics):
    s = sorted(((t, y[t] - base[t]) for t in use), key=lambda x: -x[1])
    if s[0][1] < best_min: return []
    return [t for i, (t, v) in enumerate(s) if i == 0 or (v >= above and v >= s[0][1] - 0.25)][:3]
mine = topics[:14]
for name, use in (("your 14 topics", mine), ("26 topics", topics)):
    print(name)
    for bm in (0.1, 0.15, 0.2, 0.25):
        top = tp = fp = fn = none = 0
        for v in V:
            tg = tags(test[v["id"]], bm, use=use); want = set(truth(v))
            none += not tg; top += bool(tg) and tg[0] in want
            tp += len(set(tg) & want); fp += len(set(tg) - want); fn += len(want - set(tg))
        print(f"  best>= {bm}: test top right {top}/60 (no tag {none}), tags {tp} right {fp} extra {fn} missed")
if "-v" in sys.argv:
    titles = {r["id"]: r["title"] for r in json.load(open("results-reference.json"))}
    for i, y in ref.items():
        print(f"  {str(tags(y, 0.15)):50} {titles.get(i, '')[:45]}")
