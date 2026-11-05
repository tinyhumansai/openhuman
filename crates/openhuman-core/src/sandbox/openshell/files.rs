//! Moving a file into or out of a tenant's box, over plain command execution.
//!
//! Nothing here needs more than [`BoxDriver::exec`]: a file travels as base64
//! inside the argument of a `sh -c` command, so every box driver supports it
//! and no box contract grows. Paths are relative to the box's work folder
//! ([`BOX_WORKDIR`](super::registry::BOX_WORKDIR), the directory every command
//! runs in), and every path that reaches the shell is single-quoted and
//! written as `./<path>`, so a file named `-` is a file and not an option or
//! standard input. Every command first resolves the path inside the box and
//! refuses one that leaves the work folder (a symlink pointing out of it
//! included).
//!
//! - **Write** sends [`WRITE_CHUNK_BYTES`] at a time (a Linux argument is at
//!   most 128 KiB; the encoding adds a third), appending to a part file
//!   beside `<path>` whose name is unique to the transfer and created
//!   exclusively, so neither a file the user already has nor another
//!   upload's chunks can be overwritten. The last command checks the part
//!   file's length and only then renames it over `<path>`, so a reader never
//!   sees half a file, and a box replaced mid-write (it forgets the earlier
//!   chunks) fails the length check instead of keeping a truncated file. A
//!   failure removes the part file; one abandoned by a cancelled transfer is
//!   removed by the next write into its folder once it is an hour old.
//! - **Read** checks the file exists and reads its size first, refuses a file
//!   over the limit before moving any of it, then reads
//!   [`READ_CHUNK_BYTES`] at a time (a hosted exec keeps at most 512 KB of
//!   output per stream) and checks the total. The file's identity (device,
//!   inode and modification time) is taken with its size and checked before
//!   and after every chunk, so a file replaced mid-read is an error rather
//!   than a mix of two versions. A read that comes back short is an error,
//!   never a short file.
//!
//! Both are keyed on the whole tenant like a command is; a call with no tenant
//! is refused before a box is touched.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;

use super::{tenant_for_box, BoxDriver, BoxError, BoxRegistry, BoxRequest, BoxSettings};
use crate::config::OpenShellRuntimeConfig;
use crate::core::runtime::{NoTenant, Tenant};
use crate::sandbox::types::SandboxExecResult;

/// The largest file the host moves into or out of a box: 8 MiB.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Raw bytes per write command: 64 KiB once encoded, under a Linux
/// argument's 128 KiB.
pub(crate) const WRITE_CHUNK_BYTES: usize = 48 * 1024;

/// Raw bytes per read command: about 342 KB once encoded, under a hosted
/// exec's 512 KB of output per stream.
pub(crate) const READ_CHUNK_BYTES: usize = 256 * 1024;

/// How long one transfer command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The exit status the size check uses for "not a regular file".
const NOT_A_FILE_EXIT: i32 = 66;

/// A part file this old was abandoned (its transfer was cancelled or its host
/// went away, so nothing removed it); the next write into the folder removes it.
const STALE_PART_MINUTES: u32 = 60;

/// The pattern every part file's name matches (see [`part_path`]).
const STALE_PART_GLOB: &str = ".openhuman-transfer-????????????????????????????????.tmp";

/// The file-name prefix reserved for part files: a path naming one is refused,
/// so the stale sweep only ever meets files a transfer made.
const PART_PREFIX: &str = ".openhuman-transfer-";

/// The longest box message kept in an error.
const DETAIL_MAX_CHARS: usize = 200;

/// Write `bytes` to `path` in the box of the tenant `tenant` resolves to,
/// creating parent folders and replacing any file already there.
///
/// `path` is relative to the box's work folder.
///
/// # Errors
///
/// [`BoxError::NoTenant`], [`BoxError::InvalidPath`] and
/// [`BoxError::FileTooLarge`] (over [`MAX_FILE_BYTES`]) before any box is
/// touched; [`BoxError::FileTransfer`] when a command fails (the part file is
/// removed); otherwise the settings and registry errors.
pub async fn write_file_in<D: BoxDriver>(
    registry: &BoxRegistry<D>,
    tenant: Result<Tenant, NoTenant>,
    openshell: &OpenShellRuntimeConfig,
    path: &str,
    bytes: &[u8],
) -> Result<(), BoxError> {
    let tenant = tenant_for_box(tenant)?;
    check_path(path)?;
    let size = bytes.len() as u64;
    if size > MAX_FILE_BYTES {
        return Err(BoxError::FileTooLarge {
            size,
            limit: MAX_FILE_BYTES,
        });
    }
    let settings = BoxSettings::from_config(openshell)?;
    let target = Target {
        registry,
        tenant: &tenant,
        settings: &settings,
        path,
    };

    let part = part_path(path);
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![bytes]
    } else {
        bytes.chunks(WRITE_CHUNK_BYTES).collect()
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let command = write_command(path, &part, chunk, index, chunks.len(), size);
        let failed = match target.run(&command).await {
            Ok(result) if result.exit_code == 0 => continue,
            Ok(result) => failed_command("write", path, &result),
            Err(error) => error,
        };
        // Best effort: the box may be gone, and the transfer failed already.
        let _ = target
            .run(&format!("rm -f -- {}", quote(&relative(&part))))
            .await;
        return Err(failed);
    }
    Ok(())
}

/// Read the file at `path` from the box of the tenant `tenant` resolves to.
///
/// `path` is relative to the box's work folder. A file over `max_bytes`, or
/// over [`MAX_FILE_BYTES`], is refused before any of it is transferred.
///
/// # Errors
///
/// [`BoxError::NoTenant`] and [`BoxError::InvalidPath`] before any box is
/// touched; [`BoxError::FileNotFound`] when `path` is not a regular file;
/// [`BoxError::FileTooLarge`] over the limit; [`BoxError::ShortRead`] when
/// fewer bytes arrive than the file's size; [`BoxError::FileTransfer`] when a
/// command fails; otherwise the settings and registry errors.
pub async fn read_file_in<D: BoxDriver>(
    registry: &BoxRegistry<D>,
    tenant: Result<Tenant, NoTenant>,
    openshell: &OpenShellRuntimeConfig,
    path: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, BoxError> {
    let tenant = tenant_for_box(tenant)?;
    check_path(path)?;
    let settings = BoxSettings::from_config(openshell)?;
    let target = Target {
        registry,
        tenant: &tenant,
        settings: &settings,
        path,
    };

    let quoted = quote(&relative(path));
    let sized = target
        .run(&format!(
            "test -f {quoted} || exit {NOT_A_FILE_EXIT}; {GUARD} \
             inside \"$(realpath -- {quoted})\"; stat -L -c '%s %d:%i:%y:%z' -- {quoted}"
        ))
        .await?;
    if sized.exit_code == NOT_A_FILE_EXIT {
        return Err(BoxError::FileNotFound(path.to_owned()));
    }
    if sized.exit_code != 0 {
        return Err(failed_command("read", path, &sized));
    }
    let unreported = || BoxError::FileTransfer {
        action: "read",
        path: path.to_owned(),
        detail: "the sandbox did not report the file's size".to_owned(),
    };
    // The size, then the identity: the rest of the line (its times hold spaces).
    let (size, identity) = sized
        .stdout
        .trim()
        .split_once(char::is_whitespace)
        .ok_or_else(unreported)?;
    let size: u64 = size.parse().map_err(|_| unreported())?;
    let identity = identity.trim();
    let limit = max_bytes.min(MAX_FILE_BYTES);
    if size > limit {
        return Err(BoxError::FileTooLarge { size, limit });
    }

    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    while (bytes.len() as u64) < size {
        let want = (size - bytes.len() as u64).min(READ_CHUNK_BYTES as u64);
        let command = read_command(&quoted, identity, bytes.len() + 1, want);
        let chunk = target.run(&command).await?;
        if chunk.exit_code != 0 {
            return Err(failed_command("read", path, &chunk));
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(chunk.stdout.trim())
            .map_err(|_| BoxError::FileTransfer {
                action: "read",
                path: path.to_owned(),
                detail: "the sandbox returned data that could not be decoded".to_owned(),
            })?;
        if decoded.len() as u64 != want {
            return Err(BoxError::ShortRead {
                expected: size,
                got: bytes.len() as u64 + decoded.len() as u64,
            });
        }
        bytes.extend_from_slice(&decoded);
    }
    Ok(bytes)
}

/// One transfer's box and path, to run its commands against.
struct Target<'a, D: BoxDriver> {
    registry: &'a BoxRegistry<D>,
    tenant: &'a Tenant,
    settings: &'a BoxSettings,
    path: &'a str,
}

impl<D: BoxDriver> Target<'_, D> {
    async fn run(&self, command: &str) -> Result<SandboxExecResult, BoxError> {
        tracing::debug!(
            path_len = self.path.len(),
            "[sandbox:openshell] running a file transfer command"
        );
        let request = BoxRequest {
            command: command.to_owned(),
            env: BTreeMap::new(),
            timeout: COMMAND_TIMEOUT,
        };
        self.registry
            .exec(self.tenant, self.settings, &request)
            .await
    }
}

/// Shell that defines `inside`, which exits the command unless the path it is
/// given (already resolved) is the work folder or under it.
const GUARD: &str = "base=$(realpath .) || exit 1; \
    inside() { case \"$1\" in \"$base\"|\"$base\"/*) ;; \
    *) echo 'it resolves outside the sandbox work folder' >&2; exit 1;; esac; }; ";

/// The command for chunk `index` of `count`. The first resolves the folder
/// inside the work folder, creates it, and starts the part file exclusively;
/// the rest append; the last checks the part file holds all `size` bytes and
/// renames it into place.
fn write_command(
    path: &str,
    part: &str,
    chunk: &[u8],
    index: usize,
    count: usize,
    size: u64,
) -> String {
    let (quoted_path, quoted_part) = (quote(&relative(path)), quote(&relative(part)));
    let folder = quote(&relative(
        path.rsplit_once('/').map_or(".", |(folder, _)| folder),
    ));
    let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
    let last = index + 1 == count;
    let mut command = String::new();
    if index == 0 || last {
        command.push_str(GUARD);
        command.push_str(&format!(
            "if test -d {quoted_path}; then echo 'it is a folder' >&2; exit 1; fi; "
        ));
    }
    if index == 0 {
        // Check the deepest folder that exists already, then create the rest
        // and check the whole again: nothing is made outside the work folder.
        command.push_str(&format!(
            "d={folder}; while ! test -e \"$d\" && ! test -L \"$d\"; do d=$(dirname \"$d\"); done; \
             inside \"$(realpath -- \"$d\")\"; mkdir -p -- {folder} || exit 1; \
             inside \"$(realpath -- {folder})\"; \
             find {folder} -maxdepth 1 -type f -name '{STALE_PART_GLOB}' -mmin +{STALE_PART_MINUTES} \
             -exec rm -f -- {{}} + 2>/dev/null; \
             ( set -C; printf %s '{encoded}' | base64 -d > {quoted_part} ) || exit 1"
        ));
    } else {
        command.push_str(&format!(
            "if test -L {quoted_part}; then echo 'the part file is a link' >&2; exit 1; fi; \
             printf %s '{encoded}' | base64 -d >> {quoted_part}"
        ));
    }
    if last {
        command.push_str(&format!(
            "; inside \"$(realpath -- {folder})\"; \
             if test \"$(wc -c < {quoted_part})\" -eq {size}; \
             then mv -f -- {quoted_part} {quoted_path}; \
             else echo 'the file was not written in full' >&2; exit 1; fi"
        ));
    }
    command
}

/// The command that reads `want` bytes of the file `quoted` from byte
/// `start` (1-based), failing if the file is not the one whose `identity` was
/// taken, before or after the read.
fn read_command(quoted: &str, identity: &str, start: usize, want: u64) -> String {
    let same = format!(
        "[ \"$(stat -L -c '%d:%i:%y:%z' -- {quoted})\" = {} ] || \
         {{ echo 'the file changed while it was read' >&2; exit 1; }}; ",
        quote(identity)
    );
    format!(
        "{GUARD}inside \"$(realpath -- {quoted})\"; {same}\
         tail -c +{start} -- {quoted} | head -c {want} | base64 -w0; {same}"
    )
}

/// A unique part-file path beside `path`: hidden, and named for this transfer
/// alone (not for the target, so a target name near the filesystem's 255-byte
/// limit still stages) so no other file or transfer shares it.
fn part_path(path: &str) -> String {
    let name = format!("{PART_PREFIX}{}.tmp", uuid::Uuid::new_v4().simple());
    match path.rsplit_once('/') {
        Some((folder, _)) => format!("{folder}/{name}"),
        None => name,
    }
}

/// `path` as an explicit relative path, so a name starting with `-` is never
/// read as an option, nor `-` as standard input.
fn relative(path: &str) -> String {
    format!("./{path}")
}

/// Refuse a path that is not a file inside the box's work folder.
fn check_path(path: &str) -> Result<(), BoxError> {
    let refusal = if path.is_empty() {
        "it is empty"
    } else if path.contains('\0') {
        "it contains a NUL byte"
    } else if path.starts_with('/') {
        "it must be relative to the sandbox work folder"
    } else if path.split('/').any(|part| part == "..") {
        "it must not contain `..`"
    } else if matches!(path.rsplit('/').next(), Some("" | ".")) {
        "it names a folder, not a file"
    } else if path
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with(PART_PREFIX))
    {
        "that file name is reserved for transfers"
    } else {
        return Ok(());
    };
    Err(BoxError::InvalidPath(refusal))
}

/// `value` as one single-quoted shell word.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// A failed command as an error: the box's first message line, else why.
fn failed_command(action: &'static str, path: &str, result: &SandboxExecResult) -> BoxError {
    let message = result
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(DETAIL_MAX_CHARS).collect::<String>());
    let detail = if result.timed_out {
        "the sandbox took too long".to_owned()
    } else {
        message.unwrap_or_else(|| format!("the command exited with status {}", result.exit_code))
    };
    BoxError::FileTransfer {
        action,
        path: path.to_owned(),
        detail,
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
