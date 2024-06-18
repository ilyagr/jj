// Copyright 2020-2023 The Jujutsu Authors
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

use itertools::Itertools as _;
use jj_lib::op_store::BranchTarget;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use super::find_branches_with;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Forget a branch without marking it for deletion
///
/// A forgotten branch will not impact remotes on future pushes. It may be
/// recreated on future pulls if it still exists in the remote.
#[derive(clap::Args, Clone, Debug)]
#[command(group = clap::ArgGroup::new("scope").multiple(false).required(true))]
pub struct BranchForgetArgs {
    /// The branches to forget
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select branches by wildcard pattern. For details, see
    /// https://martinvonz.github.io/jj/latest/revsets/#string-patterns.
    #[arg(required = true, value_parser = StringPattern::parse)]
    names: Vec<StringPattern>,

    /// Forget everything about a branch, including its local and remote targets
    ///
    /// Fetching from remotes that contain a branch of this name will recreate
    /// the remote-tracking branches, and possibly the local branch as well.
    #[arg(long, short, group = "scope")]
    pub global: bool,

    /// Forget the local branch (if it exists) and untrack all of its remote
    /// counterparts
    ///
    /// This does not affect remote-tracking `branchname@remote` branches. If
    /// any remote-tracking branches exist, you can recreate a local branch with
    /// `jj branch track branchname@remote`.
    ///
    /// This operation is sufficient to prevent `jj git push` from trying to
    /// move or delete the remote branches, until one of the remote branches
    /// becomes tracked again.
    ///
    /// This operation does affect the local git repo's branches if you are
    /// using `jj git export` or are in a repository that's co-located with Git.
    //
    // TODO(ilyagr): This could become the default in the future.
    // TODO(ilyagr): We may want to have a third scope option: `--from-remote
    // REMOTE` (or just `--remote`). This only seems compatible with making `--local` the default if
    // we disallow `jj branch forget --local --remote REMOTE`.
    #[arg(long, short, group = "scope")]
    pub local: bool,
}

pub fn cmd_branch_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let matched_branches = find_forgettable_branches(repo.view(), &args.names)?;

    let mut tx = workspace_command.start_transaction();
    assert!(
        args.local || args.global,
        "clap should ensure --local or --global is specified"
    );

    for (name, branch_target) in &matched_branches {
        tx.repo_mut()
            .set_local_branch_target(name, RefTarget::absent());
        for (remote_name, _) in &branch_target.remote_refs {
            if args.global {
                tx.repo_mut()
                    .set_remote_branch(name, remote_name, RemoteRef::absent());
            } else if args.local {
                tx.repo_mut().untrack_remote_branch(name, remote_name)
            }
        }
    }
    if args.global {
        writeln!(
            ui.status(),
            "Forgot {} branches and their state on the remotes.",
            matched_branches.len()
        )?;
    } else {
        writeln!(
            ui.status(),
            "Forgot {} local branches.",
            matched_branches.len()
        )?;
    }
    let matched_branches_str = matched_branches.iter().map(|(name, _)| name).join(", ");
    tx.finish(
        ui,
        if args.global {
            format!("forget branch {matched_branches_str} globally",)
        } else {
            format!("forget branch {matched_branches_str} locally",)
        },
    )?;
    Ok(())
}

fn find_forgettable_branches<'a>(
    view: &'a View,
    name_patterns: &[StringPattern],
) -> Result<Vec<(&'a str, BranchTarget<'a>)>, CommandError> {
    find_branches_with(name_patterns, |pattern| {
        view.branches().filter(|(name, _)| pattern.matches(name))
    })
}
