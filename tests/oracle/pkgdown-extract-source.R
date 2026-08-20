#!/usr/bin/env Rscript

# Portions of this file are adapted from pkgdown revision
# f69b62a7e74b58b42170284b6f3f56c674e7a3f8:
# https://github.com/r-lib/pkgdown/blob/f69b62a7e74b58b42170284b6f3f56c674e7a3f8/R/rd.R
# https://github.com/r-lib/pkgdown/blob/f69b62a7e74b58b42170284b6f3f56c674e7a3f8/R/package.R
#
# MIT License
#
# Copyright (c) 2025 pkgdown authors
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.

# Reproduces pkgdown 2.2.1.9000's extract_source() decision for one Rd file.
# The implementation intentionally mirrors pkgdown's extract_source() rather
# than checking the generated text with a Rust substring assertion.

args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 1L) {
  cat("ERROR expected exactly one Rd file path\n")
  quit(status = 2L)
}

path <- args[[1L]]
findings <- character()

tag <- function(x) {
  name <- attr(x, "Rd_tag")
  if (is.null(name)) {
    return()
  }
  gsub("\\", "tag_", name, fixed = TRUE)
}

set_class <- function(x) {
  structure(
    x,
    class = c(attr(x, "class"), tag(x), "tag"),
    Rd_tag = NULL,
    srcref = NULL,
    macros = NULL
  )
}

set_classes <- function(rd) {
  if (is.list(rd)) {
    rd[] <- lapply(rd, set_classes)
  }
  set_class(rd)
}

result <- tryCatch(
  {
    x <- set_classes(tools::parse_Rd(path, fragment = FALSE, encoding = "UTF-8"))
    nl <- vapply(x, function(node) inherits(node, "TEXT") && node == "\n", logical(1))
    comment <- vapply(x, function(node) inherits(node, "COMMENT"), logical(1))

    first_comment <- cumsum(!(nl | comment)) == 0
    lines <- as.character(x[first_comment])
    text <- paste0(lines, collapse = "")

    if (grepl("roxygen2", text)) {
      m <- gregexpr("R/[^ ]+\\.[rR]", text)
      sources <- regmatches(text, m)[[1L]]
      if (length(sources) > 0L && sources[[1L]] != "-1") {
        for (source in sources) {
          cat("SOURCE ", source, "\n", sep = "")
        }
      }
    }
    TRUE
  },
  error = function(condition) {
    findings <<- c(findings, conditionMessage(condition))
    FALSE
  }
)

if (!isTRUE(result)) {
  for (finding in findings) {
    cat("ERROR ", gsub("[\\r\\n]+", " ", finding), "\n", sep = "")
  }
  cat("STATUS failed\n")
  quit(status = 1L)
}

cat("STATUS ok\n")
