# chastitube icon: brief

Build a Blender script that models and renders the chastitube app icon. Raytraced with Cycles.
Everything procedural in Blender. Do NOT use any image generation tool and do not download images.

## The object (design is fixed; don't change it)
A small cylindrical cage: a flat 4x4 grid of metal bars bent into a cylinder.
- 8 vertical round rods evenly spaced around the circumference (so a front view shows about 4 cells across).
- Horizontal flat strap rings (rectangular cross-section, a bit wider than the rods are thick) splitting the height
  into 4 rows of openings: a top rim ring, 3 middle rings, and the base band at the bottom.
- Rods pass just inside the rings, with small weld blobs where they cross.
- Height about 1.1 x the diameter.
- Top: open, finished with a rolled/rounded rim ring (no lid). The inside back bars are visible through the openings.
- Bottom: a heavy base band (about 1/6 of the total height, thicker than the other rings) with 12 hex-head bolts
  evenly spaced around it, each with a washer under the head. Bolt heads stick out clearly; this is the detail
  that should still read at small sizes.
- It floats: no floor, no shadow catcher, transparent background.

## Materials: 4 variants, same geometry, all textured (noise, grime, edge wear, not flat colour)
1. rust: dark iron bars, orange-brown rust collecting in crevices, at welds and over the base band; bolts slightly brighter worn steel.
2. black: near-black oiled steel, convex edges worn to bright steel; bolts brass.
3. zinc: pale galvanised grey with mottled noise, light grime in corners; bolts dark steel.
4. enamel: chipped red-orange enamel paint on the bars and rings, bare metal showing where it's chipped and on edges; bolts bare steel.


## Camera and light
- Square frame, 3/4 view from about 25 degrees above, so the bolt ring and the inside back bars both show.
  Moderate perspective (around a 50 mm lens). The cage fills about 85% of the frame.
- Warm key light from upper left, cool rim light from behind right, soft fill. Make the metal read, not go black.

## Rendering rules (this is a shared desktop)
- Cycles on CPU only, never GPU. Run Blender with: nice -n 19 /usr/bin/blender -b -noaudio --factory-startup, and set the
  render threads to 4. Only one Blender process at a time. Set TMPDIR to the tmp/ folder here.
- 512x512, enough samples with the denoiser on that it's clean, film transparent.

## Output (everything inside this folder, art/)
- cage_icon.py: the script; `--variant rust|black|zinc|enamel` renders one.
- renders/cage_<variant>_512.png for all 4, plus downscaled copies at 256, 128, 64, 48 and 32 px (good resampling).
- sheet.png: a comparison sheet. Row 1: the four variants at 512 on dark #141414. Row 2: the same on light #e8e8e8.
  Row 3: each variant at 48 px and 32 px (shown at actual size) on both backgrounds. Label each column with the variant name.
- render.sh: renders all four and rebuilds the sheet.

Don't make design decisions beyond this brief. If something isn't specified, pick the plainest option and list it
in your final message along with the files written and how long the renders took.
