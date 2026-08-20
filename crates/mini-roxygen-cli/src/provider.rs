//! Composition of static base facts and installed package metadata.

use std::collections::BTreeSet;

use mini_roxygen_core::S3GenericProvider;

#[derive(Debug, Default)]
pub(crate) struct ComposedS3Provider {
    pub(crate) generics: BTreeSet<String>,
}

impl S3GenericProvider for ComposedS3Provider {
    fn is_s3_generic(&self, name: &str) -> bool {
        self.generics.contains(name)
    }
}

pub(crate) fn compose(
    installed: impl IntoIterator<Item = String>,
    base: &[&str],
) -> ComposedS3Provider {
    let mut generics = installed.into_iter().collect::<BTreeSet<_>>();
    generics.extend(base.iter().copied().map(str::to_owned));
    ComposedS3Provider { generics }
}

#[cfg(test)]
mod tests {
    use mini_roxygen_core::S3GenericProvider;

    use super::compose;

    #[test]
    fn composition_unions_installed_and_base_facts() {
        let provider = compose(["installed".to_owned()], &["base"]);
        assert!(provider.is_s3_generic("installed"));
        assert!(provider.is_s3_generic("base"));
        assert!(!provider.is_s3_generic("missing"));
    }
}
