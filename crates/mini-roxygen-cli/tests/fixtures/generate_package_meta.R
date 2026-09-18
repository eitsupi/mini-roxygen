# Generate deterministic synthetic installed-package metadata fixtures.
# Provenance: hand-constructed fictional package data generated locally with
# R 4.6.1; no installed package tree or network input is used.
#
# Usage:
#   Rscript generate_package_meta.R OUTPUT R_VERSION PRIORITY BUILT

args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 4L) {
  stop("usage: Rscript generate_package_meta.R OUTPUT R_VERSION PRIORITY BUILT")
}
stopifnot(as.character(getRversion()) == "4.6.1")

output <- args[[1L]]
r_version <- args[[2L]]
priority <- args[[3L]]
include_built <- identical(args[[4L]], "yes")

description <- c(
  Package = "fixturepkg",
  Version = "0.1.0",
  Title = "A synthetic package metadata fixture"
)
if (nzchar(priority)) {
  description <- c(description, Priority = priority)
}

metadata <- list(DESCRIPTION = description)
if (include_built) {
  metadata$Built <- list(
    R = structure(
      list(as.integer(strsplit(r_version, ".", fixed = TRUE)[[1L]])),
      class = c("R_system_version", "package_version", "numeric_version")
    ),
    Platform = "x86_64-pc-linux-gnu",
    Date = "2026-01-01 00:00:00 UTC",
    OStype = "unix"
  )
}
class(metadata) <- "packageDescription2"
saveRDS(metadata, output, compress = FALSE, version = 3L)
