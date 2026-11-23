"""Try different wordings of one topic question; see how well it separates real hits from the rest."""
import json, statistics, sys
from laya import Router
from score import V, truth

def state(v):  # same text as tag.py
    return (f"Title: {v['title']}\nChannel: {v['channel']}\nYouTube category: {v['category']}\n"
            f"Keywords: {', '.join(v['keywords'][:25])}\nDescription: {v['description'][:600]}")

TOPIC = "programming"
WORDINGS = {
 "old": "Is this video about programming (writing software, programming languages, code)?",
 "computer programming": "Is this video about computer programming?",
 "computer programming + desc": "Is this video about computer programming (writing software, programming languages, code)?",
 "shows code": "Does this video show someone writing computer code?",
}
r = Router()
for name, q in WORDINGS.items():
    ys, ns = [], []
    for v in V:
        p = r.predict(state(v), {"q": {"type": "noul", "instructions": q}}, model="typed-decisions")["answers"]["q"]["noul"]
        (ys if TOPIC in truth(v) else ns).append(p)
    if "-v" in sys.argv:
        hi = sorted(((p, v) for p, v in zip(ns, [v for v in V if TOPIC not in truth(v)])), key=lambda x: -x[0])[:5]
        for p, v in hi: print(f"     {p:.2f} {v['title'][:55]}  [{', '.join(truth(v))}]")
    pairs = sum(a > b for a in ys for b in ns) / (len(ys) * len(ns))
    print(f"{name:28} real {statistics.mean(ys):.2f} (min {min(ys):.2f})  others median {statistics.median(ns):.2f} "
          f"(max {max(ns):.2f})  ranked right {pairs:.0%}", flush=True)
