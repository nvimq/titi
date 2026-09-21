//! Git-backed checkpoints: a rewind point that can undo code, not only text.
//!
//! A session checkpoint records how far the transcript had got. This records
//! where the workspace was, as a git commit, so rewinding can put the files
//! back too. The commit is local and never pushed; a dirty tree is committed
//! as-is, because that is exactly the state worth returning to.

use std::path::Path;
use std::process::Command;

/// A commit that captures the workspace, or why one could not be made.
pub fn snapshot(workspace: &Path, label: &str) -> Result<String, String> {
    if !is_repo(workspace) {
        return Err("not a git repository".into());
    }
    run(workspace, &["add", "-A"])?;
    // Nothing to commit is not a failure: the tree already matches HEAD, and
    // that commit is the snapshot.
    let status = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(workspace)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        run(
            workspace,
            &[
                "-c",
                "user.name=titi",
                "-c",
                "user.email=titi@localhost",
                "commit",
                "--no-verify",
                "-m",
                &format!("titi checkpoint: {label}"),
            ],
        )?;
    }
    run(workspace, &["rev-parse", "HEAD"])
}

/// Put the workspace back at `commit`.
///
/// Refuses a dirty tree: restoring would throw away work the checkpoint does
/// not know about, and the user can checkpoint that first.
pub fn restore(workspace: &Path, commit: &str) -> Result<(), String> {
    if !is_repo(workspace) {
        return Err("not a git repository".into());
    }
    let dirty = run(workspace, &["status", "--porcelain"])?;
    if !dirty.is_empty() {
        return Err("workspace has uncommitted changes; checkpoint them first".into());
    }
    run(workspace, &["reset", "--hard", commit])?;
    Ok(())
}

fn is_repo(workspace: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {}: {stderr}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            &["init"][..],
            &["config", "user.email", "t@t"],
            &["config", "user.name", "t"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        dir
    }

    #[test]
    fn a_snapshot_captures_a_later_change() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        let first = snapshot(dir.path(), "before").unwrap();

        std::fs::write(dir.path().join("a.txt"), "two").unwrap();
        let _second = snapshot(dir.path(), "after").unwrap();

        restore(dir.path(), &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "one"
        );
    }

    #[test]
    fn restoring_refuses_a_dirty_tree() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        let first = snapshot(dir.path(), "before").unwrap();
        std::fs::write(dir.path().join("a.txt"), "uncommitted").unwrap();

        let error = restore(dir.path(), &first).unwrap_err();
        assert!(error.contains("uncommitted"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "uncommitted"
        );
    }

    #[test]
    fn a_directory_that_is_not_a_repo_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let error = snapshot(dir.path(), "x").unwrap_err();
        assert!(error.contains("not a git repository"), "{error}");
    }
}
