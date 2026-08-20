# Third-party notices

mini-roxygen is distributed under the project license in `LICENSE`. The
following components and compatibility materials are acknowledged separately.
The MIT permission and warranty terms at the end of this file apply to the
listed MIT components; their copyright notices and source links identify the
corresponding material.

## lifecycle

The built-in lifecycle badge substitutions use the lifecycle stage vocabulary
and Rd badge conventions from the public lifecycle project:
<https://github.com/r-lib/lifecycle>. The relevant copyright is 2023
lifecycle authors. lifecycle is MIT licensed.

## roxygen2

The compatibility behavior and output conventions are informed by roxygen2,
and several compatibility fixtures exercise those conventions. The public
project is <https://github.com/r-lib/roxygen2>; the relevant copyright is 2023
roxygen2 authors. roxygen2 is MIT licensed. mini-roxygen does not promise
byte-for-byte compatibility with roxygen2.

## r-polars

The S7 documentation-shape fixture exercises a documentation pattern that is
also visible in the public r-polars documentation: an S7 class object aliased
to a documented binding, with the topic identity carried by `@rdname`,
`@aliases`, and `@order`. The bundled fixture uses its own identifiers and
prose; no r-polars source, naming, or wording is reproduced. The public project
is <https://github.com/pola-rs/r-polars>; the relevant copyright is 2024 polars
authors. r-polars is MIT licensed.

## pkgdown

`tests/oracle/pkgdown-extract-source.R` contains a small adaptation of
pkgdown's source-extraction decision. That file retains its complete in-file
MIT notice and source links; this notice is an additional index of the same
provenance. The relevant copyright is 2025 pkgdown authors. pkgdown is MIT
licensed. The public project is <https://github.com/r-lib/pkgdown>.

## MARC relator vocabulary

`crates/mini-roxygen-core/src/marc_roles.rs` records the 302-code compatibility
subset accepted by the R `utils::MARC_relator_db` validation path. The current
table is derived from the Library of Congress Linked Data Service relator
dataset, which is identified as public-domain data at
<https://id.loc.gov/about/> and
<https://id.loc.gov/vocabulary/relators.json>.

Every code in the table is present in that dataset. The labels for `mte` and
`pbd` follow the Library of Congress values (`metal engraver` and `publisher
director`). Three Library of Congress codes (`voc`, `waw`, and `wfw`) are
intentionally excluded because R 4.6.1 warns for them, preserving the
accepted-code compatibility subset.

For `mte` and `pbd`, R 4.6.1 displays `metal-engraver` and `publishing
director`, respectively, while this table uses the LC labels above. Exact
label-text parity with R is not promised.

The current labels and data are derived from the Library of Congress
public-domain dataset. R was used only to select the 302-code compatibility
subset; the historical R provenance does not change that stated LC data
source. R's `utils` package is licensed under GPL-2 or GPL-3; those historical
references are recorded at <https://www.gnu.org/licenses/old-licenses/gpl-2.0.html>
and <https://www.gnu.org/licenses/gpl-3.0.html>.

## MIT license terms

Copyright holders for the MIT components listed above retain their respective
copyright. Permission is hereby granted, free of charge, to any person
obtaining a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including without
limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom
the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
