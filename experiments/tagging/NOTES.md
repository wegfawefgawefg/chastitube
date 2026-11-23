# Topic tagging experiment (2026-11-22)

Can a small local model put broad topic labels (games, math, biology...) on
videos, so history can be searched and grouped by topic?

## Setup

- `gather.py`: 60 real videos (20 topic searches x 3) with YouTube's details from the player API: title, channel, category,
  keywords, description. Saved in videos.jsonl.
- `tag.py [model]`: Laya 0.5.2 (convaiinnovations/laya, Apache 2.0), one
  yes/no question per topic (14 topics) plus a "kind of video" choice.
  CPU-only torch in `.venv` (1.2 GB).
- `score.py`: compares with hand labels. "Bias-corrected" subtracts each
  topic's median score across all videos before ranking, because some
  topics (programming, game development) score high on everything.

## What YouTube gives

- category: too coarse alone. Math, biology and history are all
  "Education"; lots of things are "People & Blogs".
- keywords: about half the videos have none.
- title and description: always there, and enough for the model.

## Results (60 videos)

| | top topic right | tags right / extra / missed |
|---|---|---|
| english, raw | 51 | 72 / 34 / 6 |
| english, bias-corrected | 53 | 73 / 27 / 5 |
| typed-decisions, raw | **54** | 68 / 30 / 10 |
| typed-decisions, bias-corrected | 52 | 60 / 17 / 18 |

(An earlier 65-video set that included 5 history videos gave typed-decisions
bias-corrected 59/65. The scores per video are the same; the set changed. The
median correction cuts extra tags but doesn't reliably win on the top topic.)

Misses: "games" vs "game development" mixed up on game guides; a Terraria
"Full Movie" called comedy; "Rust for Dummies" called comedy; Apple II
assembly videos called graphics or electronics.

Cost on CPU (Python): about 4-5 s per video, 2.8 GB RAM while running.

## Without Python: Rust + ONNX Runtime (onnx-rs/)

`onnx-rs` runs desplega/laya-onnx (typed-decisions/fp32) through the `ort`
crate, with the input building copied from laya-js / laya's common.py.
ONNX Runtime links statically: one binary, no Python, no extra .so.
`compare.py a.json b.json` compares two results files.

- Same answers as Python on all 60 videos (scores within 0.01, same kind of
  video 60/60).
- Ask one question per model run (`CHUNK=1`) with memory patterns off
  (`MEMPAT=0`): peak 2.5 GB (all 15 at once: 7.2 GB), and faster.
- 8 threads: 3.1 s per video. 4 threads: 4.6 s per video (Python at 4
  threads: 11.4 s). Loads in 1.3 s (Python 7 s).
- Model download 1.7 GB (encoder 1.58 GB + head 106 MB + tokenizer).
  Adds about 25 MB to a binary.
- int8 (`quantize.py`, dynamic, per channel): 400 MB, 2 s per video, under
  1 GB RAM, but answers move by up to 0.4 and kind of video changes on 15 of
  60, even with the head left at fp32. Not usable as is.

## Not tried yet

- Laya's Learner (learns from corrected labels, seconds on CPU).
- ROCm GPU torch (model card says about 40 ms per decision on GPU).

## Rewording the programming question (2026-11-23)

wording.py asks one question per video, alone (so scores differ a little from tag.py,
where all 15 go in one batch). "Real" = the 12 videos hand-labelled programming.

| wording | real avg | others median | ranked right |
|---|---|---|---|
| about programming (writing software, languages, code)? | 0.63 | 0.36 | 98% |
| about computer programming? | 0.67 | 0.44 | 97% |
| about computer programming (same description)? | 0.64 | 0.33 | 97% |
| does it show someone writing computer code? | 0.60 | 0.28 | 97% |

Wording moves the whole scale up or down but doesn't separate better. Highest
non-programming with every wording: a Monster Hunter Long Sword guide (0.66-0.70)
and a DIY PCB video (0.59-0.65); Terraria videos, a Rome history and a linear
algebra preview come next. The per-topic median correction already does what
the rewording does.

## Shrinking the model (2026-11-23)

The official checkpoint is fp16; the ONNX export widened it to fp32. Every
real weight converts to fp16 and back exactly (only a few constants of 32
numbers or fewer don't, and those stay fp32). `shrink.py` makes the copies;
each was run on the 60 videos with onnx-rs (CHUNK=1 MEMPAT=0, 8 threads).

| copy | size | answers vs fp32 | s/video | peak RAM |
|---|---|---|---|---|
| fp32 (desplega export) | 1.69 GB | - | 3.1 | 2.5 GB |
| fp16 storage, cast to fp32 at load | 846 MB | bit-identical | 3.2 | 2.4 GB |
| fp16 + zstd -19 byte-split (download only) | about 720 MB | bit-identical | - | - |
| 8-bit weights, blocks of 32 (MatMulNBits) | 645 MB | within 0.02, kind 59/60 | 4.1 | 0.98 GB |
| 4-bit weights, blocks of 32 | 463 MB | within 0.15, kind 52/60 | 3.1 | 0.86 GB |
| int8 dynamic (quantize.py) | 445 MB | within 0.46, kind 44/60 | 2.0 | 0.98 GB |

Lossless floor: the low byte of each fp16 weight doesn't compress at all
(395 of 395 MB); the high byte (sign + exponent) goes from 395 to 275 MB.

## Cutoff and topic list, as built into chastitube (2026-11-23)

Typical score per topic = median over 101 ordinary videos from unrelated
searches (tagger/reference.jsonl, half of videos-wide.jsonl). A topic is
tagged when the video is 0.3 above that (the best one needs 0.15), at most 3.
`rule.py` compares cutoffs on the 60 test videos and the reference videos.

- With only the 14 interest topics, unrelated videos get forced into the
  nearest one (ASMR -> programming, knitting -> comedy, real estate -> travel).
  Raising the cutoff throws away good tags before it stops that.
- Adding 12 general topics (sports, fitness, vehicles, money, ...) puts them in
  sensible places; only 7 of 102 got no topic. Test set top topic: 53 -> 50/60
  (some "misses" are new topics the hand labels don't know, like technology).
- "science" takes biology lectures whatever the wording, so the owner's list
  leaves it out (math and biology are their own topics); the default list keeps it.
- "languages" took "Rust in 100 Seconds"; reworded as "language learning:
  learning to speak a foreign language like Spanish, Japanese or English".
