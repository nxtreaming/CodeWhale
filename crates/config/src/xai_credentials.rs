//! Naming and cleanup policy for Codewhale-owned xAI OAuth generations.
//!
//! Config stores only a validated basename. Callers can therefore never turn
//! the generation pointer into an arbitrary path read or deletion primitive.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const XAI_OAUTH_GENERATION_PREFIX: &str = "xai-auth-";
pub const XAI_OAUTH_GENERATION_SUFFIX: &str = ".json";
pub const LEGACY_XAI_OAUTH_FILE_NAME: &str = "xai-auth.json";

#[must_use]
pub fn is_valid_xai_oauth_generation(value: &str) -> bool {
    let path = Path::new(value);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || path.file_name().and_then(|name| name.to_str()) != Some(value)
    {
        return false;
    }
    let Some(id) = value
        .strip_prefix(XAI_OAUTH_GENERATION_PREFIX)
        .and_then(|value| value.strip_suffix(XAI_OAUTH_GENERATION_SUFFIX))
    else {
        return false;
    };
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn validate_xai_oauth_generation(value: &str) -> Result<&str> {
    if !is_valid_xai_oauth_generation(value) {
        bail!(
            "invalid Codewhale-owned xAI OAuth generation; expected xai-auth-<32 lowercase hex>.json"
        );
    }
    Ok(value)
}

pub fn xai_oauth_credentials_dir() -> Result<PathBuf> {
    let directory = crate::codewhale_home()?.join("credentials");
    canonicalize_existing_ancestor(&directory)
}

/// Resolve system aliases (for example macOS `/var` -> `/private/var`) before
/// the no-follow owned-file reader walks the path. Missing state directories
/// remain missing: only the nearest existing ancestor is canonicalized and
/// the lexical tail is appended without probing it.
fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolving the Codewhale credentials directory")?
            .join(path)
    };
    if absolute
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!(
            "Codewhale credentials directory must be lexically normalized: {}",
            crate::quote_os_path(&absolute)
        );
    }
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot resolve Codewhale credentials directory {}",
                        crate::quote_os_path(&absolute)
                    )
                })?;
                missing.push(name.to_os_string());
                existing = existing.parent().ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot resolve Codewhale credentials directory {}",
                        crate::quote_os_path(&absolute)
                    )
                })?;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "resolving Codewhale credentials directory {}",
                        crate::quote_os_path(&absolute)
                    )
                });
            }
        }
    }
}

pub fn xai_oauth_generation_path(generation: &str) -> Result<PathBuf> {
    Ok(xai_oauth_credentials_dir()?.join(validate_xai_oauth_generation(generation)?))
}

pub fn legacy_xai_oauth_path() -> Result<PathBuf> {
    Ok(xai_oauth_credentials_dir()?.join(LEGACY_XAI_OAUTH_FILE_NAME))
}

/// Delete one superseded generation after its replacement pointer committed.
/// The basename is validated before any filesystem access.
pub fn remove_xai_oauth_generation(generation: &str) -> Result<bool> {
    let path = xai_oauth_generation_path(generation)?;
    remove_owned_file(&path)
}

/// Explicit logout policy: remove the legacy Codewhale-owned file and every
/// valid generated xAI OAuth file. Unknown files in the credentials directory
/// are never touched.
pub fn clear_all_xai_oauth_credentials() -> Result<usize> {
    let directory = xai_oauth_credentials_dir()?;
    clear_xai_oauth_credentials_in(&directory)
}

fn clear_xai_oauth_credentials_in(directory: &Path) -> Result<usize> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect Codewhale credentials directory {}",
                    crate::quote_os_path(directory)
                )
            });
        }
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to inspect Codewhale credentials directory {}",
                crate::quote_os_path(directory)
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name != LEGACY_XAI_OAUTH_FILE_NAME && !is_valid_xai_oauth_generation(name) {
            continue;
        }
        if remove_owned_file(&entry.path())? {
            removed += 1;
        }
    }
    Ok(removed)
}

fn remove_owned_file(path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect Codewhale-owned xAI OAuth file {}",
                    crate::quote_os_path(path)
                )
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "refusing to delete non-regular Codewhale-owned xAI OAuth path {}",
            crate::quote_os_path(path)
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            bail!(
                "refusing to delete reparse-point Codewhale-owned xAI OAuth path {}",
                crate::quote_os_path(path)
            );
        }
    }
    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove Codewhale-owned xAI OAuth file {}",
            crate::quote_os_path(path)
        )
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_directory_canonicalizes_only_the_existing_prefix() {
        let directory = tempfile::tempdir().expect("temp dir");
        let canonical = directory.path().canonicalize().expect("canonical root");
        let unresolved = directory.path().join("missing").join("credentials");
        assert_eq!(
            canonicalize_existing_ancestor(&unresolved).expect("resolve missing tail"),
            canonical.join("missing").join("credentials")
        );
        assert!(!unresolved.exists());
        assert!(
            canonicalize_existing_ancestor(&canonical.join("missing/../escape")).is_err(),
            "owned credential roots must reject traversal components"
        );
    }

    #[test]
    fn generation_names_are_strict_basenames() {
        let valid = "xai-auth-0123456789abcdef0123456789abcdef.json";
        assert!(is_valid_xai_oauth_generation(valid));
        for invalid in [
            "../xai-auth-0123456789abcdef0123456789abcdef.json",
            "/tmp/xai-auth-0123456789abcdef0123456789abcdef.json",
            "xai-auth-0123456789ABCDEF0123456789ABCDEF.json",
            "xai-auth-short.json",
            "xai-auth.json",
        ] {
            assert!(!is_valid_xai_oauth_generation(invalid), "{invalid}");
        }
    }

    #[test]
    fn logout_cleanup_removes_only_owned_xai_files() {
        let directory = tempfile::tempdir().expect("temp dir");
        let generation = "xai-auth-0123456789abcdef0123456789abcdef.json";
        fs::write(directory.path().join(generation), "secret").expect("generation");
        fs::write(directory.path().join(LEGACY_XAI_OAUTH_FILE_NAME), "legacy").expect("legacy");
        fs::write(directory.path().join("other-provider.json"), "keep").expect("other provider");

        assert_eq!(
            clear_xai_oauth_credentials_in(directory.path()).expect("clear"),
            2
        );
        assert!(directory.path().join("other-provider.json").exists());
        assert!(!directory.path().join(generation).exists());
        assert!(!directory.path().join("xai-auth.json").exists());
    }
}
