// Copyright 2024 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use futures::{StreamExt, TryFutureExt, TryStreamExt};
use itertools::Itertools;
use jj_lib::backend::{BackendError, FileId, MergedTreeId, TreeValue};
use jj_lib::conflicts::{materialize_tree_value, MaterializedTreeValue};
use jj_lib::diff::{find_line_ranges, Diff, DiffHunk};
use jj_lib::files::{self, ContentHunk, MergeResult};
use jj_lib::matchers::Matcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::{MergedTree, MergedTreeBuilder};
use jj_lib::object_id::ObjectId;
use jj_lib::repo_path::{RepoPath, RepoPathBuf};
use jj_lib::store::Store;
use pollster::FutureExt;
use thiserror::Error;

// TODO: this type needs rewriting
#[derive(Debug, Error)]
pub enum BuiltinWebToolError {
    #[error("Failed to record changes")]
    Record(#[from] scm_record::RecordError),
    #[error(transparent)]
    ReadFileBackend(BackendError),
    #[error("Failed to read file {path:?} with ID {id}", id = id.hex())]
    ReadFileIo {
        path: RepoPathBuf,
        id: FileId,
        source: std::io::Error,
    },
    #[error(transparent)]
    ReadSymlink(BackendError),
    #[error("Failed to decode UTF-8 text for item {item} (this should not happen)")]
    DecodeUtf8 {
        source: std::str::Utf8Error,
        item: &'static str,
    },
    #[error("Rendering {item} {id} is unimplemented for the builtin difftool/mergetool")]
    Unimplemented { item: &'static str, id: String },
    #[error("Backend error")]
    BackendError(#[from] jj_lib::backend::BackendError),
}

// TODO: Move this into diffedit3. EntriesToCompare should have a dummy
// implementation. Makes sense for FakeData
#[derive(Debug)]
struct JJEntriesToCompare(diffedit3::EntriesToCompare);

// TODO: Store executable byte, allow comparing if both sides are executable.
struct PathMetadata;

#[derive(Clone, Debug)]
enum FileInfo {
    Missing,
    TextFile { text: String, executable: bool },
    Unsupported(String),
}

fn read_file_contents(
    store: &Store,
    tree: &MergedTree,
    path: &RepoPath,
) -> Result<FileInfo, BuiltinWebToolError> {
    let value = tree.path_value(path);
    let materialized_value = materialize_tree_value(store, path, value)
        .map_err(BuiltinWebToolError::BackendError)
        .block_on()?;
    match materialized_value {
        MaterializedTreeValue::Absent => Ok(FileInfo::Missing),
        // TODO: Check for binary files
        MaterializedTreeValue::File {
            id,
            executable,
            mut reader,
        } => {
            let mut buf = Vec::new();
            reader
                .read_to_end(&mut buf)
                .map_err(|err| BuiltinWebToolError::ReadFileIo {
                    path: path.to_owned(),
                    id: id.clone(),
                    source: err,
                })?;

            // TODO: Maximal size
            if seems_like_a_binary_file(buf) {
                // buf.contains(&0) ?
                return Ok(FileInfo::Unsupported(
                    "seems to be a binary file".to_string(),
                ));
            };
            let Ok(text) = String::from_utf8(buf) else {
                return Ok(FileInfo::Unsupported("not valid utf-8".to_string()));
            };
            Ok(FileInfo::TextFile { text, executable })
        }
        // TODO: This is bad
        MaterializedTreeValue::Conflict { id, contents } => Ok(FileInfo::Unsupported(
            "conflicts are not supported".to_string(),
        )),
        MaterializedTreeValue::Symlink { .. } => Ok(FileInfo::Unsupported(
            "symlinks are not supported".to_string(),
        )),
        MaterializedTreeValue::Tree { .. } => {
            Ok(FileInfo::Unsupported("dirs are not supported".to_string()))
        }
        MaterializedTreeValue::GitSubmodule { .. } => Ok(FileInfo::Unsupported(
            "submodules are not supported".to_string(),
        )),
    }
}

pub fn edit_diff_web(
    left_tree: &MergedTree,
    right_tree: &MergedTree,
    matcher: &dyn Matcher,
    // Need UI
) -> Result<MergedTreeId, BuiltinWebToolError> {
    let store = left_tree.store().clone();
    let changed_files: Vec<_> = left_tree
        .diff_stream(right_tree, matcher)
        .map(|(path, diff)| diff.map(|_| path))
        .try_collect()
        .block_on()?;

    for repo_path in changed_files {
        let (left_contents, right_contents, executable) = match (
            read_file_contents(&store, left_tree, &repo_path)?,
            read_file_contents(&store, right_tree, &repo_path)?,
        ) {
            (FileInfo::Unsupported(message), _) | (_, FileInfo::Unsupported(message)) => {
                report_error(&message);
                continue;
            }
            (FileInfo::Missing, FileInfo::TextFile { text, executable }) => {
                (None, Some(text), executable)
            }
            (FileInfo::TextFile { text, executable }, FileInfo::Missing) => {
                (Some(text), None, executable)
            }
            (
                FileInfo::TextFile {
                    text: left_text,
                    executable: left_executable,
                },
                FileInfo::TextFile {
                    text: right_text,
                    executable: right_executable,
                },
            ) => {
                if left_executable == right_executable {
                    (Some(left_text), Some(right_text), left_executable)
                } else {
                    report_error("Executable bit changed");
                    continue;
                }
            }
            (FileInfo::Missing, FileInfo::Missing) => {
                // TODO: Perhaps panic, as this is a bug in diff_stream.
                report_error("Path missing on both sides");
                continue;
            }
        };
        todo!("Populate the input")
    }
    let recorder = scm_record::Recorder::new(
        scm_record::RecordState {
            is_read_only: false,
            files,
            commits: Default::default(),
        },
        &mut input,
    );
    let result = recorder.run().map_err(BuiltinToolError::Record)?;
    let tree_id = apply_diff_builtin(store, left_tree, right_tree, changed_files, &result.files)
        .map_err(BuiltinToolError::BackendError)?;
    Ok(tree_id)
}
