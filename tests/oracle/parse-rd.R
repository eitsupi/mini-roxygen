#!/usr/bin/env Rscript

# Reads one Rd file and reports whether R's own parser accepts it.
#
# tools::parse_Rd() signals most malformed input as a *warning* and still
# returns a value, so a check that only guards against errors passes on Rd
# that R would complain about. Warnings are therefore failures here, and the
# caller decides whether any are tolerable for a given case.
#
# Output is one line per finding on stdout:
#   ERROR <message>
#   WARNING <message>
# followed by a final status line:
#   STATUS ok | STATUS failed
# Newlines inside a message are replaced so each finding stays on one line.

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
      tools::parse_Rd(path, fragment = FALSE, encoding = "UTF-8")
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
