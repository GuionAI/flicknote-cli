# License Audit

Last updated: 2026-03-02

## Allowed Licenses

All dependencies must use one of the licenses listed in `deny.toml`:

| License | Type | Commercial Use |
|---------|------|---------------|
| MIT | Permissive | Yes |
| Apache-2.0 | Permissive | Yes |
| Apache-2.0 WITH LLVM-exception | Permissive | Yes |
| BSD-2-Clause | Permissive | Yes |
| BSD-3-Clause | Permissive | Yes |
| ISC | Permissive | Yes |
| MPL-2.0 | Weak copyleft (file-level) | Yes |
| Zlib | Permissive | Yes |
| Unicode-3.0 | Permissive | Yes |

## Enforcement

License checks run via `cargo deny check licenses` in CI. Any dependency using
a license not in the allow list will fail the build.

## Decisions

### treemd crate (rejected as dependency)

The `treemd` crate (v0.5.x) was evaluated for markdown heading parsing. While
its parser API is clean, it pulls in `syntect` for syntax highlighting which
brings problematic transitive dependencies:

| Crate | License | Issue |
|-------|---------|-------|
| notify | CC0-1.0 | Not in allow list |
| clipboard-win | BSL-1.0 | Not in allow list |
| error-code | BSL-1.0 | Not in allow list |
| libfuzzer-sys | NCSA | Not in allow list |
| bincode | — | RUSTSEC-2025-0141 (unmaintained) |
| yaml-rust | — | RUSTSEC-2024-0320 (unmaintained) |

**Resolution:** Vendored the heading parser logic (~180 lines) into
`flicknote-cli/src/markdown.rs` with zero external dependencies. This provides
`parse_markdown`, `Document`, `Heading`, `HeadingNode`, `filter_headings`,
`extract_section`, `build_tree`, and `render_box_tree` — everything we need
without the syntect/TUI dependency chain.

The vendored code is derived from treemd's `parser/document.rs` (MIT licensed).
