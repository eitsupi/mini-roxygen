# mini-roxygen

`mini-roxygen` is a small, static implementation of a subset of
[roxygen2](https://roxygen2.r-lib.org/).
The `roxy` CLI reads R source files and roxygen comments, then generates Rd
documentation and a NAMESPACE file.
It does not evaluate R code, load the package, or compile a native
extension.

## Motivation

roxygen2 is powerful and is the standard way to document an R package. That
power comes from evaluation: to describe an object accurately, roxygen2 loads
the package it is documenting.

For a package with compiled code, loading the package may require building its
native code first. Regenerating the Rd files for a package written partly in
C++ or Rust can therefore require a full compile, as well as a working
toolchain, even for a documentation-only change.

The direct motivation was [r-polars](https://github.com/pola-rs/r-polars):
roxygen2 cannot produce a single Rd file there until its Rust code has
compiled, and without a warm cache, that compile alone easily takes twenty
minutes.

mini-roxygen avoids that cost by trading away some coverage. It supports a
minimal subset of roxygen comments, derives everything statically, and never
loads or compiles the package. Some documentation that depends on evaluation
is out of reach, but Rd generation itself takes only as long as the required
static processing.

## Status

This is an early release: the command surface is small and the supported
subset is still being refined. Generated files should be treated as build
artifacts to review. Byte-for-byte compatibility with roxygen2 is not
guaranteed.

## Coverage

| roxygen2 feature                              | mini-roxygen                                         |
| --------------------------------------------- | ---------------------------------------------------- |
| DESCRIPTION defaults                          | Supported                                            |
| Package documentation (`_PACKAGE`)            | Supported                                            |
| S3 methods and generic discovery              | Supported (reads installed packages)                 |
| Inheritance within the package                | Supported                                            |
| Inheritance from installed packages           | Supported (needs an R library path)                   |
| Documentation tags                            | Partial (a fixed set listed below)                   |
| NAMESPACE directives                          | Partial (no `@evalNamespace`)                        |
| `@inheritParams` selectors                    | Partial (`name` and `-name` only)                    |
| Markdown to Rd                                | Partial (no block quotes or raw HTML)                |
| Raw Rd macros                                 | Partial (`\eqn`, `\deqn`, some zero-argument macros) |
| S4 tags                                       | Partial (static directives, no class loading)        |
| S7                                            | Partial (literal `new_class()` definitions)          |
| S3 registration helpers                       | Partial (signatures are configurable)                |
| Inline `` `r ` `` expressions                 | Partial (substitutions are configurable)             |
| Data objects                                  | Partial (no generated `@format`)                     |
| Repeated scalar tags                          | Partial (one value per topic, no concatenation)      |
| R6                                            | Not supported                                        |
| `@eval`, `@template`, `@includeRmd`, `\Sexpr` | Not supported                                        |

[Compatibility with roxygen2](#compatibility-with-roxygen2) explains each
limit. Two of them are lifted by [configuration](#configuration).

## Usage

```text
roxy doc [OPTIONS] [PACKAGE_PATH]
```

`PACKAGE_PATH` defaults to the current directory.

The package must contain a `DESCRIPTION` file. An `R/` directory is optional.
When it is absent, the source set is empty. When present, the command reads the
`.R` files directly under `R/`, then writes `man/*.Rd` plus `NAMESPACE`. It
does not use `.Rbuildignore` to exclude source files.

Generated files carry the exact `mini-roxygen (roxygen2 compatible)` ownership
marker. Existing files with a recognized generated marker may be replaced or
left unchanged. A file without such a marker is treated as hand-written and is
not overwritten. A run with error diagnostics does not proceed to the
output-writing phase. Warnings and informational diagnostics are reported but
do not by themselves make the run fail.

### External R packages

R libraries serve two purposes: resolving S3 generics and inheriting
documentation from other packages with `@inherit`, `@inheritParams`, and
`@inheritSection`.
External inheritance is enabled only when at least one library path is
supplied. Without one, external inheritance remains disabled, and requests
that require it are diagnosed. Two options are available for supplying library
paths. Use either one.

`--r-lib-path PATH` is repeatable. Paths are searched in the order given.
Each value specifies exactly one path. Paths containing spaces are
supported.

```sh
roxy doc --r-lib-path /opt/R/library --r-lib-path /usr/local/R/library path/to/package
```

`--r-lib-paths PATH_LIST` takes one OS-native path-list value, such as the
result of R's own library-path calculation:

```sh
roxy doc --r-lib-paths \
  "$(Rscript -e 'cat(.libPaths(), sep=.Platform$path.sep)')" path/to/package
```

Only the supplied paths are used. R/Rscript, `R_LIBS`, `.libPaths()`, and other
automatic environment or installation searches are not consulted. A missing
optional metadata file is treated as local-only information, while malformed
metadata is reported as a warning.

The supported catalog versions are R 4.5 and R 4.6. The catalog is selected
based on the major.minor version in `base/DESCRIPTION`, so a 4.5.x
installation uses the 4.5 catalog and a 4.6.x installation uses the 4.6
one.
Patch releases use the catalog for their corresponding major.minor version.
If no library path is supplied, `base` is missing, or the detected version
is unknown, older, newer, or unparseable, the command warns and uses R 4.6
semantics as a fallback. The warning includes the detected version when one is
available, so the fallback stays visible in automated builds.

## Compatibility with roxygen2

The implementation is guided by roxygen2 8.1.0. Byte-for-byte equivalence
with roxygen2 is not part of the compatibility contract.

The intended compatibility boundary is semantic: generated Rd should be
accepted by R's `tools::parse_Rd`, and generated NAMESPACE files should be
accepted by R's NAMESPACE parser with the intended directive meaning.

Headers, source-reference comments, directive ordering, whitespace, section
boundaries, and `importFrom` line wrapping may differ at the byte level.
Output whitespace is determined by the AST and renderer and is not part of the
semantic compatibility guarantee.

### Supported tags

The documentation model supports the usual scalar and structured Rd fields:
`@title`, `@description`, `@details`, `@return`/`@returns`, `@seealso`,
`@references`, `@note`, `@format`, `@source`, `@author`, `@param`, `@name`,
`@rdname`, `@aliases`, `@keywords`, `@examples`, `@examplesIf`, `@usage`,
`@section`, `@order`, `@method`, `@noRd`, `@inherit`, `@inheritParams`, and
`@inheritSection`. Multiple contributions are merged with source-aware
diagnostics for conflicts, missing parameters, cycles, and ambiguous
identities.

The NAMESPACE subset includes:

- `@export`, `@exportS3Method`, `@import`, and `@importFrom`
- `@rawNamespace` and `@useDynLib`
- `@exportPattern`, `@exportClass`, `@exportMethod`, `@importClassesFrom`, and
  `@importMethodsFrom`.

The S4-related tags produce static NAMESPACE directives. They do not load R
classes or inspect S4 method tables. Ordinary documentation can be attached to
statically parseable R source, but runtime-generated R6 objects and methods are
not discovered by loading a package. A minimal S7 subset recognizes literal
`new_class()` definitions with a direct `constructor = function(...)` argument.
These signatures are propagated through simple aliases. S7 generics, unions,
multi-dispatch, method metadata, properties, and runtime introspection are not
supported.

Markdown is enabled for every documentation block. The supported conversion
covers ordinary paragraphs, emphasis, strong text, links, inline code, lists,
tables, and fenced or indented code blocks within the implemented Rd subset.
Level-1 Markdown headings are flattened into prose with a warning. Use an
explicit `@section Title:` contribution when named section structure is needed.
Level-2 and deeper headings become Rd subsections. Markdown links to local or
external help topics are retained as Rd links when their target can be resolved
or checked.

### Inheritance

Inheritance within the package is resolved statically, including recursive
parameter inheritance.

`@inherit`, `@inheritParams`, and `@inheritSection` can also inherit from
topics in installed packages.
The donor's help database is read and converted into the internal Rd model.
Parameters, sections, and prose are projected into the inheriting topic.
Donor-relative links are qualified with the donor's package while that context
is still known. Every inheritable component is covered: parameters, return,
title, description, details, `@seealso` content, sections, references, examples,
and author.

External lookup remains disabled until a library path is supplied. While it is
disabled, a request that would need it produces an
`external-inheritance-disabled` warning
naming the topic, so a missing configuration does not silently drop
documentation. See [External R packages](#external-r-packages) for the
options that supply the paths.

### One value per field

Each scalar prose field can have only one value per topic, including `@seealso`,
`@references`, `@note`, and `@author`. This is a compatibility boundary:
implicit concatenation across repeated tags is not performed.

Put related entries in one Markdown body, usually a paragraph or a Markdown
list, instead of repeating the tag. The same rule applies when blocks are
merged with `@rdname`. A repeated valid value produces a source-aware
`DuplicateTag` error, and the first value is retained while diagnostics are collected.
Empty or invalid tags are reported as parse diagnostics and do not
consume the slot. `@seealso NULL` suppresses package fallback documentation.
It does not erase an explicit value from another block.

`@examples` and `@examplesIf` share one topic-wide slot and are likewise not
concatenated. When an examples section needs multiple parts or conditions, put
them in one body with blank lines, comments, or an explicit R `if` statement.

### Static subsets

Some inputs are accepted only within a static subset.

**`Authors@R`** accepts statically parseable `person()` calls and vectors of
`person()` calls, with a restricted argument and string-escape grammar. The
generated author sections use recognized role codes. Unsupported forms are
diagnosed rather than evaluated.

**Inline code** is classified syntactically. A parseable single R expression is
emitted as `\code`. Code that cannot be classified safely is emitted as `\verb`.
The source spelling of generated usage, including defaults and multiline
expressions, is retained rather than evaluated.

**Raw Rd** support is intentionally limited. Equation macros that can be
isolated safely from the Markdown event stream are represented structurally
in the one- and two-argument forms of `\eqn` and `\deqn`. The zero-argument
prose macros `\R`, `\dots`, `\ldots`, `\cr`, and `\sspace` are also recognized.
Every other raw Rd macro, `\tab` included, and any malformed or overlapping
equation input produces a source-aware error and prevents that topic from
being generated. CLI processing continues to collect diagnostics from other
topics and exits nonzero.

**`@inheritParams`** supports the narrow `name` and `-name` selector forms. The
richer roxygen2 selection tail is not implemented: unsupported selector syntax
produces an `unsupported-selection` diagnostic and that inheritance request is
not applied.

**Repeated inheritance requests.** After targets are resolved to semantic donor
identities, each semantically identical `@inherit`, `@inheritParams`, or
`@inheritSection` request after the first produces one source-aware warning.
Section titles use the same formatting-insensitive semantic key as
section lookup, and only the first request is resolved. Field lists are compared
as sets. Parameter selector order remains significant where it can change the
selected result. Requests with different selections or donors retain their
original order so fallback and parameter union behavior are preserved. `NULL`
inheritance suppression keeps its topic-wide meaning.

**Namespace names** are decoded and validated before rendering. Non-syntactic
or reserved names are automatically quoted, directives are deduplicated and
sorted by rendered spelling, and `importFrom` names are merged per package.
The original spelling supplied by the author, such as the choice of single
quotes, is not preserved. Decoded names are rendered with the canonical
double-quote spelling when quoting is required. The output's physical wrapping
is a rendering choice.

**DESCRIPTION** supplies package documentation defaults such as title,
description, links, and authors when a package topic does not override or
suppress them. `Encoding` is accepted only when it is UTF-8. Roxygen and
markdown settings in DESCRIPTION do not switch the parser mode. Defaults are
applied after complete topic assembly, so explicit values win regardless of
block order. A `NULL` value contributed by a block suppresses the
corresponding fallback, but does not erase an explicit value from another block.
Multiple explicit single-value contributions are errors. `Collate` fields do not
reorder the source files. Their presence is retained only for the static
namespace and S3 ordering checks that need it.

**Data-object topics** receive static `\docType{data}`, usage, and `datasets`
keyword output. The automatic format description that roxygen2 obtains by
evaluating an object is not generated. Without an explicit format, inherited
format, or `@format NULL`, mini-roxygen emits a `missing-data-format` warning.

**S3 generic discovery** combines installed package metadata with a static base
catalog checked against R 4.5.3 and R 4.6.1. The catalog resolves base
primitive, group, and ordinary generics even when a standard installation has
no `base/Meta/nsInfo.rds`. The provider reads known S3 registrations
from `base` and `recommended` packages, and from packages named by the target
package's `Depends` and `Imports` fields.

For dotted method names, candidate generic prefixes are checked left to right
and the first, shortest proven generic is selected. A package-local binding
always shadows provider metadata for that candidate. If no prefix can be proven
to be a generic, the result remains unresolved rather than inferring intent
from dotted spelling. Use an explicit `@method` or
`@exportS3Method` when you need to state intent directly.

**Mixed document types under one `@rdname`.** When package documentation and a
data-object contribution share one `@rdname`, roxygen2 handles the mixed values
in source order. mini-roxygen reports an explicit error instead, because a
likely typo should not be hidden by source-order recovery.

### Not supported

These constructs require evaluation or file inclusion: `@eval`, `@evalRd`,
`@evalNamespace`, `@template`, `@templateVar`, `@includeRmd`, and `\Sexpr`.
General inline R evaluation is also unsupported. Inline `` `Rd ` `` expressions and
executable R code blocks are not run.

These roxygen2 tags are not implemented: `@concept`, `@describeIn`,
`@docType`, `@example`, `@include`, `@inheritDotParams`, `@rawRd`, and
`@slot`. Using one produces an `unknown-tag` warning and the run continues, so
the omission is visible rather than silent. The tags listed under
[Supported tags](#supported-tags) are the ones the model accepts.

`@noMd` is diagnosed because Markdown is always enabled. `@md` is accepted only
as a redundant declaration of that mode.

Block quotes, thematic breaks, raw HTML, and other unsupported Markdown
constructs are diagnosed with source locations and recovered where possible.
These are ordinary limits of the underlying Markdown conversion, not a claim
that roxygen2 itself accepts every such construct without restriction.

## Configuration

Inline `` `r ` `` expressions are never evaluated. Instead, a substitution
table provides their results directly. S3 registrations made through a helper
function can't be proven statically on their own, so declaring the helper's
signature makes them visible to the static analysis.

Both live in `mini-roxygen.toml`, read independently of DESCRIPTION. The file
is searched for at the package root only, not in parent or nested
directories. The schema is strict: it may contain only the
`[inline-r.substitutions]` and `[s3]` tables described below.

### Inline R substitutions

```toml
[inline-r.substitutions]
'lifecycle::badge("stable")' = '\strong{[Stable]}'
'pkg::version()' = '0.1.0'
```

Keys are source spellings and every value must be a quoted TOML string.

The Markdown code-span parser applies its normal outer-whitespace rule before
lookup. After that boundary handling, each key must match the inline `` `r ` ``
expression exactly, including internal source spelling and arguments. Internal
whitespace is not normalized, because whitespace inside an R string, raw
string, or comment can change its meaning.

Each value is the final Rd fragment: not Markdown, and not an R expression. Use
Rd markup such as `\emph{...}` and `\code{...}` in replacement values. Values
are parsed and writer-validated before the table is used. An empty string is a
valid substitution and intentionally emits no fragment. Invalid entries are
diagnosed together and are not partially applied.

The nine badge spellings from the [lifecycle](https://lifecycle.r-lib.org/)
package are built in, so `` `r lifecycle::badge("stable")` `` resolves to its Rd
badge without configuration.

User entries override built-ins with the same key. A user entry that is never
encountered produces an unused-substitution warning. Built-in entries do not.
This mechanism is static lookup, not R evaluation: `` `r expression` `` is replaced
only when an exact configured key exists.

### Static S3 registrars

```toml
[[s3.registrars]]
function = "register_s3_method"
arguments = ["class", "generic", "method"]
```

One registrar signature is built in and always enabled: vctrs'
[`s3_register(generic, class, method)`](https://vctrs.r-lib.org/reference/s3_register.html),
the helper most commonly vendored into packages for conditional registration.
Configured tables add exact bare or qualified callees. Each `arguments` array
must contain `generic` and `class` exactly once and may contain `method` once.
Argument names are matched exactly, without R's partial argument matching.

The generic and class must be statically known string literals to establish a
registration fact. If the method target is omitted or `NULL`, it defaults to
`generic.class`.
A bare symbol names a local method, and function or computed targets do not
create a named method block.

Registration facts provide Rd method metadata only: they never add NAMESPACE
directives. A documented target matched by a registration inherits its method
metadata even without `@exportS3Method NULL`, but it must still carry an export
tag or that NULL suppression, otherwise a warning is emitted. An
`@exportS3Method NULL` tag without resolvable registration metadata or an
explicit `@method` is an error.

Only registrar calls whose generic and class are statically proven string
literals are extracted. Dynamic runtime registrations are not evaluated. They
produce an informational diagnostic, but no registration fact, no Rd or
NAMESPACE directive, and no fatal error. Decodable names and computed-expression
arguments are treated as dynamic runtime values. Undecodable names, other
statically non-string values, and undecodable string literals are invalid
registrar calls.
Ambiguous or malformed calls are diagnosed without guessing.

## Diagnostics

R source is parsed statically, and documentation blocks are associated with
top-level source expressions without executing them.

Validation is strict. Tag names must begin with an ASCII letter. Required
values, singleton fields, malformed word lists, `@section` title/body
separators, and `@order` integer values are all checked. `@section` requires a
colon separating its title from its body. Ordered contributions sort ascending,
with missing orders last and source order breaking ties.

Diagnostics retain the originating source file and byte range whenever a source
location exists. This makes malformed tags, unsupported evaluation, inheritance
failures, namespace validation errors, and Markdown recovery diagnostics
actionable without requiring the user to determine which source block produced
the affected generated file.

The inline-R diagnostic codes are `undefined-inline-r-substitution` (error),
`invalid-inline-r-substitution` (error), `unused-inline-r-substitution`
(warning), and `unsupported-inline-r` (error).

## License

MIT. See [LICENSE](https://github.com/eitsupi/mini-roxygen/blob/main/LICENSE).
Third-party licensing and attribution details are in
[THIRD_PARTY_NOTICES.md](https://github.com/eitsupi/mini-roxygen/blob/main/THIRD_PARTY_NOTICES.md).
