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

//! Code for working with copies and renames.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use futures::Stream;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::future::ready;
use futures::future::try_join_all;
use futures::stream::Fuse;
use futures::stream::FuturesOrdered;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CopyHistory;
use crate::backend::CopyId;
use crate::backend::CopyRecord;
use crate::backend::TreeValue;
use crate::dag_walk;
use crate::merge::Diff;
use crate::merge::Merge;
use crate::merge::MergedTreeValue;
use crate::merge::SameChange;
use crate::merged_tree::MergedTree;
use crate::merged_tree::TreeDiffEntry;
use crate::merged_tree::TreeDiffStream;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

/// A collection of CopyRecords.
#[derive(Default, Debug)]
pub struct CopyRecords {
    records: Vec<CopyRecord>,
    // Maps from `source` or `target` to the index of the entry in `records`.
    // Conflicts are excluded by keeping an out of range value.
    sources: HashMap<RepoPathBuf, usize>,
    targets: HashMap<RepoPathBuf, usize>,
}

impl CopyRecords {
    /// Adds information about `CopyRecord`s to `self`. A target with multiple
    /// conflicts is discarded and treated as not having an origin.
    pub fn add_records(
        &mut self,
        copy_records: impl IntoIterator<Item = BackendResult<CopyRecord>>,
    ) -> BackendResult<()> {
        for record in copy_records {
            let r = record?;
            self.sources
                .entry(r.source.clone())
                // TODO: handle conflicts instead of ignoring both sides.
                .and_modify(|value| *value = usize::MAX)
                .or_insert(self.records.len());
            self.targets
                .entry(r.target.clone())
                // TODO: handle conflicts instead of ignoring both sides.
                .and_modify(|value| *value = usize::MAX)
                .or_insert(self.records.len());
            self.records.push(r);
        }
        Ok(())
    }

    /// Returns true if there are copy records associated with a source path.
    pub fn has_source(&self, source: &RepoPath) -> bool {
        self.sources.contains_key(source)
    }

    /// Gets any copy record associated with a source path.
    pub fn for_source(&self, source: &RepoPath) -> Option<&CopyRecord> {
        self.sources.get(source).and_then(|&i| self.records.get(i))
    }

    /// Returns true if there are copy records associated with a target path.
    pub fn has_target(&self, target: &RepoPath) -> bool {
        self.targets.contains_key(target)
    }

    /// Gets any copy record associated with a target path.
    pub fn for_target(&self, target: &RepoPath) -> Option<&CopyRecord> {
        self.targets.get(target).and_then(|&i| self.records.get(i))
    }

    /// Gets all copy records.
    pub fn iter(&self) -> impl Iterator<Item = &CopyRecord> {
        self.records.iter()
    }
}

/// Whether or not the source path was deleted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyOperation {
    /// The source path was not deleted.
    Copy,
    /// The source path was renamed to the destination.
    Rename,
}

/// A `TreeDiffEntry` with copy information.
#[derive(Debug)]
pub struct CopiesTreeDiffEntry {
    /// The path.
    pub path: CopiesTreeDiffEntryPath,
    /// The resolved tree values if available.
    pub values: BackendResult<Diff<MergedTreeValue>>,
}

/// Path and copy information of `CopiesTreeDiffEntry`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopiesTreeDiffEntryPath {
    /// The source path and copy information if this is a copy or rename.
    pub source: Option<(RepoPathBuf, CopyOperation)>,
    /// The target path.
    pub target: RepoPathBuf,
}

impl CopiesTreeDiffEntryPath {
    /// The source path.
    pub fn source(&self) -> &RepoPath {
        self.source.as_ref().map_or(&self.target, |(path, _)| path)
    }

    /// The target path.
    pub fn target(&self) -> &RepoPath {
        &self.target
    }

    /// Whether this entry was copied or renamed from the source. Returns `None`
    /// if the path is unchanged.
    pub fn copy_operation(&self) -> Option<CopyOperation> {
        self.source.as_ref().map(|(_, op)| *op)
    }

    /// Returns source/target paths as [`Diff`] if they differ.
    pub fn to_diff(&self) -> Option<Diff<&RepoPath>> {
        let (source, _) = self.source.as_ref()?;
        Some(Diff::new(source, &self.target))
    }
}

/// Wraps a `TreeDiffStream`, adding support for copies and renames.
pub struct CopiesTreeDiffStream<'a> {
    inner: TreeDiffStream<'a>,
    source_tree: MergedTree,
    target_tree: MergedTree,
    copy_records: &'a CopyRecords,
}

impl<'a> CopiesTreeDiffStream<'a> {
    /// Create a new diff stream with copy information.
    pub fn new(
        inner: TreeDiffStream<'a>,
        source_tree: MergedTree,
        target_tree: MergedTree,
        copy_records: &'a CopyRecords,
    ) -> Self {
        Self {
            inner,
            source_tree,
            target_tree,
            copy_records,
        }
    }

    fn resolve_copy_source(
        &self,
        source: &RepoPath,
        values: BackendResult<Diff<MergedTreeValue>>,
    ) -> BackendResult<(CopyOperation, Diff<MergedTreeValue>)> {
        let target_value = values?.after;
        let source_value = self.source_tree.path_value(source)?;
        // If the source path is deleted in the target tree, it's a rename.
        let source_value_at_target = self.target_tree.path_value(source)?;
        let copy_op = if source_value_at_target.is_absent() || source_value_at_target.is_tree() {
            CopyOperation::Rename
        } else {
            CopyOperation::Copy
        };
        Ok((copy_op, Diff::new(source_value, target_value)))
    }
}

impl Stream for CopiesTreeDiffStream<'_> {
    type Item = CopiesTreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Some(diff_entry) = ready!(self.inner.as_mut().poll_next(cx)) {
            let Some(CopyRecord { source, .. }) = self.copy_records.for_target(&diff_entry.path)
            else {
                let target_deleted =
                    matches!(&diff_entry.values, Ok(diff) if diff.after.is_absent());
                if target_deleted && self.copy_records.has_source(&diff_entry.path) {
                    // Skip the "delete" entry when there is a rename.
                    continue;
                }
                return Poll::Ready(Some(CopiesTreeDiffEntry {
                    path: CopiesTreeDiffEntryPath {
                        source: None,
                        target: diff_entry.path,
                    },
                    values: diff_entry.values,
                }));
            };

            let (copy_op, values) = match self.resolve_copy_source(source, diff_entry.values) {
                Ok((copy_op, values)) => (copy_op, Ok(values)),
                // Fall back to "copy" (= path still exists) if unknown.
                Err(err) => (CopyOperation::Copy, Err(err)),
            };
            return Poll::Ready(Some(CopiesTreeDiffEntry {
                path: CopiesTreeDiffEntryPath {
                    source: Some((source.clone(), copy_op)),
                    target: diff_entry.path,
                },
                values,
            }));
        }

        Poll::Ready(None)
    }
}

/// Maps `CopyId`s to `CopyHistory`s
pub type CopyGraph = IndexMap<CopyId, CopyHistory>;

fn collect_descendants(copy_graph: &CopyGraph) -> IndexMap<CopyId, IndexSet<CopyId>> {
    let mut ancestor_map: IndexMap<CopyId, IndexSet<CopyId>> = IndexMap::new();

    // Collect ancestors
    //
    // Keys in the map will be ordered with parents before children. The set of
    // ancestors for a given key will also be ordered with parents before
    // children.
    let heads = dag_walk::heads(
        copy_graph.keys(),
        |id| *id,
        |id| copy_graph[*id].parents.iter(),
    );
    for id in dag_walk::topo_order_forward(
        heads,
        |id| *id,
        |id| copy_graph[*id].parents.iter(),
        |id| panic!("Cycle detected in copy history graph involving CopyId {id}"),
    )
    .expect("Could not walk CopyGraph")
    {
        // For each ID we visit, we should have visited all of its parents first.
        let mut ancestors = IndexSet::new();
        for parent in &copy_graph[id].parents {
            ancestors.extend(ancestor_map[parent].iter().cloned());
            ancestors.insert(parent.clone());
        }
        ancestor_map.insert(id.clone(), ancestors);
    }

    // Reverse ancestor map to descendant map
    let mut result: IndexMap<CopyId, IndexSet<CopyId>> = IndexMap::new();
    for (id, ancestors) in ancestor_map {
        for ancestor in ancestors {
            result.entry(ancestor).or_default().insert(id.clone());
        }
        // Make sure every CopyId in the graph has an entry in the descendants map, even
        // if it has no descendants of its own.
        result.entry(id.clone()).or_default();
    }
    result
}

/// Iterate over the ancestors of a starting CopyId, visiting children before
/// parents.
pub fn traverse_copy_history<'a>(
    copies: &'a CopyGraph,
    initial_id: &'a CopyId,
) -> impl Iterator<Item = &'a CopyId> {
    let mut valid = HashSet::from([initial_id]);
    copies.iter().filter_map(move |(id, history)| {
        if valid.contains(id) {
            valid.extend(history.parents.iter());
            Some(id)
        } else {
            None
        }
    })
}

/// Returns whether `maybe_child` is a descendant of `parent`
pub fn is_ancestor(copies: &CopyGraph, ancestor: &CopyId, descendant: &CopyId) -> bool {
    for history in dag_walk::dfs(
        [descendant],
        |id| *id,
        |id| copies.get(*id).unwrap().parents.iter(),
    ) {
        if history == ancestor {
            return true;
        }
    }
    false
}

/// Describes the source of a CopyHistoryDiffTerm
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CopyHistorySource {
    /// The file was copied from a source at a different path
    Copy(RepoPathBuf),
    /// The file was renamed from a source at a different path
    Rename(RepoPathBuf),
    /// The source and target have the same path
    Normal,
}

/// Describes a single term of a copy-aware diff
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct CopyHistoryDiffTerm {
    /// The current value of the target, if present
    pub target_value: Option<TreeValue>,
    /// List of sources, whether they were copied, renamed, or neither, and the
    /// original value
    pub sources: Vec<(CopyHistorySource, MergedTreeValue)>,
}

/// Like a `TreeDiffEntry`, but takes `CopyHistory`s into account
#[derive(Debug)]
pub struct CopyHistoryTreeDiffEntry {
    /// The final source path (after copy/rename if applicable)
    pub target_path: RepoPathBuf,
    /// The resolved values for the target and source(s), if available
    pub diffs: BackendResult<Merge<CopyHistoryDiffTerm>>,
}

impl CopyHistoryTreeDiffEntry {
    // Simple conversion case where no copy tracing is needed
    fn normal(diff_entry: TreeDiffEntry) -> Self {
        let target_path = diff_entry.path;
        let diffs = diff_entry.values.map(|diff| {
            let sources = if diff.before.is_absent() {
                vec![]
            } else {
                vec![(CopyHistorySource::Normal, diff.before)]
            };
            Merge::from_vec(
                diff.after
                    .into_iter()
                    .map(|target_value| CopyHistoryDiffTerm {
                        target_value,
                        sources: sources.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        });
        Self { target_path, diffs }
    }
}

/// Adapts a `TreeDiffStream` to follow copies / renames.
pub struct CopyHistoryDiffStream<'a> {
    inner: Fuse<TreeDiffStream<'a>>,
    /// Synthetic `TreeDiffEntry`s from splitting shadowing entries (same path,
    /// different copy IDs) into separate deletion + creation entries.
    pending_inner: VecDeque<TreeDiffEntry>,
    before_tree: &'a MergedTree,
    after_tree: &'a MergedTree,
    pending: FuturesOrdered<BoxFuture<'static, Option<CopyHistoryTreeDiffEntry>>>,
}

impl<'a> CopyHistoryDiffStream<'a> {
    /// Creates an iterator over the differences between two trees, taking copy
    /// history into account. Generally prefer
    /// `MergedTree::diff_stream_with_copy_history()` instead of calling this
    /// directly.
    pub fn new(
        inner: TreeDiffStream<'a>,
        before_tree: &'a MergedTree,
        after_tree: &'a MergedTree,
    ) -> Self {
        Self {
            inner: inner.fuse(),
            pending_inner: VecDeque::with_capacity(2),
            before_tree,
            after_tree,
            pending: FuturesOrdered::new(),
        }
    }
}

impl Stream for CopyHistoryDiffStream<'_> {
    type Item = CopyHistoryTreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // First, check if we have newly-finished futures. If this returns Pending, we
            // intentionally fall through to poll `self.inner`.
            if let Poll::Ready(Some(next)) = self.pending.poll_next_unpin(cx) {
                if let Some(entry) = next {
                    return Poll::Ready(Some(entry));
                }
                // The future evaluated successfully but does not wish to provide any diff
                // entries
                continue;
            }

            // If we didn't have queued results above, we want to check our wrapped stream
            // for the next non-copy-matched diff entry.
            let next_diff_entry = match self.pending_inner.pop_front() {
                Some(entry) => entry,
                None => match ready!(self.inner.poll_next_unpin(cx)) {
                    Some(diff_entry) => diff_entry,
                    None if self.pending.is_empty() => return Poll::Ready(None),
                    _ => return Poll::Pending,
                },
            };

            let Ok(Diff { before, after }) = &next_diff_entry.values else {
                self.pending
                    .push_back(Box::pin(ready(Some(CopyHistoryTreeDiffEntry::normal(
                        next_diff_entry,
                    )))));
                continue;
            };

            // Don't try copy-tracing if we have conflicts on either side.
            //
            // TODO: consider accepting conflicts if the copy IDs can be resolved.
            let Some(before) = before.as_resolved() else {
                self.pending
                    .push_back(Box::pin(ready(Some(CopyHistoryTreeDiffEntry::normal(
                        next_diff_entry,
                    )))));
                continue;
            };
            let Some(after) = after.as_resolved() else {
                self.pending
                    .push_back(Box::pin(ready(Some(CopyHistoryTreeDiffEntry::normal(
                        next_diff_entry,
                    )))));
                continue;
            };

            match (before, after) {
                // If we have files with matching copy_ids, no need to do copy-tracing.
                (
                    Some(TreeValue::File { copy_id: id1, .. }),
                    Some(TreeValue::File { copy_id: id2, .. }),
                ) if id1 == id2 => {
                    self.pending.push_back(Box::pin(ready(Some(
                        CopyHistoryTreeDiffEntry::normal(next_diff_entry),
                    ))));
                }

                // Shadowing: same path, different copy IDs (or non-file →
                // file). Split into a deletion of the old value and a
                // creation of the new value, then re-process both through
                // the main loop. This lets the deletion go through
                // `file_was_renamed_away` (if the old value is a File),
                // suppressing it when the old file was actually renamed
                // elsewhere — e.g., in chain renames like a→b, old_b→c.
                //
                // Note: same-path-parent cases are unaffected because
                // `classify_source` returns Normal (not Rename), so
                // `file_was_renamed_away` returns false for them.
                (Some(other), Some(f @ TreeValue::File { .. })) => {
                    self.pending_inner.push_back(TreeDiffEntry {
                        path: next_diff_entry.path.clone(),
                        values: Ok(Diff {
                            before: Merge::resolved(Some(other.clone())),
                            after: Merge::resolved(None),
                        }),
                    });
                    self.pending_inner.push_back(TreeDiffEntry {
                        path: next_diff_entry.path.clone(),
                        values: Ok(Diff {
                            before: Merge::resolved(None),
                            after: Merge::resolved(Some(f.clone())),
                        }),
                    });
                    continue;
                }

                // New file with copy history — do copy-tracing.
                (None, Some(f @ TreeValue::File { .. })) => {
                    let future = tree_diff_entry_from_copies(
                        self.before_tree.clone(),
                        self.after_tree.clone(),
                        f.clone(),
                        next_diff_entry.path.clone(),
                    );
                    self.pending.push_back(Box::pin(future));
                }

                (Some(f @ TreeValue::File { .. }), None) => {
                    // A file is has been either deleted or renamed. Use
                    // reversed copy-tracing to check which. If it was renamed,
                    // we emit nothing now. The rename entry is emitted on a
                    // different loop iteration, whenever the corresponding
                    // entry on the "after" side is processed.
                    let before_tree = self.before_tree.clone();
                    let after_tree = self.after_tree.clone();
                    let f = f.clone();
                    self.pending.push_back(Box::pin(async move {
                        if file_was_renamed_away(before_tree, after_tree, f).await {
                            None
                        } else {
                            Some(CopyHistoryTreeDiffEntry::normal(next_diff_entry))
                        }
                    }));
                }

                // Anything else (e.g. non-file deletions, file => non-file),
                // issue a simple diff entry.
                _ => {
                    self.pending.push_back(Box::pin(ready(Some(
                        CopyHistoryTreeDiffEntry::normal(next_diff_entry),
                    ))));
                }
            }
        }
    }
}

async fn classify_source(
    after_tree: &MergedTree,
    after_id: &CopyId,
    before_path: RepoPathBuf,
    before_id: &CopyId,
    copy_graph: &CopyGraph,
) -> BackendResult<CopyHistorySource> {
    let history = copy_graph
        .get(after_id)
        .expect("copy_graph should already include after_id");

    // First, check to see if we're looking at the same path with different copy
    // IDs, but an ancestor relationship between the histories. If so, this is a
    // "normal" diff source.
    if history.current_path == before_path
        && (is_ancestor(copy_graph, after_id, before_id)
            || is_ancestor(copy_graph, before_id, after_id))
    {
        return Ok(CopyHistorySource::Normal);
    }

    let after_val = after_tree.path_value_async(&before_path).await?;
    // We're getting our arguments from `find_diff_sources_from_copies`, so we
    // shouldn't have to worry about missing paths or conflicts. So let's just
    // be lazy and `.expect()` our way out of all the `Option`s.
    let after_id = after_val
        .to_copy_id_merge()
        .expect("expected merge of `TreeValue::File`s")
        .resolve_trivial(SameChange::Accept)
        .expect("expected no CopyId conflicts")
        .clone()
        // This may be absent, but we check for that later, so use a placeholder for now
        .unwrap_or_else(CopyId::placeholder);

    // Renames can come in two forms:
    // 1) before_path is no longer present in after_tree, or
    // 2) before_path in before_tree & after_tree are not ancestors/descendants of
    //    each other
    //
    //    NB: for this case, a file with the same copy_id is considered to be an
    //    ancestor of itself
    if after_val.is_absent()
        || !(is_ancestor(copy_graph, before_id, &after_id)
            || is_ancestor(copy_graph, &after_id, before_id))
    {
        Ok(CopyHistorySource::Rename(before_path))
    } else {
        Ok(CopyHistorySource::Copy(before_path))
    }
}

async fn tree_diff_entry_from_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    file: TreeValue,
    target_path: RepoPathBuf,
) -> Option<CopyHistoryTreeDiffEntry> {
    Some(CopyHistoryTreeDiffEntry {
        target_path,
        diffs: diffs_from_copies(before_tree, after_tree, file).await,
    })
}

/// Checks whether `before_file`, which is assumed to exist in `before_tree` but
/// not in `after_tree` at its current path, was deleted or whether it was
/// renamed to something else in the `after` tree.
async fn file_was_renamed_away(
    before_tree: MergedTree,
    after_tree: MergedTree,
    before_file: TreeValue,
) -> bool {
    // Call `diffs_from_copies` with the trees reversed. From the reversed
    // perspective, `file` looks like a new entry in the "before" tree.
    //
    // `diffs_from_copies` will check whether it corresponds to something from
    // the "after" tree. If not, `diffs_from_copies` will see `before_file` as a
    // completely new file creation and return a diff entry with no sources.
    // This means that the removal of `before_file` in the `before_tree` is a
    // genuine deletion. If the removal of `before_file` was a rename,
    // `diffs_from_copies` will return a `CopyHistorySource::Rename` (it will
    // detect the reverse rename). In unusual cases, we may get a `Copy` instead
    // — this means the target path exists in both trees, so the deletion is a
    // separate real event and should not be suppressed.

    // TODO: Figure out what's going on with this Copy case. (Aside:
    // CopyHistorySource::Normal shouldn't happen because `before_file` is not
    // in `after_tree`). Here's what AI thinks about that:

    // AI: The previous Claude instance tried treating `Rename` and `Copy` the same.
    // AI: This broke the reverse case of `test_copy_diffstream_copy`. The scenario:
    //
    // AI: - `foo` exists, `bar` is a copy of `foo`. Then `bar` is deleted.
    // AI: - Reversed copy-tracing for `bar` finds `foo` as a relative. Since `foo`
    // AI:   still exists at its path in both trees, `classify_source` returns
    // AI:   `Copy(foo)`, not `Rename(foo)`.
    // AI: - If we suppressed on `Copy`, we'd lose `bar`'s deletion — but `bar` was
    // AI:   genuinely deleted (`foo` still exists, it wasn't a rename).
    //
    // AI: The distinction is clear: `Rename(path)` means the source path is absent
    // AI: from the "after" tree (in the reversed sense), so the file truly moved
    // AI: away. `Copy(path)` means the source still exists, so the deletion is a
    // AI: separate real event.
    diffs_from_copies(after_tree, before_tree, before_file)
        .await
        .is_ok_and(|diffs| {
            // Note: as of this writing, `diffs` will always be a resolved
            // conflict with a single add. The conflict logic should be correct,
            // but is not tested.
            diffs.adds().any(|term| {
                term.sources
                    .iter()
                    .any(|(src, _)| matches!(src, CopyHistorySource::Rename(_)))
            })
        })
}

async fn diffs_from_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    file: TreeValue,
) -> BackendResult<Merge<CopyHistoryDiffTerm>> {
    let copy_id = file.copy_id().ok_or(BackendError::Other(
        "Expected TreeValue::File with a CopyId".into(),
    ))?;
    let copy_graph: CopyGraph = before_tree
        .store()
        .backend()
        .get_related_copies(copy_id)
        .await?
        .into_iter()
        .map(|related| (related.id, related.history))
        .collect();
    let descendants = collect_descendants(&copy_graph);

    let copies =
        find_diff_sources_from_copies(&before_tree, copy_id, &copy_graph, &descendants).await?;

    try_join_all(copies.into_iter().map(async |(path, val)| {
        classify_source(
            &after_tree,
            copy_id,
            path,
            val.copy_id()
                .expect("expected TreeValue::File with a CopyId"),
            &copy_graph,
        )
        .await
        .map(|source| (source, Merge::resolved(Some(val))))
    }))
    .await
    .map(|sources| {
        Merge::resolved(CopyHistoryDiffTerm {
            target_value: Some(file),
            sources,
        })
    })
}

// Finds at most one related TreeValue::File present in `tree` per parent listed
// in `file`'s CopyHistory.
//
// TODO: figure out a way to select better relatives (see TODOs below) and/or
// return multiple relatives per parent.
async fn find_diff_sources_from_copies(
    tree: &MergedTree,
    copy_id: &CopyId,
    copy_graph: &CopyGraph,
    descendants: &IndexMap<CopyId, IndexSet<CopyId>>,
) -> BackendResult<Vec<(RepoPathBuf, TreeValue)>> {
    // Related copies MUST contain ancestors AND descendants. It may also contain
    // unrelated copies.
    let history = copy_graph.get(copy_id).ok_or(BackendError::Other(
        "CopyId should be present in `get_related_copies()` result".into(),
    ))?;

    let mut sources = vec![];

    // TODO: this correctly finds the shallowest relative, but it only finds
    // one. I'm not sure what is the best thing to do when one of our parents
    // itself has multiple parents. E.g., if we have a CopyHistory graph like
    //
    //      D
    //      |
    //      C
    //     / \
    //    A   B
    //
    // where D is `file`, C is its parent but is not present in `tree`, but both A
    // and B are present, this will find either A or B, not both. Should we
    // return both A and B instead? I don't think there's a way to do that with
    // the current dag_walk functions. Do we care enough to implement something
    // new there that pays more attention to the depth in the DAG? Perhaps
    // a variant of closest_common_node?
    'parents: for parent_copy_id in &history.parents {
        let mut absent_ancestors = vec![];

        // First, try to find the parent or a direct ancestor in the tree
        for ancestor_id in traverse_copy_history(copy_graph, parent_copy_id) {
            let ancestor_history = copy_graph.get(ancestor_id).ok_or(BackendError::Other(
                "Ancestor CopyId should be present in `get_related_copies()` result".into(),
            ))?;
            if let Some(ancestor) = tree.copy_value(ancestor_id).await? {
                sources.push((ancestor_history.current_path.clone(), ancestor));
                continue 'parents;
            } else {
                absent_ancestors.push(ancestor_id);
            }
        }

        // If not, then try descendants of the parent
        //
        // TODO: This will find a relative, when what we really want is probably the
        // "closest" relative.
        for descendant_id in &descendants[parent_copy_id] {
            if let Some(descendant) = tree.copy_value(descendant_id).await? {
                sources.push((copy_graph[descendant_id].current_path.clone(), descendant));
                continue 'parents;
            }
        }

        // Finally, try descendants of any ancestor
        //
        // TODO: This will find a relative, when what we really want is probably the
        // "closest" relative.
        for ancestor_id in absent_ancestors {
            for descendant_id in &descendants[ancestor_id] {
                if let Some(descendant) = tree.copy_value(descendant_id).await? {
                    sources.push((copy_graph[descendant_id].current_path.clone(), descendant));
                    continue 'parents;
                }
            }
        }
    }

    if history.parents.is_empty() {
        // If there are no parents, let's instead look for a descendant (this handles
        // the reverse-diff case of a file rename.
        for descendant_id in &descendants[copy_id] {
            if let Some(descendant) = tree.copy_value(descendant_id).await? {
                sources.push((copy_graph[descendant_id].current_path.clone(), descendant));
                break;
            }
        }
    }

    Ok(sources)
}
