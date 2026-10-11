# Static macOS application icon (#3733)

`glyph.png` is the unchanged tracked 960×960 transparent iOS asset from
fn-ios (`guionai/flicknote`), revision
`300261c7467827208dfd71f7af0eb6615d9935cb`, path
`Resources/AppIcon.icon/Assets/flicknote-icon-no-bg 2.png`.
SHA256: `3fd1895f445c1bbcc8fe9bbde7e55e718abd7f069daf32dfc4c09dc79d03a3ae`.
The sibling checkout is not needed for generation or packaging.

The iOS `icon.json` uses system-dark fill, centered scale 1.1, neutral shadow
and translucency. This static rendition preserves the glyph geometry and alpha,
centered at 1.1 times the 824 px silhouette on a 1024 px canvas. A dark
`#222222`–`#080808` gradient and rounded corners with transparent exterior
adapt the composition to macOS. Dynamic material/shadow variants are not reproduced.
The legacy 320 px with-background iOS image is a visual reference only.

Regenerate on macOS with ImageMagick 7 and Apple's `iconutil`:

```bash
bash flicknote-gpui/assets/app-icon/generate.sh
iconutil --convert iconset flicknote-gpui/assets/app-icon/FlickNote.icns \
  --output /ABSOLUTE/NEW/OWNED/FlickNote.iconset
```

Generation used ImageMagick 7.1.2-11. The ICNS contains 16, 32, 128, 256 and
512 point images at 1×/2× (16–1024 px). Inspect actual exported PNGs at large
and small sizes when regenerating. Ordinary packaging only copies the committed
ICNS; it requires neither ImageMagick nor the iOS repository.
