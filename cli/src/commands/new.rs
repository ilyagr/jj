// Copyright 2020 The Jujutsu Authors
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

use std::io::Write;
use std::rc::Rc;

use clap::ArgGroup;
use indexmap::IndexSet;
use itertools::Itertools;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::repo::{MutableRepo, Repo};
use jj_lib::revset::{RevsetExpression, RevsetIteratorExt};
use jj_lib::rewrite::{merge_commit_trees, rebase_commit};
use jj_lib::settings::UserSettings;
use tracing::instrument;

use crate::cli_util::{
    self, short_commit_hash, user_error, CommandError, CommandHelper, RevisionArg,
    WorkspaceCommandHelper,
};
use crate::ui::Ui;

/// Create a new, empty change and edit it in the working copy
///
/// Note that you can create a merge commit by specifying multiple revisions as
/// argument. For example, `jj new main @` will create a new commit with the
/// `main` branch and the working copy as parents.
///
/// For more information, see
/// https://github.com/martinvonz/jj/blob/main/docs/working-copy.md.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("order").args(&["insert_after", "insert_before"])))]
pub(crate) struct NewArgs {
    /// Parent(s) of the new change
    #[arg(default_value = "@")]
    pub(crate) revisions: Vec<RevisionArg>,
    /// Ignored (but lets you pass `-r` for consistency with other commands)
    #[arg(short = 'r', hide = true)]
    unused_revision: bool,
    /// The change description to use
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,
    /// Deprecated. Please prefix the revset with `all:` instead.
    #[arg(long, short = 'L', hide = true)]
    allow_large_revsets: bool,
    /// Insert the new change between the target commit(s) and their children
    #[arg(long, short = 'A', visible_alias = "after")]
    insert_after: bool,
    /// Insert the new change between the target commit(s) and their parents
    #[arg(long, short = 'B', visible_alias = "before")]
    insert_before: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_new(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &NewArgs,
) -> Result<(), CommandError> {
    if args.allow_large_revsets {
        return Err(user_error(
            "--allow-large-revsets has been deprecated.
Please use `jj new 'all:x|y'` instead of `jj new --allow-large-revsets x y`.",
        ));
    }
    let mut workspace_command = command.workspace_helper(ui)?;
    assert!(
        !args.revisions.is_empty(),
        "expected a non-empty list from clap"
    );
    let target_commits = cli_util::resolve_all_revs(&workspace_command, ui, &args.revisions)?
        .into_iter()
        .collect_vec();
    let mut tx = workspace_command.start_transaction("new empty commit");
    let mut num_rebased;
    let new_commit;
    if args.insert_before {
        // Instead of having the new commit as a child of the changes given on the
        // command line, add it between the changes' parents and the changes.
        // The parents of the new commit will be the parents of the target commits
        // which are not descendants of other target commits.
        let new_parents_commits =
            get_parents_for_insert_before(tx.base_workspace_helper(), &target_commits)?;
        let new_children_commits = target_commits;
        let merged_tree = merge_commit_trees(tx.repo(), &new_parents_commits)?;
        let new_parents_commit_id = new_parents_commits.iter().map(|c| c.id().clone()).collect();
        new_commit = tx
            .mut_repo()
            .new_commit(command.settings(), new_parents_commit_id, merged_tree.id())
            .set_description(cli_util::join_message_paragraphs(&args.message_paragraphs))
            .write()?;
        num_rebased = new_children_commits.len();
        for child_commit in new_children_commits {
            rebase_commit(
                command.settings(),
                tx.mut_repo(),
                &child_commit,
                &[new_commit.clone()],
            )?;
        }
    } else {
        let parent_ids = target_commits.iter().map(|c| c.id().clone()).collect_vec();
        let parents = RevsetExpression::commits(parent_ids);
        let commits_to_rebase: Vec<Commit> = if args.insert_after {
            get_children_for_insert_after(tx.base_workspace_helper(), &parents)?
        } else {
            vec![]
        };
        let merged_tree = merge_commit_trees(tx.repo(), &target_commits)?;
        let parent_ids = target_commits.iter().map(|c| c.id().clone()).collect_vec();
        let mut new_commit_array = vec![tx
            .mut_repo()
            .new_commit(command.settings(), parent_ids, merged_tree.id())
            .set_description(cli_util::join_message_paragraphs(&args.message_paragraphs))
            .write()?];
        num_rebased = commits_to_rebase.len();
        rebase_commits_replacing_certain_parents(
            tx.mut_repo(),
            command.settings(),
            &commits_to_rebase,
            &target_commits,
            &new_commit_array,
        )?;
        new_commit = new_commit_array.remove(0);
    }
    num_rebased += tx.mut_repo().rebase_descendants(command.settings())?;
    if num_rebased > 0 {
        writeln!(ui.stderr(), "Rebased {num_rebased} descendant commits")?;
    }
    tx.edit(&new_commit).unwrap();
    tx.finish(ui)?;
    Ok(())
}

/// Rebases exactly `children_to_replace.len()` commits. Does not call
/// `rebase_descendants`.
///
/// Requirements: none of `parents_to_replace` or `replacement_parents` are
/// descendants of `children_to_rebase.`
fn rebase_commits_replacing_certain_parents(
    mut_repo: &mut MutableRepo,
    settings: &UserSettings,
    children_to_rebase: &[Commit],
    parents_to_replace: &[Commit],
    replacement_parents: &[Commit],
) -> Result<(), CommandError> {
    for child_commit in children_to_rebase {
        let parents_to_replace_ids: IndexSet<CommitId> = parents_to_replace
            .iter()
            .map(|commit| commit.id().clone())
            .collect();
        let mut removed_something = false;
        let mut new_parent_commit_ids: IndexSet<&CommitId> = child_commit
            .parent_ids()
            .iter()
            .filter(|id| {
                let remove = parents_to_replace_ids.contains(*id);
                removed_something = removed_something || remove;
                !remove
            })
            .collect();
        if removed_something {
            // Add the ids rather than commits themselves to de-duplicate
            // TODO: Check if de-duplication or `removed_something` is unnecessary
            new_parent_commit_ids.extend(replacement_parents.iter().map(|commit| commit.id()));
        }
        let new_parent_commits: Vec<Commit> = new_parent_commit_ids
            .into_iter()
            .map(|id| mut_repo.store().get_commit(id))
            .try_collect()?;
        rebase_commit(settings, mut_repo, child_commit, &new_parent_commits)?;
    }
    Ok(())
}

fn get_children_for_insert_after(
    workspace_helper: &WorkspaceCommandHelper,
    parents: &Rc<RevsetExpression>,
) -> Result<Vec<Commit>, CommandError> {
    let repo = workspace_helper.repo().as_ref();
    // Each vscode-file://vscode-app/usr/share/code/resources/app/out/vs/code/electron-sandbox/workbench/workbench.htmlchild of the targets will be rebased: its set of parents will be updated
    // so that the targets are replaced by the new commit.
    // Exclude children that are ancestors of the new commit
    let to_rebase = parents.children().minus(&parents.ancestors());
    let commits_to_rebase = to_rebase
        .resolve(repo)?
        .evaluate(repo)?
        .iter()
        .commits(repo.store())
        .try_collect()?;
    workspace_helper.check_rewritable(&commits_to_rebase)?;
    Ok(commits_to_rebase)
}

fn get_parents_for_insert_before(
    workspace_helper: &WorkspaceCommandHelper,
    target_commits: &[Commit],
) -> Result<Vec<Commit>, CommandError> {
    let repo = workspace_helper.repo().as_ref();
    let target_ids = target_commits.iter().map(|c| c.id().clone()).collect_vec();
    workspace_helper.check_rewritable(target_commits)?;
    let new_children = RevsetExpression::commits(target_ids.clone());
    let new_parents = new_children.parents();
    if let Some(commit_id) = new_children
        .dag_range_to(&new_parents)
        .resolve(repo)?
        .evaluate(repo)?
        .iter()
        .next()
    {
        return Err(user_error(format!(
            "Refusing to create a loop: commit {} would be both an ancestor and a descendant of \
             the new commit",
            short_commit_hash(&commit_id),
        )));
    }
    let mut new_parents_commits: Vec<Commit> = new_parents
        .resolve(repo)?
        .evaluate(repo)?
        .iter()
        .commits(repo.store())
        .try_collect()?;
    // The git backend does not support creating merge commits involving the root
    // commit.
    if new_parents_commits.len() > 1 {
        let root_commit = repo.store().root_commit();
        new_parents_commits.retain(|c| c != &root_commit);
    }
    Ok(new_parents_commits)
}
