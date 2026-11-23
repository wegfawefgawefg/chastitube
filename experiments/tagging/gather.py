"""Build a test set: real videos across topics, with YouTube's details.
Writes videos.jsonl. Rerunning picks whatever YouTube returns today."""
import json, urllib.request

CLIENT = {"clientName": "WEB", "clientVersion": "2.20251120.01.00", "hl": "en", "gl": "US"}

def post(ep, body):
    body = dict(body, context={"client": CLIENT})
    req = urllib.request.Request(f"https://www.youtube.com/youtubei/v1/{ep}?prettyPrint=false",
                                 data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=15))

def find(o, key, out):
    if isinstance(o, dict):
        for k, v in o.items():
            out.append(v) if k == key else find(v, key, out)
    elif isinstance(o, list):
        for v in o: find(v, key, out)
    return out

SEARCHES = ["monster hunter wilds long sword guide", "terraria calamity playthrough", "palworld base building",
            "3blue1brown linear algebra", "calculus lecture mit", "group theory explained",
            "sapolsky lecture", "molecular biology explained", "rust programming tutorial",
            "game engine devlog", "vulkan renderer from scratch", "guitar pedal teardown",
            "apple ii 6502 assembly", "breakcore production", "steve reich music for 18 musicians",
            "japanese home cooking", "lisbon travel vlog", "history of the roman empire documentary",
            "diy electronics pcb", "stand up comedy special clip"]
out, seen = [], set()
for q in SEARCHES:
    vids = [v["videoId"] for v in find(post("search", {"query": q}), "videoRenderer", []) if "videoId" in v]
    n = 0
    for vid in vids:
        if vid in seen or n == 3: continue
        d = post("player", {"videoId": vid})
        vd, mf = d.get("videoDetails", {}), d.get("microformat", {}).get("playerMicroformatRenderer", {})
        if not vd.get("title"): continue
        seen.add(vid); n += 1
        out.append({"id": vid, "query": q, "title": vd["title"], "channel": vd.get("author", ""),
                    "category": mf.get("category", ""), "keywords": vd.get("keywords", []),
                    "description": vd.get("shortDescription", "")})
with open("videos.jsonl", "w") as f:
    for v in out: f.write(json.dumps(v, ensure_ascii=False) + "\n")
print(len(out), "videos")
