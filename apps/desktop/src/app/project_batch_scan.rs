//! Read-only discovery of local Git working directories.

use std::{
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectKind {
    Repository,
    LinkedCheckout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScannedProject {
    pub name: String,
    pub path: PathBuf,
    pub relative_path: String,
    pub kind: ProjectKind,
}

#[derive(Debug, Default)]
pub(super) struct ScanResult {
    pub projects: Vec<ScannedProject>,
    /// Locations that could not be inspected (intentionally ignored folders
    /// and symlinks are not counted).
    pub skipped: usize,
}

pub(super) fn scan_git_projects(root: &Path, cancelled: &AtomicBool) -> Result<ScanResult, String> {
    check_cancelled(cancelled)?;
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Cannot access {}: {error}", root.display()))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(format!("Not a regular directory: {}", root.display()));
    }

    let mut result = ScanResult::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        check_cancelled(cancelled)?;
        if directory != root {
            match fs::symlink_metadata(&directory) {
                Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
                Ok(_) => continue,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            }
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == root => {
                return Err(format!("Cannot read {}: {error}", root.display()));
            }
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };

        match project_kind(&directory) {
            Ok(Some(kind)) => {
                let relative = directory.strip_prefix(root).unwrap_or(&directory);
                let relative_path = if relative.as_os_str().is_empty() {
                    ".".to_owned()
                } else {
                    relative.to_string_lossy().into_owned()
                };
                let name = directory
                    .file_name()
                    .map(|part| part.to_string_lossy().into_owned())
                    .unwrap_or_else(|| directory.display().to_string());
                result.projects.push(ScannedProject {
                    name,
                    path: directory.clone(),
                    relative_path,
                    kind,
                });
            }
            Ok(None) if looks_like_bare_repo(&directory) => continue,
            Ok(None) => {}
            Err(_) => result.skipped += 1,
        }

        for entry in entries {
            check_cancelled(cancelled)?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            if ignored_directory(&entry.file_name().to_string_lossy()) {
                continue;
            }
            match entry.metadata() {
                Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
                    pending.push(entry.path());
                }
                Ok(_) => {}
                Err(_) => result.skipped += 1,
            }
        }
    }
    check_cancelled(cancelled)?;
    result
        .projects
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(result)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Project scan cancelled.".to_owned())
    } else {
        Ok(())
    }
}

fn project_kind(directory: &Path) -> io::Result<Option<ProjectKind>> {
    let marker = directory.join(".git");
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if is_link(&metadata) {
        return Ok(None);
    }
    if metadata.is_dir() {
        let head = fs::symlink_metadata(marker.join("HEAD"));
        let objects = fs::symlink_metadata(marker.join("objects"));
        return Ok((head.is_ok_and(|value| value.is_file() && !is_link(&value))
            && objects.is_ok_and(|value| value.is_dir() && !is_link(&value)))
        .then_some(ProjectKind::Repository));
    }
    if !metadata.is_file() {
        return Ok(None);
    }

    // Read only the pointer, not the destination. Gitfiles are used for
    // worktrees, submodules, and repositories with a separate Git directory.
    let mut bytes = Vec::new();
    File::open(marker)?.take(8192).read_to_end(&mut bytes)?;
    let gitfile_path = bytes
        .split(|&byte| byte == b'\n' || byte == b'\r')
        .next()
        .and_then(|line| line.strip_prefix(b"gitdir: "));
    Ok(gitfile_path
        .filter(|path| path.iter().any(|byte| !byte.is_ascii_whitespace()))
        .map(|_| ProjectKind::LinkedCheckout))
}

fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn looks_like_bare_repo(directory: &Path) -> bool {
    let regular_file = |name| {
        fs::symlink_metadata(directory.join(name))
            .is_ok_and(|metadata| metadata.is_file() && !is_link(&metadata))
    };
    let regular_dir = |name| {
        fs::symlink_metadata(directory.join(name))
            .is_ok_and(|metadata| metadata.is_dir() && !is_link(&metadata))
    };
    regular_file("HEAD") && regular_dir("objects") && regular_dir("refs")
}

fn ignored_directory(name: &str) -> bool {
    [
        ".git",
        ".codux",
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        ".next",
        ".nuxt",
        ".turbo",
        ".cache",
        ".venv",
        "venv",
        "__pycache__",
        "coverage",
        ".gradle",
        ".dart_tool",
        ".parcel-cache",
        "bower_components",
    ]
    .iter()
    .any(|ignored| name.eq_ignore_ascii_case(ignored))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("codux-batch-scan-{}-{nonce}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn repository(&self, path: &str) {
            let git = self.0.join(path).join(".git");
            fs::create_dir_all(git.join("objects")).unwrap();
            fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        }

        fn scan(&self) -> ScanResult {
            scan_git_projects(&self.0, &AtomicBool::new(false)).unwrap()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_base_and_nested_same_name_repositories() {
        let root = TestDirectory::new();
        root.repository("");
        root.repository("first/app");
        root.repository("second/app");
        fs::create_dir_all(root.0.join("other")).unwrap();
        let found = root.scan();
        assert_eq!(found.projects.len(), 3);
        assert_eq!(found.projects[0].relative_path, ".");
        assert_eq!(found.projects[1].name, "app");
        assert_eq!(found.projects[2].name, "app");
        assert_ne!(
            found.projects[1].relative_path,
            found.projects[2].relative_path
        );
        assert!(
            found
                .projects
                .iter()
                .all(|project| project.kind == ProjectKind::Repository)
        );
        assert_eq!(found.skipped, 0);
    }

    #[test]
    fn recognizes_gitfiles_but_not_garbage_or_bare_repositories() {
        let root = TestDirectory::new();
        fs::create_dir(root.0.join("checkout")).unwrap();
        fs::write(
            root.0.join("checkout/.git"),
            b"gitdir: ../repo/.git/worktrees/w\r\n",
        )
        .unwrap();
        fs::create_dir(root.0.join("fake")).unwrap();
        fs::write(root.0.join("fake/.git"), b"not a gitfile").unwrap();
        fs::create_dir_all(root.0.join("empty/.git")).unwrap();
        fs::create_dir_all(root.0.join("bare/objects")).unwrap();
        fs::create_dir_all(root.0.join("bare/refs")).unwrap();
        fs::write(root.0.join("bare/HEAD"), b"ref: refs/heads/main\n").unwrap();
        let found = root.scan();
        assert_eq!(found.projects.len(), 1);
        assert_eq!(found.projects[0].kind, ProjectKind::LinkedCheckout);
        assert_eq!(found.projects[0].name, "checkout");
    }

    #[test]
    fn ignores_dependency_output_and_git_metadata_directories() {
        let root = TestDirectory::new();
        root.repository("");
        for path in [
            "node_modules/app",
            "BUILD/app",
            ".codux/app",
            ".git/objects/app",
        ] {
            root.repository(path);
        }
        root.repository("src/app");
        let found = root.scan();
        assert_eq!(found.projects.len(), 2);
        assert_eq!(found.projects[0].relative_path, ".");
        assert_eq!(found.projects[1].name, "app");
    }

    #[test]
    fn rejects_cancelled_and_non_directory_roots() {
        let root = TestDirectory::new();
        assert!(scan_git_projects(&root.0, &AtomicBool::new(true)).is_err());
        let file = root.0.join("file");
        fs::write(&file, b"content").unwrap();
        assert!(scan_git_projects(&file, &AtomicBool::new(false)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_scan_or_accept_symlinked_directories() {
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        outside.repository("");
        let link = root.0.join("link");
        std::os::unix::fs::symlink(&outside.0, &link).unwrap();
        assert!(root.scan().projects.is_empty());
        assert!(scan_git_projects(&link, &AtomicBool::new(false)).is_err());
    }
}
