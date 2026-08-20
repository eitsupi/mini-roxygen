#!/usr/bin/env Rscript

# Runs R's generation-header wrapping oracle over one input per line.

args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 1L) {
  cat("ERROR expected exactly one cases file path\n")
  quit(status = 2L)
}

inputs <- readLines(args[[1L]], warn = FALSE, encoding = "UTF-8")
for (index in seq_along(inputs)) {
  cat("CASE ", index, "\n", sep = "")
  lines <- strwrap(
    inputs[[index]],
    initial = "% ",
    prefix = "%   ",
    width = 80
  )
  for (line in lines) {
    cat("LINE ", line, "\n", sep = "")
  }
  cat("END\n")
}
cat("STATUS ok\n")
