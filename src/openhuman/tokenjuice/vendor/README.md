# Vendored TokenJuice Rules

These JSON rule files are vendored from the upstream
[vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice) repository.

## Upstream

- Repository: https://github.com/vincentkoc/tokenjuice
- Upstream path: `src/rules/**/*.json`
- Licence: MIT (Copyright (c) 2026 Vincent Koc)

## Licence note

The upstream project is MIT-licensed. The full licence text is reproduced below.

```
MIT License

Copyright (c) 2026 Vincent Koc

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## File naming convention

Upstream files live in subdirectory paths like `git/status.json`.  Because we
embed all rules in a single directory here, `/` in the id is replaced with `__`
in the filename (e.g. `git/status.json` → `git__status.json`).

## Subset vendored

Only a representative subset of upstream rules is vendored here for the initial
v1 release.  Additional rules from the upstream repository can be added by
copying the JSON verbatim into this directory — the `include_str!` macro in
`rules/builtin.rs` will pick them up automatically.
