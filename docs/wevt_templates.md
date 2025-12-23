# WEVT_TEMPLATE extraction: an offline template cache for rendering “template-less” EVTX records

## The problem (offline template cache for template-less records)

Windows EVTX records often **don’t carry a complete, self-contained “format string”** for their XML. Instead, a record can reference a **template definition** that lives elsewhere:

- On-disk EVTX files store record payloads as **BinXML**.
- The “shape” of the XML (element names, attribute names, substitution slots) can be defined in **provider templates** that are embedded in Windows binaries (EXE/DLL/SYS) as `WEVT_TEMPLATE` resources.

This becomes painful when:

- You carve records out of slack/unallocated space (no reliable access to the original provider binaries).
- You want to render logs **offline** on a system that doesn’t have the original provider manifests installed.

So the goal is to build an offline template cache:

1. Extract all `WEVT_TEMPLATE` resources from a corpus of binaries.
2. Parse them into templates and **stable join keys**.
3. Use those templates later to render records that are missing template metadata.

This is exactly what [issue #103](https://github.com/omerbenamram/evtx/issues/103) was about.

## What this repo implements

This repo adds:

- A **production-grade, deterministic** parser for the `WEVT_TEMPLATE` payload format:
  - `CRIM` (manifest) → provider directory → `WEVT` provider element directory → elements like `EVNT`/`TTBL`/`TEMP`
  - aligned to the `libfwevt` documentation and behavior
- A BinXML mode for `WEVT_TEMPLATE` templates:
  - **WEVT inline-name encoding** (names stored inline, not via chunk string tables)
  - **strict MS-EVEN6 NameHash validation**
- An `evtx_dump` subcommand that can extract templates from many binaries:
  - supports `--input` (multi), `--glob` (multi), `--recursive`
  - emits JSONL for downstream processing (“cache without committing to a DB”)

All of this ships under the optional Cargo feature **`wevt_templates`** to keep default builds lean.

## Where templates live: PE resources → WEVT_TEMPLATE → CRIM

Provider templates are embedded as a PE resource type `WEVT_TEMPLATE`. The resource data blob typically starts with `CRIM...` and contains:

- A CRIM header (version, provider count)
- An array of provider descriptors pointing at provider blocks
- Each provider has a `WEVT` header with an element descriptor directory
- Elements include:
  - `EVNT`: event definitions (this is where the canonical `template_offset` join lives)
  - `TTBL`: template table containing `TEMP` template definitions
  - `CHAN`, `KEYW`, `LEVL`, `OPCO`, `TASK`, `MAPS` (metadata tables)

The key observation (mirroring `libfwevt`) is:

> In `EVNT`, each event definition includes a `template_offset` (relative to CRIM) that points directly to a `TEMP` definition.

## TTBL/TEMP: templates + substitution items

Inside `TTBL`, templates are stored as a sequence of `TEMP` entries:

- TEMP header includes:
  - descriptor counts (`item_descriptor_count`, `item_name_count`)
  - `template_items_offset` (relative to CRIM)
  - template GUID (identifier)
  - BinXML fragment immediately after the header
- Template item descriptors are stored *outside* the BinXML fragment (at `template_items_offset`)
  - each descriptor describes a substitution slot (type/count/length) and points to a UTF-16 name

This is how we can render useful placeholders:

- `TEMP` gives the XML “shape” (element names and substitution tokens)
- item descriptors/names give semantic names for `{sub:N}` such as `{sub:0:Foo}`

## BinXML dialect: EVTX chunk vs WEVT inline names

EVTX record BinXML typically resolves element/attribute names via chunk string tables (offset-based references).

In `WEVT_TEMPLATE` payloads, BinXML uses a different encoding:

- Names are stored inline as:
  - `u16 NameHash`
  - `u16 NameNumChars`
  - `UTF-16LE chars`
  - `u16 NUL`

We implement this as a separate name encoding mode (internally `WevtInline`) and enforce the MS-EVEN6 NameHash.

### NameHash (strict)

NameHash is computed over UTF-16 code units:

```
hash = 0
for each u16 code_unit in name:
    hash = hash * 65599 + code_unit
stored_hash = low_16_bits(hash)
```

If `stored_hash` doesn’t match, parsing fails (by design; “no best-effort”).

## The join keys (how to actually use this offline)

There are two practical joins:

1. **Template GUID** (strong, stable):\n
   - EVTX template definitions carry a GUID\n
   - WEVT `TEMP` templates carry the same GUID\n
   - If you have an EVTX record that exposes the template GUID (e.g. from a `TemplateInstance`), this is the cleanest join.

2. **Provider event → template_offset → template GUID**:\n
   - `EVNT` event definition includes `template_offset`.\n
   - Resolve it to a `TEMP` at that offset.\n
   - You now have the template GUID (and the full template definition).

The CLI emits these joins so you can build a simple offline cache index without inventing a database format.

## End-to-end: build a cache and use it

### 1) Build the cache (extract templates from binaries)

Build/run the CLI with the feature enabled:

```bash
cargo run --release --features wevt_templates --bin evtx_dump -- \
  extract-wevt-templates --help
```

Example using the public `services.exe` sample (stored as a `.gif` in this repo):

```bash
cargo run --release --features wevt_templates --bin evtx_dump -- \
  extract-wevt-templates \
  --input samples_local/services.exe.gif \
  --output-dir /tmp/wevt_cache \
  --overwrite \
  --split-ttbl \
  --dump-temp-xml \
  --dump-events \
  --dump-items \
  > /tmp/wevt_cache/index.jsonl
```

What you get:

- `/tmp/wevt_cache/*.bin`: raw `WEVT_TEMPLATE` resource blobs (CRIM payloads)
- `/tmp/wevt_cache/temp/*.bin`: raw `TEMP` slices
- `/tmp/wevt_cache/temp_xml/*.xml`: rendered template XML skeletons
- `/tmp/wevt_cache/index.jsonl`: JSONL describing resources, events, template GUIDs, and template items

### 2) Look up a template GUID for an event (offline join)

Assuming you know:

- `provider_guid` (from the record’s `<System><Provider Guid="...">`)
- `event_id` and `version` (from the record’s `<System><EventID>` and version field)

You can find the template GUID from the JSONL:

```bash
jq -r '
  select(has("provider_guid")) |
  select(.provider_guid=="{PROVIDER_GUID}" and .event_id=={EVENT_ID} and .version=={VERSION}) |
  .template_guid
' /tmp/wevt_cache/index.jsonl | head -n1
```

Then locate the corresponding rendered template XML skeleton:

```bash
jq -r '
  select(has("output_path") and (.output_path|endswith(".xml"))) |
  select(.guid=="{TEMPLATE_GUID}") |
  .output_path
' /tmp/wevt_cache/index.jsonl | head -n1
```

### 3) Apply it to a carved record (what remains)

The cache solves the hard part: **offline extraction and parsing of provider templates**, plus **stable joins**.

To fully render a carved record end-to-end you still need the record’s **substitution values array** (the `{sub:N}` values). Once you have those values (from the record’s BinXML TemplateInstance data), you can:

- pick the template (by GUID or by event→template join)
- substitute `{sub:N(:Name)?}` slots with actual values (with proper escaping)

This last “apply substitutions” step is not yet wired as a single CLI command, but the format pieces are now in place to build it cleanly without heuristics.

## Implementation map (where to read the code)

- Template extraction + CLI wiring:\n
  - `src/bin/evtx_dump.rs` (subcommand `extract-wevt-templates`)\n
  - `src/wevt_templates/mod.rs` (public API + re-exports)\n
  - `src/wevt_templates/extract.rs` (PE resource extraction)\n
  - `src/wevt_templates/binxml.rs` (WEVT inline-name BinXML parsing helpers)\n
  - `src/wevt_templates/render.rs` (XML rendering helpers)\n
  - `src/wevt_templates/temp.rs` (TTBL/TEMP discovery helpers)\n
- Spec-backed manifest parsing:\n
  - `src/wevt_templates/manifest/mod.rs` (module entrypoint)\n
  - `src/wevt_templates/manifest/types.rs` (CRIM/WEVT/EVNT/TTBL/TEMP types)\n
  - `src/wevt_templates/manifest/parse.rs` (spec-backed parsing)\n
  - `src/wevt_templates/manifest/error.rs` (parse error types)\n
- BinXML dialect support:\n
  - `src/binxml/name.rs` (WEVT inline-name parsing + strict NameHash)\n
  - `src/binxml/deserializer.rs` (threading `BinXmlNameEncoding` through token parsing)\n

## Testing strategy

We avoid shipping proprietary Windows binaries:

- Committed minimal synthetic PE fixture for `WEVT_TEMPLATE` extraction.
- Synthetic CRIM/WEVT/TTBL/TEMP blobs for structural correctness + join tests.
- Ignored integration test against the `services.exe` sample if present locally (or downloaded by the test when enabled).

## Future work

If we want truly end-to-end “render carved record using cache”, the missing piece is a small API/CLI that:

1. parses a record’s TemplateInstance substitution array
2. resolves template GUID via cache
3. applies substitution values to the template definition

There’s also room to expand parsing of `MAPS` (e.g. `BMAP`) if/when the format is fully nailed down.

## References (primary sources)

- Issue #103 (original feature gap / motivation): `https://github.com/omerbenamram/evtx/issues/103`\n
- MS-EVEN6 BinXml (inline name format + NameHash algorithm): `https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-even6/c73573ae-1c90-43a2-a65f-ad7501155956`\n
- libfwevt (manifest format + reference implementation): `https://github.com/libyal/libfwevt`\n
- libfwevt manifest spec doc (CRIM/WEVT/EVNT/TTBL/TEMP tables): `https://github.com/libyal/libfwevt/blob/main/documentation/Windows%20Event%20manifest%20binary%20format.asciidoc`\n
- libevtx (EVTX format reference): `https://github.com/libyal/libevtx/blob/main/documentation/Windows%20XML%20Event%20Log%20(EVTX).asciidoc`


