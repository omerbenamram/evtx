# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0 - 2022-08-29]

## Added

- A new feature for `evtx-dump` which allows selective dumping of event ranges.
- Added builds for apple silicon macs via cross compilation.

### Changed

- Ignore invalid header flags - thanks @Oskar65536
- Don't panic when a date has an invalid value (Use 1061.1.1 if raw value is 0,
  return an error otherwise) #209
- Use `insta` for snapshot testing
- Convert `#text` to an array if multiple elements with the same name exist

### Fixed

- https://github.com/omerbenamram/evtx/issues/201
- https://github.com/omerbenamram/evtx/issues/209
- https://github.com/omerbenamram/evtx/issues/221

## [0.7.2 - 2021-04-01]

### Changed

- Fix flags to be proper bitmasks and add no-CRC flag (#188) - thanks @Robo210

## [0.7.1 - 2021-03-26]

### Changed

- `fast-alloc` is no longer on by default, to support static MUSL builds for
  `evtx-dump` to enable it, build with `--features fast-alloc`.
- static binaries are now published for linux! take `evtx-dump` with you
  everywhere :)
- CI was migrated to github actions from azure pipelines.

## [0.6.9 - 2021-01-30]

### Fixed

- Fixed some imports which mistakingly imported serde internals.

## [0.6.8 - 2020-10-01]

### Fixed

- Allow for arbitrarily large EVTX files to parse correctly. (#128)

## [0.6.7 - 2020-08-28]

### Added

- calculated_chunk_count field to EvtxParser
- impl Debug for EvtxParser

### Changed

- Use calculated chunk count rather than header chunk count to continue parsing
  past 4294901760 bytes of chunk data.
- Moved function/error chunk indexes to u64 instead of u16 to allow for chunk
  indexes larger than u16 MAX

## [0.6.6 - 2020-01-22]

### Fixed

Another tiny fix where the parser might loop for very specific samples -
@codekoala thanks for the patch!

## [0.6.5 - 2020-01-14]

10% Speedup by using LTO on release.

### Changed

- Enabled link-time-optimizations.

## [0.6.4 - 2020-01-14]

This release should make `evtx_dump` 3 times faster on windows machines! Also -
about 25% faster on linux machines.

_NOTE_: this does not affect library code using `evtx`, only the binary target
`evtx_dump`.

If you are using `evtx` as a library, you might benefit significantly from
adapting a custom allocator!

### Changed

- Added `jemalloc`/`rpmalloc`(windows) to take advantage of smarter allocation
  management.
- Use buffered writing on `evtx_dump`.
- Better utilization of cached strings.

## [0.6.3 - 2020-01-11]

This version should be 10-15% faster!

### Fixed

- When using separate json attributes, if the element's value is empty, remove
  the empty mapping. #71

## [0.6.2 - 2019-12-17]

### Fixed

- An edge case where huge files could cause the parser to get stuck.

## [0.6.1 - 2019-12-05]

### Fixed

- A regression with `--seperate-json-attributes` caused by improvements in 0.6.0
  to JSON parsing for non-standard xml documents.

## [0.6.0 - 2019-11-26]

### Added

- Support for `EntityRef` nodes.

### Changed

- Error reporting should be better with this version.

### Fixed

- A bug where parser was accepting NUL bytes as strings.
- Fixed a bug where UTF-16 strings would yield more bytes after UTF-8 conversion
  and would be rejected.
- Support an edge case when some data might be missing from `OpenStartElement`
  node.
- A bug where XML records having multiple nodes with the same name will be
  incorrectly converted to JSON, ex.

```
<HTTPResponseHeadersInfo>
    <Header>HTTP/1.1 200 OK</Header>
    <Header>Connection: keep-alive</Header>
    <Header>Date: Thu, 18 May 2017 11:37:58 GMT</Header>
    <Header>Content-Length: 813</Header>
    <Header>Content-Type: application/pkix-crl</Header>
    <Header>Last-Modified: Tue, 02 May 2017 22:24:24 GMT</Header>
    <Header>ETag: 0x8D491A9FD112A27</Header>
    <Header>Server: Windows-Azure-Blob/1.0 Microsoft-HTTPAPI/2.0</Header>
    <Header>x-ms-request-id: 477c132d-0001-0045-443b-c49ae1000000</Header>
    <Header>x-ms-version: 2009-09-19</Header>
    <Header>x-ms-lease-status: unlocked</Header>
    <Header>x-ms-blob-type: BlockBlob</Header>
</HTTPResponseHeadersInfo>
```

Will now be converted to:

```json
{
  "HTTPResponseHeadersInfo": {
    "Header": "x-ms-blob-type: BlockBlob",
    "Header_1": "HTTP/1.1 200 OK",
    "Header_10": "x-ms-version: 2009-09-19",
    "Header_11": "x-ms-lease-status: unlocked",
    "Header_2": "Connection: keep-alive",
    "Header_3": "Date: Thu, 18 May 2017 11:37:58 GMT",
    "Header_4": "Content-Length: 813",
    "Header_5": "Content-Type: application/pkix-crl",
    "Header_6": "Last-Modified: Tue, 02 May 2017 22:24:24 GMT",
    "Header_7": "ETag: 0x8D491A9FD112A27",
    "Header_8": "Server: Windows-Azure-Blob/1.0 Microsoft-HTTPAPI/2.0",
    "Header_9": "x-ms-request-id: 477c132d-0001-0045-443b-c49ae1000000"
  }
}
```

## [0.5.1 - 2019-10-30]

### Fixed

- A bug which causes a panic (bounds check) on some corrupted records.

## [0.5.0 - 2019-10-07]

### Added

- `EvtxParser::records_json_value()` to allow working with records with a
  `serde_json::Value`. See `test_into_json_value_records` for an example.
- `EvtxRecord::into_output`, allowing serializing a record using a user-defined
  `BinXmlOutput` type.

### Changed

- `SerializedEvtxRecord` is now generic over it's `data`, allowing a simplified
  `BinXmlOutput` trait.

## [0.4.2 - 2019-09-05]

### Added

- `--separate_json_attributes` to allow producing a flat JSON structure.

### Changed

- updated deps.

## [0.4.0 - 2019-06-01]

File output is now supported by `evtx_dump`

### Added

- `--output` to allow writing to files, `--no-confirm-overwrite` to allow binary
  to overwrite existing files.

### Changed

- Logs are now printed to stderr instead of stdout
- Failure exit code is now `1` instead of `-1`
- Some of the structs used in parsing evtx have been moved to
  [`winstructs`](https://github.com/omerbenamram/winstructs)

## [0.3.3] - 2019-05-23

### Fixed

- A sneaky dbg! print found it's way into the release, added
  `#![deny(clippy:dbg_macro)]` to ensure this won't happen again.

## [0.3.2] - 2019-05-20

### Changed

- `EvtxParser::from_read_seek` is now public.
- updated deps.

## [0.3.1] - 2019-05-19

Implemented Ansi codecs!

### Added

- `--ansi-codec` to control the codec that will be used to decode ansi encoded
  strings inside the document.

### Fixed

- Parser will now print nicer messages when passed non-evtx files.

## [0.3.0] - 2019-05-14

This is a minor release due to the removal of `failure`.

### Added

- `--backtraces` to control backtraces in errors
- `-v, -vv, -vv` to control trace output in `evtx_dump`.

### Changed

- All errors in the crate are all of a uniform `evtx::err::Error` type. Errors
  are implemented with `snafu`, and are std compatible. In addition, errors now
  all contain backtraces.

### Fixed

- Parser will now correctly parse files which refer to binxml fragments as sized
  values. (#33)

## [0.2.6] - 2019-05-09

### Fixed

- Parser is less strict with samples that contain multiple EOF markers (inside
  nested XML fragments)

## [0.2.5] - 2019-05-03

This version is the first .2 version to have python support!

### Added

- `IntoIterChunks` for owned iteration over the chunks.

## [0.2.4] - 2019-05-01

### Added

- `--no-indent` flag for xml and json
- `--dont-show-record-number` to avoid printing records number.
- `-o jsonl` for JSON lines output (same as
  `-o json --no-indent --dont-show-record-number`).

### Fixed

- Parser is less strict in dirty samples which contain some amount of corrupted
  binxml data, and will try to recover the record.

- Don't unwrap on empty binxmlname elements.

## [0.2.2] - 2019-04-29

### Added

- Performance improvements. Parser should be ~15% faster (thanks @ohadravid)
- `--validate-checksums` flag to optionally be strict about checksum checks for
  chunk headers.

### Fixed

- Fixed missing data when parsing `StringArray` nodes. (thanks @ohadravid)
- Samples containing empty chunks (thanks @ohadravid)

## [0.2.1] - 2019-04-21

### Changed

- More API is now public, for use by library authors who want access to lower
  level primitives and types.

## [0.2.0] - 2019-04-20

This release contains some minor breaking changes to the API.

### Added

- Added JSON output support! JSON support is powered by serde and is zero-copy!
  This means there isn't much performance difference between the XML output and
  the JSON output.

- The deserializer is now lazy (thanks @ohadravid !). This will allow to perform
  some filtering on records based on their metadata before serializing them to
  save time.

### Changed

- Changed parallel iteration to rely only on `ParserSettings`, so
  `.parallel_records` has been removed.
- `EvtxParser` now needs to be mutable when deserializing records.
- When outputting target as XML, inner xml strings will be escaped, when using
  JSON, they will not be escaped.

### Fixed

- Parser will now coerce values of booleans which are not zero or one to true.

## [0.1.9] - 2019-04-19

### Added

- Now supporting `SystemTime`, floating types, and all numerical array types.

### Fixed

- strip nuls from ascii strings as well.

### Changed

- Now using `quick-xml`, which microbenchmarks show that is about 15-20% faster
  than `xml-rs`.

## [0.1.8] - 2019-04-18

### Fixed

- Removed trailing nul terminators from all strings.

### Changed

- Changed hex formatting padding.
- Changed binary output formatting to hexdump.

## [0.1.7] - 2019-04-18

### Fixed

- Range error when reading last chunk (#2)

### Changed

- Parser will now try to read more records even when surpassing the declared
  chunk number.

## [0.1.6] - 2019-04-13

### Fixed

- Fixed missing xml close tag (#1)

### Changed

- Removed `.unwrap()` from xml parsing code.

## [0.1.5] - 2019-04-02

### Added

- renamed associated binary to `evtx_dump`

### Fixed

- changed `assert_eq` to `debug_assert_eq`, to ensure the library won't crash in
  FFI.

## [0.1.4] - 2019-04-01

### Fixed

- A regression introduced from [#6](https://github.com/omerbenamram/evtx/pull/6)
  for files with a single chunk.

## [0.1.3] - 2019-04-01

### Changed

- Removed some uses on `.unwrap()` inside the records iterator, to communicate
  errors better.

### Fixed

- A bug with files that have only a single chunk failing at the end.

## [0.1.2] - 2019-03-31

### Added

- Multithreading support via rayon

### Changed

- Removed unsafe memory mapping code, use generics instead.

### Fixed

## [0.1.1] - 2019-03-30

### Added

### Changed

- Fixed a bug with chunk iteration

### Fixed

- Fixed a bug with chunk iteration

## [0.1.0] - 2019-03-30

Initial Release
