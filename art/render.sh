#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p renders tmp
for variant in rust black zinc enamel; do
  TMPDIR="$PWD/tmp" nice -n 19 /usr/bin/blender -b -noaudio --factory-startup --python-exit-code 1 --python cage_icon.py -- --variant "$variant"
done
python3 - <<'PY'
from PIL import Image, ImageDraw, ImageFont
from pathlib import Path
root=Path('.'); out=root/'renders'; variants=('rust','black','zinc','enamel')
for v in variants:
    im=Image.open(out/f'cage_{v}_512.png').convert('RGBA')
    for s in (256,128,64,48,32): im.resize((s,s),Image.Resampling.LANCZOS).save(out/f'cage_{v}_{s}.png')
font=ImageFont.load_default(); colw=512; w=colw*4; h=512*2+172
sheet=Image.new('RGB',(w,h),(20,20,20)); draw=ImageDraw.Draw(sheet)
for x,v in enumerate(variants):
    draw.text((x*colw+12,8),v,fill=(240,240,240),font=font)
    for row,bg,y in ((0,(20,20,20),22),(1,(232,232,232),534)):
        tile=Image.new('RGBA',(512,512),bg+(255,)); tile.alpha_composite(Image.open(out/f'cage_{v}_512.png').convert('RGBA')); sheet.paste(tile.convert('RGB'),(x*colw,y))
    # actual-size assets are deliberately not enlarged: dark and light swatches.
    for j,(size,bg) in enumerate(((48,(20,20,20)),(32,(20,20,20)),(48,(232,232,232)),(32,(232,232,232)))):
        x0=x*colw+16+j*118; y0=1058
        draw.rectangle((x0-5,y0-5,x0+size+5,y0+size+5),fill=bg)
        sheet.paste(Image.open(out/f'cage_{v}_{size}.png').convert('RGBA'),(x0,y0),Image.open(out/f'cage_{v}_{size}.png').convert('RGBA'))
sheet.save(root/'sheet.png')
PY
