---
description: >-
  Write .docx and .pptx files, and read PDF, Word, PowerPoint and Excel files
  back in, with every limit stated up front.
icon: file-lines
---

# Documents

Two tools write files a person can open, and the ingest path reads the common office formats back in. Both run in a separate loadable module, not in the app itself.

## Writing

| Tool | Produces |
| --- | --- |
| `generate_document` | A `.docx` from a structured spec: title, author, headings, paragraphs and bullet lists. |
| `generate_presentation` | A `.pptx` from slides: titles, bullets and images. |

The spec is checked before anything is written. The limits are part of the contract:

| `.docx` | Limit |
| --- | --- |
| Sections | 128 |
| Paragraphs per section | 200 |
| Bullets per section | 200 |
| Characters per title, author or heading | 2,000 |
| Characters per paragraph | 20,000 |
| Characters in total | 2,000,000 |

| `.pptx` | Limit |
| --- | --- |
| Slides | 64 |
| Bullets per slide | 32 |
| Characters per text field | 2,000 |
| Images per slide | 6 |
| Images per deck | 8 |
| Bytes per image | 5 MiB |

The total character cap is the one that matters most. Without it, sections times paragraphs times characters per paragraph would allow a spec that no reader could open.

The finished file arrives as an [artifact](../chat.md). It appears in the thread's files panel with download and reveal-in-folder.

## Reading

Attach a file, or point a [memory source](../memory.md) at one, and the ingest path extracts its text. Each piece of text keeps its origin: a page number for a PDF, or the part path for an Office file.

| Format | Notes |
| --- | --- |
| PDF | Up to 4,096 source pages. A page with no text layer is kept and flagged as a scan candidate. That flag suggests using vision. It does not claim OCR is needed. |
| DOCX, PPTX, XLSX | Slide and sheet order comes from the file's own manifest, so an unreferenced slide is left out and not guessed from its filename. Spreadsheets read shared and inline strings and use cached cell values. Formulas are never evaluated. |

The bounds are 64 MiB in, 256 result sections and 200,000 bytes of text out. The result says whether it was truncated. Text is never cut mid-character. Rendering PDF pages to images is opt-in for each page, with at most 8 pages per call and 2,048 pixels on the longest edge.

Nothing is written to disk during extraction, no office application is launched, and there is no network OCR.

## Where it runs

Document work happens in the `tinydocs` module. It is a signed native library that downloads on first use and is checked against a SHA-256 pinned in OpenHuman. The document writers, PDF and Office readers, and image inspection are not part of the app's own code in any build. With the `documents` feature off, the generation tools are absent and PDF/Office extraction and image inspection are unavailable.

## See also

- [Image and video generation](media-generation.md): the other tools that produce a file.
- [Memory](../memory.md): where a document ends up if you add it as a source.
- [Loadable modules](../../developing/loadable-modules.md): how a module is pinned and loaded.
