use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SendError, SyncSender};
use std::sync::Arc;
use std::{fs, io};

use crate::compiler::Program;
use crate::matcher::path_matches;

use anyhow::anyhow;

pub fn glob(
    relative_to: impl Into<PathBuf>,
    program: Arc<Program>,
) -> impl Iterator<Item = anyhow::Result<PathBuf>> + Send {
    let (tx, rx) = sync_channel(4096);

    // Start at the program absolute prefix if the program is an absolute glob
    let current_dir = program
        .absolute_prefix
        .clone()
        .unwrap_or_else(|| relative_to.into());

    rayon::spawn(move || {
        // Don't relativize paths if this is an absolute program
        let output_relative_to = if program.absolute_prefix.is_some() {
            Path::new("")
        } else {
            &current_dir
        };

        glob_to(tx, output_relative_to, &current_dir, &program)
    });
    rx.into_iter()
}

fn glob_to(
    tx: SyncSender<anyhow::Result<PathBuf>>,
    relative_to: &Path,
    target: &Path,
    program: &Program,
) {
    match fs::read_dir(target) {
        Ok(results) => rayon::scope(|scope| -> Result<(), SendError<_>> {
            // Try the parent dir in case the glob matches it
            let parent_path = target.join("..");

            handle_path_candidate(
                &parent_path,
                || fs::metadata(&parent_path),
                &tx,
                relative_to,
                program,
                &scope,
            )?;

            // All of the real results from the directory listing
            for result in results {
                match result {
                    Ok(dir_entry) => {
                        let dir_entry_path = dir_entry.path();

                        handle_path_candidate(
                            &dir_entry_path,
                            || dir_entry.metadata(),
                            &tx,
                            relative_to,
                            program,
                            &scope,
                        )?;
                    }
                    Err(err) => {
                        let wrapped_err = anyhow!("{}: {}", target.display(), err);
                        tx.send(Err(wrapped_err))?;
                    }
                }
            }

            Ok(())
        })
        .unwrap_or(()),
        Err(err) => {
            let wrapped_err = anyhow!("{}: {}", target.display(), err);
            let _ = tx.send(Err(wrapped_err));
        }
    }
}

fn handle_path_candidate<'a>(
    path: &Path,
    get_metadata: impl FnOnce() -> io::Result<fs::Metadata>,
    tx: &SyncSender<anyhow::Result<PathBuf>>,
    relative_to: &'a Path,
    program: &'a Program,
    scope: &rayon::Scope<'a>,
) -> Result<(), SendError<anyhow::Result<PathBuf>>> {
    let path_candidate = path.strip_prefix(relative_to).unwrap_or(&path);

    let result = path_matches(path_candidate, program);

    log::debug!(
        "path_candidate={}, result={:?}",
        path_candidate.display(),
        result
    );

    // If it is a valid prefix and a dir, recurse
    if result.valid_as_prefix && get_metadata().is_ok_and(|m| m.is_dir()) {
        let tx = tx.clone();
        let path_candidate = path_candidate.to_owned();
        scope.spawn(move |_| glob_to(tx, relative_to, &path_candidate, program));
    }

    // If it is valid as a complete match, send it out
    if result.valid_as_complete_match {
        tx.send(Ok(path_candidate.to_owned()))?;
    }

    Ok(())
}
