# `.MTA` samples

Exported logs, their `LocaleMetaData\*.MTA` archives, and what Windows renders from each
archive, for `tests/test_mta.rs`. `docs/mta.md` describes the format.

Everything except `setup-export` comes from run
[40586836404](https://github.com/omerbenamram/evtx/actions/runs/40586836404) of
[`mta-samples.yml`](../../.github/workflows/mta-samples.yml), on the Windows version in
`windows-info.txt`. `SHA256SUMS` lists the files the run made. To make new ones, run the
workflow and copy its `mta-samples` artifact over this folder.

| Sample | Events | What it has |
| --- | --- | --- |
| `custom` | 137 | `MtaTest-Provider/Operational`, a test publisher ([`MtaTest.man`](../../scripts/mta/MtaTest.man)) with a custom level, tasks, task-specific and global opcodes, keywords, `%n`, `%t` and `%%` escapes, non-ASCII text and events without a message. Archived in English (`_1033`) and German (`_1031`). |
| `custom-merged` | 142 | The test publisher's `Operational` and `Admin` channels in one export, so two events share each `EventRecordID` from 1 to 5. |
| `classic` | 4 | `Application` events from two classic event sources, one of them with a message file that does not exist. |
| `system-runner` | 50 | The last 50 events of the runner's `System` log. |
| `security-sample` | 60 | Records 132 to 191 of `samples/security.evtx`. |
| `system-sample` | 80 | Records 164 to 243 of `samples/system.evtx`, from 27 publishers. |
| `sysmon-sample` | 20 | Records 1742 to 1761 of `samples/sysmon.evtx`. The runner has no Sysmon, so the archive has no strings for them. |
| `setup-export` | 239 | `Microsoft-Windows-Servicing` events saved with Event Viewer on another machine, with Event Viewer's CSV export of them (`setup-export.csv`). Contributed by Joss Gray ([@dev-joss](https://github.com/dev-joss)) in [#284](https://github.com/omerbenamram/evtx/pull/284). |

The generated exports fit in one chunk each.

## `windows/<sample>_<LCID>.jsonl`

What wevtapi returns for each event of `<sample>.evtx`, from
[`MtaProbe.cs`](../../scripts/mta/MtaProbe.cs). The probe runs after the test publisher and
classic sources are uninstalled, and opens publisher metadata with `LogFilePath` set to the
`.evtx`, so that only the archive is read. One line per event, in file order:

| Field | Value |
| --- | --- |
| `index` | The event's position in the `.evtx`, from 0. |
| `provider`, `event_record_id`, `event_id`, `qualifiers`, `level`, `task`, `opcode`, `keywords` | The `System` fields of Windows' own XML for the event, as text, or `null` when missing. |
| `metadata_error` | The error from `EvtOpenPublisherMetadata`: 0, or 15004 when the archive does not have the publisher. The fields below are then left out. |
| `message`, `level_name`, `task_name`, `opcode_name`, `keyword_names` | What `EvtFormatMessage` returns for the event's message and its level, task, opcode and keyword names, with the NULs it includes. `keyword_names` is a list of NUL-terminated names that ends with another NUL. |
| `<field>_error` | The error `EvtFormatMessage` returned for the field: 0, or 1168 for an event without a message. |
