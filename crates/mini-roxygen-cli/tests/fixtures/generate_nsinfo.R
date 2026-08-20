# Generate the deterministic installed-package namespace fixture used by the
# CLI tests. Run with: Rscript generate_nsinfo.R nsinfo-positive.rds

args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 1L) {
  stop("usage: Rscript generate_nsinfo.R OUTPUT")
}

s3methods <- matrix(
  c("print", "foo", NA_character_, NA_character_,
    "+", "bar", NA_character_, NA_character_),
  ncol = 4L,
  byrow = TRUE
)
saveRDS(
  list(S3methods = s3methods),
  file = args[[1L]],
  version = 3L,
  compress = FALSE
)
