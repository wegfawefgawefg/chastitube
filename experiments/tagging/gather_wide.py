"""Wider test set for checking shrunk models: 3 videos from each of many unrelated searches.
Writes videos-wide.jsonl (no hand labels; compared model against model)."""

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


SEARCHES = ["evening news", "premier league highlights", "nba game recap", "formula 1 onboard", "car review 2025",
            "makeup tutorial", "fashion haul", "kids songs", "asmr whisper", "stock market today", "personal finance tips",
            "philosophy lecture", "chess opening", "woodworking project", "gardening tips", "dog training",
            "cat videos", "fishing trip", "hiking alone", "van life", "minecraft lets play", "fortnite montage",
            "speedrun world record", "piano cover", "metal guitar solo", "kpop dance practice", "jazz live",
            "podcast clip", "true crime documentary", "conspiracy theory debunk", "space telescope news", "physics explained",
            "chemistry experiment", "medical lecture", "workout routine", "yoga for beginners", "vegan recipe",
            "street food", "restaurant review", "home renovation", "real estate tour", "unboxing iphone",
            "pc build guide", "linux tips", "excel tutorial", "language learning japanese", "learn spanish",
            "history of china", "world war 2 documentary", "political debate", "climate change explained",
            "stand up comedy", "movie trailer", "film analysis", "anime review", "book review",
            "motivational speech", "meditation music", "lofi beats", "pottery wheel", "knitting tutorial",
            "3d printing", "drone footage", "airplane landing", "train journey", "ship engine",
            "car crash compilation", "magic tricks revealed"]
out, seen = [], set()
for q in SEARCHES:
    try: vids = [v["videoId"] for v in find(post("search", {"query": q}), "videoRenderer", []) if "videoId" in v]
    except Exception as e: print("skip", q, e); continue
    n = 0
    for vid in vids:
        if vid in seen or n == 3: continue
        try: d = post("player", {"videoId": vid})
        except Exception: continue
        vd, mf = d.get("videoDetails", {}), d.get("microformat", {}).get("playerMicroformatRenderer", {})
        if not vd.get("title"): continue
        seen.add(vid); n += 1
        out.append({"id": vid, "query": q, "title": vd["title"], "channel": vd.get("author", ""),
                    "category": mf.get("category", ""), "keywords": vd.get("keywords", []),
                    "description": vd.get("shortDescription", "")})
with open("videos-wide.jsonl", "w") as f:
    for v in out: f.write(json.dumps(v, ensure_ascii=False) + "\n")
print(len(out), "videos")
