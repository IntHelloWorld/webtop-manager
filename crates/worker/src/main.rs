use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Job {
    Preflight {
        source: PathBuf,
    },
    Snapshot {
        source: PathBuf,
        destination: PathBuf,
    },
    Restore {
        archive: PathBuf,
        destination: PathBuf,
    },
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    files: u64,
    directories: u64,
    symlinks: u64,
    skipped_special: u64,
    original_bytes: u64,
    archive_bytes: Option<u64>,
    sha256: Option<String>,
    sensitive_paths: Vec<String>,
}

fn main() -> Result<()> {
    let job_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: webtop-worker JOB.json")?;
    let job: Job = serde_json::from_reader(BufReader::new(File::open(job_path)?))?;
    let report = match job {
        Job::Preflight { source } => scan(&source)?,
        Job::Snapshot {
            source,
            destination,
        } => snapshot(&source, &destination)?,
        Job::Restore {
            archive,
            destination,
        } => restore(&archive, &destination)?,
    };
    serde_json::to_writer(std::io::stdout().lock(), &report)?;
    Ok(())
}

fn scan(source: &Path) -> Result<Report> {
    ensure_absolute(source)?;
    let mut report = Report::default();
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        if entry.path() == source {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        let file_type = metadata.file_type();
        if file_type.is_file() {
            report.files += 1;
            report.original_bytes = report.original_bytes.saturating_add(metadata.len());
        } else if file_type.is_dir() {
            report.directories += 1;
        } else if file_type.is_symlink() {
            report.symlinks += 1;
        } else {
            report.skipped_special += 1;
        }
        if is_sensitive(entry.path(), source) {
            report
                .sensitive_paths
                .push(relative_display(entry.path(), source));
        }
    }
    report.sensitive_paths.sort();
    report.sensitive_paths.dedup();
    Ok(report)
}

fn snapshot(source: &Path, destination: &Path) -> Result<Report> {
    ensure_absolute(source)?;
    ensure_absolute(destination)?;
    let mut report = scan(source)?;
    let parent = destination
        .parent()
        .context("snapshot destination has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.partial",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .context("snapshot filename is not UTF-8")?
    ));
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .context("create temporary snapshot")?;
    let encoder = zstd::Encoder::new(BufWriter::new(output), 9)?;
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    let mut hardlinks: HashMap<(u64, u64), PathBuf> = HashMap::new();

    let result = (|| -> Result<()> {
        for entry in WalkDir::new(source).follow_links(false) {
            let entry = entry?;
            if entry.path() == source {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            let kind = metadata.file_type();
            if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
                continue;
            }
            let relative = entry.path().strip_prefix(source)?;
            if kind.is_file() && metadata.nlink() > 1 {
                let key = (metadata.dev(), metadata.ino());
                if let Some(first_path) = hardlinks.get(&key) {
                    let mut header = tar::Header::new_gnu();
                    header.set_metadata(&metadata);
                    header.set_entry_type(tar::EntryType::Link);
                    header.set_size(0);
                    archive.append_link(&mut header, relative, first_path)?;
                    continue;
                }
                hardlinks.insert(key, relative.to_owned());
            }
            archive.append_path_with_name(entry.path(), relative)?;
        }
        let encoder = archive.into_inner()?;
        let mut output = encoder.finish()?;
        output.flush()?;
        output.get_ref().sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let (sha256, archive_bytes) = hash_file(&temporary)?;
    fs::rename(&temporary, destination)?;
    sync_parent(parent)?;
    report.sha256 = Some(sha256);
    report.archive_bytes = Some(archive_bytes);
    Ok(report)
}

fn restore(archive: &Path, destination: &Path) -> Result<Report> {
    ensure_absolute(archive)?;
    ensure_absolute(destination)?;
    if destination.exists() && destination.read_dir()?.next().is_some() {
        bail!("restore destination must be empty");
    }
    fs::create_dir_all(destination)?;
    let input = BufReader::new(File::open(archive)?);
    let decoder = zstd::Decoder::new(input)?;
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.unpack_in(destination)? {
            bail!("archive entry escaped restore destination");
        }
    }
    scan(destination)
}

fn is_sensitive(path: &Path, root: &Path) -> bool {
    const NAMES: &[&str] = &[".ssh", ".gnupg", ".aws", ".config", "keyring", "secrets"];
    path.strip_prefix(root)
        .ok()
        .into_iter()
        .flat_map(Path::components)
        .any(|part| {
            let value = part.as_os_str().to_string_lossy().to_ascii_lowercase();
            NAMES.iter().any(|name| value == *name)
        })
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn ensure_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("worker paths must be absolute");
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let bytes = std::io::copy(&mut file, &mut hasher)?;
    Ok((hex::encode(hasher.finalize()), bytes))
}

fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    #[test]
    fn detects_sensitive_directories() {
        assert!(is_sensitive(
            Path::new("/config/home/user/.ssh/id_ed25519"),
            Path::new("/config")
        ));
        assert!(!is_sensitive(
            Path::new("/config/home/user/project/main.rs"),
            Path::new("/config")
        ));
    }

    #[test]
    fn snapshot_round_trip_preserves_hidden_links_and_permissions() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let archive = temporary.path().join("desk.tar.zst");
        let restored = temporary.path().join("restored");
        fs::create_dir(&source).unwrap();
        let original = source.join(".hidden");
        fs::write(&original, b"persistent data").unwrap();
        fs::set_permissions(&original, fs::Permissions::from_mode(0o640)).unwrap();
        fs::hard_link(&original, source.join("hardlink")).unwrap();
        symlink(".hidden", source.join("symlink")).unwrap();

        let report = snapshot(&source, &archive).unwrap();
        assert_eq!(report.files, 2);
        assert_eq!(report.symlinks, 1);
        assert!(report.sha256.is_some());
        restore(&archive, &restored).unwrap();

        assert_eq!(
            fs::read(restored.join(".hidden")).unwrap(),
            b"persistent data"
        );
        assert_eq!(
            fs::read_link(restored.join("symlink")).unwrap(),
            PathBuf::from(".hidden")
        );
        let first = fs::metadata(restored.join(".hidden")).unwrap();
        let second = fs::metadata(restored.join("hardlink")).unwrap();
        assert_eq!(first.ino(), second.ino());
        assert_eq!(first.permissions().mode() & 0o777, 0o640);
    }
}
