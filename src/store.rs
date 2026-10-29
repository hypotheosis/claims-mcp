//! Durable storage of the catalog file: locking, loading, and atomic saving.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::catalog::{Catalog, SCHEMA_VERSION, UNSEALED_SCHEMA_VERSION};
use crate::error::{ClaimsError, Result};
use crate::seal;

/// Paths derived once from the catalog path.
#[derive(Debug)]
struct StorePaths {
    catalog: PathBuf,
    lock: PathBuf,
    temp: PathBuf,
    directory: PathBuf,
}

/// Handle to the catalog file. Cheap to clone (an `Arc` bump).
///
/// Every operation takes an exclusive lock on the sidecar `.<name>.lock` file, re-reads the
/// catalog from disk, and, for mutations, writes `.<name>.tmp` and renames it over the
/// catalog. The file is therefore always complete, and changes made outside the server
/// (git checkout, another instance) are always seen. Hand edits are seen too, and detected:
/// every record carries an integrity seal (`src/seal.rs`), and writes are refused while any
/// seal fails to verify.
#[derive(Debug, Clone)]
pub struct CatalogStore {
    paths: Arc<StorePaths>,
}

impl CatalogStore {
    /// Creates a store for the catalog at `catalog_path`. Nothing is touched on disk until the
    /// first operation; a missing file is treated as an empty catalog.
    pub fn new(catalog_path: impl Into<PathBuf>) -> Self {
        let catalog = catalog_path.into();
        let directory = match catalog.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let file_name = catalog
            .file_name()
            .map_or_else(|| "claims.json".into(), |name| name.to_string_lossy());
        let lock = directory.join(format!(".{file_name}.lock"));
        let temp = directory.join(format!(".{file_name}.tmp"));
        Self {
            paths: Arc::new(StorePaths {
                catalog,
                lock,
                temp,
                directory,
            }),
        }
    }

    /// Returns the catalog file path.
    pub fn path(&self) -> &Path {
        &self.paths.catalog
    }

    /// Returns the directory that holds the catalog; its git repository is the one anchors
    /// are pinned to.
    pub fn directory(&self) -> &Path {
        &self.paths.directory
    }

    /// The catalog file and its sidecar lock and temp files.
    pub fn own_files(&self) -> [&Path; 3] {
        [&self.paths.catalog, &self.paths.lock, &self.paths.temp]
    }

    /// Runs `operation` against a fresh read of the catalog without writing.
    ///
    /// Blocking file I/O runs on tokio's blocking thread pool so the async reactor stays free.
    ///
    /// # Errors
    ///
    /// Lock, read, or parse failures, or whatever `operation` returns.
    pub async fn read<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Catalog) -> Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.read_blocking(operation)).await?
    }

    /// Runs `operation` against a fresh read of the catalog and saves the result atomically,
    /// re-sealing the records it changed. If `operation` fails, nothing is written.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::UnsealedCatalog`] for a schema v1 catalog and
    /// [`ClaimsError::CatalogTampered`] if any seal fails (both checked before `operation`
    /// runs), lock, read, parse, or write failures, or whatever `operation`
    /// returns.
    pub async fn mutate<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Catalog) -> Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.mutate_blocking(operation)).await?
    }

    /// Seals every unsealed record as adopted and saves the catalog; see [`seal::adopt`].
    /// Returns the names of the records it sealed.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::AdoptRefused`] if a sealed record was changed or a claim is missing, and
    /// lock, read, parse, or write failures.
    pub async fn adopt(&self) -> Result<Vec<String>> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.adopt_blocking()).await?
    }

    /// Synchronous body of [`CatalogStore::adopt`].
    fn adopt_blocking(&self) -> Result<Vec<String>> {
        let _lock = self.lock()?;
        let mut catalog = self.load()?;
        let adopted = seal::adopt(&mut catalog)?;
        if !adopted.is_empty() {
            self.save(&catalog)?;
        }
        Ok(adopted)
    }

    /// Synchronous body of [`CatalogStore::read`].
    fn read_blocking<T>(&self, operation: impl FnOnce(&Catalog) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        operation(&self.load()?)
    }

    /// Synchronous body of [`CatalogStore::mutate`].
    fn mutate_blocking<T>(&self, operation: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        // Rust note: `_lock` keeps the file handle alive until the end of the function; the OS
        // lock is released automatically when it is dropped (RAII, like a `with` block).
        let _lock = self.lock()?;
        let mut catalog = self.load()?;
        let integrity = seal::verify(&catalog);
        if !integrity.verified {
            let path = self.paths.catalog.clone();
            let never_sealed = catalog.schema_version() == UNSEALED_SCHEMA_VERSION
                && !seal::has_any_seal(&catalog);
            let error = if never_sealed {
                ClaimsError::UnsealedCatalog { path }
            } else {
                ClaimsError::CatalogTampered {
                    path,
                    problems: integrity.problems,
                }
            };
            tracing::error!("{error}");
            return Err(error);
        }
        let output = operation(&mut catalog)?;
        seal::reseal(&mut catalog);
        self.save(&catalog)?;
        Ok(output)
    }

    /// Opens (creating if needed) the sidecar lock file and takes an exclusive lock on it.
    fn lock(&self) -> Result<File> {
        let path = &self.paths.lock;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|source| io_error(path, source))?;
        file.lock().map_err(|source| io_error(path, source))?;
        Ok(file)
    }

    /// Reads and parses the catalog; a missing file yields an empty catalog.
    /// The catalog's own invariants are checked so a bad merge or hand edit is reported, not used.
    fn load(&self) -> Result<Catalog> {
        let path = &self.paths.catalog;
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Catalog::default()),
            Err(source) => return Err(io_error(path, source)),
        };
        let catalog: Catalog =
            serde_json::from_slice(&bytes).map_err(|source| ClaimsError::Parse {
                path: path.clone(),
                source,
            })?;
        if ![SCHEMA_VERSION, UNSEALED_SCHEMA_VERSION].contains(&catalog.schema_version()) {
            return Err(ClaimsError::UnsupportedSchema {
                path: path.clone(),
                found: catalog.schema_version(),
                supported: SCHEMA_VERSION,
            });
        }
        catalog.validate()?;
        Ok(catalog)
    }

    /// Serializes the catalog and atomically replaces the catalog file with it.
    fn save(&self, catalog: &Catalog) -> Result<()> {
        let paths = &self.paths;
        let mut bytes = serde_json::to_vec_pretty(catalog)?;
        bytes.push(b'\n');
        if let Err(error) = self.replace_catalog(&bytes) {
            // Best effort: a leftover temp file is harmless (the next save truncates it), so a
            // failure to remove it is ignored and the original error is reported.
            let _ = fs::remove_file(&paths.temp);
            return Err(error);
        }
        // The new catalog is already in place at this point. Reporting a failed directory sync
        // as a failed mutation could make the caller retry and apply the change twice, so it is
        // only logged.
        if let Err(error) = sync_directory(&paths.directory) {
            tracing::warn!("{error}");
        }
        Ok(())
    }

    /// Writes `bytes` to the temp file, flushes it to disk, and renames it over the catalog.
    fn replace_catalog(&self, bytes: &[u8]) -> Result<()> {
        let paths = &self.paths;
        let mut temp = File::create(&paths.temp).map_err(|source| io_error(&paths.temp, source))?;
        temp.write_all(bytes)
            .and_then(|()| temp.sync_all())
            .map_err(|source| io_error(&paths.temp, source))?;
        fs::rename(&paths.temp, &paths.catalog).map_err(|source| io_error(&paths.catalog, source))
    }
}

/// Makes the rename itself durable by syncing the containing directory (Unix only; on other
/// platforms directories cannot be opened as files and the rename is already durable enough).
#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|source| io_error(directory, source))
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> ClaimsError {
    ClaimsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, new_claim};

    fn temp_store() -> (tempfile::TempDir, CatalogStore) {
        let directory = tempfile::tempdir().expect("create temp dir");
        let store = CatalogStore::new(directory.path().join("claims.json"));
        (directory, store)
    }

    #[test]
    fn missing_file_reads_as_empty_catalog() {
        let (_directory, store) = temp_store();
        let count = store
            .read_blocking(|catalog| Ok(catalog.claims().len()))
            .expect("read");
        assert_eq!(count, 0);
        assert!(!store.path().exists());
    }

    #[test]
    fn mutate_persists_and_read_sees_it() {
        let (directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("persisted"), at(0)))
            .expect("mutate");

        let statement = store
            .read_blocking(|catalog| Ok(catalog.claims()[0].statement.clone()))
            .expect("read");
        assert_eq!(statement, "persisted");
        assert!(!directory.path().join(".claims.json.tmp").exists());
    }

    #[test]
    fn output_is_deterministic_and_ends_with_newline() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("a"), at(0)))
            .expect("mutate");
        let first = fs::read(store.path()).expect("read file");
        store.mutate_blocking(|_| Ok(())).expect("rewrite");
        let second = fs::read(store.path()).expect("read file");

        assert_eq!(first, second);
        assert_eq!(first.last(), Some(&b'\n'));
    }

    #[test]
    fn failed_operation_leaves_file_untouched() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("a"), at(0)))
            .expect("mutate");
        let before = fs::read(store.path()).expect("read file");

        let result = store.mutate_blocking(|catalog| {
            catalog.add_claim(new_claim("b"), at(1))?;
            Err::<(), _>(ClaimsError::Validation("forced failure".into()))
        });

        assert!(result.is_err());
        assert_eq!(fs::read(store.path()).expect("read file"), before);
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let (_directory, store) = temp_store();
        let document =
            r#"{"schema_version":3,"next_claim_seq":1,"next_evidence_seq":1,"claims":[]}"#;
        fs::write(store.path(), document).expect("write file");
        let result = store.read_blocking(|_| Ok(()));
        assert!(matches!(
            result,
            Err(ClaimsError::UnsupportedSchema { found: 3, .. })
        ));
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        let (_directory, store) = temp_store();
        fs::write(store.path(), "{ not json").expect("write file");
        assert!(matches!(
            store.read_blocking(|_| Ok(())),
            Err(ClaimsError::Parse { .. })
        ));
    }

    #[test]
    fn inconsistent_catalog_is_rejected_on_load() {
        let (_directory, store) = temp_store();
        let document = r#"{"schema_version":1,"next_claim_seq":1,"next_evidence_seq":1,"claims":[{
            "id":"CLM-0001","statement":"s","kind":"functional","level":"unit",
            "verification_method":"test","status":{"state":"active"},
            "created_at":"2026-10-23T00:00:00Z","updated_at":"2026-10-23T00:00:00Z"}]}"#;
        fs::write(store.path(), document).expect("write file");
        assert!(matches!(
            store.read_blocking(|_| Ok(())),
            Err(ClaimsError::Validation(_))
        ));
    }

    /// Rewrites the first claim's statement directly in the file, as an agent might.
    fn hand_edit_first_statement(store: &CatalogStore) {
        let text = fs::read_to_string(store.path()).expect("read file");
        fs::write(store.path(), text.replacen("\"first\"", "\"edited\"", 1)).expect("write");
    }

    #[test]
    fn writes_are_sealed_and_verify() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("first"), at(0)))
            .expect("mutate");
        let report = store
            .read_blocking(|catalog| Ok(seal::verify(catalog)))
            .expect("read");
        assert!(report.verified, "{:?}", report.problems);
        assert!(fs::read_to_string(store.path())
            .expect("read file")
            .contains("\"schema_version\": 2"));
    }

    #[test]
    fn tampered_catalog_refuses_writes_but_allows_reads() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("first"), at(0)))
            .expect("mutate");
        hand_edit_first_statement(&store);
        let before = fs::read(store.path()).expect("read file");

        let result = store.mutate_blocking(|catalog| catalog.add_claim(new_claim("b"), at(1)));
        let Err(ClaimsError::CatalogTampered { problems, .. }) = result else {
            panic!("expected CatalogTampered, got {result:?}");
        };
        assert_eq!(problems[0].record, "CLM-0001");
        assert_eq!(fs::read(store.path()).expect("read file"), before);
        let statement = store
            .read_blocking(|catalog| Ok(catalog.claims()[0].statement.clone()))
            .expect("reads still work");
        assert_eq!(statement, "edited");
    }

    #[test]
    fn lowered_schema_version_is_tampering_not_an_unmigrated_catalog() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("first"), at(0)))
            .expect("mutate");
        let text = fs::read_to_string(store.path()).expect("read file");
        let lowered = text.replacen("\"schema_version\": 2", "\"schema_version\": 1", 1);
        fs::write(store.path(), lowered).expect("write file");

        let result = store.mutate_blocking(|_| Ok(()));
        let Err(ClaimsError::CatalogTampered { problems, .. }) = result else {
            panic!("expected CatalogTampered, got {result:?}");
        };
        assert_eq!(problems[0].record, "schema_version");
        assert!(matches!(
            store.adopt_blocking(),
            Err(ClaimsError::AdoptRefused { .. })
        ));
    }

    #[test]
    fn adopt_migrates_a_v1_file_and_unblocks_writes() {
        let (_directory, store) = temp_store();
        let document = r#"{"schema_version":1,"next_claim_seq":2,"next_evidence_seq":1,"claims":[{
            "id":"CLM-0001","statement":"s","kind":"functional","level":"unit",
            "verification_method":"test","status":{"state":"active"},
            "created_at":"2026-10-23T00:00:00Z","updated_at":"2026-10-23T00:00:00Z"}]}"#;
        fs::write(store.path(), document).expect("write file");
        assert!(matches!(
            store.mutate_blocking(|_| Ok(())),
            Err(ClaimsError::UnsealedCatalog { .. })
        ));

        let adopted = store.adopt_blocking().expect("adopt");
        assert_eq!(adopted, ["CLM-0001", "next_claim_seq", "next_evidence_seq"]);
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("after"), at(1)))
            .expect("writes work after adoption");
        assert!(store.adopt_blocking().expect("adopt again").is_empty());
    }

    #[test]
    fn relative_path_without_directory_uses_current_directory() {
        let store = CatalogStore::new("claims.json");
        assert_eq!(store.paths.lock, PathBuf::from("./.claims.json.lock"));
    }

    #[tokio::test]
    async fn async_wrappers_run_on_blocking_pool() {
        let (_directory, store) = temp_store();
        store
            .mutate(|catalog| catalog.add_claim(new_claim("async"), at(0)))
            .await
            .expect("mutate");
        let count = store
            .read(|catalog| Ok(catalog.claims().len()))
            .await
            .expect("read");
        assert_eq!(count, 1);
    }

    #[test]
    fn concurrent_writers_never_lose_updates() {
        let (_directory, store) = temp_store();
        let writers: Vec<_> = (0..4)
            .map(|writer| {
                // A separate store per thread opens its own lock handle, like another process.
                let store = CatalogStore::new(store.path());
                std::thread::spawn(move || {
                    for round in 0..10 {
                        store
                            .mutate_blocking(|catalog| {
                                let statement = format!("writer {writer} round {round}");
                                catalog.add_claim(new_claim(&statement), at(0))
                            })
                            .expect("mutate");
                    }
                })
            })
            .collect();
        for handle in writers {
            handle.join().expect("writer thread");
        }
        // Loading also runs Catalog::validate, so duplicate ids would fail this read.
        let count = store
            .read_blocking(|catalog| Ok(catalog.claims().len()))
            .expect("read");
        assert_eq!(count, 40);
    }
}
