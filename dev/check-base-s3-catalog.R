#!/usr/bin/env Rscript

# Developer-only oracle checker. Run with either:
#   Rscript dev/check-base-s3-catalog.R --generate
#   Rscript dev/check-base-s3-catalog.R --check
# The checker intentionally compares sets, while the checked-in Rust slice
# keeps a deterministic byte-order sort for lookup and review.

args <- commandArgs(trailingOnly = TRUE)
mode <- if (length(args) == 0L) "--check" else args[[1L]]

oracle <- sort(unique(tools:::.get_S3_generics_in_base()))
if (identical(mode, "--generate")) {
  cat(oracle, sep = "\n")
  cat("\n")
  quit(status = 0L)
}
if (!identical(mode, "--check")) {
  stop("usage: check-base-s3-catalog.R [--generate|--check]")
}

catalog_path <- file.path("crates", "mini-roxygen-cli", "src", "base_catalog.rs")
source <- paste(readLines(catalog_path, warn = FALSE), collapse = "\n")
start <- regexpr("const BASE_S3_GENERICS:.*\\[", source)[[1L]]
end <- regexpr("\\];", substring(source, start), fixed = FALSE)[[1L]]
if (start < 1L || end < 1L) {
  stop("could not locate BASE_S3_GENERICS in the Rust source")
}
body <- substring(source, start, start + end - 1L)
quoted <- gregexpr('"[^"]*"', body, perl = TRUE)[[1L]]
if (quoted[[1L]] < 0L) {
  stop("the Rust catalog has no entries")
}
raw <- regmatches(body, list(quoted))[[1L]]
catalog <- sort(unique(gsub('^"|",?$', "", raw)))
if (!identical(catalog, oracle)) {
  missing <- setdiff(oracle, catalog)
  extra <- setdiff(catalog, oracle)
  stop(sprintf("catalog differs from this R oracle (missing: %s; extra: %s)",
               paste(missing, collapse = ", "), paste(extra, collapse = ", ")))
}
cat(sprintf("catalog matches %d names from %s\n", length(oracle), R.version.string))
