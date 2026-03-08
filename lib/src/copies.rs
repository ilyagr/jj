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
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use futures::Stream;
use futures::StreamExt as _;
use futures::future::try_join_all;
use indexmap::IndexMap;
use indexmap::IndexSet;
use pollster::FutureExt as _;

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
    pub fn add_records(&mut self, copy_records: impl IntoIterator<Item = CopyRecord>) {
        for r in copy_records {
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

    async fn resolve_copy_source(
        &self,
        source: &RepoPath,
        values: BackendResult<Diff<MergedTreeValue>>,
    ) -> BackendResult<(CopyOperation, Diff<MergedTreeValue>)> {
        let target_value = values?.after;
        let source_value = self.source_tree.path_value(source).await?;
        // If the source path is deleted in the target tree, it's a rename.
        let source_value_at_target = self.target_tree.path_value(source).await?;
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

            let (copy_op, values) = match self
                .resolve_copy_source(source, diff_entry.values)
                .block_on()
            {
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
/// parents. The `CopyGraph` argument should be sorted in topological order.
fn iterate_ancestors<'a>(
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
            diff.after.into_map(|target_value| CopyHistoryDiffTerm {
                target_value,
                sources: sources.clone(),
            })
        });
        Self { target_path, diffs }
    }
}

/// Adapts a `TreeDiffStream` to follow copies / renames.
///
/// Generally prefer `MergedTree::diff_stream_with_copy_history()` instead of
/// calling this directly.
pub fn copy_history_diff_stream<'a>(
    inner: TreeDiffStream<'a>,
    before_tree: &'a MergedTree,
    after_tree: &'a MergedTree,
) -> impl Stream<Item = CopyHistoryTreeDiffEntry> + 'a {
    let before_tree = before_tree.clone();
    let after_tree = after_tree.clone();
    inner
        .map(move |entry| resolve_diff_entry_copies(before_tree.clone(), after_tree.clone(), entry))
        .buffered(64)
        .flat_map(futures::stream::iter)
}

/// Returns true if `id1` and `id2` represent a simple same-path evolution:
/// one is the sole direct parent of the other, and both are at the same path.
/// This excludes merges (multiple parents) and indirect ancestry through
/// different paths (e.g. rename round-trips like foo→bar→foo).
async fn is_simple_same_path_evolution(tree: &MergedTree, id1: &CopyId, id2: &CopyId) -> bool {
    let backend = tree.store().backend();
    let Ok(h1) = backend.read_copy(id1).await else {
        return false;
    };
    let Ok(h2) = backend.read_copy(id2).await else {
        return false;
    };
    h1.current_path == h2.current_path
        && (h1.parents == [id2.clone()] || h2.parents == [id1.clone()])
}

/// Classifies a `TreeDiffEntry` into zero or more [`CopyHistoryTreeDiffEntry`]s
/// by examining copy histories. Returns a `Vec` because shadowing cases (same
/// path, different copy IDs) decompose into a deletion + copy-traced creation.
///
/// This is the core logic of [`copy_history_diff_stream`], extracted into an
/// async function so it can be pipelined via `.buffered()`.
async fn resolve_diff_entry_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    diff_entry: TreeDiffEntry,
) -> Vec<CopyHistoryTreeDiffEntry> {
    let Ok(ref diff) = diff_entry.values else {
        return vec![CopyHistoryTreeDiffEntry::normal(diff_entry)];
    };

    // Don't try copy-tracing if we have conflicts on either side.
    //
    // TODO: consider handling conflicts, especially in the simpler case where the
    // corresponding "copy ID conflict" can be resolved.
    let (Some(before), Some(after)) = (diff.before.as_resolved(), diff.after.as_resolved()) else {
        return vec![CopyHistoryTreeDiffEntry::normal(diff_entry)];
    };

    match (before, after) {
        // If we have files with matching copy_ids, no need to do copy-tracing.
        (
            Some(TreeValue::File { copy_id: id1, .. }),
            Some(TreeValue::File { copy_id: id2, .. }),
        ) if id1 == id2 => vec![CopyHistoryTreeDiffEntry::normal(diff_entry)],

        // New file with copy history — needs copy-tracing.
        (None, Some(f @ TreeValue::File { .. })) => {
            let f = f.clone();
            vec![CopyHistoryTreeDiffEntry {
                target_path: diff_entry.path,
                diffs: diffs_from_copies(before_tree, after_tree, f).await,
            }]
        }

        // Same path, different copy IDs (or non-file → file).
        (Some(other), Some(f @ TreeValue::File { .. })) => {
            let before_copy_id = other.copy_id().cloned();
            let after_copy_id = f.copy_id().unwrap().clone();
            let other = other.clone();
            let f = f.clone();

            // When copy IDs differ but one is the sole direct parent of the
            // other at the same path, this is a simple evolution (e.g. a file
            // gaining new copy metadata). Emit as normal, don't split.
            if let Some(before_id) = &before_copy_id {
                if is_simple_same_path_evolution(&before_tree, before_id, &after_copy_id).await {
                    return vec![CopyHistoryTreeDiffEntry::normal(diff_entry)];
                }
            }
            // Otherwise, decompose into deletion + copy-traced creation.
            // The deletion is suppressed if `file_was_renamed_away` detects the
            // old file was renamed elsewhere (chain renames, swaps, etc.).
            let mut results = Vec::with_capacity(2);

            let suppress_deletion = matches!(&other, TreeValue::File { .. })
                && file_was_renamed_away(before_tree.clone(), after_tree.clone(), other.clone())
                    .await;
            if !suppress_deletion {
                results.push(CopyHistoryTreeDiffEntry::normal(TreeDiffEntry {
                    path: diff_entry.path.clone(),
                    values: Ok(Diff {
                        before: Merge::resolved(Some(other)),
                        after: Merge::resolved(None),
                    }),
                }));
            }

            results.push(CopyHistoryTreeDiffEntry {
                target_path: diff_entry.path,
                diffs: diffs_from_copies(before_tree, after_tree, f).await,
            });

            results
        }

        // File disappeared — might be a deletion or a rename.
        (Some(f @ TreeValue::File { .. }), None) => {
            let f = f.clone();
            // Use reversed copy-tracing to check whether the file was renamed
            // rather than deleted. If renamed, suppress — the rename entry is
            // emitted when the corresponding "after" side is processed.
            if file_was_renamed_away(before_tree, after_tree, f).await {
                vec![]
            } else {
                vec![CopyHistoryTreeDiffEntry::normal(diff_entry)]
            }
        }

        // Anything else (e.g. non-file deletions, file → non-file).
        _ => vec![CopyHistoryTreeDiffEntry::normal(diff_entry)],
    }
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
    after_file: TreeValue,
) -> BackendResult<Merge<CopyHistoryDiffTerm>> {
    let copy_id = after_file.copy_id().ok_or(BackendError::Other(
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

    try_join_all(copies.into_iter().map(async |(before_path, before_val)| {
        classify_source(
            &after_tree,
            copy_id,
            before_path,
            before_val
                .copy_id()
                .expect("expected TreeValue::File with a CopyId"),
            &copy_graph,
        )
        .await
        .map(|source| (source, Merge::resolved(Some(before_val))))
    }))
    .await
    .map(|sources| {
        Merge::resolved(CopyHistoryDiffTerm {
            target_value: Some(after_file),
            sources,
        })
    })
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
    let after_path = &history.current_path;

    // First, check to see if we're looking at the same path with different copy
    // IDs, but an ancestor relationship between the histories. If so, this is a
    // "normal" diff source.
    if *after_path == before_path
        && (is_ancestor(copy_graph, after_id, before_id)
            || is_ancestor(copy_graph, before_id, after_id))
    {
        return Ok(CopyHistorySource::Normal);
    }

    let after_tree_before_path_val = after_tree.path_value(&before_path).await?;
    // We're getting our arguments from `find_diff_sources_from_copies`, so we
    // shouldn't have to worry about missing paths or conflicts. So let's just
    // be lazy and `.expect()` our way out of all the `Option`s.
    let Some(after_tree_before_path_id) = after_tree_before_path_val
        .to_copy_id_merge()
        .expect("expected merge of `TreeValue::File`s")
        .resolve_trivial(SameChange::Accept)
        .expect("expected no CopyId conflicts")
        .clone()
    else {
        // before_path is no longer present in after_tree
        return Ok(CopyHistorySource::Rename(before_path));
    };

    if is_ancestor(copy_graph, before_id, &after_tree_before_path_id)
        || is_ancestor(copy_graph, &after_tree_before_path_id, before_id)
    {
        Ok(CopyHistorySource::Copy(before_path))
    } else {
        //  before_path in before_tree & after_tree are not ancestors/descendants of
        //  each other
        Ok(CopyHistorySource::Rename(before_path))
    }
}

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

    if history.parents.is_empty() {
        // If there are no parents, let's look for a descendant (this handles
        // the reverse-diff case of a file rename.
        for descendant_id in &descendants[copy_id] {
            if let Some(descendant) = tree.copy_value(descendant_id).await? {
                return Ok(vec![(
                    copy_graph[descendant_id].current_path.clone(),
                    descendant,
                )]);
            }
        }
    }

    let mut sources = vec![];

    // Finds at most one related TreeValue::File present in `tree` per parent listed
    // in `file`'s CopyHistory.
    //
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
        for ancestor_id in iterate_ancestors(copy_graph, parent_copy_id) {
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
            for descendant_id in descendants[ancestor_id].difference(&descendants[parent_copy_id]) {
                if let Some(descendant) = tree.copy_value(descendant_id).await? {
                    sources.push((copy_graph[descendant_id].current_path.clone(), descendant));
                    continue 'parents;
                }
            }
        }
    }
    Ok(sources)
}
