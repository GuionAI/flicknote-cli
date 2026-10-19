# Menu bar template (#4315)

`template.png` is the alpha silhouette of the existing transparent brand
[`glyph.png`](../app-icon/glyph.png), with black RGB, trimmed, fitted to 32 pixels
and centered in a 36-pixel transparent canvas. AppKit displays it at 18 points
and applies template rendering for Light/Dark, highlight and accessibility.
No new brand geometry or AI bitmap is introduced.

Regenerate with ImageMagick 7:

```sh
magick flicknote-gpui/assets/app-icon/glyph.png -trim +repage -resize 32x32 \
  -gravity center -background none -extent 36x36 \
  -channel RGB -evaluate set 0 +channel -strip \
  PNG32:flicknote-gpui/assets/tray/template.png
```

The image is compiled into the executable; packaging requires no extra resource.
