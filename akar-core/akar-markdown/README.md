# Akar Markdown Extension

Markdown wiki / Open Knowledge Format reader for the Akar database engine.

Reads a folder of Markdown notes — the layout used by "open knowledge format"
wikis such as Obsidian vaults — where each note may carry YAML frontmatter and
links written as `[[wikilinks]]`, and exposes the result both as Rust types and
as a Cypher table function. No third-party parser dependencies: the frontmatter
reader implements a documented YAML subset and the link scanner is line oriented.

**Tests:** 44

## Public API

| Item | Purpose |
|------|---------|
| `parse_wiki_dir(root)` | Read every note below `root` into a `WikiGraph` |
| `parse_note(path, root)` | Read a single note relative to `root` |
| `WikiGraph` | `notes` in id order; `relations()` (deduplicated edges), `get(id)` |
| `WikiNote` | `id`, `path`, `frontmatter`, `body`, `links` |
| `WikiRelation` | `from`, `to` (raw link target), `label` |
| `WikiLink` | `target`, `label`, `anchor`, `line` (1-based, whole file) |
| `FrontMatter` | `get`, `title`, `tags`, `aliases`, `is_empty`, `len`, `iter` |
| `FrontMatterValue` | `Scalar(String)` or `List(Vec<String>)` |
| `extract_wikilinks(body)` | Scan a body for links, skipping fenced code blocks |
| `MarkdownError` | `Io`, `NotADirectory`, `NotUnderRoot` |
| `MarkdownExtension` | Registers the table function with the `MARKDOWN` extension |

A note **id** is its path relative to the root, without the extension, using `/`
as the separator on every platform (`notes/foo.md` → `notes/foo`).
`parse_wiki_dir` walks recursively, skips dot-directories and dot-files, accepts
the `.md` extension in any letter case, and returns notes sorted by id.
Relation targets stay **unresolved**: `to` is the link target exactly as
written, so a caller decides how a target maps to a note.

## Table function

```sql
CALL read_markdown_wiki('/path/to/vault');
```

The path argument is the wiki root folder. Akar's Cypher grammar has no `YIELD`
clause, so the columns are addressed by the field names declared on the produced
chunk:

| column | type | nullable | contents |
|--------|------|----------|----------|
| `node` | STRING | no | a note id |
| `rel`  | STRING | yes | the raw `[[wikilink]]` target on a link row, `NULL` on the note's own row |

Rows are emitted note by note, in graph order (ids ascending): the note's own
row `(id, NULL)` first, then one `(id, target)` row per link in the body, in
document order. Targets are emitted exactly as written, so repeated links
produce repeated rows — use `WikiGraph::relations()` for a deduplicated edge
list. The wiki is re-read on every call.

Enable with the `markdown-extension` feature of `akar-main`.

## Supported frontmatter subset

A block is recognized when a note starts with a `---` line and a later line is
exactly `---`; a leading UTF-8 BOM and `\r\n` line endings are tolerated.

```markdown
---
title: Hello World
tags: [rust, graph]
aliases:
  - greeting
note: "a \"quoted\" word"
---

Body starts here, see [[other#section|Other]].
```

Understood inside the block:

- `key: value` scalars — plain, `'single quoted'`, or `"double quoted"`. Inside
  double quotes `\"` and `\\` are unescaped; every other backslash sequence
  (for example `\n`) is preserved verbatim rather than interpreted.
- `key:` followed by `- item` lines → a list.
- `key: [a, b, "c"]` → an inline (flow) list; quoted elements may contain commas.
- `#` starts a comment on a whole line and on a plain (unquoted) value.

Limitations — these are **skipped without error**, so a slightly malformed note
never fails a whole wiki:

- Nested maps (indented `a: 1` blocks and inline `{a: 1}` flow maps).
- Multi-line block scalars (`|`, `>`) and anchors/aliases (`&anchor`, `*alias`).
- An opening `---` with no closing `---`: the whole file becomes the body and the
  frontmatter is empty.
- A value with an unbalanced opening quote or `[` is kept as a plain scalar.
- A key whose value is empty and is not followed by list items is recorded as
  `Scalar("")`.

Entries are stored in a `BTreeMap`, so `iter()` order is deterministic (sorted by
key) and a key repeated in the block keeps its last value.

## Wikilinks

`[[Target]]`, `[[Target|Label]]`, `[[Target#Anchor]]` and
`[[Target#Anchor|Label]]`. Target, anchor and label are trimmed; an empty target
skips the link, and an empty label or anchor is dropped. Links inside fenced code
blocks (``` or `~~~`, with an optional info string) are ignored, including after
an unterminated fence, and `\[[x]]` is not a link. `line` is the 1-based number
of the line in the whole file, not in the body.
