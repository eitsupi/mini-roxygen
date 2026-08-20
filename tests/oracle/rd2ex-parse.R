#!/usr/bin/env Rscript

# Extract one Rd file with tools::Rd2ex and parse the resulting R source.
#
# The output contract matches parse-rd.R so the Rust test helper can reuse the
# same skip/require behavior for environments without R.
args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 1L) {
  cat("ERROR expected exactly one Rd file path\n")
  cat("STATUS failed\n")
  quit(status = 2L)
}

path <- args[[1L]]
findings <- character()

flatten <- function(text) {
  gsub("[\r\n]+", " \\\\n ", trimws(text))
}

record <- function(kind, condition) {
  findings <<- c(findings, paste(kind, flatten(conditionMessage(condition))))
}

result <- withCallingHandlers(
  tryCatch(
    {
      extracted <- tempfile(fileext = ".R")
      on.exit(unlink(extracted), add = TRUE)
      tools::Rd2ex(path, out = extracted, encoding = "UTF-8")
      parse(file = extracted, encoding = "UTF-8")
      TRUE
    },
    error = function(condition) {
      record("ERROR", condition)
      FALSE
    }
  ),
  warning = function(condition) {
    record("WARNING", condition)
    invokeRestart("muffleWarning")
  }
)

for (finding in findings) {
  cat(finding, "\n", sep = "")
}

if (isTRUE(result) && length(findings) == 0L) {
  cat("STATUS ok\n")
  quit(status = 0L)
}

cat("STATUS failed\n")
quit(status = 1L)
