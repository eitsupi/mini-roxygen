# This file proves an inline R Markdown conversion error rejects only its topic.

#' Failing Markdown fixture.
#'
#' This contains unsupported inline evaluation: `r 1 + 1`.
markdown_failing <- function() NULL
