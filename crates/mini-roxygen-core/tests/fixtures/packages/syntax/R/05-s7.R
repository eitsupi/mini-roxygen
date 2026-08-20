# Static S7 constructor and alias assignments exercise class metadata lookup.

RenderOptions <- S7::new_class(
  "RenderOptions",
  constructor = function(..., compact = TRUE) NULL
)

#' Synthetic renderer options.
#' @name RenderOptions
#' @param ... Additional constructor arguments.
#' @param compact Whether compact rendering is enabled.
NULL

#' @rdname RenderOptions
new_render_options <- RenderOptions

LabelStyle <- new_class(
  "LabelStyle",
  constructor = function(template, ..., separator = NULL) NULL
)

#' Synthetic label styling.
#' @name label_style_scheme
#' @param template The label template.
NULL

#' @rdname label_style_scheme
#' @aliases LabelStyle
#' @param separator The separator between label parts.
#' @order 0
new_label_style <- LabelStyle
