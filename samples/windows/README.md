# Windows' own XML

`types.evtx` is a log that Windows writes, and `types.evtx.jsonl` is the XML Windows renders
for each of its events. Together they show how Windows stores and renders every manifest input
type, so evtx's rendering can be checked against Windows instead of against another parser.

Both come from run [40969797567](https://github.com/omerbenamram/evtx/actions/runs/40969797567)
of [`windows-xml.yml`](../../.github/workflows/windows-xml.yml), on the Windows version in
`windows-info.txt`. `SHA256SUMS` has the log's hash. To make new ones, run the workflow and copy
the `generated/` folder of its `windows-xml` artifact over this folder. The artifact's `samples/`
folder has the same rendering for every log in `samples/` (about 100 MB, so it is not kept here).

## `types.evtx`

[`generate-types-sample.ps1`](../../scripts/windows/generate-types-sample.ps1) installs the test
provider in [`TypesTest.man`](../../scripts/windows/TypesTest.man), writes the events listed in
[`TypesWriter.cs`](../../scripts/windows/TypesWriter.cs) with `EventWrite`, exports the
provider's channel with `wevtutil epl` and uninstalls the provider. The payloads are written
byte by byte, so they can hold values .NET would not encode.

| Event ID | Template | Events | What it has |
| --- | --- | --- | --- |
| 1 | `T_Scalars` | 3 | Every input type once: typical, minimum and maximum values. |
| 2 | `T_OutTypes` | 3 | Output types: `win:Xml`, `win:Utf8`, hex, IP and socket addresses, `xs:dateTime`. They change neither the stored value type nor Windows' XML, which renders each value by its input type. |
| 3 | `T_Text` | 21 | Strings that are hard to render: C0 and C1 controls, CR/LF/tab, lone and reversed surrogates, U+FFFE and U+FFFF, markup and `]]>`, a surrogate pair, a BOM, `%1` and backslashes. |
| 4 | `T_Counted` | 5 | Counted strings with NULs inside or at the end. |
| 5 | `T_Ansi` | 5 | ANSI strings with bytes 0x80 to 0xFF, invalid and truncated UTF-8, and controls. |
| 6 | none | 1 | An event without data. |
| 7 | `T_FileTime` | 14 | FILETIME values: 0 and 1 tick, around 1970, sub-second digits, the last tick of year 9999 and the next one, 0x7FFFFFFFFFFFFFFF and 0xFFFFFFFFFFFFFFFF. |
| 8 | `T_SysTime` | 9 | SYSTEMTIME values: valid ones, all zero, month 13, February 30, hour 24, 1000 ms, year 30827. |
| 9 | `T_Float` | 10 | Float and double values: 0.1, 1, negative zero, NaN, infinities, subnormals, large and small values, the maximum, pi. |
| 10 | `T_Bool` | 5 | Booleans stored as 0, 1, 2, -1 and 65536. |
| 11 | `T_UserData` | 1 | Template XML with entity references, character references and CDATA. The stored template keeps `&amp;`, `&lt;` and `&gt;` as entity reference tokens and the CDATA section as a CDATA token; character references, `&quot;` and `&apos;` are already text. |
| 12 to 32 | `T_A_<type>` | 3 each | Every input type as an array, in `T_Scalars`' order, with 3, 1 and 0 items. Windows can't render the SID arrays with items (error 13), and the Event Log service stores `ProcessingErrorData` (error 50, not supported) instead of the binary arrays. |

## `types.evtx.jsonl`

What [`RenderProbe.cs`](../../scripts/windows/RenderProbe.cs) gets from wevtapi for each event,
one line per event, in the order `EvtNext` returns them:

| Field | Value |
| --- | --- |
| `index` | The event's position in that order, from 0. |
| `record_id` | The record number in the event's bookmark. |
| `xml` | `EvtRender(EvtRenderEventXml)`, exactly as returned. |
| `render_error`, `next_error`, `seek_error`, `query_error` | The Win32 error when wevtapi could not render, read or query an event, instead of `xml`. |
