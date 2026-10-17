//! Step 6 and `storage.restore`: the `tar.zst` archive of what the migration
//! moves and deletes.
//!
//! The archive is streamed: files are read, hashed and written one at a time
//! into a zstd encoder over the output file, so a 16 GB, 300k-file profile
//! never sits in memory. Entries are stored under `workspace/<path>`, the
//! place they had in layout v1, whatever their location when archived (moved
//! entries are read from `files/`). The last tar entry is `MANIFEST.json`
//! (path, size and sha256 of every file); the same manifest is written beside
//! the archive as `<name>.manifest.json`.
//!
//! [`verify`] re-reads the whole archive and compares every entry to the
//! manifest. The migration writes `<name>.tmp`, verifies it, and only then
//! renames it into place, so a file with the final name has been verified.
//! [`restore`] verifies first, then puts the entries back under `workspace/`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::plan::Action;
use super::state::LayoutError;

/// The manifest's tar entry name.
pub const MANIFEST_ENTRY: &str = "MANIFEST.json";
/// The prefix every archived path carries.
pub const ROOT: &str = "workspace";
const ZSTD_LEVEL: i32 = 3;

/// One archived file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// The path inside the archive (`workspace/skills/a/SKILL.md`).
    pub path: String,
    /// `file` or `symlink`.
    pub kind: String,
    /// Bytes of a file (0 for a symlink).
    pub size: u64,
    /// Hex sha256 of the contents (of the link target for a symlink).
    pub sha256: String,
}

/// A top-level plan entry the archive holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRecord {
    /// The path relative to `workspace/`.
    pub path: String,
    /// What the migration did with it.
    pub action: Action,
}

/// The archive's table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version.
    pub version: u32,
    /// The user the profile belongs to.
    pub user: String,
    /// When the archive was written.
    pub created_ms: i64,
    /// The plan entries archived.
    pub entries: Vec<EntryRecord>,
    /// Every file, in archive order.
    pub files: Vec<FileRecord>,
}

/// One thing to archive: where it is now and what the plan calls it.
#[derive(Debug, Clone)]
pub struct Source {
    /// The plan path relative to `workspace/`.
    pub rel: String,
    /// The action recorded in the manifest.
    pub action: Action,
    /// Where the entry is on disk now.
    pub at: PathBuf,
}

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

fn io(error: impl Into<anyhow::Error>) -> LayoutError {
    LayoutError::Other(error.into())
}

/// Streams `sources` into `dest` (created, replaced if present) and returns
/// the manifest it wrote. The sources are only read.
///
/// # Errors
///
/// An I/O error. A partly written `dest` is removed.
pub fn write(
    dest: &Path,
    user: &str,
    created_ms: i64,
    sources: &[Source],
) -> Result<Manifest, LayoutError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
        restrict(parent, 0o700);
    }
    let result = write_inner(dest, user, created_ms, sources);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn write_inner(
    dest: &Path,
    user: &str,
    created_ms: i64,
    sources: &[Source],
) -> Result<Manifest, LayoutError> {
    let file = File::create(dest)?;
    restrict(dest, 0o600);
    let encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), ZSTD_LEVEL)?;
    let mut tar = tar::Builder::new(encoder);
    tar.follow_symlinks(false);
    let mut manifest = Manifest {
        version: 1,
        user: user.to_owned(),
        created_ms,
        entries: sources
            .iter()
            .map(|s| EntryRecord {
                path: s.rel.clone(),
                action: s.action,
            })
            .collect(),
        files: Vec::new(),
    };
    for source in sources {
        let root = format!("{ROOT}/{}", source.rel);
        for item in WalkDir::new(&source.at)
            .follow_links(false)
            .sort_by_file_name()
        {
            let item = item.map_err(io)?;
            let relative = item.path().strip_prefix(&source.at).map_err(io)?;
            let name = if relative.as_os_str().is_empty() {
                root.clone()
            } else {
                format!("{root}/{}", relative.to_string_lossy().replace('\\', "/"))
            };
            let kind = item.file_type();
            if kind.is_dir() {
                tar.append_dir(&name, item.path())?;
            } else if kind.is_symlink() {
                let target = std::fs::read_link(item.path())?;
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                tar.append_link(&mut header, &name, &target)?;
                manifest.files.push(FileRecord {
                    path: name,
                    kind: "symlink".to_owned(),
                    size: 0,
                    sha256: hex::encode(Sha256::digest(target.to_string_lossy().as_bytes())),
                });
            } else if kind.is_file() {
                let meta = item.metadata().map_err(io)?;
                let mut header = tar::Header::new_gnu();
                header.set_metadata(&meta);
                let mut reader = HashingReader {
                    inner: BufReader::new(File::open(item.path())?),
                    hasher: Sha256::new(),
                };
                // The size in the header is what the tar stream will carry; a
                // file that changes while it is read fails the append.
                tar.append_data(&mut header, &name, &mut reader)?;
                manifest.files.push(FileRecord {
                    path: name,
                    kind: "file".to_owned(),
                    size: meta.len(),
                    sha256: hex::encode(reader.hasher.finalize()),
                });
            }
        }
    }
    let body = serde_json::to_vec(&manifest).map_err(io)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(u64::try_from(created_ms / 1000).unwrap_or(0));
    tar.append_data(&mut header, MANIFEST_ENTRY, body.as_slice())?;
    let encoder = tar.into_inner()?;
    let mut out = encoder.finish()?;
    out.flush()?;
    out.get_ref().sync_all()?;
    write_sidecar(dest, &body)?;
    tracing::info!(
        target: "openhuman::storage",
        files = manifest.files.len(),
        "[layout] archive written"
    );
    Ok(manifest)
}

/// `<archive>.manifest.json`.
pub fn sidecar_path(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(".manifest.json");
    archive.with_file_name(name)
}

fn write_sidecar(archive: &Path, body: &[u8]) -> Result<(), LayoutError> {
    std::fs::write(sidecar_path(archive), body)?;
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

fn open_archive(
    path: &Path,
) -> Result<tar::Archive<zstd::stream::read::Decoder<'static, BufReader<File>>>, LayoutError> {
    let decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
    Ok(tar::Archive::new(decoder))
}

/// Re-reads `archive` and checks every entry against the manifest it carries:
/// every manifest file present with the recorded size and sha256, no file the
/// manifest does not list.
///
/// # Errors
///
/// [`LayoutError::Verify`] naming the first discrepancy (or a read/decode
/// failure, which is also a corrupt archive).
pub fn verify(archive: &Path) -> Result<Manifest, LayoutError> {
    let bad = |m: String| LayoutError::Verify(format!("archive {}: {m}", archive.display()));
    let mut tar = open_archive(archive)?;
    let mut seen: BTreeMap<String, (String, u64, String)> = BTreeMap::new();
    let mut manifest: Option<Manifest> = None;
    for entry in tar.entries().map_err(|e| bad(format!("unreadable: {e}")))? {
        let mut entry = entry.map_err(|e| bad(format!("unreadable entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| bad(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        let kind = entry.header().entry_type();
        if path == MANIFEST_ENTRY {
            let mut body = Vec::new();
            entry
                .read_to_end(&mut body)
                .map_err(|e| bad(format!("manifest unreadable: {e}")))?;
            manifest = Some(
                serde_json::from_slice(&body).map_err(|e| bad(format!("manifest damaged: {e}")))?,
            );
        } else if kind.is_file() {
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = entry
                    .read(&mut buf)
                    .map_err(|e| bad(format!("{path}: {e}")))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                size += n as u64;
            }
            seen.insert(
                path,
                ("file".to_owned(), size, hex::encode(hasher.finalize())),
            );
        } else if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| bad(e.to_string()))?
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            seen.insert(
                path,
                (
                    "symlink".to_owned(),
                    0,
                    hex::encode(Sha256::digest(target.as_bytes())),
                ),
            );
        }
    }
    let manifest = manifest.ok_or_else(|| bad("no manifest entry".to_owned()))?;
    for record in &manifest.files {
        match seen.remove(&record.path) {
            None => return Err(bad(format!("{} is missing", record.path))),
            Some((kind, size, sha)) => {
                if kind != record.kind || size != record.size || sha != record.sha256 {
                    return Err(bad(format!("{} does not match the manifest", record.path)));
                }
            }
        }
    }
    if let Some(extra) = seen.keys().next() {
        return Err(bad(format!("{extra} is not in the manifest")));
    }
    Ok(manifest)
}

/// Verifies `archive`, then extracts every entry under `profile_dir` (the
/// entries carry the `workspace/` prefix). Existing files are overwritten.
/// Returns the manifest.
///
/// # Errors
///
/// A failed verification (nothing is extracted then), a path that would leave
/// `profile_dir`, or an I/O error.
pub fn restore(archive: &Path, profile_dir: &Path) -> Result<Manifest, LayoutError> {
    let manifest = verify(archive)?;
    let mut tar = open_archive(archive)?;
    tar.set_preserve_permissions(true);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path == Path::new(MANIFEST_ENTRY) {
            continue;
        }
        if !path.starts_with(ROOT) {
            return Err(LayoutError::Verify(format!(
                "{} is outside {ROOT}/",
                path.display()
            )));
        }
        if !entry.unpack_in(profile_dir)? {
            return Err(LayoutError::Verify(format!(
                "{} escapes the profile directory",
                path.display()
            )));
        }
    }
    Ok(manifest)
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
