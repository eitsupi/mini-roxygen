# This file proves ordinary documentation, implicit intro fields, Markdown, and all major Rd prose sections.
# The implicit intro below exercises roxygen's title/description/details convention.

#' Basic fixture title.
#'
#' This is the **description** with `inline_code` and a <https://example.com/guide> link.
#'
#' These details keep *emphasis* and explain the documented calculation.
#'
#' A second details paragraph makes the implicit intro genuinely multi-paragraph.
#' @param x, y The two values to add.
#' @return The combined value.
#' @examples
#' result <- basic(1, 2)
#' @section More information: This named section is part of the fixture.
#' @note This note is retained in the generated Rd.
#' @references A fixture reference.
#' @seealso The <https://stat.ethz.ch/R-manual/R-devel/library/base/html/base-package.html> base package.
#' @author The mini-roxygen maintainers.
#' @keywords fixtures functions
basic <- function(x, y) x + y
