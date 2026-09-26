# IBM Plex Sans (OFL 1.1, see license.txt)

Embedded in the WebAssembly build only (a browser canvas cannot use system
fonts). Both files are trimmed from the upstream IBM Plex Sans 3.x TTFs with
fontTools:

- `IBMPlexSans-Regular.ttf`: every glyph and layout feature kept (Latin,
  Cyrillic, Greek, symbols; hinting kept because GPUI rasterizes hinted);
  glyph names, `DSIG`, `meta` and the long name records dropped.
  200,500 → 184,072 bytes.
- `IBMPlexSans-SemiBold.ttf`: additionally limited to Latin, combining marks,
  punctuation, currency, arrows, math and ✓. Bold Cyrillic/Greek text falls
  back to the Regular face. 202,632 → 104,744 bytes.

```
COMMON="--layout-features=* --no-glyph-names --drop-tables+=DSIG,meta --name-IDs=0,1,2,3,4,5,6,16,17 --hinting"
pyftsubset IBMPlexSans-Regular.ttf --unicodes='*' --glyphs='*' $COMMON --output-file=IBMPlexSans-Regular.ttf
pyftsubset IBMPlexSans-SemiBold.ttf $COMMON --output-file=IBMPlexSans-SemiBold.ttf \
  --unicodes='U+0000-024F,U+0300-036F,U+1E00-1EFF,U+2000-206F,U+20A0-20CF,U+2100-214F,U+2190-21FF,U+2200-22FF,U+2713,U+FB01-FB02'
```
