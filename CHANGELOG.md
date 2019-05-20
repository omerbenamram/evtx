# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.2] - 2019-05-20

### Changed
- `EvtxParser::from_read_seek` is now public.
- updated deps.

## [0.3.1] - 2019-05-19

Implemented Ansi codecs! 

### Added
- `--ansi-codec` to control the codec that will be used to decode ansi encoded strings inside the document.

### Fixed
- Parser will now print nicer messages when passed non-evtx files.

## [0.3.0] - 2019-05-14

This is a minor release due to the removal of `failure`. 

### Added
- `--backtraces` to control backtraces in errors
- `-v, -vv, -vv` to control trace output in `evtx_dump`. 

### Changed
- All errors in the crate are all of a uniform `evtx::err::Error` type.
  Errors are implemented with `snafu`, and are std compatible.
  In addition, errors now all contain backtraces.  

### Fixed
- Parser will now correctly parse files which refer to binxml fragments as sized values. (#33)

## [0.2.6] - 2019-05-09

### Fixed
- Parser is less strict with samples that contain multiple EOF markers (inside nested XML fragments)

## [0.2.5] - 2019-05-03

This version is the first .2 version to have python support!

### Added
- `IntoIterChunks` for owned iteration over the chunks.

## [0.2.4] - 2019-05-01

### Added
- `--no-indent` flag for xml and json
- `--dont-show-record-number` to avoid printing records number. 
- `-o jsonl` for JSON lines output (same as `-o json --no-indent --dont-show-record-number`).

### Fixed
- Parser is less strict in dirty samples which contain 
  some amount of corrupted binxml data, and will try to recover the record.

- Don't unwrap on empty binxmlname elements.

## [0.2.2] - 2019-04-29

### Added
- Performance improvements. Parser should be ~15% faster (thanks @ohadravid)
- `--validate-checksums` flag to optionally be strict about checksum checks for chunk headers.

### Fixed
- Fixed missing data when parsing `StringArray` nodes. (thanks @ohadravid)  
- Samples containing empty chunks (thanks @ohadravid)

## [0.2.1] - 2019-04-21

### Changed
- More API is now public, for use by library authors who want access to lower level primitives and types.

## [0.2.0] - 2019-04-20

This release contains some minor breaking changes to the API.

### Added
- Added JSON output support!
  JSON support is powered by serde and is zero-copy! 
  This means there isn't much performance difference between the XML output and the JSON output.

- The deserializer is now lazy (thanks @ohadravid !).
  This will allow to perform some filtering on records based on their metadata before serializing them to save time. 

### Changed
- Changed parallel iteration to rely only on `ParserSettings`, so `.parallel_records` has been removed.
- `EvtxParser` now needs to be mutable when deserializing records.
- When outputting target as XML, inner xml strings will be escaped, when using JSON, they will not be escaped. 

### Fixed
- Parser will now coerce values of booleans which are not zero or one to true.  

## [0.1.9] - 2019-04-19

### Added
- Now supporting `SystemTime`, floating types, and all numerical array types.

### Fixed
- strip nuls from ascii strings as well.  

### Changed
- Now using `quick-xml`, which microbenchmarks show that is about 15-20% faster than `xml-rs`. 

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
- Parser will now try to read more records even when surpassing the declared chunk number.


## [0.1.6] - 2019-04-13

### Fixed
- Fixed missing xml close tag (#1) 

### Changed
- Removed `.unwrap()` from xml parsing code.

## [0.1.5] - 2019-04-02

### Added
- renamed associated binary to `evtx_dump`

### Fixed
- changed `assert_eq` to `debug_assert_eq`, to ensure the library won't crash in FFI.

## [0.1.4] - 2019-04-01

### Fixed
- A regression introduced from [#6](https://github.com/omerbenamram/evtx/pull/6) for files with a single chunk.

## [0.1.3] - 2019-04-01

### Changed
- Removed some uses on `.unwrap()` inside the records iterator, to communicate errors better.

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






