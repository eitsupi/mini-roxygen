#!/usr/bin/env Rscript

args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 2L) {
  cat("ERROR expected package name and package library path\n")
  cat("STATUS failed\n")
  quit(status = 2L)
}

package <- args[[1L]]
package.lib <- args[[2L]]
namespace <- file.path(package.lib, package, "NAMESPACE")
findings <- character()

record <- function(kind, condition) {
  message <- gsub("[\\r\\n]+", " \\\\n ", trimws(conditionMessage(condition)))
  findings <<- c(findings, paste(kind, message))
}

run <- function(expression) {
  withCallingHandlers(
    tryCatch({
      force(expression)
      TRUE
    }, error = function(condition) {
      record("ERROR", condition)
      FALSE
    }),
    warning = function(condition) {
      record("WARNING", condition)
      invokeRestart("muffleWarning")
    }
  )
}

syntax_ok <- run(base::parse(namespace, keep.source = TRUE))
namespace_ok <- run(base::parseNamespaceFile(package, package.lib, mustExist = TRUE))

for (finding in findings) {
  cat(finding, "\n", sep = "")
}

if (isTRUE(syntax_ok) && isTRUE(namespace_ok) && length(findings) == 0L) {
  cat("STATUS ok\n")
  quit(status = 0L)
}

cat("STATUS failed\n")
quit(status = 1L)
