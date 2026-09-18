# Changelog

## [Unreleased]

### Fixed

- Correct the stated minimum supported Rust version from 1.88 to 1.89:
  `arity-parser` 0.5.1 resolves `smol_str` 0.3.6, so the 0.1.0 dependency
  graph already required Rust 1.89.

## [0.1.0] - 2026-08-20

### Added

- Add the `mini-roxygen-core` library and `roxy` CLI for statically analyzing
  R source and roxygen comments, then generating Rd documentation and a
  NAMESPACE file without evaluating or loading the package.
- Support the core roxygen2 subset for package metadata, documentation fields,
  inheritance, S3 and static S4/S7 directives, Markdown, raw Rd, and
  configurable inline-R substitutions.
- Require Rust 1.88 as the minimum supported Rust version. R is not a runtime
  dependency.
- Define semantic compatibility with roxygen2; byte-for-byte output
  compatibility is not guaranteed.
