//! Asset output sets: preflight every destination, stage beside it, and roll
//! back completed publications on ordinary I/O errors. This is not a filesystem
//! transaction across crashes, concurrent writers or multiple filesystems.
use crate::Result;
use std::{
    collections::BTreeSet,
    fs::{self, Metadata, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

fn existing(path: &Path) -> Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(Some(m)),
        Ok(_) => Err(format!(
            "output must be a regular file, not a symlink or directory: {}",
            path.display()
        )
        .into()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("inspect output {}: {e}", path.display()).into()),
    }
}

struct Destination {
    path: PathBuf,
    original: Option<Metadata>,
    stage: Option<PathBuf>,
    published: bool,
}

impl Destination {
    fn unchanged(&self) -> Result<()> {
        let current = existing(&self.path)?;
        let unchanged = match (&self.original, &current) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                #[cfg(unix)]
                let identity = same_file(a, b);
                #[cfg(not(unix))]
                let identity = true;
                identity && a.len() == b.len() && a.modified()? == b.modified()?
            }
            _ => false,
        };
        if !unchanged {
            return Err(format!(
                "output changed while preparing assets: {}",
                self.path.display()
            )
            .into());
        }
        Ok(())
    }

    fn prepare(&mut self, bytes: &[u8]) -> Result<()> {
        let parent = self.path.parent().expect("canonical parent");
        let stage = loop {
            let n = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".swankit-assets-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("stage output {}: {e}", self.path.display()).into()),
            }
        };
        self.stage = Some(stage.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("new"))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        self.unchanged()?;
        if self.original.is_some() {
            // A backup link preserves the original inode and contents without
            // truncation, including when another name refers to the same file.
            fs::hard_link(&self.path, stage.join("old"))?;
            self.unchanged()?;
        }
        Ok(())
    }

    fn publish(&mut self) -> Result<()> {
        self.unchanged()?;
        let new = self
            .stage
            .as_ref()
            .expect("prepared destination")
            .join("new");
        if self.original.is_some() {
            fs::rename(&new, &self.path)?;
        } else {
            // Unlike rename, an exclusive link cannot replace a new destination
            // that appeared after preflight, even when --force was requested.
            fs::hard_link(&new, &self.path)?;
        }
        self.published = true;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if self.published {
            let stage = self.stage.as_ref().expect("published destination");
            if self.original.is_some() {
                fs::rename(stage.join("old"), &self.path)?;
            } else {
                fs::remove_file(&self.path)?;
            }
            self.published = false;
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if let Some(stage) = &self.stage {
            fs::remove_dir_all(stage)?;
            self.stage = None;
        }
        Ok(())
    }
}

/// Refuse output aliases of inputs or each other, including directory symlink
/// aliases and (on Unix) hard links. Leaf symlinks are always refused.
pub(super) fn write(files: Vec<(PathBuf, Vec<u8>)>, inputs: &[PathBuf], force: bool) -> Result<()> {
    write_with(files, inputs, force, |_| Ok(()))
}

fn write_with(
    files: Vec<(PathBuf, Vec<u8>)>,
    inputs: &[PathBuf],
    force: bool,
    mut before_publish: impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    let protected = inputs
        .iter()
        .map(|path| Ok((fs::canonicalize(path)?, fs::metadata(path)?)))
        .collect::<Result<Vec<_>>>()?;
    let mut paths = BTreeSet::new();
    let mut destinations: Vec<Destination> = Vec::new();
    for (path, _) in &files {
        let name = path.file_name().ok_or("output needs a file name")?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent)
            .map_err(|e| format!("output parent {}: {e}", parent.display()))?;
        let path = parent.join(name);
        let original = existing(&path)?;
        if !paths.insert(path.clone())
            || destinations.iter().any(|d| match (&original, &d.original) {
                (Some(a), Some(b)) => same_file(a, b),
                _ => false,
            })
        {
            return Err(format!("asset outputs alias the same file: {}", path.display()).into());
        }
        if protected.iter().any(|(input, metadata)| {
            *input == path || original.as_ref().is_some_and(|m| same_file(m, metadata))
        }) {
            return Err(format!("asset output aliases an input: {}", path.display()).into());
        }
        if original.is_some() && !force {
            return Err(format!(
                "output already exists: {}; use --force to replace an existing output",
                path.display()
            )
            .into());
        }
        destinations.push(Destination {
            path,
            original,
            stage: None,
            published: false,
        });
    }
    let result = (|| -> Result<()> {
        for (d, (_, bytes)) in destinations.iter_mut().zip(&files) {
            d.prepare(bytes)
                .map_err(|e| format!("prepare {}: {e}", d.path.display()))?;
        }
        for (index, d) in destinations.iter_mut().enumerate() {
            before_publish(index)?;
            d.publish()
                .map_err(|e| format!("publish {}: {e}", d.path.display()))?;
        }
        Ok(())
    })();
    let mut errors = Vec::new();
    if result.is_err() {
        for d in destinations.iter_mut().rev() {
            if let Err(e) = d.rollback() {
                errors.push(format!(
                    "restore {}: {e}; backup retained at {:?}",
                    d.path.display(),
                    d.stage
                ));
            }
        }
    }
    // Keep every backup if rollback failed, for explicit recovery.
    if errors.is_empty() {
        for d in &mut destinations {
            if let Err(e) = d.cleanup() {
                errors.push(format!("clean staging beside {}: {e}", d.path.display()));
            }
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "{}; {}",
            result
                .err()
                .map_or_else(|| "asset outputs published".to_owned(), |e| e.to_string()),
            errors.join("; ")
        )
        .into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_publication_failure_restores_fresh_and_forced_destinations() {
        for force in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "asset-output-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let paths = [root.join("rom"), root.join("ips"), root.join("map")];
            if force {
                for p in &paths {
                    fs::write(p, b"sentinel").unwrap();
                }
            }
            let error = write_with(
                paths
                    .iter()
                    .enumerate()
                    .map(|(index, p)| (p.clone(), vec![index as u8]))
                    .collect(),
                &[],
                force,
                |index| {
                    if index == 2 {
                        // Remove the last staged file so publication itself
                        // gets a real I/O error after the ROM and IPS succeed.
                        for entry in fs::read_dir(&root)? {
                            let new = entry?.path().join("new");
                            if new.is_file() && fs::read(&new)? == [2] {
                                fs::remove_file(new)?;
                            }
                        }
                    }
                    Ok(())
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("publish"), "{error}");
            for p in &paths {
                if force {
                    assert_eq!(fs::read(p).unwrap(), b"sentinel");
                } else {
                    assert!(!p.exists());
                }
            }
            assert_eq!(
                fs::read_dir(&root).unwrap().count(),
                if force { 3 } else { 0 }
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
}
