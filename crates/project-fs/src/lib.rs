#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

//! Capability-confined filesystem materialization for project plans.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use project_core::{GenerationPlan, MaterializedProject, ProjectTarget, ProjectWriter};

/// A writer confined to an opened output-directory capability.
pub struct RootConfinedWriter {
    root: Dir,
}

impl RootConfinedWriter {
    /// Opens the configured output root as the adapter's only filesystem capability.
    ///
    /// # Errors
    ///
    /// Returns [`FsWriterError`] when the root cannot be created, opened, or used as a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, FsWriterError> {
        std::fs::create_dir_all(root.as_ref()).map_err(FsWriterError::CreateRoot)?;
        let root = Dir::open_ambient_dir(root.as_ref(), ambient_authority())
            .map_err(FsWriterError::OpenRoot)?;
        Ok(Self { root })
    }
}

impl ProjectWriter for RootConfinedWriter {
    type Error = FsWriterError;

    fn write(
        &self,
        plan: &GenerationPlan,
        target: &ProjectTarget,
    ) -> Result<MaterializedProject, Self::Error> {
        // `create_dir` is an atomic reservation: an existing target always fails.
        self.root
            .create_dir(target.as_path())
            .map_err(|error| map_target_creation_error(target, error))?;
        let result = write_plan(&self.root, target, plan);
        if result.is_err() {
            let _ = self.root.remove_dir_all(target.as_path());
        }
        result.map(|written_files| MaterializedProject {
            target: target.clone(),
            written_files,
        })
    }
}

fn write_plan(
    root: &Dir,
    target: &ProjectTarget,
    plan: &GenerationPlan,
) -> Result<Vec<PathBuf>, FsWriterError> {
    let mut written_files = Vec::with_capacity(plan.files.len());
    for file in &plan.files {
        validate_relative_path(file.relative_path())?;
        let destination = target.as_path().join(file.relative_path());
        let parent = destination
            .parent()
            .ok_or_else(|| FsWriterError::UnsafePlanPath(file.relative_path().to_path_buf()))?;
        root.create_dir_all(parent)
            .map_err(FsWriterError::CreateParent)?;
        root.write(&destination, file.contents())
            .map_err(FsWriterError::WriteFile)?;
        written_files.push(file.relative_path().to_path_buf());
    }
    Ok(written_files)
}

fn map_target_creation_error(target: &ProjectTarget, error: io::Error) -> FsWriterError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        FsWriterError::DestinationExists(target.as_str().to_owned())
    } else {
        FsWriterError::CreateTarget(error)
    }
}

fn validate_relative_path(path: &Path) -> Result<(), FsWriterError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(FsWriterError::UnsafePlanPath(path.to_path_buf()));
    }
    Ok(())
}

/// Failures while safely materializing a project plan.
#[derive(Debug)]
pub enum FsWriterError {
    /// The configured root could not be created.
    CreateRoot(io::Error),
    /// The configured root could not be opened as a capability.
    OpenRoot(io::Error),
    /// The final target was already reserved.
    DestinationExists(String),
    /// The final target could not be atomically reserved.
    CreateTarget(io::Error),
    /// A plan contained a path that could escape the project root.
    UnsafePlanPath(PathBuf),
    /// A parent directory could not be created.
    CreateParent(io::Error),
    /// A generated file could not be written.
    WriteFile(io::Error),
}

impl fmt::Display for FsWriterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateRoot(error) => write!(formatter, "could not create output root: {error}"),
            Self::OpenRoot(error) => write!(formatter, "could not open output root: {error}"),
            Self::DestinationExists(target) => {
                write!(formatter, "project destination already exists: {target}")
            }
            Self::CreateTarget(error) => {
                write!(formatter, "could not reserve project destination: {error}")
            }
            Self::UnsafePlanPath(path) => write!(
                formatter,
                "generation plan contains unsafe path: {}",
                path.display()
            ),
            Self::CreateParent(error) => {
                write!(formatter, "could not create generated-file parent: {error}")
            }
            Self::WriteFile(error) => write!(formatter, "could not write generated file: {error}"),
        }
    }
}

impl std::error::Error for FsWriterError {}

#[cfg(test)]
mod tests {
    use super::*;
    use project_core::{DefaultProjectAuthor, ProjectAuthor, ProjectBlueprint, ProjectKind};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicUsize = AtomicUsize::new(0);

    fn temporary_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rust-factory-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn plan() -> GenerationPlan {
        DefaultProjectAuthor
            .plan(&ProjectBlueprint::v1(
                "generated-example",
                "generated_example",
                ProjectKind::Library,
                "Apache-2.0",
                None,
            ))
            .expect("valid blueprint")
    }

    #[test]
    fn materializes_a_plan_below_the_configured_root() {
        let root = temporary_root();
        let writer = RootConfinedWriter::new(&root).expect("root");
        let project = writer
            .write(&plan(), &ProjectTarget::new("example").expect("target"))
            .expect("write");
        assert_eq!(project.target.as_str(), "example");
        assert!(root.join("example/Cargo.toml").is_file());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn refuses_to_overwrite_an_existing_destination() {
        let root = temporary_root();
        let writer = RootConfinedWriter::new(&root).expect("root");
        let target = ProjectTarget::new("example").expect("target");
        writer.write(&plan(), &target).expect("first write");
        assert!(matches!(
            writer.write(&plan(), &target),
            Err(FsWriterError::DestinationExists(_))
        ));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_traversal_paths_before_materialization() {
        assert!(matches!(
            validate_relative_path(Path::new("../escape")),
            Err(FsWriterError::UnsafePlanPath(_))
        ));
        assert!(matches!(
            validate_relative_path(Path::new("/absolute")),
            Err(FsWriterError::UnsafePlanPath(_))
        ));
    }

    #[test]
    fn generated_workspace_passes_its_quality_gate() {
        let root = temporary_root();
        let writer = RootConfinedWriter::new(&root).expect("root");
        writer
            .write(
                &plan(),
                &ProjectTarget::new("quality-fixture").expect("target"),
            )
            .expect("write");
        let output = Command::new("make")
            .arg("check")
            .current_dir(root.join("quality-fixture"))
            .output()
            .expect("make");
        assert!(
            output.status.success(),
            "generated quality gate failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
