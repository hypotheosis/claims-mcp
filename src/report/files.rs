//! The PDF report's Typst template, its text helpers, and the commented notes example,
//! compiled into the binary. Every install therefore carries the report files that match its
//! version: `claims-mcp report init` writes them into a project, and the MCP server serves them
//! as read-only resources (issue #18).

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;

/// One embedded report file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportFile {
    /// File name in the report directory; also the last segment of its resource URI.
    pub name: &'static str,
    /// What the file is for, shown in the MCP resource listing.
    pub description: &'static str,
    /// MIME type served with the resource.
    pub mime_type: &'static str,
    /// The file's exact contents.
    pub contents: &'static str,
}

impl ReportFile {
    /// MCP resource URI, e.g. `claims://report/claims-report.typ`.
    pub fn uri(&self) -> String {
        format!("{URI_PREFIX}{}", self.name)
    }
}

/// Prefix of every report resource URI.
const URI_PREFIX: &str = "claims://report/";

/// Directory `report init` writes to when none is given, relative to the current directory.
pub const DEFAULT_DIR: &str = "report";

/// Name of the project's own notes file, which `report init` creates from the example.
pub const NOTES_FILE: &str = "audit-notes.toml";

/// The commented example every project's notes file starts from.
pub const NOTES_EXAMPLE: ReportFile = ReportFile {
    name: "audit-notes.example.toml",
    description: "Commented example of the report's notes file (the auditor's assessment and \
        points for the evaluator). Copy it to audit-notes.toml next to the template, fill it \
        in, and pass it with `--input notes=<path>`.",
    mime_type: "application/toml",
    // Rust note: `include_str!` reads the file at compile time, so the binary carries this
    // repository's copy byte for byte; the path is relative to this source file.
    contents: include_str!("../../report/audit-notes.example.toml"),
};

/// Every embedded file, in the order `report init` writes them.
///
/// Rust note: a `static` (unlike a `const`) has one address for the whole program, so
/// `by_uri` can hand out `&'static` references into it.
pub static FILES: [ReportFile; 3] = [
    ReportFile {
        name: "claims-report.typ",
        description: "Typst template that renders a claims catalog as a PDF audit report. \
            Compile it with `typst compile --root .`; its header comment lists the inputs.",
        mime_type: "text/plain",
        contents: include_str!("../../report/claims-report.typ"),
    },
    ReportFile {
        name: "text.typ",
        description: "Text helpers imported by claims-report.typ; keep it next to the template.",
        mime_type: "text/plain",
        contents: include_str!("../../report/text.typ"),
    },
    NOTES_EXAMPLE,
];

/// The embedded file served at `uri`, if any.
pub fn by_uri(uri: &str) -> Option<&'static ReportFile> {
    let name = uri.strip_prefix(URI_PREFIX)?;
    FILES.iter().find(|file| file.name == name)
}

/// What `init` did with one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitAction {
    /// The file was written (created or replaced).
    Wrote,
    /// The file already existed and was left untouched.
    Kept,
}

/// Writes the report files into `dir`, creating it if needed.
///
/// The template, its helpers, and the notes example are always replaced, so they match this
/// binary. The project's notes file (`NOTES_FILE`) is created from the example only when it does
/// not exist yet: it holds the auditor's own prose and is never overwritten.
///
/// # Arguments
///
/// * `dir` - Directory to write into, usually `report/` under the project root.
///
/// # Returns
///
/// Each path in write order, with what was done to it.
///
/// # Errors
///
/// Creating `dir` or writing any file fails; the error names the path.
pub fn init(dir: &Path) -> anyhow::Result<Vec<(PathBuf, InitAction)>> {
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut done = Vec::with_capacity(FILES.len() + 1);
    for file in &FILES {
        let path = dir.join(file.name);
        fs::write(&path, file.contents)
            .with_context(|| format!("cannot write {}", path.display()))?;
        done.push((path, InitAction::Wrote));
    }
    let notes = dir.join(NOTES_FILE);
    let action = create_new(&notes, NOTES_EXAMPLE.contents)
        .with_context(|| format!("cannot write {}", notes.display()))?;
    done.push((notes, action));
    Ok(done)
}

/// Writes `contents` to `path` only if `path` does not exist.
///
/// `create_new` checks and creates in one step, so a file that appears between a check and
/// the write cannot be overwritten.
fn create_new(path: &Path, contents: &str) -> std::io::Result<InitAction> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(contents.as_bytes())?;
            Ok(InitAction::Wrote)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(InitAction::Kept),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path of a file under `report/` in this repository.
    fn repo_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("report")
            .join(name)
    }

    #[test]
    fn embedded_files_match_the_repository() {
        for file in &FILES {
            let on_disk = fs::read_to_string(repo_file(file.name)).expect("read report file");
            assert_eq!(file.contents, on_disk, "{} drifted", file.name);
        }
    }

    #[test]
    fn by_uri_finds_each_file_and_nothing_else() {
        for file in &FILES {
            assert_eq!(by_uri(&file.uri()), Some(file));
        }
        assert_eq!(
            NOTES_EXAMPLE.uri(),
            "claims://report/audit-notes.example.toml"
        );
        assert_eq!(by_uri("claims://report/text-tests.typ"), None);
        assert_eq!(by_uri("claims://report/"), None);
        assert_eq!(by_uri("file:///text.typ"), None);
    }

    #[test]
    fn init_writes_every_file_and_creates_notes_from_the_example() {
        let directory = tempfile::tempdir().expect("temp dir");
        let dir = directory.path().join("report");

        let done = init(&dir).expect("init");

        let names: Vec<_> = done
            .iter()
            .map(|(path, action)| (path.file_name().expect("name").to_owned(), *action))
            .collect();
        assert_eq!(
            names,
            [
                "claims-report.typ",
                "text.typ",
                "audit-notes.example.toml",
                NOTES_FILE
            ]
            .map(|name| (name.into(), InitAction::Wrote))
        );
        for file in &FILES {
            assert_eq!(
                fs::read_to_string(dir.join(file.name)).expect("read"),
                file.contents
            );
        }
        assert_eq!(
            fs::read_to_string(dir.join(NOTES_FILE)).expect("read"),
            NOTES_EXAMPLE.contents
        );
    }

    #[test]
    fn init_keeps_existing_notes_and_refreshes_the_template() {
        let directory = tempfile::tempdir().expect("temp dir");
        let dir = directory.path();
        fs::write(dir.join(NOTES_FILE), "artifact = \"mine\"\n").expect("write notes");
        fs::write(dir.join("claims-report.typ"), "stale").expect("write template");

        let done = init(dir).expect("init");

        assert_eq!(done.last(), Some(&(dir.join(NOTES_FILE), InitAction::Kept)));
        assert_eq!(
            fs::read_to_string(dir.join(NOTES_FILE)).expect("read"),
            "artifact = \"mine\"\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("claims-report.typ")).expect("read"),
            FILES[0].contents
        );
    }
}
