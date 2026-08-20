# A multiline signature with a default block, grouped parameters, and dots.

#' Multiline signature fixture.
#'
#' This description uses **Markdown** even though the package config disables it.
#' @param x,y The values to combine.
#' @param ... Additional arguments.
#' @return The combined value.
#' @export
signature_fixture <- function(
  x,
  y = {
    x + 1
  },
  ...
) {
  x + y
}
