use std::collections::{BTreeMap, BTreeSet};

use crate::arity_adapter::can_parse_namespace_source;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::generation::render_namespace_header;
use crate::model::{NamespaceRequest, PackageModel};
use crate::source::{SourceMap, Span};
use crate::tags::{NamespaceTag, PlainText, TagOrigin};

use super::ir::{
    NamespaceBuildOutput, NamespaceDirective, NamespaceDirectiveKey, NamespaceObjectName,
    NamespacePackageName, NamespaceS3MethodArgument, NamespaceVerbatim, NonEmptyNamespaceNames,
};
use super::render::quote_name;
#[cfg(test)]
use super::s3::EmptyS3GenericProvider;
use super::s3::{S3Analyzer, S3ExportAnalysis, S3GenericProvider};

/// Builds NAMESPACE text without reading or writing files.
///
/// When `current_package` is `Some`, matching typed `import()` and
/// `importFrom()` requests are omitted. The opaque comma-bearing `@import`
/// form is passed through as written and is not filtered, matching upstream.
/// Its value is not interpreted, so the package it names cannot be established
/// without guessing. `None` leaves all imports eligible for output.
#[must_use]
#[cfg(test)]
pub fn build_namespace(
    package: &PackageModel,
    current_package: Option<&str>,
) -> NamespaceBuildOutput {
    build_namespace_inner(package, None, current_package, &EmptyS3GenericProvider)
}

/// Builds NAMESPACE text using caller-supplied S3 generic facts.
#[must_use]
#[cfg(test)]
pub fn build_namespace_with_provider(
    package: &PackageModel,
    current_package: Option<&str>,
    provider: &dyn S3GenericProvider,
) -> NamespaceBuildOutput {
    build_namespace_inner(package, None, current_package, provider)
}

/// Builds NAMESPACE text with source filenames available for temporal S3
/// analysis.
///
/// Cross-file alias order is accepted only when the source map proves the
/// default filename order. Same-file assignment spans remain usable in every
/// entry point.
#[must_use]
#[cfg(test)]
pub fn build_namespace_with_sources(
    package: &PackageModel,
    sources: &SourceMap,
    current_package: Option<&str>,
) -> NamespaceBuildOutput {
    build_namespace_inner(
        package,
        Some(sources),
        current_package,
        &EmptyS3GenericProvider,
    )
}

/// Builds NAMESPACE text with caller-supplied S3 generic facts.
#[must_use]
pub fn build_namespace_with_sources_and_provider(
    package: &PackageModel,
    sources: &SourceMap,
    current_package: Option<&str>,
    provider: &dyn S3GenericProvider,
) -> NamespaceBuildOutput {
    build_namespace_inner(package, Some(sources), current_package, provider)
}

fn build_namespace_inner(
    package: &PackageModel,
    sources: Option<&SourceMap>,
    current_package: Option<&str>,
    provider: &dyn S3GenericProvider,
) -> NamespaceBuildOutput {
    let mut diagnostics = Diagnostics::new();
    let mut normalized = Vec::new();
    let mut analyzer = S3Analyzer::new(package, sources, provider);
    let mut warned_s3_objects = BTreeSet::new();

    // Collect and validate each request in model order. Final output ordering
    // is independent of this order, but diagnostics retain source traversal.
    for request in &package.namespace {
        collect_request(
            request,
            current_package,
            &mut analyzer,
            &mut warned_s3_objects,
            &mut normalized,
            &mut diagnostics,
        );
    }

    // Equal directives are deduplicated.
    let mut deduplicated = BTreeMap::<NamespaceDirectiveKey, NamespaceDirective>::new();
    for directive in normalized {
        let key = directive.key();
        deduplicated.entry(key).or_insert(directive);
    }

    // ImportFrom requests need package-level unioning after exact-request
    // deduplication. Names are keyed by their rendered spelling because
    // roxygen2 quotes before sorting and deduplicating them. The decoded
    // spelling is retained as a tie-breaker so a future non-injective
    // quote_name implementation cannot silently drop a distinct name.
    let mut merged = BTreeMap::<String, (NamespacePackageName, BTreeSet<(String, String)>)>::new();
    let mut directives = Vec::new();
    for directive in deduplicated.into_values() {
        match directive {
            NamespaceDirective::ImportFrom { package, names } => {
                let entry = merged
                    .entry(package.0.clone())
                    .or_insert_with(|| (package.clone(), BTreeSet::new()));
                entry.1.extend(
                    names
                        .0
                        .into_iter()
                        .map(|name| (quote_name(name.as_str()), name.0)),
                );
            }
            other => directives.push(other),
        }
    }
    for (_, (package, names)) in merged {
        let names = names
            .into_iter()
            .map(|(_, name)| NamespaceObjectName::new(name).expect("merged name is non-empty"))
            .collect::<Vec<_>>();
        // Every input ImportFrom had a non-empty list, so this cannot become
        // empty during set union.
        directives.push(NamespaceDirective::ImportFrom {
            package,
            names: NonEmptyNamespaceNames::new(names).expect("merged import has a name"),
        });
    }

    // roxygen2 orders the fully rendered lines under LC_COLLATE=C, so order
    // the same thing it orders rather than the values behind it: quoting a
    // name changes where the line sorts. Rendering once up front keeps the
    // comparator from repeating that work.
    //
    // String comparison in Rust is byte-lexicographic, which is what
    // LC_COLLATE=C means for the text we emit. Do not "improve" this into a
    // locale-aware or case-insensitive comparison; the collation is the point.
    let mut rendered = directives
        .into_iter()
        .map(|directive| directive.render())
        .collect::<Vec<_>>();
    rendered.sort();

    let mut content = render_namespace_header();
    for text in rendered {
        content.push_str(&text);
        content.push('\n');
    }
    NamespaceBuildOutput {
        content,
        diagnostics,
    }
}

fn collect_request(
    request: &NamespaceRequest,
    current_package: Option<&str>,
    analyzer: &mut S3Analyzer<'_, '_, '_>,
    warned_s3_objects: &mut BTreeSet<String>,
    directives: &mut Vec<NamespaceDirective>,
    diagnostics: &mut Diagnostics,
) {
    let NamespaceTagAndValue { tag_name, value } = tag_name_and_value(&request.tag);
    let tag = tag_origin(&request.tag);
    let words = value.words();

    match &request.tag {
        NamespaceTag::Export(_) => {
            if words.is_empty() {
                let Some(object) = request.object.as_ref() else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@export needs a name or an implicit object name",
                    );
                    return;
                };
                let Some(name) = normalize_object_name(object.as_str()) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@export contains an empty or undecodable name",
                    );
                    return;
                };
                if let Some(method) = request.method.as_ref() {
                    if !request.object_is_function {
                        invalid(
                            diagnostics,
                            &tag_name,
                            tag.clone(),
                            "@method with @export must be used with a function",
                        );
                        return;
                    }
                    let Some(generic) = auto_quote_method_argument(&method.generic.value) else {
                        invalid(
                            diagnostics,
                            &tag_name,
                            tag.clone(),
                            "@method contains an empty or undecodable generic",
                        );
                        return;
                    };
                    let Some(class) = auto_quote_method_argument(&method.class.value) else {
                        invalid(
                            diagnostics,
                            &tag_name,
                            tag.clone(),
                            "@method contains an empty or undecodable class",
                        );
                        return;
                    };
                    directives.push(NamespaceDirective::S3Method { generic, class });
                    return;
                }
                match analyzer.analyze(object) {
                    S3ExportAnalysis::OrdinaryExport => {
                        directives.push(NamespaceDirective::Export { name });
                    }
                    S3ExportAnalysis::S3Method { generic, class } => {
                        directives.push(NamespaceDirective::S3Method {
                            generic: NamespaceS3MethodArgument::AutoQuoted(generic),
                            class: NamespaceS3MethodArgument::AutoQuoted(class),
                        });
                    }
                    S3ExportAnalysis::Unresolved => {
                        if warned_s3_objects.insert(object.as_str().to_owned()) {
                            unresolved_s3_export(diagnostics, request, object.as_str());
                        }
                    }
                }
                return;
            }

            let names = words
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(names) = names else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@export contains an empty or undecodable name",
                );
                return;
            };
            directives.extend(
                names
                    .into_iter()
                    .map(|name| NamespaceDirective::Export { name }),
            );
        }
        NamespaceTag::Import(_) => {
            if words.is_empty() {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@import requires at least one package name",
                );
                return;
            }

            if words.iter().any(|word| word.value.contains(',')) {
                let verbatim = words
                    .iter()
                    .map(|word| word.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let Some(value) = NamespaceVerbatim::new(verbatim) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@import contains empty or undecodable directive text",
                    );
                    return;
                };
                directives.push(NamespaceDirective::ImportVerbatim { value });
                return;
            }

            let packages = words
                .iter()
                .map(|word| normalize_package_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(packages) = packages else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@import contains an empty or undecodable package name",
                );
                return;
            };
            for (package, word) in packages.into_iter().zip(&words) {
                if Some(package.as_str()) == current_package {
                    warn_self_import(
                        diagnostics,
                        &tag_name,
                        package.as_str(),
                        Some(word.span),
                        &tag,
                    );
                } else {
                    directives.push(NamespaceDirective::Import { package });
                }
            }
        }
        NamespaceTag::ImportFrom(_) => {
            if words.len() < 2 {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@importFrom requires a package and at least one name",
                );
                return;
            }
            let Some(package) = normalize_package_name(&words[0].value) else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@importFrom contains an empty or undecodable package name",
                );
                return;
            };
            if Some(package.as_str()) == current_package {
                warn_self_import(
                    diagnostics,
                    &tag_name,
                    package.as_str(),
                    Some(words[0].span),
                    &tag,
                );
                return;
            }
            let names = words[1..]
                .iter()
                .filter_map(|word| normalize_object_name(&word.value))
                .collect::<Vec<_>>();
            if names.len() != words.len() - 1 {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@importFrom contains an empty or undecodable name",
                );
                return;
            }
            directives.push(NamespaceDirective::ImportFrom {
                package,
                names: NonEmptyNamespaceNames::new(names).expect("validated import has a name"),
            });
        }
        NamespaceTag::UseDynLib(_) => {
            if words.is_empty() {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@useDynLib requires a package name",
                );
                return;
            }
            if value.as_str().contains(',') {
                let Some(value) = NamespaceVerbatim::new(value.as_str().to_owned()) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag,
                        "@useDynLib contains empty or undecodable directive text",
                    );
                    return;
                };
                directives.push(NamespaceDirective::UseDynLib { value });
            } else {
                let Some(package) = normalize_package_name(&words[0].value) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag,
                        "@useDynLib contains an empty or undecodable package name",
                    );
                    return;
                };
                let package = quote_name(package.as_str());
                let routines = if words.len() == 1 {
                    vec![package]
                } else {
                    let Some(routines) = words[1..]
                        .iter()
                        .map(|word| normalize_object_name(&word.value))
                        .collect::<Option<Vec<_>>>()
                    else {
                        invalid(
                            diagnostics,
                            &tag_name,
                            tag,
                            "@useDynLib contains an empty or undecodable routine name",
                        );
                        return;
                    };
                    routines
                        .iter()
                        .map(|routine| format!("{package},{}", quote_name(routine.as_str())))
                        .collect()
                };
                for value in routines {
                    let Some(value) = NamespaceVerbatim::new(value) else {
                        invalid(
                            diagnostics,
                            &tag_name,
                            tag.clone(),
                            "@useDynLib contains empty or undecodable directive text",
                        );
                        return;
                    };
                    directives.push(NamespaceDirective::UseDynLib { value });
                }
            }
        }
        NamespaceTag::RawNamespace(_) => {
            let Some(value) = NamespaceVerbatim::new(value.as_str().to_owned()) else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@rawNamespace contains empty or undecodable directive text",
                );
                return;
            };
            if !can_parse_namespace_source(value.as_str()) {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@rawNamespace failed to parse as R source",
                );
                return;
            }
            directives.push(NamespaceDirective::RawNamespace { value });
        }
        NamespaceTag::ExportPattern(_) => {
            let patterns = words
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(patterns) = patterns.filter(|patterns| !patterns.is_empty()) else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@exportPattern requires a pattern",
                );
                return;
            };
            directives.extend(
                patterns
                    .into_iter()
                    .map(|pattern| NamespaceDirective::ExportPattern { pattern }),
            );
        }
        NamespaceTag::ExportClass(_) => {
            if words.len() != 1 {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@exportClass requires exactly one class name",
                );
                return;
            }
            let names = words
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(names) = names else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@exportClass contains an empty or undecodable name",
                );
                return;
            };
            directives.extend(
                names
                    .into_iter()
                    .map(|name| NamespaceDirective::ExportClass { name }),
            );
        }
        NamespaceTag::ExportMethod(_) => {
            let names = words
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(names) = names else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@exportMethod contains an empty or undecodable name",
                );
                return;
            };
            directives.extend(
                names
                    .into_iter()
                    .map(|name| NamespaceDirective::ExportMethod { name }),
            );
        }
        NamespaceTag::ImportClassesFrom(_) => {
            if words.len() < 2 {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@importClassesFrom requires a package and at least one class",
                );
                return;
            }
            let Some(package) = words
                .first()
                .and_then(|word| normalize_package_name(&word.value))
            else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@importClassesFrom requires a package and at least one class",
                );
                return;
            };
            if Some(package.as_str()) == current_package {
                warn_self_import(
                    diagnostics,
                    &tag_name,
                    package.as_str(),
                    Some(words[0].span),
                    &tag,
                );
                return;
            }
            let names = words[1..]
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(names) = names.filter(|names| !names.is_empty()) else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@importClassesFrom requires a package and at least one class",
                );
                return;
            };
            directives.extend(names.into_iter().map(|name| {
                NamespaceDirective::ImportClassesFrom {
                    package: package.clone(),
                    name,
                }
            }));
        }
        NamespaceTag::ImportMethodsFrom(_) => {
            if words.len() < 2 {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag.clone(),
                    "@importMethodsFrom requires a package and at least one method",
                );
                return;
            }
            let Some(package) = words
                .first()
                .and_then(|word| normalize_package_name(&word.value))
            else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@importMethodsFrom requires a package and at least one method",
                );
                return;
            };
            if Some(package.as_str()) == current_package {
                warn_self_import(
                    diagnostics,
                    &tag_name,
                    package.as_str(),
                    Some(words[0].span),
                    &tag,
                );
                return;
            }
            let names = words[1..]
                .iter()
                .map(|word| normalize_object_name(&word.value))
                .collect::<Option<Vec<_>>>();
            let Some(names) = names.filter(|names| !names.is_empty()) else {
                invalid(
                    diagnostics,
                    &tag_name,
                    tag,
                    "@importMethodsFrom requires a package and at least one method",
                );
                return;
            };
            directives.extend(names.into_iter().map(|name| {
                NamespaceDirective::ImportMethodsFrom {
                    package: package.clone(),
                    name,
                }
            }));
        }
        NamespaceTag::ExportS3Method(_) => match words.as_slice() {
            [generic, class] => {
                let Some(generic) = NamespaceS3MethodArgument::literal(generic.value.clone())
                else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method contains an empty or undecodable generic",
                    );
                    return;
                };
                let Some(class) = NamespaceS3MethodArgument::literal(class.value.clone()) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method contains an empty or undecodable class",
                    );
                    return;
                };
                directives.push(NamespaceDirective::S3Method { generic, class });
            }
            [generic] => {
                // `@exportS3Method NULL` asks for no directive at all: the
                // method is registered at load time instead, typically by an
                // s3_register() call in .onLoad. Producing nothing is the whole
                // behaviour, and it holds whatever the tag is attached to —
                // nothing about the documented object is consulted, so this
                // decision comes before any question about it. It also
                // suppresses roxygen2's missing-export warning, which we do not
                // emit yet; when we do, it has to honour this tag.
                if generic.value == "NULL" {
                    return;
                }
                let Some(object) = request.object.as_ref() else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method must be used with a known function object",
                    );
                    return;
                };
                let object_is_function = request.object.as_ref().is_some_and(|object| {
                    request.object_is_function || analyzer.is_proven_function(object.as_str())
                });
                if !object_is_function {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method must be used with a function",
                    );
                    return;
                }
                let Some((package, generic_component)) = generic.value.rsplit_once("::") else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method must have form package::generic",
                    );
                    return;
                };
                if package.is_empty() || generic_component.is_empty() {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method must have form package::generic",
                    );
                    return;
                }
                let prefix = format!("{generic_component}.");
                let Some(class) = object.as_str().strip_prefix(&prefix) else {
                    let message = format!(
                        "@exportS3Method generic ({generic_component}) doesn't match function ({})",
                        object.as_str()
                    );
                    invalid(diagnostics, &tag_name, tag.clone(), &message);
                    return;
                };
                if class.is_empty() {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method derives an empty class from the function name",
                    );
                    return;
                }
                let Some(generic) = NamespaceS3MethodArgument::literal(generic.value.clone())
                else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method contains an empty or undecodable generic",
                    );
                    return;
                };
                let Some(class) = NamespaceS3MethodArgument::literal(class.to_owned()) else {
                    invalid(
                        diagnostics,
                        &tag_name,
                        tag.clone(),
                        "@exportS3Method contains an empty or undecodable class",
                    );
                    return;
                };
                directives.push(NamespaceDirective::S3Method { generic, class });
            }
            _ => invalid(
                diagnostics,
                &tag_name,
                tag.clone(),
                "@exportS3Method requires one or two words",
            ),
        },
    }
}

fn warn_self_import(
    diagnostics: &mut Diagnostics,
    tag_name: &str,
    package: &str,
    package_span: Option<Span>,
    origin: &TagOrigin,
) {
    let code = DiagnosticCode::SelfImport;
    diagnostics.push(
        Diagnostic::new(
            code.default_severity(),
            code,
            format!("@{tag_name} imports from the current package `{package}`"),
            Label::new(
                package_span.unwrap_or_else(|| origin_span(origin)),
                "this is the package being documented",
            ),
        )
        .with_context("tag", tag_name.to_owned())
        .with_context("package", package.to_owned()),
    );
}

struct NamespaceTagAndValue<'a> {
    tag_name: String,
    value: &'a PlainText,
}

fn tag_name_and_value(tag: &NamespaceTag) -> NamespaceTagAndValue<'_> {
    let value = match tag {
        NamespaceTag::Export(value)
        | NamespaceTag::ExportS3Method(value)
        | NamespaceTag::Import(value)
        | NamespaceTag::ImportFrom(value)
        | NamespaceTag::RawNamespace(value)
        | NamespaceTag::UseDynLib(value)
        | NamespaceTag::ExportPattern(value)
        | NamespaceTag::ExportClass(value)
        | NamespaceTag::ExportMethod(value)
        | NamespaceTag::ImportClassesFrom(value)
        | NamespaceTag::ImportMethodsFrom(value) => value,
    };
    NamespaceTagAndValue {
        tag_name: tag_name(tag).to_owned(),
        value: &value.value,
    }
}

fn tag_name(tag: &NamespaceTag) -> &str {
    let origin = match tag {
        NamespaceTag::Export(value)
        | NamespaceTag::ExportS3Method(value)
        | NamespaceTag::Import(value)
        | NamespaceTag::ImportFrom(value)
        | NamespaceTag::RawNamespace(value)
        | NamespaceTag::UseDynLib(value)
        | NamespaceTag::ExportPattern(value)
        | NamespaceTag::ExportClass(value)
        | NamespaceTag::ExportMethod(value)
        | NamespaceTag::ImportClassesFrom(value)
        | NamespaceTag::ImportMethodsFrom(value) => &value.origin,
    };
    match origin {
        TagOrigin::Explicit { name, .. } => name.value.as_str(),
        TagOrigin::Implicit { .. } => "namespace",
    }
}

fn tag_origin(tag: &NamespaceTag) -> TagOrigin {
    match tag {
        NamespaceTag::Export(value)
        | NamespaceTag::ExportS3Method(value)
        | NamespaceTag::Import(value)
        | NamespaceTag::ImportFrom(value)
        | NamespaceTag::RawNamespace(value)
        | NamespaceTag::UseDynLib(value)
        | NamespaceTag::ExportPattern(value)
        | NamespaceTag::ExportClass(value)
        | NamespaceTag::ExportMethod(value)
        | NamespaceTag::ImportClassesFrom(value)
        | NamespaceTag::ImportMethodsFrom(value) => value.origin.clone(),
    }
}

fn origin_span(origin: &TagOrigin) -> Span {
    match origin {
        TagOrigin::Explicit { full_span, .. } => *full_span,
        TagOrigin::Implicit { intro_span } => *intro_span,
    }
}

fn invalid(diagnostics: &mut Diagnostics, tag_name: &str, origin: TagOrigin, message: &str) {
    diagnostics.push(
        Diagnostic::new(
            DiagnosticCode::InvalidNamespaceDirective.default_severity(),
            DiagnosticCode::InvalidNamespaceDirective,
            message,
            Label::new(
                origin_span(&origin),
                format!("invalid namespace directive @{tag_name}"),
            ),
        )
        .with_context("tag", tag_name.to_owned()),
    );
}

fn unresolved_s3_export(diagnostics: &mut Diagnostics, request: &NamespaceRequest, object: &str) {
    let span = request
        .object_spelling
        .unwrap_or_else(|| origin_span(&tag_origin(&request.tag)));
    diagnostics.push(
        Diagnostic::new(
            DiagnosticCode::UnresolvedS3Export.default_severity(),
            DiagnosticCode::UnresolvedS3Export,
            format!(
                "cannot distinguish {object} as an S3 method from a plain dotted function without evaluating R"
            ),
            Label::new(span, "automatic S3 classification is unresolved"),
        )
        .with_help(
            "add @method <generic> <class> before the bare @export, or replace it with @exportS3Method <generic> <class>",
        )
        .with_context("object", object.to_owned()),
    );
}

fn normalize_package_name(raw: &str) -> Option<NamespacePackageName> {
    decode_word(raw).and_then(NamespacePackageName::new)
}

fn normalize_object_name(raw: &str) -> Option<NamespaceObjectName> {
    decode_word(raw).and_then(NamespaceObjectName::new)
}

fn auto_quote_method_argument(raw: &str) -> Option<NamespaceS3MethodArgument> {
    let value = decode_word(raw)?;
    (!value.is_empty() && !value.contains('\0'))
        .then_some(NamespaceS3MethodArgument::AutoQuoted(value))
}

fn decode_word(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let bytes = raw.as_bytes();
    let quoted = matches!(bytes.first(), Some(b'"' | b'\'' | b'`'));
    if quoted {
        let quote = bytes[0];
        if bytes.last() != Some(&quote) || bytes.len() < 2 {
            return None;
        }
        let value = &raw[1..raw.len() - 1];
        if value.contains('\\') {
            return None;
        }
        Some(value.to_owned())
    } else {
        Some(raw.to_owned())
    }
}
