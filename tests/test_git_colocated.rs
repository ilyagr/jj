// Copyright 2022 The Jujutsu Authors
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

use std::path::Path;

use git2::Oid;
use itertools::Itertools;

use crate::common::{get_stderr_string, TestEnvironment};

pub mod common;

#[test]
fn test_git_colocated() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&workspace_root).unwrap();

    // Create an initial commit in Git
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    git_repo
        .index()
        .unwrap()
        .add_path(Path::new("file"))
        .unwrap();
    let tree1_oid = git_repo.index().unwrap().write_tree().unwrap();
    let tree1 = git_repo.find_tree(tree1_oid).unwrap();
    let signature = git2::Signature::new(
        "Someone",
        "someone@example.com",
        &git2::Time::new(1234567890, 60),
    )
    .unwrap();
    git_repo
        .commit(
            Some("refs/heads/master"),
            &signature,
            &signature,
            "initial",
            &tree1,
            &[],
        )
        .unwrap();
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"e61b6729ff4292870702f2f72b2a60165679ef37"
    );

    // Import the repo
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  3e9369cd54227eb88455e1834dbc08aad6a16ac4
    ◉  e61b6729ff4292870702f2f72b2a60165679ef37 master HEAD@git initial
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"e61b6729ff4292870702f2f72b2a60165679ef37"
    );

    // Modify the working copy. The working-copy commit should changed, but the Git
    // HEAD commit should not
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  b26951a9c6f5c270e4d039880208952fd5faae5e
    ◉  e61b6729ff4292870702f2f72b2a60165679ef37 master HEAD@git initial
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"e61b6729ff4292870702f2f72b2a60165679ef37"
    );

    // Create a new change from jj and check that it's reflected in Git
    test_env.jj_cmd_success(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  9dbb23ff2ff5e66c43880f1042369d704f7a321e
    ◉  b26951a9c6f5c270e4d039880208952fd5faae5e HEAD@git
    ◉  e61b6729ff4292870702f2f72b2a60165679ef37 master initial
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        git_repo.head().unwrap().target().unwrap().to_string(),
        @"b26951a9c6f5c270e4d039880208952fd5faae5e"
    );
}

#[test]
fn test_git_colocated_export_branches_on_snapshot() {
    // Checks that we export branches that were changed only because the working
    // copy was snapshotted

    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&workspace_root).unwrap();
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);

    // Create branch pointing to the initial commit
    std::fs::write(workspace_root.join("file"), "initial").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "foo"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  438471f3fbf1004298d8fb01eeb13663a051a643 foo
    ◉  0000000000000000000000000000000000000000
    "###);

    // The branch gets updated when we modify the working copy, and it should get
    // exported to Git without requiring any other changes
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  fab22d1acf5bb9c5aa48cb2c3dd2132072a359ca foo
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(git_repo
        .find_reference("refs/heads/foo")
        .unwrap()
        .target()
        .unwrap()
        .to_string(), @"fab22d1acf5bb9c5aa48cb2c3dd2132072a359ca");
}

#[test]
fn test_git_colocated_rebase_on_import() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&workspace_root).unwrap();
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);

    // Make some changes in jj and check that they're reflected in git
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["commit", "-m", "add a file"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["branch", "set", "master"]);
    test_env.jj_cmd_success(&workspace_root, &["commit", "-m", "modify a file"]);
    // TODO: We shouldn't need this command here to trigger an import of the
    // refs/heads/master we just exported
    test_env.jj_cmd_success(&workspace_root, &["st"]);

    // Move `master` and HEAD backwards, which should result in commit2 getting
    // hidden, and a new working-copy commit at the new position.
    let commit2_oid = git_repo
        .find_branch("master", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap();
    let commit2 = git_repo.find_commit(commit2_oid).unwrap();
    let commit1 = commit2.parents().next().unwrap();
    git_repo.branch("master", &commit1, true).unwrap();
    git_repo.set_head("refs/heads/master").unwrap();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  7f96185cfbe36341d0f9a86ebfaeab67a5922c7e
    ◉  4bcbeaba9a4b309c5f45a8807fbf5499b9714315 master HEAD@git add a file
    ◉  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_git_colocated_branches() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&workspace_root).unwrap();
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);
    test_env.jj_cmd_success(&workspace_root, &["new", "-m", "foo"]);
    test_env.jj_cmd_success(&workspace_root, &["new", "@-", "-m", "bar"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  3560559274ab431feea00b7b7e0b9250ecce951f bar
    │ ◉  1e6f0b403ed2ff9713b5d6b1dc601e4804250cda foo
    ├─╯
    ◉  230dd059e1b059aefc0da06a2e5a7dbf22362f22 master HEAD@git
    ◉  0000000000000000000000000000000000000000
    "###);

    // Create a branch in jj. It should be exported to Git even though it points to
    // the working- copy commit.
    test_env.jj_cmd_success(&workspace_root, &["branch", "set", "master"]);
    insta::assert_snapshot!(
        git_repo.find_reference("refs/heads/master").unwrap().target().unwrap().to_string(),
        @"3560559274ab431feea00b7b7e0b9250ecce951f"
    );
    insta::assert_snapshot!(
        git_repo.head().unwrap().target().unwrap().to_string(),
        @"230dd059e1b059aefc0da06a2e5a7dbf22362f22"
    );

    // Update the branch in Git
    let target_id = test_env.jj_cmd_success(
        &workspace_root,
        &["log", "--no-graph", "-T=commit_id", "-r=description(foo)"],
    );
    git_repo
        .reference(
            "refs/heads/master",
            Oid::from_str(&target_id).unwrap(),
            true,
            "test",
        )
        .unwrap();
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    Working copy now at: 096dc80da670 (no description set)
    Parent commit      : 230dd059e1b0 (no description set)
    @  096dc80da67094fbaa6683e2a205dddffa31f9a8
    │ ◉  1e6f0b403ed2ff9713b5d6b1dc601e4804250cda master foo
    ├─╯
    ◉  230dd059e1b059aefc0da06a2e5a7dbf22362f22 HEAD@git
    ◉  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_git_colocated_branch_forget() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let _git_repo = git2::Repository::init(&workspace_root).unwrap();
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);
    test_env.jj_cmd_success(&workspace_root, &["new"]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "set", "foo"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  65b6b74e08973b88d38404430f119c8c79465250 foo
    ◉  230dd059e1b059aefc0da06a2e5a7dbf22362f22 master HEAD@git
    ◉  0000000000000000000000000000000000000000
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    foo: 65b6b74e0897 (no description set)
    master: 230dd059e1b0 (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "forget", "foo"]);
    insta::assert_snapshot!(stdout, @"");
    // A forgotten branch is deleted in the git repo. For a detailed demo explaining
    // this, see `test_branch_forget_export` in `test_branch_command.rs`.
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    master: 230dd059e1b0 (no description set)
    "###);
}

#[test]
fn test_git_colocated_fail_if_git_ref_looks_like_subdir() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    git2::Repository::init(&workspace_root).unwrap();
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "main"]);
    let assert = test_env
        .jj_cmd(&workspace_root, &["branch", "create", "main/sub"])
        .assert()
        .success()
        .stdout("");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Failed to export some branches:
      main/sub
    Hint: Git doesn't allow a branch name that looks like a parent directory of
    another (e.g. `foo` and `foo/bar`). Try to rename the branches that failed to
    export or their "parent" branches.
    "###);
}

#[test]
fn test_git_colocated_fetch_deleted_or_moved_branch() {
    let test_env = TestEnvironment::default();
    let origin_path = test_env.env_root().join("origin");
    git2::Repository::init(&origin_path).unwrap();
    test_env.jj_cmd_success(&origin_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&origin_path, &["describe", "-m=A"]);
    test_env.jj_cmd_success(&origin_path, &["branch", "create", "A"]);
    test_env.jj_cmd_success(&origin_path, &["new", "-m=B_to_delete"]);
    test_env.jj_cmd_success(&origin_path, &["branch", "create", "B_to_delete"]);
    test_env.jj_cmd_success(&origin_path, &["new", "-m=original C", "@-"]);
    test_env.jj_cmd_success(&origin_path, &["branch", "create", "C_to_move"]);

    let clone_path = test_env.env_root().join("clone");
    git2::Repository::clone(origin_path.to_str().unwrap(), &clone_path).unwrap();
    test_env.jj_cmd_success(&clone_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&clone_path, &["new", "A"]);
    insta::assert_snapshot!(get_log_output(&test_env, &clone_path), @r###"
    @  0335878796213c3a701f1c9c34dcae242bee4131
    │ ◉  8d4e006fd63547965fbc3a26556a9aa531076d32 C_to_move original C
    ├─╯
    │ ◉  929e298ae9edf969b405a304c75c10457c47d52c B_to_delete B_to_delete
    ├─╯
    ◉  a86754f975f953fa25da4265764adc0c62e9ce6b A master HEAD@git A
    ◉  0000000000000000000000000000000000000000
    "###);

    test_env.jj_cmd_success(&origin_path, &["branch", "delete", "B_to_delete"]);
    // Move branch C sideways
    test_env.jj_cmd_success(&origin_path, &["describe", "C_to_move", "-m", "moved C"]);
    let stdout = test_env.jj_cmd_success(&clone_path, &["git", "fetch"]);
    insta::assert_snapshot!(stdout, @"");
    // "original C" and "B_to_delete" are abandoned, as the corresponding branches
    // were deleted or moved on the remote (#864)
    insta::assert_snapshot!(get_log_output(&test_env, &clone_path), @r###"
    ◉  04fd29df05638156b20044b3b6136b42abcb09ab C_to_move moved C
    │ @  0335878796213c3a701f1c9c34dcae242bee4131
    ├─╯
    ◉  a86754f975f953fa25da4265764adc0c62e9ce6b A master HEAD@git A
    ◉  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_git_colocated_external_checkout() {
    let test_env = TestEnvironment::default();
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&repo_path).unwrap();
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&repo_path, &["ci", "-m=A"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m=B", "root"]);
    test_env.jj_cmd_success(&repo_path, &["new"]);

    // Checked out anonymous branch
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  53637cd508ff02427dd78eca98f5b2450a6370ce
    ◉  66f4d1806ae41bd604f69155dece64062a0056cf HEAD@git B
    │ ◉  a86754f975f953fa25da4265764adc0c62e9ce6b master A
    ├─╯
    ◉  0000000000000000000000000000000000000000
    "###);

    // Check out another branch by external command
    git_repo
        .set_head_detached(
            git_repo
                .find_reference("refs/heads/master")
                .unwrap()
                .target()
                .unwrap(),
        )
        .unwrap();

    // The old working-copy commit gets abandoned, but the whole branch should not
    // be abandoned. (#1042)
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  0521ce3b8c4e29aab79f3c750e2845dcbc4c3874
    │ ◉  66f4d1806ae41bd604f69155dece64062a0056cf B
    ◉ │  a86754f975f953fa25da4265764adc0c62e9ce6b master HEAD@git A
    ├─╯
    ◉  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_git_colocated_squash_undo() {
    let test_env = TestEnvironment::default();
    let repo_path = test_env.env_root().join("repo");
    git2::Repository::init(&repo_path).unwrap();
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&repo_path, &["ci", "-m=A"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m=B"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "master"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  rlvkpnrzqnoo 2a3078eda7fe B master
    ◉  qpvuntsmwlqt a86754f975f9 A HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);

    test_env.jj_cmd_success(&repo_path, &["squash", "-m=A+B"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  royxmykxtrkr 83c0d8df2b78
    ◉  qpvuntsmwlqt 1873a0811bf5 A+B master HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    // There should be no divergence here; 2f376ea1478c should be hidden
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  rlvkpnrzqnoo 2a3078eda7fe B master
    ◉  qpvuntsmwlqt a86754f975f9 A HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);
}

// This test is just like the above test but with a couple of `jj git push`
// inserted
//
// TODO: Fix the BUG it reveals. The bug may be related to `jj git push` not
// calling `jj git export` properly, or it may be a bug in `jj git export.
#[test]
fn test_git_colocated_squash_push_undo() {
    let test_env = TestEnvironment::default();
    let source_path = test_env.env_root().join("source");
    git2::Repository::init_bare(&source_path).unwrap();
    let repo_path = test_env.env_root().join("repo");
    git2::Repository::clone(&source_path.as_os_str().to_string_lossy(), &repo_path).unwrap();
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&repo_path, &["ci", "-m=A"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m=B"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "master"]);
    test_env.jj_cmd_success(&repo_path, &["git", "push", "--all"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  rlvkpnrzqnoo 2a3078eda7fe B master
    ◉  qpvuntsmwlqt a86754f975f9 A HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);

    test_env.jj_cmd_success(&repo_path, &["squash", "-m=A+B"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  yqosqzytrlsw 91096ede1b94
    ◉  qpvuntsmwlqt 3ca5eff09136 A+B master* HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);
    test_env.jj_cmd_success(&repo_path, &["git", "push", "--all"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  yqosqzytrlsw 91096ede1b94
    ◉  qpvuntsmwlqt 3ca5eff09136 A+B master HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);
    let stdout = get_truncated_op_log(&test_env, &repo_path, 9);
    insta::assert_snapshot!(stdout, @r###"
    @  e3ba083e16da test-username@host.example.com 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00
    │  push all branches to git remote origin
    │  args: jj git push --all
    ◉  98b717fda32d test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  squash commit 2a3078eda7fe17eeb3cd3e390f7476dff078e35f
    │  args: jj squash '-m=A+B'
    ◉  05b9c30f160f test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  push all branches to git remote origin
    │  args: jj git push --all
    "###);
    // Restore to before the squash
    test_env.jj_cmd_success(&repo_path, &["op", "restore", "@--"]);
    // BUG: `master` didn't get back to B
    //
    // DO NOT SUBMIT TODO: Where did master@origin go?
    // Before, we had:
    /*
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    ◉  qpvuntsmwlqt 3ca5eff09136 A+B master !divergence!
    │ @  rlvkpnrzqnoo 2a3078eda7fe B
    │ ◉  qpvuntsmwlqt a86754f975f9 A !divergence!
    ├─╯
    ◉  zzzzzzzzzzzz 000000000000
    "###);
     */
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  rlvkpnrzqnoo 2a3078eda7fe B master*
    ◉  qpvuntsmwlqt a86754f975f9 A HEAD@git
    ◉  zzzzzzzzzzzz 000000000000
    "###);
}

// TODO: Fix the BUG this reveals.
#[test]
fn test_git_colocated_undo_branch_creation() {
    let test_env = TestEnvironment::default();
    let source_path = test_env.env_root().join("source");
    git2::Repository::init_bare(&source_path).unwrap();
    let repo_path = test_env.env_root().join("repo");
    git2::Repository::clone(&source_path.as_os_str().to_string_lossy(), &repo_path).unwrap();
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo=."]);
    test_env.jj_cmd_success(&repo_path, &["ci", "-m=0"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m=A"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "master"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m=B"]);

    // Test the setup
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  mzvwutvlkqwt bc15ec74b27a B
    ◉  rlvkpnrzqnoo 3495bd79af6e A master HEAD@git
    ◉  qpvuntsmwlqt a56846756248 0
    ◉  zzzzzzzzzzzz 000000000000
    "###);

    // Undo the branch creation
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "branch"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  mzvwutvlkqwt bc15ec74b27a B branch
    ◉  rlvkpnrzqnoo 3495bd79af6e A master HEAD@git
    ◉  qpvuntsmwlqt a56846756248 0
    ◉  zzzzzzzzzzzz 000000000000
    "###);
    test_env.jj_cmd_success(&repo_path, &["op", "undo"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  mzvwutvlkqwt bc15ec74b27a B
    ◉  rlvkpnrzqnoo 3495bd79af6e A master HEAD@git
    ◉  qpvuntsmwlqt a56846756248 0
    ◉  zzzzzzzzzzzz 000000000000
    "###);

    // Undo both branch creation and a push
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "branch"]);
    test_env.jj_cmd_success(&repo_path, &["git", "push", "--all"]);
    let stdout = get_truncated_op_log(&test_env, &repo_path, 9);
    insta::assert_snapshot!(stdout, @r###"
    @  9aeab0cb4e3f test-username@host.example.com 2001-02-03 04:05:18.000 +07:00 - 2001-02-03 04:05:18.000 +07:00
    │  push all branches to git remote origin
    │  args: jj git push --all
    ◉  0f50694025ec test-username@host.example.com 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    │  point branch branch to commit bc15ec74b27a6d49c2fed80fe9af240c5962ae47
    │  args: jj branch set branch
    ◉  30fcb20e1854 test-username@host.example.com 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00
    │  undo operation 021f54e02ac6e604d7790db9144e12e4328fa67ab649abfb96fc9f6df4fc3ef5a55aae9a8c7ece1e77318cde59fcab92197cd5aae3f8afb48983f3a5e869281a
    │  args: jj op undo
    "###);
    test_env.jj_cmd_success(&repo_path, &["op", "restore", "@--"]);
    // BUG: Branch shouldn't be there.
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  mzvwutvlkqwt bc15ec74b27a B branch@origin
    ◉  rlvkpnrzqnoo 3495bd79af6e A master HEAD@git
    ◉  qpvuntsmwlqt a56846756248 0
    ◉  zzzzzzzzzzzz 000000000000
    "###);

    // TODO: Delete this??
    // Undo both branch creation and a push (second time)
    test_env.jj_cmd_success(&repo_path, &["branch", "set", "branch"]);
    test_env.jj_cmd_success(&repo_path, &["git", "push", "--all"]);
    let stdout = get_truncated_op_log(&test_env, &repo_path, 18);
    insta::assert_snapshot!(stdout, @r###"
    @  d20e52d544e9 test-username@host.example.com 2001-02-03 04:05:22.000 +07:00 - 2001-02-03 04:05:22.000 +07:00
    │  point branch branch to commit bc15ec74b27a6d49c2fed80fe9af240c5962ae47
    │  args: jj branch set branch
    ◉  8e516ab64656 test-username@host.example.com 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00
    │  restore to operation 30fcb20e18545ce609925590a4672a0e565359a7229143c8e45b1ebe7b5868c15e049a5aff02b7ec3b5fcd52442ed93ab8688209cfa2575fd07dff555cc03ae6
    │  args: jj op restore @--
    ◉  9aeab0cb4e3f test-username@host.example.com 2001-02-03 04:05:18.000 +07:00 - 2001-02-03 04:05:18.000 +07:00
    │  push all branches to git remote origin
    │  args: jj git push --all
    ◉  0f50694025ec test-username@host.example.com 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    │  point branch branch to commit bc15ec74b27a6d49c2fed80fe9af240c5962ae47
    │  args: jj branch set branch
    ◉  30fcb20e1854 test-username@host.example.com 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00
    │  undo operation 021f54e02ac6e604d7790db9144e12e4328fa67ab649abfb96fc9f6df4fc3ef5a55aae9a8c7ece1e77318cde59fcab92197cd5aae3f8afb48983f3a5e869281a
    │  args: jj op undo
    ◉  021f54e02ac6 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  point branch branch to commit bc15ec74b27a6d49c2fed80fe9af240c5962ae47
    │  args: jj branch set branch
    "###);
    test_env.jj_cmd_success(&repo_path, &["op", "restore", "@--"]);
    insta::assert_snapshot!(get_log_output_divergence(&test_env, &repo_path), @r###"
    @  mzvwutvlkqwt bc15ec74b27a B branch
    ◉  rlvkpnrzqnoo 3495bd79af6e A master HEAD@git
    ◉  qpvuntsmwlqt a56846756248 0
    ◉  zzzzzzzzzzzz 000000000000
    "###);
}

fn get_truncated_op_log(test_env: &TestEnvironment, repo_path: &Path, lines: usize) -> String {
    let result = test_env.jj_cmd_success(repo_path, &["op", "log"]);
    result.lines().take(lines).join("\n")
}

fn get_log_output_divergence(test_env: &TestEnvironment, repo_path: &Path) -> String {
    let template = r###"
    separate(" ",
      change_id.short(),
      commit_id.short(),
      description.first_line(),
      branches,
      git_head,
      if(divergent, "!divergence!"),
    )
    "###;
    test_env.jj_cmd_success(repo_path, &["log", "-T", template, "-r=all()"])
}

fn get_log_output(test_env: &TestEnvironment, workspace_root: &Path) -> String {
    let template = r#"separate(" ", commit_id, branches, git_head, description)"#;
    test_env.jj_cmd_success(workspace_root, &["log", "-T", template, "-r=all()"])
}

#[test]
fn test_git_colocated_unreachable_commits() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&workspace_root).unwrap();

    // Create an initial commit in Git
    let empty_tree_oid = git_repo.treebuilder(None).unwrap().write().unwrap();
    let tree1 = git_repo.find_tree(empty_tree_oid).unwrap();
    let signature = git2::Signature::new(
        "Someone",
        "someone@example.com",
        &git2::Time::new(1234567890, 60),
    )
    .unwrap();
    let oid1 = git_repo
        .commit(
            Some("refs/heads/master"),
            &signature,
            &signature,
            "initial",
            &tree1,
            &[],
        )
        .unwrap();
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"2ee37513d2b5e549f7478c671a780053614bff19"
    );

    // Add a second commit in Git
    let tree2 = git_repo.find_tree(empty_tree_oid).unwrap();
    let signature = git2::Signature::new(
        "Someone",
        "someone@example.com",
        &git2::Time::new(1234567890, 62),
    )
    .unwrap();
    let oid2 = git_repo
        .commit(
            None,
            &signature,
            &signature,
            "next",
            &tree2,
            &[&git_repo.find_commit(oid1).unwrap()],
        )
        .unwrap();
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"2ee37513d2b5e549f7478c671a780053614bff19"
    );

    // Import the repo while there is no path to the second commit
    test_env.jj_cmd_success(&workspace_root, &["init", "--git-repo", "."]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  66ae47cee4f8c28ee8d7e4f5d9401b03c07e22f2
    ◉  2ee37513d2b5e549f7478c671a780053614bff19 master HEAD@git initial
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        git_repo.head().unwrap().peel_to_commit().unwrap().id().to_string(),
        @"2ee37513d2b5e549f7478c671a780053614bff19"
    );

    // Check that trying to look up the second commit fails gracefully
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["show", &oid2.to_string()]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Revision "8e713ff77b54928dd4a82aaabeca44b1ae91722c" doesn't exist
    "###);
}
