//! One explicitly selected, locally generated catalog with compile-time pins.
//!
//! This is a recovery data path, not an official boon-data installation. The
//! unchanged official installer remains responsible for all other catalogs.

use std::{fs, io::Read, path::{Path, PathBuf}};

use boon_parser::hero_stats::StatCatalog;
use pyo3::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const VERSION: &str = "6742";
pub(crate) const SOURCE_COMMIT: &str = "e180c06bed109fd30b6d77d763ae8e8fa9b8b7bc";
const SOURCE_REPOSITORY: &str = "SteamTracking/GameTracking-Deadlock";
const SOURCE_PATH: &str = "game/citadel/pak01_dir/scripts";
const GENERATOR_COMMIT: &str = "e5403736815cdb9bb8949d93934ca668833abfe3";
const GENERATOR_SHA256: &str = "79fcf9fe6aa3f23e7c2ca261f1ec5f358d4dd323d7b1824f48d58f1c771982c6";

#[derive(Clone, Copy)]
struct Artifact {
    name: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const ARTIFACTS: [Artifact; 5] = [
    Artifact { name: "abilities.json", bytes: 13823732, sha256: "0d19e3fb7e5ead1c3f8bba459f0256126a4d9fb9c0e58cf5078a3652b4d37eb3" },
    Artifact { name: "heroes.json", bytes: 2024875, sha256: "e3b59329835faa5b27be91d2c510e24eef166851b8671355f1270ffc2c64d35e" },
    Artifact { name: "manifest.json", bytes: 15441, sha256: "e4f609ce2d3da75ae740c1957f76c8ccb9a6ca5fae7a0ba5974ce8532960ba0c" },
    Artifact { name: "misc.json", bytes: 718835, sha256: "20893af880944cf1791ea247ee34cc23305011233f4da9544a255cd7b9cffa72" },
    Artifact { name: "modifiers.json", bytes: 3650295, sha256: "938e15567131fd2ca497920a92b302704c6f0739dc56aa746eb7f15b13cf616b" },
];

fn directory_is_plain(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("pinned replay catalog directory {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("pinned replay catalog directory must be a plain directory: {}", path.display()));
    }
    Ok(metadata)
}

fn same_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (before, after);
        false
    }
}

fn recheck_directory(path: &Path, before: &fs::Metadata) -> Result<(), String> {
    let after = directory_is_plain(path)?;
    if !same_identity(before, &after) {
        return Err(format!("pinned replay catalog directory changed during reading: {}", path.display()));
    }
    Ok(())
}

#[cfg(unix)]
fn open_artifact(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // Refuse a raced leaf symlink and avoid blocking on a raced FIFO. Directory
    // identities are checked separately; this is not a whole-path openat claim.
    fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_artifact(_path: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported,
        "pinned local replay catalogs require a Unix parser runtime with no-follow support"))
}

fn read_artifact(directory: &Path, artifact: Artifact) -> Result<Vec<u8>, String> {
    let path = directory.join(artifact.name);
    let before = fs::symlink_metadata(&path)
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    let correct_type_and_size = |metadata: &fs::Metadata| {
        !metadata.file_type().is_symlink() && metadata.is_file() && metadata.len() == artifact.bytes
    };
    if !correct_type_and_size(&before) {
        return Err(format!("pinned replay catalog {} has wrong type or byte count", artifact.name));
    }
    let mut file = open_artifact(&path)
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    let opened = file.metadata()
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    if !correct_type_and_size(&opened) || !same_identity(&before, &opened) {
        return Err(format!("pinned replay catalog {} changed before opening", artifact.name));
    }
    let mut bytes = Vec::with_capacity(artifact.bytes as usize);
    (&mut file).take(artifact.bytes + 1).read_to_end(&mut bytes)
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    let after_handle = file.metadata()
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    let after_path = fs::symlink_metadata(&path)
        .map_err(|error| format!("pinned replay catalog {}: {error}", artifact.name))?;
    if !correct_type_and_size(&after_handle) || !correct_type_and_size(&after_path)
        || !same_identity(&opened, &after_handle) || !same_identity(&opened, &after_path)
    {
        return Err(format!("pinned replay catalog {} changed during reading", artifact.name));
    }
    if bytes.len() as u64 != artifact.bytes
        || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
    {
        return Err(format!("pinned replay catalog {} SHA-256 or byte count mismatch", artifact.name));
    }
    Ok(bytes)
}

fn validate_manifest(manifest: &Value) -> Result<(), String> {
    if manifest["client_version"] != VERSION
        || manifest["release_key"] != VERSION
        || manifest["source"]["repository"] != SOURCE_REPOSITORY
        || manifest["source"]["commit"] != SOURCE_COMMIT
        || manifest["source"]["path"] != SOURCE_PATH
        || manifest["source_revision"] != "11077051"
        || manifest["snapshot"]["generator_sha256"] != GENERATOR_SHA256
        || manifest["catalogs"]["polars_version"] != "1.38.1"
        || manifest["artifacts"].as_object().map(|items| items.len()) != Some(4)
    {
        return Err("pinned replay catalog manifest has unexpected exact-version provenance".into());
    }
    for artifact in ARTIFACTS.iter().filter(|artifact| artifact.name != "manifest.json") {
        let item = &manifest["artifacts"][artifact.name];
        if item["bytes"].as_u64() != Some(artifact.bytes) || item["sha256"] != artifact.sha256 {
            return Err(format!("pinned replay catalog manifest disagrees for {}", artifact.name));
        }
    }
    Ok(())
}

/// Load only the exact 6742 bundle. Other versions keep the official path.
/// The buffers passed to StatCatalog are the same buffers whose hashes passed.
fn load(root: &Path, version: &str) -> Result<Option<StatCatalog>, String> {
    if version != VERSION {
        return Ok(None);
    }
    if !root.is_absolute() {
        return Err("Replay catalog root must be an absolute path".into());
    }
    let root_identity = directory_is_plain(root)?;
    let directory = root.join(VERSION);
    let directory_identity = directory_is_plain(&directory)?;
    let files: std::collections::HashMap<&str, Vec<u8>> = ARTIFACTS
        .iter()
        .map(|artifact| read_artifact(&directory, *artifact).map(|bytes| (artifact.name, bytes)))
        .collect::<Result<_, _>>()?;
    recheck_directory(root, &root_identity)?;
    recheck_directory(&directory, &directory_identity)?;
    let manifest: Value = serde_json::from_slice(&files["manifest.json"])
        .map_err(|error| format!("pinned replay catalog manifest JSON: {error}"))?;
    validate_manifest(&manifest)?;
    let catalog = StatCatalog::from_json_bytes(
        &files["heroes.json"], &files["abilities.json"],
        &files["modifiers.json"], &files["misc.json"],
    ).map_err(|error| format!("pinned replay catalog definitions: {error}"))?;
    if catalog.data_version != VERSION || catalog.snapshot_version != VERSION
        || catalog.source_commit != SOURCE_COMMIT
    {
        return Err("pinned replay catalogs do not share the recorded 6742 source".into());
    }
    Ok(Some(catalog))
}

pub(crate) fn from_environment(version: &str) -> Result<Option<StatCatalog>, String> {
    if version != VERSION {
        return Ok(None);
    }
    // Preserve the previously exported environment alias for existing callers.
    match std::env::var_os("BOON_REPLAY_CATALOG_ROOT").or_else(||std::env::var_os("BOON_LAB_CATALOG_ROOT")) {
        Some(root) => load(&PathBuf::from(root), version),
        None => Ok(None),
    }
}

/// Read-only qualification of the complete pinned catalog, without a replay.
#[pyfunction]
pub(crate) fn _verify_replay_catalog(root: PathBuf, version: &str) -> PyResult<String> {
    let catalog = load(&root, version)
        .map_err(crate::InvalidDemoError::new_err)?
        .ok_or_else(|| crate::InvalidDemoError::new_err("no bundled catalog is pinned for this exact version"))?;
    let _modifiers = boon_parser::EffectiveModifierState::with_catalog(&catalog);
    let _urn = boon_parser::urn::UrnState::with_catalog(&catalog)
        .map_err(|error| crate::InvalidDemoError::new_err(error.to_string()))?;
    serde_json::to_string(&json!({
        "schema": "boon-pinned-local-replay-catalog-v1",
        "distribution": "locally-generated-recovery-bundle",
        "client_version": catalog.data_version,
        "snapshot_version": catalog.snapshot_version,
        "source_repository": SOURCE_REPOSITORY,
        "source_commit": catalog.source_commit,
        "generator_repository": "pnxenopoulos/boon-data",
        "generator_commit": GENERATOR_COMMIT,
        "generator_sha256": GENERATOR_SHA256,
        "active_modifier_rules_initialized": true,
        "urn_rules_initialized": true,
        "artifacts": ARTIFACTS.iter().map(|artifact| (artifact.name, json!({
            "bytes": artifact.bytes, "sha256": artifact.sha256,
        }))).collect::<std::collections::BTreeMap<_, _>>(),
    })).map_err(|error| crate::InvalidDemoError::new_err(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: Artifact = Artifact {
        name: "fixture.json", bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };

    #[test]
    fn other_versions_cannot_be_aliased_to_the_bundle() {
        assert!(load(Path::new("unused"), "6745").unwrap().is_none());
        assert!(load(Path::new("unused"), "06742").unwrap().is_none());
        assert!(load(Path::new("unused"), "latest").unwrap().is_none());
    }

    #[test]
    fn exact_version_requires_absolute_existing_plain_directories() {
        assert!(load(Path::new("relative"), VERSION).is_err());
        let root = tempfile::tempdir().unwrap();
        assert!(load(root.path(), VERSION).is_err());
        fs::write(root.path().join(VERSION), b"not a directory").unwrap();
        assert!(load(root.path(), VERSION).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn artifact_type_size_and_hash_are_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_artifact(root.path(), ABC).is_err());
        let file = root.path().join(ABC.name);
        fs::create_dir(&file).unwrap();
        assert!(read_artifact(root.path(), ABC).is_err());
        fs::remove_dir(&file).unwrap();
        fs::write(&file, b"abc").unwrap();
        assert_eq!(read_artifact(root.path(), ABC).unwrap().as_slice(), b"abc");
        fs::write(&file, b"abd").unwrap();
        assert!(read_artifact(root.path(), ABC).is_err());
        fs::write(&file, b"abcd").unwrap();
        assert!(read_artifact(root.path(), ABC).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_open_is_nonblocking_and_not_a_regular_artifact() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join(ABC.name);
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a live NUL-terminated path and the mode is valid.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let file = open_artifact(&fifo).unwrap();
        assert!(!file.metadata().unwrap().is_file());
        assert!(read_artifact(root.path(), ABC).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn plain_directory_identity_changes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("catalog");
        fs::create_dir(&child).unwrap();
        let before = directory_is_plain(&child).unwrap();
        fs::rename(&child, root.path().join("old-catalog")).unwrap();
        fs::create_dir(&child).unwrap();
        assert!(recheck_directory(&child, &before).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_catalogs_and_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("real.json");
        fs::write(&file, b"abc").unwrap();
        symlink(&file, root.path().join(ABC.name)).unwrap();
        assert!(open_artifact(&root.path().join(ABC.name)).is_err());
        assert!(read_artifact(root.path(), ABC).is_err());
        let directory = root.path().join("real");
        fs::create_dir(&directory).unwrap();
        symlink(&directory, root.path().join(VERSION)).unwrap();
        assert!(load(root.path(), VERSION).is_err());
    }

    #[test]
    fn malformed_or_wrong_source_manifests_are_rejected() {
        assert!(validate_manifest(&json!({})).is_err());
        assert!(validate_manifest(&json!({"client_version":"6745"})).is_err());
        assert!(validate_manifest(&json!({"client_version":"6742", "source":{"commit":"wrong"}})).is_err());
    }
}
