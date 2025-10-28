// Copyright 2025 The Jujutsu Authors
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

//! Utilities to compute unified diffs, AKA Git's diff-3 style, of 2 sides
#![expect(missing_docs)]

use std::borrow::Borrow;
use std::mem;
use std::ops::Range;

use bstr::BStr;
use bstr::BString;
use itertools::Itertools as _;
use pollster::FutureExt as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::conflicts::ConflictMaterializeOptions;
use crate::conflicts::MaterializedFileValue;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialize_merge_result_to_bytes;
use crate::diff::ContentDiff;
use crate::diff::DiffHunk;
use crate::diff::DiffHunkKind;
use crate::merge::Merge;
use crate::object_id::ObjectId as _;
use crate::repo_path::RepoPath;

#[derive(Clone, Debug)]
pub struct FileContent<T> {
    /// false if this file is likely text; true if it is likely binary.
    pub is_binary: bool,
    pub contents: T,
}

impl FileContent<Merge<BString>> {
    pub fn is_empty(&self) -> bool {
        self.contents.as_resolved().is_some_and(|c| c.is_empty())
    }
}

pub fn file_content_for_diff<T>(
    path: &RepoPath,
    file: &mut MaterializedFileValue,
    map_resolved: impl FnOnce(BString) -> T,
) -> BackendResult<FileContent<T>> {
    // If this is a binary file, don't show the full contents.
    // Determine whether it's binary by whether the first 8k bytes contain a null
    // character; this is the same heuristic used by git as of writing: https://github.com/git/git/blob/eea0e59ffbed6e33d171ace5be13cde9faa41639/xdiff-interface.c#L192-L198
    const PEEK_SIZE: usize = 8000;
    // TODO: currently we look at the whole file, even though for binary files we
    // only need to know the file size. To change that we'd have to extend all
    // the data backends to support getting the length.
    let contents = BString::new(file.read_all(path).block_on()?);
    let start = &contents[..PEEK_SIZE.min(contents.len())];
    Ok(FileContent {
        is_binary: start.contains(&b'\0'),
        contents: map_resolved(contents),
    })
}

#[derive(Clone, Debug)]
pub struct GitDiffPart {
    /// Octal mode string or `None` if the file is absent.
    pub mode: Option<&'static str>,
    pub hash: String,
    pub content: FileContent<BString>,
}

#[derive(Debug, Error)]
pub enum UnifiedDiffError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("Access denied to {path}")]
    AccessDenied {
        path: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

pub fn git_diff_part(
    path: &RepoPath,
    value: MaterializedTreeValue,
    materialize_options: &ConflictMaterializeOptions,
) -> Result<GitDiffPart, UnifiedDiffError> {
    const DUMMY_HASH: &str = "0000000000";
    let mode;
    let mut hash;
    let content;
    match value {
        MaterializedTreeValue::Absent => {
            return Ok(GitDiffPart {
                mode: None,
                hash: DUMMY_HASH.to_owned(),
                content: FileContent {
                    is_binary: false,
                    contents: BString::default(),
                },
            });
        }
        MaterializedTreeValue::AccessDenied(err) => {
            return Err(UnifiedDiffError::AccessDenied {
                path: path.as_internal_file_string().to_owned(),
                source: err,
            });
        }
        MaterializedTreeValue::File(mut file) => {
            mode = if file.executable { "100755" } else { "100644" };
            hash = file.id.hex();
            content = file_content_for_diff(path, &mut file, |content| content)?;
        }
        MaterializedTreeValue::Symlink { id, target } => {
            mode = "120000";
            hash = id.hex();
            content = FileContent {
                // Unix file paths can't contain null bytes.
                is_binary: false,
                contents: target.into(),
            };
        }
        MaterializedTreeValue::GitSubmodule(id) => {
            // TODO: What should we actually do here?
            mode = "040000";
            hash = id.hex();
            content = FileContent {
                is_binary: false,
                contents: BString::default(),
            };
        }
        MaterializedTreeValue::FileConflict(file) => {
            mode = match file.executable {
                Some(true) => "100755",
                Some(false) | None => "100644",
            };
            hash = DUMMY_HASH.to_owned();
            content = FileContent {
                is_binary: false, // TODO: are we sure this is never binary?
                contents: materialize_merge_result_to_bytes(&file.contents, materialize_options),
            };
        }
        MaterializedTreeValue::OtherConflict { id } => {
            mode = "100644";
            hash = DUMMY_HASH.to_owned();
            content = FileContent {
                is_binary: false,
                contents: id.describe().into(),
            };
        }
        MaterializedTreeValue::Tree(_) => {
            panic!("Unexpected tree in diff at path {path:?}");
        }
    }
    hash.truncate(10);
    Ok(GitDiffPart {
        mode: Some(mode),
        hash,
        content,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineType {
    Context,
    Removed,
    Added,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffTokenType {
    Matching,
    Different,
}

type DiffTokenVec<'content> = Vec<(DiffTokenType, &'content [u8])>;

pub struct UnifiedDiffHunk<'content> {
    pub left_line_range: Range<usize>,
    pub right_line_range: Range<usize>,
    pub lines: Vec<(DiffLineType, DiffTokenVec<'content>)>,
}

impl<'content> UnifiedDiffHunk<'content> {
    fn extend_context_lines(&mut self, lines: impl IntoIterator<Item = &'content [u8]>) {
        let old_len = self.lines.len();
        self.lines.extend(lines.into_iter().map(|line| {
            let tokens = vec![(DiffTokenType::Matching, line)];
            (DiffLineType::Context, tokens)
        }));
        self.left_line_range.end += self.lines.len() - old_len;
        self.right_line_range.end += self.lines.len() - old_len;
    }

    fn extend_removed_lines(&mut self, lines: impl IntoIterator<Item = DiffTokenVec<'content>>) {
        let old_len = self.lines.len();
        self.lines
            .extend(lines.into_iter().map(|line| (DiffLineType::Removed, line)));
        self.left_line_range.end += self.lines.len() - old_len;
    }

    fn extend_added_lines(&mut self, lines: impl IntoIterator<Item = DiffTokenVec<'content>>) {
        let old_len = self.lines.len();
        self.lines
            .extend(lines.into_iter().map(|line| (DiffLineType::Added, line)));
        self.right_line_range.end += self.lines.len() - old_len;
    }
}

// TODO: Split in more commits
pub fn unified_diff_hunks<'content>(
    contents: [&'content BStr; 2],
    context: usize,
    diff_by_line: impl FnOnce([&'content BStr; 2]) -> ContentDiff<'content>,
) -> Vec<UnifiedDiffHunk<'content>> {
    let mut hunks = vec![];
    let mut current_hunk = UnifiedDiffHunk {
        left_line_range: 0..0,
        right_line_range: 0..0,
        lines: vec![],
    };
    let diff = diff_by_line(contents);
    let mut diff_hunks = diff.hunks().peekable();
    while let Some(hunk) = diff_hunks.next() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                // Just use the right (i.e. new) content. We could count the
                // number of skipped lines separately, but the number of the
                // context lines should match the displayed content.
                let [_, right] = hunk.contents[..].try_into().unwrap();
                let mut lines = right.split_inclusive(|b| *b == b'\n').fuse();
                if !current_hunk.lines.is_empty() {
                    // The previous hunk line should be either removed/added.
                    current_hunk.extend_context_lines(lines.by_ref().take(context));
                }
                let before_lines = if diff_hunks.peek().is_some() {
                    lines.by_ref().rev().take(context).collect()
                } else {
                    vec![] // No more hunks
                };
                let num_skip_lines = lines.count();
                if num_skip_lines > 0 {
                    let left_start = current_hunk.left_line_range.end + num_skip_lines;
                    let right_start = current_hunk.right_line_range.end + num_skip_lines;
                    if !current_hunk.lines.is_empty() {
                        hunks.push(current_hunk);
                    }
                    current_hunk = UnifiedDiffHunk {
                        left_line_range: left_start..left_start,
                        right_line_range: right_start..right_start,
                        lines: vec![],
                    };
                }
                // The next hunk should be of DiffHunk::Different type if any.
                current_hunk.extend_context_lines(before_lines.into_iter().rev());
            }
            DiffHunkKind::Different => {
                let [left_lines, right_lines] =
                    unzip_diff_hunks_to_lines(ContentDiff::by_word(hunk.contents).hunks());
                current_hunk.extend_removed_lines(left_lines);
                current_hunk.extend_added_lines(right_lines);
            }
        }
    }
    if !current_hunk.lines.is_empty() {
        hunks.push(current_hunk);
    }
    hunks
}

/// Splits `[left, right]` hunk pairs into `[left_lines, right_lines]`.
pub fn unzip_diff_hunks_to_lines<'content, I>(diff_hunks: I) -> [Vec<DiffTokenVec<'content>>; 2]
where
    I: IntoIterator,
    I::Item: Borrow<DiffHunk<'content>>,
{
    let mut left_lines: Vec<DiffTokenVec<'content>> = vec![];
    let mut right_lines: Vec<DiffTokenVec<'content>> = vec![];
    let mut left_tokens: DiffTokenVec<'content> = vec![];
    let mut right_tokens: DiffTokenVec<'content> = vec![];

    for hunk in diff_hunks {
        let hunk = hunk.borrow();
        match hunk.kind {
            DiffHunkKind::Matching => {
                // TODO: add support for unmatched contexts
                debug_assert!(hunk.contents.iter().all_equal());
                for token in hunk.contents[0].split_inclusive(|b| *b == b'\n') {
                    left_tokens.push((DiffTokenType::Matching, token));
                    right_tokens.push((DiffTokenType::Matching, token));
                    if token.ends_with(b"\n") {
                        left_lines.push(mem::take(&mut left_tokens));
                        right_lines.push(mem::take(&mut right_tokens));
                    }
                }
            }
            DiffHunkKind::Different => {
                let [left, right] = hunk.contents[..]
                    .try_into()
                    .expect("hunk should have exactly two inputs");
                for token in left.split_inclusive(|b| *b == b'\n') {
                    left_tokens.push((DiffTokenType::Different, token));
                    if token.ends_with(b"\n") {
                        left_lines.push(mem::take(&mut left_tokens));
                    }
                }
                for token in right.split_inclusive(|b| *b == b'\n') {
                    right_tokens.push((DiffTokenType::Different, token));
                    if token.ends_with(b"\n") {
                        right_lines.push(mem::take(&mut right_tokens));
                    }
                }
            }
        }
    }

    if !left_tokens.is_empty() {
        left_lines.push(left_tokens);
    }
    if !right_tokens.is_empty() {
        right_lines.push(right_tokens);
    }
    [left_lines, right_lines]
}
