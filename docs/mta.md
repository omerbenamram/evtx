# Localized strings from `.MTA` files (`LocaleMetaData`)

An `.evtx` file holds each event's data, but not the strings needed to display it: the message,
and the names of the level, task, opcode and keywords. Windows looks those up in the publisher's
resources, which only exist on a machine that has the publisher installed.

When a log is saved *with display information* (Event Viewer's *Save All Events As*, or
`wevtutil epl` followed by `wevtutil al`), Windows also writes the strings, for one locale, to
`LocaleMetaData\<log name>_<LCID>.MTA` next to the `.evtx`. MS-EVEN6 calls the pair a
*localized log* (section 3.1.1.7) and specifies how the server writes it
(`EvtRpcLocalizeExportLog`, section 3.1.4.18), but leaves the file format and the matching of
strings to events out of scope. The client API is `EvtArchiveExportedLog`. Archiving does not
change the `.evtx`.

`evtx::mta` reads these files. Nothing else outside Windows does, as far as we could find: Event
Viewer and other tools read them through wevtapi, which opens them when publisher metadata is
requested with `EvtOpenPublisherMetadata(LogFilePath = <the .evtx>)`.

## Using it

```text
evtx_dump --mta 'LocaleMetaData/Security_1033.MTA' Security.evtx
```

```rust
let mta = evtx::mta::MtaFile::from_path("LocaleMetaData/Security_1033.MTA")?;
let settings = evtx::ParserSettings::new().mta(Some(std::sync::Arc::new(mta)));
```

Every record whose publisher is in the file gets a `RenderingInfo` element as the last child of
`Event`, shaped like the one Windows adds to forwarded events. Strings the file does not have are
left out, and so is the whole element when the publisher is not in the file. Records that already
have a `RenderingInfo` (forwarded events) are left as they are.

```xml
<RenderingInfo>
  <Message>An account was successfully logged on. ...</Message>
  <Level>Log Always</Level>
  <Task>Logon</Task>
  <Opcode>Info</Opcode>
  <Keywords>
    <Keyword>Audit Success</Keyword>
  </Keywords>
</RenderingInfo>
```

In JSON, repeated `Keyword` elements become `Keyword`, `Keyword_1`, ... as other repeated
elements do. Unlike Windows' own `RenderingInfo`, there is no `Culture` attribute, `Channel` or
`Provider`: the file does not say which locale it holds (only its name does), and the other two
are in `System` already.

[`MtaFile::rendering_info`](https://docs.rs/evtx/latest/evtx/mta/struct.MtaFile.html) looks up
the strings for given `System` fields without rendering anything.

## File format

All integers are little-endian. Strings are UTF-16LE.

### Header and sections

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | Magic `MTAFile\0` |
| 8 | 4 | Major version, 1 |
| 12 | 4 | Minor version, 1 |
| 16 | 4 | Size of the section descriptor table in bytes |
| 20 | 4 | Number of sections |
| 24 | | Section descriptors |

Each section descriptor is a u64 file offset, a u64 size, a u32 name length in bytes and the
name, without a NUL. Windows writes three sections, `EVT`, `MSG` and `PUB`, right after the
table. The parser needs all three, ignores sections with other names and takes the first section
of each name.

### Pages and records

A section is a chain of pages. Each page is an 888-byte header followed by up to 100 records:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `0xFFFFFFFF`, or 0 on an empty page |
| 4 | 56 | 14 copies of the highest entry index on the page (unused here) |
| 60 | 4 | `0xFFFFFFFF` |
| 64 | 8 | Page number (Windows leaves garbage in the high 32 bits) |
| 72 | 8 | Offset of the next page, or of this page on the last one |
| 80 | 8 | Offset of this page |
| 88 | 800 | Offsets of up to 100 records, 0 for an unused slot |

Offsets are relative to the start of the section. A record is a u32 entry index, a u32 payload
size and the payload. An empty section is a single empty page.

### `MSG`: strings

Each entry is a u32 byte length (including the terminating NUL, 0 for an empty string) and the
text. Strings are stored as Windows rendered them, with insertions done: unresolved inserts
such as `%1` and `%%1936` stay as they are.

### `EVT`: events

One entry per event of the exported log. The entry index is the event's 0-based position in the
`.evtx`, and the payload is the event's `EventRecordID` (u64, from `System/EventRecordID`, not
the record header) and the `MSG` index of its message (u64). Events without a message point to
an empty string.

### `PUB`: publishers

One entry per publisher in the log, plus `DEFAULT_PUBLISHER`, which holds the standard (winmeta)
names that publishers do not define themselves. Each payload has four tables, then the name:

| Table | Key |
| --- | --- |
| Levels | Level |
| Tasks | Task (a Windows 11 build stores `win:None` as `0x10000`, so only the low 16 bits count) |
| Opcodes | `(opcode << 16) \| task`, with task 0 for opcodes that are not tied to a task |
| Keywords | Bit number: 0 for `0x1`, 63 for `0x8000000000000000` |

A table is a u32 count followed by that many (u32 key, u32 `MSG` index) pairs. The name is a u32
byte length and the publisher name, as in `System/Provider/@Name`.

## How Windows matches strings to events

`samples/mta/windows/*.jsonl` records what wevtapi returned (`EvtFormatMessage`, with publisher
metadata opened from the archive) for each of the 869 events in `samples/mta`. These rules
reproduce all of it:

- The publisher is found by `System/Provider/@Name`. If the file does not have it, Windows has no
  strings for the event at all.
- The message is the one of the first `EVT` entry with the event's `EventRecordID`.
- Level and task names come from the publisher's table, then from `DEFAULT_PUBLISHER`, by exact
  value.
- The opcode name comes from the entry `(opcode << 16) | task`, in the publisher's table, then in
  `DEFAULT_PUBLISHER`. Events with an `EventID/@Qualifiers` other than 0 (classic event sources)
  get none.
- Each set keyword bit, lowest first, gets the publisher's name for it, or else the default one.
- A `System` element that is missing counts as 0. The first table that has a value wins, even
  with an empty name.

## Where evtx differs from Windows

Apart from merged logs, where Windows shows another event's message, these only fill in strings
that Windows leaves empty.

- **Opcodes outside the event's task.** Windows only looks up the opcode within the event's task,
  so opcode names that the file stores under task 0, like `Info` (opcode 0), go missing for most
  events (726 of the 869). evtx falls back to that entry, which is what Windows shows for the same
  events while their publisher is installed.
- **Merged logs.** A log exported from several channels can have two events with the same
  `EventRecordID`, and Windows gives the second one the first one's message. Exported logs number
  their records 1, 2, 3, ... in file order, so evtx first takes the `EVT` entry at the record's
  position (record number - 1) when it has the event's `EventRecordID`. The `custom-merged`
  sample has 5 such events.
- **Tasks** match on their low 16 bits, as above.
- **Publisher names** that match no name exactly are compared ignoring ASCII case. Every event
  in the samples has its publisher's exact name, so they do not show what Windows does here.

## Other things worth knowing

- Level 0 is named `Log Always`, as in the file. Event Viewer shows `Information` for level 0
  instead, and its CSV export shows a task with no name as its number in parentheses, such as
  `(1)`. Neither comes from the file.
- `wevtutil qe /f:RenderedXml` fails with error 87 on an archived log, so the samples' reference
  rendering comes from `EvtFormatMessage`.
- With the publisher installed, wevtapi uses the installed publisher even when given an archive,
  so reference renderings have to be made after the publisher is gone (the sample generator
  uninstalls its test publisher first).
- Rendering with an `.MTA` file is slower. `evtx_dump` renders most records from compiled
  templates, and then reads the `System` fields from each record's substitution values and
  splices the element into its output. On one thread, `samples/security_big_sample.evtx`
  (62,000 records) takes about 1.5 times as long with the publisher names alone, and about 3
  times as long with a 59 MB file that also has a 900-character message for every record. That
  makes the output 1.5 to 1.8 times as large, and loading the file takes about 120 ms of the
  time. The record API (`EvtxParser::records` and the like) is slower still, since it builds
  each record's tree to add the element to. Both give the same output.

## Samples

`samples/mta/README.md` describes the samples and how
[`.github/workflows/mta-samples.yml`](../.github/workflows/mta-samples.yml) makes them on a
Windows runner with [`scripts/mta/generate-mta-samples.ps1`](../scripts/mta/generate-mta-samples.ps1).
