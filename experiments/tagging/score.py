"""Score tagging against hand labels: the topic(s) a video is really about."""
import json, statistics, sys
V = [json.loads(l) for l in open("videos.jsonl")]
BY_QUERY = {
 "monster hunter": ["games"], "terraria": ["games"], "palworld": ["games"],
 "3blue1brown": ["math"], "calculus": ["math"], "group theory": ["math"],
 "sapolsky": ["biology"], "molecular": ["biology"], "rust programming": ["programming"],
 "game engine devlog": ["game development", "programming"], "vulkan": ["graphics", "programming", "game development"],
 "guitar pedal": ["electronics", "music production"], "apple ii": ["retro computing", "programming"],
 "breakcore": ["music production", "music"], "steve reich": ["music"], "cooking": ["cooking"],
 "lisbon": ["travel"], "roman": ["history"], "pcb": ["electronics"], "comedy": ["comedy"]}
def truth(v):
    return next(t for k, t in BY_QUERY.items() if k in v["query"])

def evaluate(path, correct):
    R = {r["id"]: r for r in json.load(open(path))}
    topics = list(next(iter(R.values()))["yes"])
    med = {t: statistics.median(r["yes"][t] for r in R.values()) for t in topics}
    top1 = 0; tp = fp = fn = 0; wrong = []
    for v in V:
        y = R[v["id"]]["yes"]
        s = {t: y[t] - med[t] if correct else y[t] for t in topics}
        ranked = sorted(s, key=lambda t: -s[t])
        want = set(truth(v))
        top1 += ranked[0] in want
        # tags: the best topic, plus any other clearly above its usual level
        best = s[ranked[0]]
        tags = {ranked[0]} | {t for t in ranked[1:3] if s[t] >= (0.3 if correct else 0.5) and s[t] >= best - 0.25}
        tp += len(tags & want); fp += len(tags - want); fn += len(want - tags)
        if ranked[0] not in want or tags - want:
            wrong.append(f"  {v['title'][:50]:50} got {sorted(tags)} want {sorted(want)}")
    n = len(V)
    print(f"{path} {'bias-corrected' if correct else 'raw'}: top topic right {top1}/{n}, "
          f"tags {tp} right, {fp} extra, {fn} missed")
    if "-v" in sys.argv: print("\n".join(wrong))

if __name__ == "__main__":
  for p in ("results-default.json", "results-typed-decisions.json"):
      for c in (False, True):
          evaluate(p, c)
