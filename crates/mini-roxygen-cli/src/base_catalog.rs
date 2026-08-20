//! Static S3 generic facts for the supported R minor releases.
//!
//! The catalog is deliberately kept in the CLI.  The core crate only knows
//! how to ask a provider for a positive fact and does not know about R
//! installations or release policy.

/// The R minor releases for which this CLI has an explicitly reviewed base
/// catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportedRMinor {
    R4_5,
    R4_6,
}

/// Oracle provenance retained next to the catalog so changes can be audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OracleProvenance {
    pub(crate) minor: SupportedRMinor,
    pub(crate) patch: &'static str,
    pub(crate) expression: &'static str,
}

pub(crate) const ORACLES: [OracleProvenance; 2] = [
    OracleProvenance {
        minor: SupportedRMinor::R4_5,
        patch: "4.5.3",
        expression: "sort(unique(tools:::.get_S3_generics_in_base()))",
    },
    OracleProvenance {
        minor: SupportedRMinor::R4_6,
        patch: "4.6.1",
        expression: "sort(unique(tools:::.get_S3_generics_in_base()))",
    },
];

// Generated from both oracle runs above. The 4.5.3 and 4.6.1 sets are
// identical (189 names), so both supported minors intentionally share this
// immutable slice while retaining separate provenance.
const BASE_S3_GENERICS: &[&str] = &[
    "!",
    "!=",
    "$",
    "$<-",
    "%%",
    "%*%",
    "%/%",
    "&",
    "*",
    "+",
    "-",
    "/",
    "<",
    "<=",
    "==",
    ">",
    ">=",
    "@",
    "@<-",
    "Arg",
    "Complex",
    "Conj",
    "Im",
    "Math",
    "Mod",
    "Ops",
    "Re",
    "Summary",
    "[",
    "[<-",
    "[[",
    "[[<-",
    "^",
    "abs",
    "acos",
    "acosh",
    "all",
    "all.equal",
    "any",
    "anyDuplicated",
    "anyNA",
    "aperm",
    "as.Date",
    "as.POSIXct",
    "as.POSIXlt",
    "as.array",
    "as.call",
    "as.character",
    "as.complex",
    "as.data.frame",
    "as.double",
    "as.environment",
    "as.expression",
    "as.function",
    "as.integer",
    "as.list",
    "as.logical",
    "as.matrix",
    "as.null",
    "as.numeric",
    "as.raw",
    "as.single",
    "as.table",
    "as.vector",
    "asin",
    "asinh",
    "atan",
    "atanh",
    "by",
    "c",
    "cbind",
    "ceiling",
    "chol",
    "chooseOpsMethod",
    "close",
    "conditionCall",
    "conditionMessage",
    "cos",
    "cosh",
    "cospi",
    "cummax",
    "cummin",
    "cumprod",
    "cumsum",
    "cut",
    "determinant",
    "diff",
    "digamma",
    "dim",
    "dim<-",
    "dimnames",
    "dimnames<-",
    "droplevels",
    "duplicated",
    "exp",
    "expm1",
    "floor",
    "flush",
    "format",
    "gamma",
    "getDLLRegisteredRoutines",
    "is.array",
    "is.finite",
    "is.infinite",
    "is.matrix",
    "is.na",
    "is.na<-",
    "is.nan",
    "is.numeric",
    "is.unsorted",
    "isSymmetric",
    "julian",
    "kappa",
    "labels",
    "length",
    "length<-",
    "lengths",
    "levels",
    "levels<-",
    "lgamma",
    "log",
    "log10",
    "log1p",
    "log2",
    "matrixOps",
    "max",
    "mean",
    "merge",
    "min",
    "months",
    "mtfrm",
    "nameOfClass",
    "names",
    "names<-",
    "nchar",
    "open",
    "plot",
    "pretty",
    "print",
    "prod",
    "qr",
    "quarters",
    "range",
    "rbind",
    "rep",
    "rep.int",
    "rep_len",
    "rev",
    "round",
    "row.names",
    "row.names<-",
    "rowsum",
    "scale",
    "seek",
    "seq",
    "seq.int",
    "sequence",
    "sign",
    "signif",
    "sin",
    "sinh",
    "sinpi",
    "solve",
    "sort",
    "sort_by",
    "split",
    "split<-",
    "sqrt",
    "subset",
    "sum",
    "summary",
    "t",
    "tan",
    "tanh",
    "tanpi",
    "toString",
    "transform",
    "trigamma",
    "trunc",
    "truncate",
    "unique",
    "units",
    "units<-",
    "unlist",
    "weekdays",
    "with",
    "within",
    "xtfrm",
    "|",
];

pub(crate) fn catalog_for(minor: SupportedRMinor) -> &'static [&'static str] {
    debug_assert!(ORACLES.iter().any(|oracle| oracle.minor == minor));
    match minor {
        SupportedRMinor::R4_5 | SupportedRMinor::R4_6 => BASE_S3_GENERICS,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{BASE_S3_GENERICS, ORACLES, SupportedRMinor, catalog_for};

    #[test]
    fn catalog_is_sorted_unique_and_has_the_oracle_cardinality() {
        assert_eq!(BASE_S3_GENERICS.len(), 189);
        assert!(BASE_S3_GENERICS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            BASE_S3_GENERICS
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            189
        );
    }

    #[test]
    fn both_minor_mappings_retain_independent_oracle_provenance() {
        assert_eq!(ORACLES[0].minor, SupportedRMinor::R4_5);
        assert_eq!(ORACLES[0].patch, "4.5.3");
        assert_eq!(ORACLES[1].minor, SupportedRMinor::R4_6);
        assert_eq!(ORACLES[1].patch, "4.6.1");
        assert_eq!(
            catalog_for(SupportedRMinor::R4_5),
            catalog_for(SupportedRMinor::R4_6)
        );
    }
}
