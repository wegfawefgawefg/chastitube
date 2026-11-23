"""Ask Laya which topics each test video is about. Prints yes-probabilities."""
import json, sys, time
from laya import Router

TOPICS = {
    "games": "video games: playing, guides, playthroughs, news about games",
    "game development": "making games or game engines",
    "programming": "writing software, programming languages, code",
    "graphics": "computer graphics, rendering, GPUs, shaders",
    "math": "mathematics: algebra, calculus, geometry, group theory, proofs",
    "biology": "living things, genetics, the brain, behaviour, medicine",
    "history": "past events, empires, wars, historical people",
    "electronics": "circuits, PCBs, soldering, hardware, guitar pedals",
    "retro computing": "old computers and consoles, 8-bit machines, assembly",
    "music": "songs, performances, composers, albums",
    "music production": "making music, mixing, synths, trackers, DAWs",
    "cooking": "food, recipes, cooking",
    "travel": "places, trips, travel vlogs",
    "comedy": "stand-up, sketches, jokes",
}
FORMAT = {"lecture": "a talk or class teaching a subject",
          "tutorial or guide": "step by step how-to",
          "explainer": "short explanation of an idea",
          "playthrough or let's play": "someone playing a game through",
          "devlog or project log": "someone showing what they built",
          "performance or song": "music being played",
          "vlog": "someone's day or trip",
          "documentary": "long factual film",
          "entertainment": "for fun: comedy, challenges, reactions"}

def state(v):
    return (f"Title: {v['title']}\nChannel: {v['channel']}\nYouTube category: {v['category']}\n"
            f"Keywords: {', '.join(v['keywords'][:25])}\nDescription: {v['description'][:600]}")

questions = {t: {"type": "noul", "instructions": f"Is this video about {t} ({d})?"} for t, d in TOPICS.items()}
questions["format"] = {"type": "choice", "instructions": "What kind of video is this?", "criteria": FORMAT}

t0 = time.time()
router = Router()
MODEL = sys.argv[1] if len(sys.argv) > 1 else None
print("loaded in", round(time.time() - t0, 1), "s", flush=True)
results, times = [], []
for line in open("videos.jsonl"):
    v = json.loads(line)
    t = time.time()
    r = router.predict(state(v), questions, model=MODEL)
    times.append(time.time() - t)
    a = r["answers"]
    yes = {k: round(a[k]["noul"], 2) for k in TOPICS}
    results.append({"id": v["id"], "title": v["title"], "yes": yes, "format": a["format"]["choice"],
                    "routing": r.get("routing", {}).get("model")})
    tags = [k for k, p in sorted(yes.items(), key=lambda x: -x[1]) if p >= 0.5]
    print(f"{v['title'][:58]:58} | {', '.join(tags) or '-':40} | {a['format']['choice']}", flush=True)
json.dump(results, open(f"results-{MODEL or 'default'}.json", "w"), indent=1)
print("per video:", round(sum(times) / len(times) * 1000), "ms; first", round(times[0] * 1000), "ms")
