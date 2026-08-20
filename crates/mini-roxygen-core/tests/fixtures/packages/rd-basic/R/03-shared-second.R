# This file proves the second half of a cross-file merge, explicit usage, and an S3 method contribution.

#' @rdname shared
#' @usage shared_explicit(value, mode = "fast")
#' @param mode The mode for the explicit shared call.
shared_explicit <- function(value, mode = "slow") value

#' Print method fixture.
#'
#' This topic contributes a statically generated S3 method usage.
#' @method print fixture
print.fixture <- function(x, ...) x
