# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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






