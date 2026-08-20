#!/usr/bin/env Rscript

args <- commandArgs(trailingOnly = TRUE)
if (length(args) == 0L) {
  cat("ERROR expected at least one Rd file path\n")
  cat("STATUS failed\n")
  quit(status = 2L)
}

findings <- character()
record <- function(kind, path, condition) {
  message <- gsub("[\\r\\n]+", " \\\\n ", trimws(conditionMessage(condition)))
  findings <<- c(findings, paste(kind, path, message))
}

for (path in args) {
  withCallingHandlers(
    tryCatch({
      tools::parse_Rd(path, fragment = FALSE, encoding = "UTF-8")
    }, error = function(condition) {
      record("ERROR", path, condition)
    }),
    warning = function(condition) {
      record("WARNING", path, condition)
      invokeRestart("muffleWarning")
    }
  )
}

for (finding in findings) {
  cat(finding, "\n", sep = "")
}

if (length(findings) == 0L) {
  cat("STATUS ok\n")
  quit(status = 0L)
}

cat("STATUS failed\n")
quit(status = 1L)
