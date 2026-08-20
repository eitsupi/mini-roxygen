//! Owns source provenance and source-file loading for every processing layer.
//!
//! The source layer keeps the input lossless. It does not evaluate R code or
//! parse roxygen blocks; those responsibilities belong to later layers.

use std::cmp::Ordering;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Identifies a source file within a source map.
///
/// The numeric representation is private so callers cannot accidentally use a
/// raw integer where a file identity is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    /// Creates a file identifier from its source-map index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the source-map index represented by this identifier.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A half-open range of UTF-8 byte offsets within one source file.
///
/// `start` is inclusive and `end` is exclusive. Line and column positions are
/// deliberately not stored here; they are derived from the owning source
/// file's line-start table when locations are rendered for a user.
/// The invariant `start <= end` is enforced by [`TextRange::new`]. The fields
/// are private so this invariant cannot be bypassed through a struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
    /// Inclusive UTF-8 byte offset at which the range starts.
    start: u32,
    /// Exclusive UTF-8 byte offset at which the range ends.
    end: u32,
}

impl TextRange {
    /// Creates a half-open byte range.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        assert!(
            start <= end,
            "TextRange start offset must not exceed end offset"
        );
        Self { start, end }
    }

    /// Returns the inclusive start offset of the range.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the exclusive end offset of the range.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Returns the number of bytes in the range.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Returns whether `offset` is contained in this range.
    ///
    /// The end offset is excluded, so an empty range contains no offset.
    #[must_use]
    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns whether the range has no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Associates a source range with its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    /// The file containing the range.
    pub file: FileId,
    /// The UTF-8 byte range within [`Span::file`].
    pub range: TextRange,
}

impl Span {
    /// Creates a source span.
    #[must_use]
    pub const fn new(file: FileId, range: TextRange) -> Self {
        Self { file, range }
    }
}

/// Associates a value with the source span from which it originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Spanned<T> {
    /// The value extracted from the source.
    pub value: T,
    /// The source provenance of [`Spanned::value`].
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Creates a value with source provenance.
    #[must_use]
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }

    /// Transforms the value while retaining its source provenance.
    #[must_use]
    pub fn map<U>(self, function: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: function(self.value),
            span: self.span,
        }
    }
}

/// An error encountered while enumerating or loading an R source file.
///
/// Source loading errors remain separate from [`crate::diagnostic::Diagnostic`]
/// because an I/O or encoding failure has no valid source span to use as a
/// primary label. Callers can report this error directly with its path instead
/// of inventing a misleading zero-length span.
#[derive(Debug)]
pub enum SourceError {
    /// The operating system could not access a path.
    Io { path: PathBuf, source: io::Error },
    /// The file contents were not valid UTF-8.
    InvalidUtf8 {
        path: PathBuf,
        source: std::str::Utf8Error,
    },
}

impl SourceError {
    /// Returns the path involved in the failed operation.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. } | Self::InvalidUtf8 { path, .. } => path,
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::InvalidUtf8 { path, source } => {
                write!(
                    formatter,
                    "source file {} is not valid UTF-8: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for SourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
        }
    }
}

/// One R source file and the immutable information needed to resolve spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// A stable display path, normally relative to the package root.
    ///
    /// Package-relative paths are used by [`SourceMap::from_package_root`]
    /// because diagnostics should not expose machine-specific absolute paths
    /// and should remain reproducible across checkout locations.
    path: PathBuf,
    /// The UTF-8 source text used for parsing and span resolution, including
    /// its original line endings but excluding one leading UTF-8 BOM.
    text: String,
    /// Whether one leading UTF-8 BOM was removed while creating this file.
    had_utf8_bom: bool,
    /// UTF-8 byte offsets at which lines begin. The first entry is always zero.
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// Creates a source file from already validated UTF-8 text.
    ///
    /// One leading UTF-8 BOM is removed from the text and recorded by
    /// [`SourceFile::had_utf8_bom`]. All offsets and locations then use the
    /// resulting text. The supplied path is retained as the display path;
    /// callers loading a package should supply a path relative to that
    /// package root.
    #[must_use]
    pub fn new(path: PathBuf, text: String) -> Self {
        let had_utf8_bom = text.starts_with('\u{FEFF}');
        let text = if had_utf8_bom {
            text.strip_prefix('\u{FEFF}')
                .expect("a string starting with a BOM must have a BOM prefix")
                .to_owned()
        } else {
            text
        };
        let line_starts = line_starts(&text);
        Self {
            path,
            text,
            had_utf8_bom,
            line_starts,
        }
    }

    /// Reads a file, validating that its bytes are UTF-8.
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self, SourceError> {
        let path = path.into();
        let bytes = fs::read(&path).map_err(|source| SourceError::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_bytes(path, bytes)
    }

    /// Validates bytes as UTF-8 and creates a source file.
    ///
    /// One leading UTF-8 BOM is removed because retaining it causes the R
    /// parser to report it as an invalid token, producing a false diagnostic
    /// for every file that has a BOM. The removed BOM is recorded by
    /// [`SourceFile::had_utf8_bom`]. All offsets and locations use the text
    /// after removal. This does not cause a practical loss of file provenance:
    /// mini-roxygen only reads R source and does not write it back, so no caller
    /// needs byte offsets into the original file. Line and column diagnostics
    /// based on the normalized text also match the positions shown in an
    /// editor.
    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self, SourceError> {
        let text = std::str::from_utf8(&bytes)
            .map_err(|source| SourceError::InvalidUtf8 {
                path: path.clone(),
                source,
            })?
            .to_owned();
        Ok(Self::new(path, text))
    }

    /// Returns the display path of this source file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the complete normalized source text with its original newlines.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns whether a leading UTF-8 BOM was removed from this source file.
    #[must_use]
    pub const fn had_utf8_bom(&self) -> bool {
        self.had_utf8_bom
    }

    /// Returns the text selected by a valid UTF-8 byte range.
    ///
    /// `None` is returned for an out-of-bounds range or an offset inside a
    /// multi-byte UTF-8 scalar value.
    #[must_use]
    pub fn text_range(&self, range: TextRange) -> Option<&str> {
        let start = usize::try_from(range.start()).ok()?;
        let end = usize::try_from(range.end()).ok()?;
        if end > self.text.len()
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return None;
        }
        self.text.get(start..end)
    }

    /// Returns the number of logical lines in this file.
    ///
    /// An empty file has one empty line, and a file ending in a newline has a
    /// final empty line after that newline.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Converts a UTF-8 byte offset into a one-based `(line, column)` pair.
    ///
    /// The line is selected with binary search over the line-start table. The
    /// column is one-based and counts Unicode scalar values from the line
    /// start, rather than counting bytes, so diagnostics remain meaningful for
    /// non-ASCII source text. `None` is returned for an out-of-bounds offset or
    /// an offset that is not a UTF-8 character boundary.
    #[must_use]
    pub fn line_column(&self, offset: u32) -> Option<(usize, usize)> {
        let offset = usize::try_from(offset).ok()?;
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }

        let line_index = self
            .line_starts
            .partition_point(|&line_start| {
                usize::try_from(line_start).is_ok_and(|start| start <= offset)
            })
            .checked_sub(1)?;
        let line_start = usize::try_from(self.line_starts[line_index]).ok()?;
        let column = self.text.get(line_start..offset)?.chars().count() + 1;
        Some((line_index + 1, column))
    }
}

/// A collection of source files addressed by registration-order [`FileId`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// Creates an empty source map.
    #[must_use]
    pub const fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Loads all direct `.R` and `.r` files under a package's `R/` directory.
    ///
    /// Files are registered in deterministic bytewise filename order. A
    /// missing `R/` directory and a package containing no matching files both
    /// produce an empty map: source enumeration is a composable input phase,
    /// so package validation can decide later whether an empty source set is a
    /// problem. `.Rbuildignore` filtering is intentionally unsupported because
    /// placing a file under `R/` that should be excluded from an R package is
    /// unreasonable, and no such examples have been identified. The
    /// `DESCRIPTION`/`Collate` field is also not consulted.
    pub fn from_package_root(package_root: impl AsRef<Path>) -> Result<Self, SourceError> {
        let package_root = package_root.as_ref();
        let mut source_map = Self::new();
        for path in enumerate_r_files(package_root)? {
            let bytes = fs::read(&path).map_err(|source| SourceError::Io {
                path: path.clone(),
                source,
            })?;
            let relative_path = path
                .strip_prefix(package_root)
                .unwrap_or(path.as_path())
                .to_path_buf();
            source_map.add_file(SourceFile::from_bytes(relative_path, bytes)?);
        }
        Ok(source_map)
    }

    /// Registers a source file and returns its registration-order identifier.
    pub fn add_file(&mut self, file: SourceFile) -> FileId {
        let index = u32::try_from(self.files.len())
            .expect("a source map cannot contain more files than FileId can address");
        self.files.push(file);
        FileId::new(index)
    }

    /// Returns the source file identified by `file`.
    ///
    /// An [`Option`] is used instead of panicking because spans can cross an
    /// API boundary; malformed or stale file identifiers should be reportable
    /// as ordinary lookup failures by callers.
    #[must_use]
    pub fn get(&self, file: FileId) -> Option<&SourceFile> {
        self.files.get(usize::try_from(file.index()).ok()?)
    }

    /// Returns the number of registered source files.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns whether no source files are registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Returns the text selected by a span, or `None` for an invalid span.
    #[must_use]
    pub fn span_text(&self, span: Span) -> Option<&str> {
        self.get(span.file)?.text_range(span.range)
    }

    /// Resolves a span's start into `(file path, one-based line, one-based column)`.
    #[must_use]
    pub fn span_location(&self, span: Span) -> Option<(&Path, usize, usize)> {
        let file = self.get(span.file)?;
        let (line, column) = file.line_column(span.range.start())?;
        Some((file.path(), line, column))
    }

    /// Compares two files by their final path components under default source
    /// ordering.
    ///
    /// This is temporal evidence only when both identifiers resolve, both
    /// filenames are valid Unicode, and the final components differ. A
    /// registration index is deliberately not consulted: callers may build a
    /// source map in any order, while R's default source order is filename
    /// order. Non-Unicode filenames are not portable enough to establish
    /// evidence outside the current target and Rust version.
    #[must_use]
    pub fn compare_filename_order(&self, left: FileId, right: FileId) -> Option<Ordering> {
        let left = self.get(left)?.path().file_name()?;
        let right = self.get(right)?.path().file_name()?;
        left.to_str()?;
        right.to_str()?;
        match left.as_encoded_bytes().cmp(right.as_encoded_bytes()) {
            Ordering::Equal => None,
            ordering => Some(ordering),
        }
    }
}

/// Enumerates direct R source files under `package_root/R`.
///
/// Only `.R` and `.r` files are returned, and subdirectories are not visited.
/// The result is sorted lexicographically by each filename's encoded byte
/// representation. Valid Unicode filenames therefore use UTF-8 byte order on
/// every platform, while platform-native non-Unicode filenames remain
/// distinct and deterministically ordered within the current target and Rust
/// version. The encoding of non-Unicode portions is not a stable, persistable
/// format, and no Unicode normalization is performed. A missing `R/`
/// directory is treated as an empty result. Other directory I/O failures are
/// returned with the path that could not be accessed.
///
/// `.Rbuildignore` filtering is intentionally unsupported. Placing a file
/// under `R/` that should be excluded from an R package is unreasonable, and
/// no such examples have been identified. Avoiding a regular-expression
/// dependency for its Perl-compatible patterns is a secondary benefit. Every
/// matching file in `R/` is therefore returned.
pub fn enumerate_r_files(package_root: impl AsRef<Path>) -> Result<Vec<PathBuf>, SourceError> {
    let r_dir = package_root.as_ref().join("R");
    let entries = match fs::read_dir(&r_dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SourceError::Io {
                path: r_dir,
                source,
            });
        }
    };

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| SourceError::Io {
            path: r_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|source| SourceError::Io {
                path: path.clone(),
                source,
            })?
            .is_file()
        {
            continue;
        }
        let Some(extension) = path.extension() else {
            continue;
        };
        if extension == OsStr::new("R") || extension == OsStr::new("r") {
            files.push(path);
        }
    }

    files.sort_unstable_by(|left, right| filename_sort_key(left).cmp(filename_sort_key(right)));

    Ok(files)
}

/// Returns the filename's lossless platform-encoded byte representation.
fn filename_sort_key(path: &Path) -> &[u8] {
    match path.file_name() {
        Some(name) => name.as_encoded_bytes(),
        None => &[],
    }
}

fn line_starts(text: &str) -> Vec<u32> {
    let mut starts = vec![0];
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(
                u32::try_from(offset + 1)
                    .expect("source files cannot exceed the TextRange byte-offset limit"),
            );
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::{
        FileId, SourceError, SourceFile, SourceMap, Span, Spanned, TextRange, enumerate_r_files,
        filename_sort_key,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let counter = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mini-roxygen-source-{}-{timestamp}-{counter}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("temporary source directory should be creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn text_range_is_half_open() {
        let range = TextRange::new(2, 5);

        assert_eq!(range.len(), 3);
        assert!(!range.is_empty());
        assert!(!range.contains(1));
        assert!(range.contains(2));
        assert!(range.contains(4));
        assert!(!range.contains(5));
    }

    #[test]
    fn empty_text_range_has_no_contained_offset() {
        let range = TextRange::new(7, 7);

        assert_eq!(range.len(), 0);
        assert!(range.is_empty());
        assert!(!range.contains(7));
    }

    #[test]
    #[should_panic]
    fn reversed_text_range_panics() {
        let _ = TextRange::new(5, 2);
    }

    #[test]
    fn text_range_accessors_return_offsets() {
        let range = TextRange::new(3, 8);

        assert_eq!(range.start(), 3);
        assert_eq!(range.end(), 8);
    }

    #[test]
    fn spanned_map_retains_span() {
        let span = Span::new(FileId::new(4), TextRange::new(2, 5));
        let mapped = Spanned::new("value", span).map(str::len);

        assert_eq!(mapped.value, 5);
        assert_eq!(mapped.span, span);
    }

    #[test]
    fn line_and_column_are_one_based_and_count_scalars() {
        let file = SourceFile::new(PathBuf::from("R/example.R"), "a\r\néx\nlast".to_owned());

        assert_eq!(file.line_count(), 3);
        assert_eq!(file.line_column(0), Some((1, 1)));
        assert_eq!(file.line_column(3), Some((2, 1)));
        assert_eq!(file.line_column(5), Some((2, 2)));
        assert_eq!(file.line_column(7), Some((3, 1)));
        assert_eq!(file.line_column(11), Some((3, 5)));
        assert_eq!(file.line_column(4), None);
    }

    #[test]
    fn line_count_handles_empty_and_trailing_newline() {
        assert_eq!(
            SourceFile::new(PathBuf::new(), String::new()).line_count(),
            1
        );
        assert_eq!(
            SourceFile::new(PathBuf::new(), "one\n".to_owned()).line_count(),
            2
        );
        assert_eq!(
            SourceFile::new(PathBuf::new(), "one\r\ntwo".to_owned()).line_count(),
            2
        );
    }

    #[test]
    fn text_ranges_and_span_locations_resolve_through_source_map() {
        let mut source_map = SourceMap::new();
        let file_id = source_map.add_file(SourceFile::new(
            PathBuf::from("R/example.R"),
            "title\n本文".to_owned(),
        ));
        let span = Span::new(file_id, TextRange::new(6, 12));

        assert_eq!(source_map.span_text(span), Some("本文"));
        assert_eq!(
            source_map.span_location(Span::new(file_id, TextRange::new(9, 12))),
            Some((Path::new("R/example.R"), 2, 2))
        );
        assert!(source_map.get(FileId::new(99)).is_none());
        assert!(
            source_map
                .span_text(Span::new(FileId::new(99), TextRange::new(0, 0)))
                .is_none()
        );
    }

    #[test]
    fn filename_order_evidence_ignores_registration_and_rejects_ambiguity() {
        let mut source_map = SourceMap::new();
        let alias = source_map.add_file(SourceFile::new(
            PathBuf::from("R/b-method.R"),
            String::new(),
        ));
        let target = source_map.add_file(SourceFile::new(
            PathBuf::from("R/a-generic.R"),
            String::new(),
        ));

        assert_eq!(
            source_map.compare_filename_order(target, alias),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            source_map.compare_filename_order(FileId::new(99), alias),
            None
        );

        let same_name = source_map.add_file(SourceFile::new(
            PathBuf::from("other/generic.R"),
            String::new(),
        ));
        let same_name_again = source_map.add_file(SourceFile::new(
            PathBuf::from("another/generic.R"),
            String::new(),
        ));
        assert_eq!(
            source_map.compare_filename_order(same_name, same_name_again),
            None
        );
    }

    #[test]
    fn enumerates_only_direct_r_files_in_byte_order() {
        let directory = TempDirectory::new();
        let r_directory = directory.path().join("R");
        fs::create_dir(&r_directory).expect("R directory should be creatable");
        for name in ["b.R", "a.R", "A.r", "notes.txt"] {
            fs::write(r_directory.join(name), "x").expect("fixture should be writable");
        }
        fs::create_dir(r_directory.join("sub")).expect("subdirectory should be creatable");
        fs::write(r_directory.join("sub/c.R"), "x").expect("nested fixture should be writable");

        let files = enumerate_r_files(directory.path()).expect("enumeration should succeed");
        let names: Vec<_> = files
            .iter()
            .map(|path| path.file_name().expect("fixture has a filename"))
            .collect();
        assert_eq!(names, ["A.r", "a.R", "b.R"]);
    }

    #[test]
    fn filename_sort_key_uses_utf8_byte_order_for_unicode() {
        let bmp = Path::new("\u{e000}.R");
        let non_bmp = Path::new("\u{10000}.R");

        assert_eq!(filename_sort_key(bmp), "\u{e000}.R".as_bytes());
        assert_eq!(filename_sort_key(non_bmp), "\u{10000}.R".as_bytes());
        assert!(filename_sort_key(bmp) < filename_sort_key(non_bmp));
    }

    #[cfg(unix)]
    #[test]
    fn filename_sort_key_distinguishes_invalid_unix_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let first = Path::new(OsStr::from_bytes(&[0x80]));
        let second = Path::new(OsStr::from_bytes(&[0xff]));

        assert_ne!(filename_sort_key(first), filename_sort_key(second));
        assert!(filename_sort_key(first) < filename_sort_key(second));
    }

    #[cfg(windows)]
    #[test]
    fn filename_sort_key_distinguishes_unpaired_windows_surrogates() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        let first = OsString::from_wide(&[0xd800]);
        let second = OsString::from_wide(&[0xd801]);

        assert_ne!(
            filename_sort_key(Path::new(first.as_os_str())),
            filename_sort_key(Path::new(second.as_os_str()))
        );
    }

    #[test]
    fn missing_r_directory_is_empty_and_package_paths_are_relative() {
        let directory = TempDirectory::new();
        let source_map = SourceMap::from_package_root(directory.path())
            .expect("missing R directory should be accepted");
        assert!(source_map.is_empty());

        let r_directory = directory.path().join("R");
        fs::create_dir(&r_directory).expect("R directory should be creatable");
        fs::write(r_directory.join("z.R"), "x").expect("fixture should be writable");
        let source_map =
            SourceMap::from_package_root(directory.path()).expect("load should succeed");
        assert_eq!(
            source_map
                .get(FileId::new(0))
                .expect("file should exist")
                .path(),
            Path::new("R/z.R")
        );
    }

    #[test]
    fn invalid_utf8_reports_the_file_path() {
        let directory = TempDirectory::new();
        let path = directory.path().join("broken.R");
        fs::write(&path, [0xff, 0xfe]).expect("fixture should be writable");

        let error = SourceFile::from_path(&path).expect_err("invalid UTF-8 should fail");
        assert!(matches!(error, SourceError::InvalidUtf8 { .. }));
        assert_eq!(error.path(), path.as_path());
        assert!(error.to_string().contains("broken.R"));
    }

    #[test]
    fn leading_bom_is_removed_and_recorded() {
        let file = SourceFile::from_bytes(PathBuf::from("R/bom.R"), b"\xef\xbb\xbfx".to_vec())
            .expect("BOM is valid UTF-8");

        assert_eq!(file.text(), "x");
        assert!(file.had_utf8_bom());
        assert_eq!(file.text_range(TextRange::new(0, 1)), Some("x"));
        assert_eq!(file.line_column(0), Some((1, 1)));
    }

    #[test]
    fn only_the_leading_bom_is_removed() {
        let file = SourceFile::from_bytes(
            PathBuf::from("R/bom.R"),
            "\u{FEFF}\u{FEFF}x\nbody\u{FEFF}".as_bytes().to_vec(),
        )
        .expect("BOMs are valid UTF-8");

        assert!(file.had_utf8_bom());
        assert_eq!(file.text(), "\u{FEFF}x\nbody\u{FEFF}");
        assert_eq!(file.text_range(TextRange::new(0, 3)), Some("\u{FEFF}"));
        assert_eq!(file.text_range(TextRange::new(9, 12)), Some("\u{FEFF}"));
    }

    #[test]
    fn bom_only_file_is_an_empty_source_file() {
        let file = SourceFile::from_bytes(PathBuf::from("R/bom.R"), b"\xef\xbb\xbf".to_vec())
            .expect("BOM is valid UTF-8");

        assert!(file.had_utf8_bom());
        assert_eq!(file.text(), "");
        assert_eq!(file.line_count(), 1);
        assert_eq!(file.text_range(TextRange::new(0, 0)), Some(""));
        assert_eq!(file.line_column(0), Some((1, 1)));
    }
}
