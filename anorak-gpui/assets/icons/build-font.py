#!/usr/bin/env python3
"""Build ../fonts/anorak-icons/AnorakIcons.ttf from the SVG icons here.

The icons (16x16 grid) were drawn for this project. GPUI draws them as text
in the "Anorak Icons" family (private-use code points, see ICONS and
src/icons.rs), which costs only the few KB of font data. GPUI's svg()
element would pull resvg/usvg into the wasm bundle (+~550 KB raw).

Needs fontTools and skia-pathops, e.g. on NixOS:
  nix-shell -p 'python3.withPackages (p: [p.fonttools p.skia-pathops])' \
    --run 'python3 build-font.py'
"""
import pathlib
import re

import pathops
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path

ICONS = [
    ("font", 0xE000),
    ("weight", 0xE001),
    ("seedling", 0xE002),
    ("clock", 0xE003),
    ("arrow-up", 0xE004),
    ("arrow-down", 0xE005),
    ("xmark", 0xE006),
    ("plus", 0xE007),
]
UPM, ASC, DESC = 1000, 800, -200
SCALE = UPM / 16
HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent / "fonts" / "anorak-icons" / "AnorakIcons.ttf"


def element_path(tag, a):
    if tag == "circle":
        cx, cy, r = float(a["cx"]), float(a["cy"]), float(a["r"])
        d = f"M{cx - r} {cy}a{r} {r} 0 1 0 {2 * r} 0a{r} {r} 0 1 0 {-2 * r} 0z"
    else:
        d = a["d"]
    p = pathops.Path()
    parse_path(d, p.getPen())
    if a.get("stroke", "none") != "none":
        cap = pathops.LineCap.ROUND_CAP if a.get("stroke-linecap") == "round" else pathops.LineCap.BUTT_CAP
        join = pathops.LineJoin.ROUND_JOIN if a.get("stroke-linejoin") == "round" else pathops.LineJoin.MITER_JOIN
        p.stroke(float(a.get("stroke-width", "1")), cap, join, 4)
        p.convertConicsToQuads()
    elif a.get("fill-rule") == "evenodd":
        p.fillType = pathops.FillType.EVEN_ODD
    p.simplify(fix_winding=True)
    return p


def icon_path(svg):
    parts = [
        element_path(tag, dict(re.findall(r'([\w-]+)="([^"]*)"', attrs)))
        for tag, attrs in re.findall(r"<(path|circle)\b([^>]*)/>", svg)
    ]
    out = pathops.Path()
    pathops.union(parts, out.getPen())
    return out


def main():
    order = [".notdef"] + [name for name, _ in ICONS]
    glyphs, metrics = {}, {}
    empty = TTGlyphPen(None)
    glyphs[".notdef"] = empty.glyph()
    metrics[".notdef"] = (UPM, 0)
    for name, _ in ICONS:
        path = icon_path((HERE / f"{name}.svg").read_text())
        pen = TTGlyphPen(None)
        path.draw(TransformPen(Cu2QuPen(pen, max_err=0.5, reverse_direction=True),
                               (SCALE, 0, 0, -SCALE, 0, ASC)))
        glyphs[name] = pen.glyph()
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({cp: name for name, cp in ICONS})
    fb.setupGlyf(glyphs)
    glyf = fb.font["glyf"]
    for name in order:
        g = glyf[name]
        g.recalcBounds(glyf)
        metrics[name] = (UPM, getattr(g, "xMin", 0))
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=ASC, descent=DESC)
    fb.setupNameTable({"familyName": "Anorak Icons", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=ASC, sTypoDescender=DESC, sTypoLineGap=0,
                usWinAscent=ASC, usWinDescent=-DESC, fsType=0)
    fb.setupPost()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    fb.save(str(OUT))
    print(OUT, OUT.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
