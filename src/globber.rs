use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use crate::compiler::Program;
use crate::matcher::path_matches;

use anyhow::anyhow;

pub fn glob(
    relative_to: impl Into<PathBuf>,
    program: Arc<Program>,
) -> impl Iterator<Item = anyhow::Result<PathBuf>> + Send {
    let (tx, rx) = channel();
    let current_dir = relative_to.into();
    rayon::spawn(move || glob_to(tx, &current_dir, &current_dir, &program));
    rx.into_iter()
}

fn glob_to(
    tx: Sender<anyhow::Result<PathBuf>>,
    relative_to: &Path,
    target: &Path,
    program: &Program,
) {
    match std::fs::read_dir(target) {
        Ok(results) => rayon::scope(|s| {
            for result in results {
                match result {
                    Ok(dir_entry) => {
                        let dir_entry_path = dir_entry.path();
                        let path_candidate = dir_entry_path
                            .strip_prefix(relative_to)
                            .unwrap_or(&dir_entry_path);

                        let result = path_matches(path_candidate, program);

                        log::debug!(
                            "path_candidate={}, result={:?}",
                            path_candidate.display(),
                            result
                        );

                        // If it is a valid prefix and a dir, recurse
                        if result.valid_as_prefix && dir_entry.metadata().is_ok_and(|m| m.is_dir())
                        {
                            let tx = tx.clone();
                            let path_candidate = path_candidate.to_owned();
                            s.spawn(move |_| glob_to(tx, relative_to, &path_candidate, program));
                        }

                        // If it is valid as a complete match, send it out
                        if result.valid_as_complete_match {
                            if tx.send(Ok(path_candidate.to_owned())).is_err() {
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        let wrapped_err = anyhow!("{}: {}", target.display(), err);
                        if tx.send(Err(wrapped_err)).is_err() {
                            break;
                        }
                    }
                }
            }
        }),
        Err(err) => {
            let wrapped_err = anyhow!("{}: {}", target.display(), err);
            let _ = tx.send(Err(wrapped_err));
        }
    }
}
