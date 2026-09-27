# Vendored better-trigram source

`better-trigram.c`, `better-trigram.h`, and `tokenizer.c` come from
[streetwriters/sqlite-better-trigram](https://github.com/streetwriters/sqlite-better-trigram)
commit `851f99f3e14f9dc0606edf40423406d1a2ea2b89` (2026-09-27 checkout).
The upstream source dedicates itself to the public domain. `tokenizer.c` has
one local correction: `u32` is `uint32_t`, not `uint16_t`, so non-BMP Unicode
codepoints are not truncated. Vendored C files have trailing whitespace
removed.

`fts5_unicode2.c` comes from SQLite `version-3.49.1`, matching upstream's
Makefile source selection. SQLite dedicates this generated file to the public
domain. The build uses the bundled SQLite headers from `libsqlite3-sys` and
links the tokenizer into the executable; no runtime shared library is needed.
