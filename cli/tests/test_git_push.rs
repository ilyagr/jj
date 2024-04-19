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

use std::path::{Path, PathBuf};

use crate::common::{get_stderr_string, get_stdout_string, TestEnvironment};

fn set_up() -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "origin"]);
    let origin_path = test_env.env_root().join("origin");
    let origin_git_repo_path = origin_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    test_env.jj_cmd_ok(&origin_path, &["describe", "-m=parent of branch1"]);
    test_env.jj_cmd_ok(&origin_path, &["new", "-m=description 1"]);
    test_env.jj_cmd_ok(&origin_path, &["branch", "create", "branch1"]);
    test_env.jj_cmd_ok(&origin_path, &["new", "root()", "-m=description 2"]);
    test_env.jj_cmd_ok(&origin_path, &["branch", "create", "branch2"]);
    test_env.jj_cmd_ok(&origin_path, &["git", "export"]);

    test_env.jj_cmd_ok(
        test_env.env_root(),
        &[
            "git",
            "clone",
            "--config-toml=git.auto-local-branch=true",
            origin_git_repo_path.to_str().unwrap(),
            "local",
        ],
    );
    let workspace_root = test_env.env_root().join("local");
    (test_env, workspace_root)
}

#[test]
fn test_git_push_nothing() {
    let (test_env, workspace_root) = set_up();
    // Show the setup. `insta` has trouble if this is done inside `set_up()`
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: urkzutzp 3571d60e (empty) description 1
      @origin: urkzutzp 3571d60e (empty) description 1
    branch2: zkxzmtrq 132be02d (empty) description 2
      @origin: zkxzmtrq 132be02d (empty) description 2
    "###);
    // No branches to push yet
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_current_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Update some branches. `branch1` is not a current branch, but `branch2` and
    // `my-branch` are.
    test_env.jj_cmd_ok(
        &workspace_root,
        &["describe", "branch1", "-m", "modified branch1 commit"],
    );
    test_env.jj_cmd_ok(&workspace_root, &["new", "branch2"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "set", "branch2"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    // Check the setup
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: urkzutzp 0d0adf03 (empty) modified branch1 commit
      @origin (ahead by 1 commits, behind by 1 commits): urkzutzp hidden 3571d60e (empty) description 1
    branch2: znkkpsqq f232e174 (empty) foo
      @origin (behind by 1 commits): zkxzmtrq 132be02d (empty) description 2
    my-branch: znkkpsqq f232e174 (empty) foo
    "###);
    // First dry-run. `branch1` should not get pushed.
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--dry-run"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Move branch branch2 from 132be02d5c96 to f232e174c898
      Add branch my-branch to f232e174c898
    Dry-run requested, not pushing.
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Move branch branch2 from 132be02d5c96 to f232e174c898
      Add branch my-branch to f232e174c898
    "###);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: urkzutzp 0d0adf03 (empty) modified branch1 commit
      @origin (ahead by 1 commits, behind by 1 commits): urkzutzp hidden 3571d60e (empty) description 1
    branch2: znkkpsqq f232e174 (empty) foo
      @origin: znkkpsqq f232e174 (empty) foo
    my-branch: znkkpsqq f232e174 (empty) foo
      @origin: znkkpsqq f232e174 (empty) foo
    "###);
}

#[test]
fn test_git_push_parent_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    test_env.jj_cmd_ok(&workspace_root, &["edit", "branch1"]);
    test_env.jj_cmd_ok(
        &workspace_root,
        &["describe", "-m", "modified branch1 commit"],
    );
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "non-empty description"]);
    std::fs::write(workspace_root.join("file"), "file").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch branch1 from 3571d60e8503 to bb58d3256266
    "###);
}

#[test]
fn test_git_push_no_matching_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches found in the default push revset: remote_branches(remote=origin)..@
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_matching_branch_unchanged() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["new", "branch1"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches found in the default push revset: remote_branches(remote=origin)..@
    Nothing changed.
    "###);
}

/// Test that `jj git push` without arguments pushes a branch to the specified
/// remote even if it's already up to date on another remote
/// (`remote_branches(remote=<remote>)..@` vs. `remote_branches()..@`).
#[test]
fn test_git_push_other_remote_has_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Create another remote (but actually the same)
    let other_remote_path = test_env
        .env_root()
        .join("origin")
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");
    test_env.jj_cmd_ok(
        &workspace_root,
        &[
            "git",
            "remote",
            "add",
            "other",
            other_remote_path.to_str().unwrap(),
        ],
    );
    // Modify branch1 and push it to `origin`
    test_env.jj_cmd_ok(&workspace_root, &["edit", "branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m=modified"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch branch1 from 3571d60e8503 to d73041288714
    "###);
    // Since it's already pushed to origin, nothing will happen if push again
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches found in the default push revset: remote_branches(remote=origin)..@
    Nothing changed.
    "###);
    // But it will still get pushed to another remote
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--remote=other"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to other:
      Add branch branch1 to d73041288714
    "###);
}

#[test]
fn test_git_push_not_fast_forward() {
    let (test_env, workspace_root) = set_up();

    // Move branch1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env.jj_cmd_ok(&origin_path, &["new", "branch1", "-m=remote"]);
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env.jj_cmd_ok(&origin_path, &["branch", "set", "branch1"]);
    test_env.jj_cmd_ok(&origin_path, &["git", "export"]);

    // Move branch1 forward to another commit locally
    test_env.jj_cmd_ok(&workspace_root, &["new", "branch1", "-m=local"]);
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["branch", "set", "branch1"]);

    // Pushing should fail
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Move branch branch1 from 3571d60e8503 to c5ab2d309152
    Error: The push conflicts with changes made on the remote (it is not fast-forwardable).
    Hint: Try fetching from the remote, then make the branch point to where you want it to be, and push again.
    "###);
}

// Short-term TODO: implement this.
// This tests whether the push checks that the remote branches are in expected
// positions. Once this is implemented, `jj git push` will be similar to `git
// push --force-with-lease`
#[test]
fn test_git_push_sideways_unexpectedly_moved() {
    let (test_env, workspace_root) = set_up();

    // Move branch1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env.jj_cmd_ok(&origin_path, &["new", "branch1", "-m=remote"]);
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env.jj_cmd_ok(&origin_path, &["branch", "set", "branch1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &origin_path), @r###"
    branch1: yostqsxw 6cb7e429 remote
      @git (behind by 1 commits): kkmpptxz 3571d60e (empty) description 1
    branch2: mzvwutvl 132be02d (empty) description 2
      @git: mzvwutvl 132be02d (empty) description 2
    "###);
    test_env.jj_cmd_ok(&origin_path, &["git", "export"]);

    // Move branch1 sideways to another commit locally
    test_env.jj_cmd_ok(&workspace_root, &["new", "root()", "-m=local"]);
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env.jj_cmd_ok(
        &workspace_root,
        &["branch", "set", "branch1", "--allow-backwards"],
    );
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: wqnwkozp eb921361 local
      @origin (ahead by 2 commits, behind by 1 commits): urkzutzp 3571d60e (empty) description 1
    branch2: zkxzmtrq 132be02d (empty) description 2
      @origin: zkxzmtrq 132be02d (empty) description 2
    "###);

    // BUG: Pushing should fail. Currently, it succeeds because moving the branch
    // sideways causes `jj` to use the analogue of `git push --force` when pushing.
    let assert = test_env
        .jj_cmd(&workspace_root, &["git", "push"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Branch changes to push to origin:
      Force branch branch1 from 3571d60e8503 to eb921361206c
    "###);
}

// Short-term TODO: implement this.
// This tests whether the push checks that the remote branches are in expected
// positions. Once this is implemented, `jj git push` will be similar to `git
// push --force-with-lease`
#[test]
fn test_git_push_deletion_unexpectedly_moved() {
    let (test_env, workspace_root) = set_up();

    // Move branch1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env.jj_cmd_ok(&origin_path, &["new", "branch1", "-m=remote"]);
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env.jj_cmd_ok(&origin_path, &["branch", "set", "branch1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &origin_path), @r###"
    branch1: yostqsxw 6cb7e429 remote
      @git (behind by 1 commits): kkmpptxz 3571d60e (empty) description 1
    branch2: mzvwutvl 132be02d (empty) description 2
      @git: mzvwutvl 132be02d (empty) description 2
    "###);
    test_env.jj_cmd_ok(&origin_path, &["git", "export"]);

    // Delete branch1 locally
    test_env.jj_cmd_ok(&workspace_root, &["branch", "delete", "branch1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1 (deleted)
      @origin: urkzutzp 3571d60e (empty) description 1
    branch2: zkxzmtrq 132be02d (empty) description 2
      @origin: zkxzmtrq 132be02d (empty) description 2
    "###);

    // BUG: Pushing should fail because the branch was moved on the remote
    let assert = test_env
        .jj_cmd(&workspace_root, &["git", "push", "--branch", "branch1"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
    "###);
}

#[test]
fn test_git_push_creation_unexpectedly_already_exists() {
    let (test_env, workspace_root) = set_up();

    // Forget branch1 locally
    test_env.jj_cmd_ok(&workspace_root, &["branch", "forget", "branch1"]);

    // Create a new branh1
    test_env.jj_cmd_ok(&workspace_root, &["new", "root()", "-m=new branch1"]);
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: znkkpsqq 95344a1a new branch1
    branch2: zkxzmtrq 132be02d (empty) description 2
      @origin: zkxzmtrq 132be02d (empty) description 2
    "###);

    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Add branch branch1 to 95344a1ab28a
    Error: The push conflicts with changes made on the remote (it is not fast-forwardable).
    Hint: Try fetching from the remote, then make the branch point to where you want it to be, and push again.
    "###);
}

#[test]
fn test_git_push_locally_created_and_rewritten() {
    let (test_env, workspace_root) = set_up();
    // Ensure that remote branches aren't tracked automatically
    test_env.add_config("git.auto-local-branch = false");

    // Push locally-created branch
    test_env.jj_cmd_ok(&workspace_root, &["new", "root()", "-mlocal 1"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "my"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Add branch my to a80733f071d5
    "###);

    // Rewrite it and push again, which would fail if the pushed branch weren't
    // set to "tracking"
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-mlocal 2"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: urkzutzp 3571d60e (empty) description 1
      @origin: urkzutzp 3571d60e (empty) description 1
    branch2: zkxzmtrq 132be02d (empty) description 2
      @origin: zkxzmtrq 132be02d (empty) description 2
    my: yostqsxw 8d761ebf (empty) local 2
      @origin (ahead by 1 commits, behind by 1 commits): yostqsxw hidden a80733f0 (empty) local 1
    "###);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch my from a80733f071d5 to 8d761ebf8825
    "###);
}

#[test]
fn test_git_push_multiple() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["branch", "delete", "branch1"]);
    test_env.jj_cmd_ok(
        &workspace_root,
        &["branch", "set", "--allow-backwards", "branch2"],
    );
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    // Check the setup
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1 (deleted)
      @origin: urkzutzp 3571d60e (empty) description 1
    branch2: vruxwmqv 2f33cb09 (empty) foo
      @origin (ahead by 1 commits, behind by 1 commits): zkxzmtrq 132be02d (empty) description 2
    my-branch: vruxwmqv 2f33cb09 (empty) foo
    "###);
    // First dry-run
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--all", "--dry-run"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
      Force branch branch2 from 132be02d5c96 to 2f33cb099c9e
      Add branch my-branch to 2f33cb099c9e
    Dry-run requested, not pushing.
    "###);
    // Dry run requesting two specific branches
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-b=branch1", "-b=my-branch", "--dry-run"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
      Add branch my-branch to 2f33cb099c9e
    Dry-run requested, not pushing.
    "###);
    // Dry run requesting two specific branches twice
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &[
            "git",
            "push",
            "-b=branch1",
            "-b=my-branch",
            "-b=branch1",
            "-b=glob:my-*",
            "--dry-run",
        ],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
      Add branch my-branch to 2f33cb099c9e
    Dry-run requested, not pushing.
    "###);
    // Dry run with glob pattern
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-b=glob:branch?", "--dry-run"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
      Force branch branch2 from 132be02d5c96 to 2f33cb099c9e
    Dry-run requested, not pushing.
    "###);

    // Unmatched branch name is error
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push", "-b=foo"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: No such branch: foo
    "###);
    let stderr = test_env.jj_cmd_failure(
        &workspace_root,
        &["git", "push", "-b=foo", "-b=glob:?branch"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: No matching branches for patterns: foo, ?branch
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
      Force branch branch2 from 132be02d5c96 to 2f33cb099c9e
      Add branch my-branch to 2f33cb099c9e
    "###);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch2: vruxwmqv 2f33cb09 (empty) foo
      @origin: vruxwmqv 2f33cb09 (empty) foo
    my-branch: vruxwmqv 2f33cb09 (empty) foo
      @origin: vruxwmqv 2f33cb09 (empty) foo
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-rall()"]);
    insta::assert_snapshot!(stdout, @r###"
    @  vruxwmqv test.user@example.com 2001-02-03 08:05:18 branch2 my-branch 2f33cb09
    │  (empty) foo
    │ ◉  zkxzmtrq test.user@example.com 2001-02-03 08:05:11 132be02d
    ├─╯  (empty) description 2
    │ ◉  urkzutzp test.user@example.com 2001-02-03 08:05:09 3571d60e
    │ │  (empty) description 1
    │ ◉  txzknzvm test.user@example.com 2001-02-03 08:05:08 8144f454
    ├─╯  (empty) parent of branch1
    ◉  zzzzzzzz root() 00000000
    "###);
}

#[test]
fn test_git_push_changes() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "bar"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--change", "@"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Creating branch push-znkkpsqqskkl for revision @
    Branch changes to push to origin:
      Add branch push-znkkpsqqskkl to 4af9f81e5232
    "###);
    // test pushing two changes at once
    std::fs::write(workspace_root.join("file"), "modified2").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-c=@", "-c=@-"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Creating branch push-vruxwmqvtpmx for revision @-
    Branch changes to push to origin:
      Force branch push-znkkpsqqskkl from 4af9f81e5232 to 1733fc32be88
      Add branch push-vruxwmqvtpmx to 33960bf6cfbe
    "###);
    // specifying the same change twice doesn't break things
    std::fs::write(workspace_root.join("file"), "modified3").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-c=@", "-c=@"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch push-znkkpsqqskkl from 1733fc32be88 to 1f5df7bfcb14
    "###);

    // specifying the same branch with --change/--branch doesn't break things
    std::fs::write(workspace_root.join("file"), "modified4").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-c=@", "-b=push-znkkpsqqskkl"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch push-znkkpsqqskkl from 1f5df7bfcb14 to ba7902338902
    "###);

    // try again with --change that moves the branch forward
    std::fs::write(workspace_root.join("file"), "modified5").unwrap();
    test_env.jj_cmd_ok(
        &workspace_root,
        &[
            "branch",
            "set",
            "-r=@-",
            "--allow-backwards",
            "push-znkkpsqqskkl",
        ],
    );
    let stdout = test_env.jj_cmd_success(&workspace_root, &["status"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy changes:
    M file
    Working copy : znkkpsqq b4e5678f bar
    Parent commit: vruxwmqv 33960bf6 push-vruxwmqvtpmx push-znkkpsqqskkl* | foo
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-c=@", "-b=push-znkkpsqqskkl"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Force branch push-znkkpsqqskkl from ba7902338902 to b4e5678f048e
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["status"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy changes:
    M file
    Working copy : znkkpsqq b4e5678f push-znkkpsqqskkl | bar
    Parent commit: vruxwmqv 33960bf6 push-vruxwmqvtpmx | foo
    "###);

    // Test changing `git.push-branch-prefix`. It causes us to push again.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &[
            "git",
            "push",
            "--config-toml",
            r"git.push-branch-prefix='test-'",
            "--change=@",
        ],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Creating branch test-znkkpsqqskkl for revision @
    Branch changes to push to origin:
      Add branch test-znkkpsqqskkl to b4e5678f048e
    "###);
}

#[test]
fn test_git_push_revisions() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "bar"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-1"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "baz"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-2a"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-2b"]);
    std::fs::write(workspace_root.join("file"), "modified again").unwrap();

    // Push an empty set
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-r=none()"]);
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches point to the specified revisions: none()
    Nothing changed.
    "###);
    // Push a revision with no branches
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-r=@--"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches point to the specified revisions: @--
    Nothing changed.
    "###);
    // Push a revision with a single branch
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-r=@-", "--dry-run"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Add branch branch-1 to c17c47217746
    Dry-run requested, not pushing.
    "###);
    // Push multiple revisions of which some have branches
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-r=@--", "-r=@-", "--dry-run"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: No branches point to the specified revisions: @--
    Branch changes to push to origin:
      Add branch branch-1 to c17c47217746
    Dry-run requested, not pushing.
    "###);
    // Push a revision with a multiple branches
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-r=@", "--dry-run"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Add branch branch-2a to 8f42a5402b93
      Add branch branch-2b to 8f42a5402b93
    Dry-run requested, not pushing.
    "###);
    // Repeating a commit doesn't result in repeated messages about the branch
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "-r=@-", "-r=@-", "--dry-run"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Add branch branch-1 to c17c47217746
    Dry-run requested, not pushing.
    "###);
}

#[test]
fn test_git_push_mixed() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "bar"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-1"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["new", "-m", "baz"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-2a"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch-2b"]);
    std::fs::write(workspace_root.join("file"), "modified again").unwrap();

    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "push", "--change=@--", "--branch=branch-1", "-r=@"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Creating branch push-vruxwmqvtpmx for revision @--
    Branch changes to push to origin:
      Add branch push-vruxwmqvtpmx to 33960bf6cfbe
      Add branch branch-1 to c17c47217746
      Add branch branch-2a to 8f42a5402b93
      Add branch branch-2b to 8f42a5402b93
    "###);
}

#[test]
fn test_git_push_existing_long_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(
        &workspace_root,
        &["branch", "create", "push-19b790168e73f7a73a98deae21e807c0"],
    );

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--change=@"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Creating branch push-vruxwmqvtpmx for revision @
    Branch changes to push to origin:
      Add branch push-vruxwmqvtpmx to 33960bf6cfbe
    "###);
}

#[test]
fn test_git_push_unsnapshotted_change() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--change", "@"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--change", "@"]);
}

#[test]
fn test_git_push_conflict() {
    let (test_env, workspace_root) = set_up();
    std::fs::write(workspace_root.join("file"), "first").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["commit", "-m", "first"]);
    std::fs::write(workspace_root.join("file"), "second").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["commit", "-m", "second"]);
    std::fs::write(workspace_root.join("file"), "third").unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["rebase", "-r", "@", "-d", "@--"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m", "third"]);
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 5e3f22ef8f5c since it has conflicts
    "###);
}

#[test]
fn test_git_push_no_description() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_ok(&workspace_root, &["describe", "-m="]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch", "my-branch"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 41658cf47e0d since it has no description
    "###);
}

#[test]
fn test_git_push_missing_author() {
    let (test_env, workspace_root) = set_up();
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .jj_cmd(&workspace_root, args)
            .env_remove(var)
            .assert()
            .success();
    };
    run_without_var("JJ_USER", &["checkout", "root()", "-m=initial"]);
    run_without_var("JJ_USER", &["branch", "create", "missing-name"]);
    let stderr = test_env.jj_cmd_failure(
        &workspace_root,
        &["git", "push", "--branch", "missing-name"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 31e9ba3415e0 since it has no author and/or committer set
    "###);
    run_without_var("JJ_EMAIL", &["checkout", "root()", "-m=initial"]);
    run_without_var("JJ_EMAIL", &["branch", "create", "missing-email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 7a4648e709e8 since it has no author and/or committer set
    "###);
}

#[test]
fn test_git_push_missing_committer() {
    let (test_env, workspace_root) = set_up();
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .jj_cmd(&workspace_root, args)
            .env_remove(var)
            .assert()
            .success();
    };
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "missing-name"]);
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-name"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 44d97da33876 since it has no author and/or committer set
    "###);
    test_env.jj_cmd_ok(&workspace_root, &["checkout", "root()"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "missing-email"]);
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 4e1ac554cc60 since it has no author and/or committer set
    "###);

    // Test message when there are multiple reasons (missing committer and
    // description)
    run_without_var("JJ_EMAIL", &["describe", "-m=", "missing-email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit c77aa5d252f7 since it has no description and it has no author and/or committer set
    "###);
}

#[test]
fn test_git_push_deleted() {
    let (test_env, workspace_root) = set_up();

    test_env.jj_cmd_ok(&workspace_root, &["branch", "delete", "branch1"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--deleted"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 3571d60e8503
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-rall()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◉  zkxzmtrq test.user@example.com 2001-02-03 08:05:11 branch2 132be02d
    │  (empty) description 2
    │ ◉  urkzutzp test.user@example.com 2001-02-03 08:05:09 3571d60e
    │ │  (empty) description 1
    │ ◉  txzknzvm test.user@example.com 2001-02-03 08:05:08 8144f454
    ├─╯  (empty) parent of branch1
    │ @  vruxwmqv test.user@example.com 2001-02-03 08:05:14 41658cf4
    ├─╯  (empty) (no description set)
    ◉  zzzzzzzz root() 00000000
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--deleted"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_conflicting_branches() {
    let (test_env, workspace_root) = set_up();
    test_env.add_config("git.auto-local-branch = true");
    let git_repo = {
        let mut git_repo_path = workspace_root.clone();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git2::Repository::open(&git_repo_path).unwrap()
    };

    // Forget remote ref, move local ref, then fetch to create conflict.
    git_repo
        .find_reference("refs/remotes/origin/branch2")
        .unwrap()
        .delete()
        .unwrap();
    test_env.jj_cmd_ok(&workspace_root, &["git", "import"]);
    test_env.jj_cmd_ok(&workspace_root, &["new", "root()", "-m=description 3"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch2"]);
    test_env.jj_cmd_ok(&workspace_root, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: urkzutzp 3571d60e (empty) description 1
      @origin: urkzutzp 3571d60e (empty) description 1
    branch2 (conflicted):
      + znkkpsqq 4e72fa37 (empty) description 3
      + zkxzmtrq 132be02d (empty) description 2
      @origin (behind by 1 commits): zkxzmtrq 132be02d (empty) description 2
    "###);

    let bump_branch1 = || {
        test_env.jj_cmd_ok(&workspace_root, &["new", "branch1", "-m=bump"]);
        test_env.jj_cmd_ok(&workspace_root, &["branch", "set", "branch1"]);
    };

    // Conflicting branch at @
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: Branch branch2 is conflicted
    Hint: Run `jj branch list` to inspect, and use `jj branch set` to fix it up.
    Nothing changed.
    "###);

    // --branch should be blocked by conflicting branch
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch", "branch2"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Branch branch2 is conflicted
    Hint: Run `jj branch list` to inspect, and use `jj branch set` to fix it up.
    "###);

    // --all shouldn't be blocked by conflicting branch
    bump_branch1();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: Branch branch2 is conflicted
    Hint: Run `jj branch list` to inspect, and use `jj branch set` to fix it up.
    Branch changes to push to origin:
      Move branch branch1 from 3571d60e8503 to edc3d360d4db
    "###);

    // --revisions shouldn't be blocked by conflicting branch
    bump_branch1();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "-rall()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Warning: Branch branch2 is conflicted
    Hint: Run `jj branch list` to inspect, and use `jj branch set` to fix it up.
    Branch changes to push to origin:
      Move branch branch1 from edc3d360d4db to b0f8d00b6aed
    "###);
}

#[test]
fn test_git_push_deleted_untracked() {
    let (test_env, workspace_root) = set_up();

    // Absent local branch shouldn't be considered "deleted" compared to
    // non-tracking remote branch.
    test_env.jj_cmd_ok(&workspace_root, &["branch", "delete", "branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "untrack", "branch1@origin"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--deleted"]);
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=branch1"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: No such branch: branch1
    "###);
}

#[test]
fn test_git_push_tracked_vs_all() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_ok(&workspace_root, &["new", "branch1", "-mmoved branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "set", "branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["new", "branch2", "-mmoved branch2"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "delete", "branch2"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "untrack", "branch1@origin"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "create", "branch3"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: yostqsxw 11c7a846 (empty) moved branch1
    branch1@origin: urkzutzp 3571d60e (empty) description 1
    branch2 (deleted)
      @origin: zkxzmtrq 132be02d (empty) description 2
    branch3: kpqxywon 0fa920c6 (empty) moved branch2
    "###);

    // At this point, only branch2 is still tracked. `jj git push --tracked` would
    // try to push it and no other branches.
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--tracked", "--dry-run"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to origin:
      Delete branch branch2 from 132be02d5c96
    Dry-run requested, not pushing.
    "###);

    // Untrack the last remaining tracked branch.
    test_env.jj_cmd_ok(&workspace_root, &["branch", "untrack", "branch2@origin"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &workspace_root), @r###"
    branch1: yostqsxw 11c7a846 (empty) moved branch1
    branch1@origin: urkzutzp 3571d60e (empty) description 1
    branch2@origin: zkxzmtrq 132be02d (empty) description 2
    branch3: kpqxywon 0fa920c6 (empty) moved branch2
    "###);

    // Now, no branches are tracked. --tracked does not push anything
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--tracked"]);
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);

    // All branches are still untracked.
    // - --all tries to push branch1, but fails because a branch with the same
    // name exist on the remote.
    // - --all succeeds in pushing branch3, since there is no branch of the same
    // name on the remote.
    // - It does not try to push branch2.
    //
    // TODO: Not trying to push branch2 could be considered correct, or perhaps
    // we want to consider this as a deletion of the branch that failed because
    // the branch was untracked. In the latter case, an error message should be
    // printed. Some considerations:
    // - Whatever we do should be consistent with what `jj branch list` does; it
    //   currently does *not* list branches like branch2 as "about to be deleted",
    //   as can be seen above.
    // - We could consider showing some hint on `jj branch untrack branch2@origin`
    //   instead of showing an error here.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stderr, @r###"
    Warning: Non-tracking remote branch branch1@origin exists
    Hint: Run `jj branch track branch1@origin` to import the remote branch.
    Branch changes to push to origin:
      Add branch branch3 to 0fa920c63bd5
    "###);
}

#[test]
fn test_git_push_moved_forward_untracked() {
    let (test_env, workspace_root) = set_up();

    test_env.jj_cmd_ok(&workspace_root, &["new", "branch1", "-mmoved branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "set", "branch1"]);
    test_env.jj_cmd_ok(&workspace_root, &["branch", "untrack", "branch1@origin"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Warning: Non-tracking remote branch branch1@origin exists
    Hint: Run `jj branch track branch1@origin` to import the remote branch.
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_moved_sideways_untracked() {
    let (test_env, workspace_root) = set_up();

    test_env.jj_cmd_ok(&workspace_root, &["new", "root()", "-mmoved branch1"]);
    test_env.jj_cmd_ok(
        &workspace_root,
        &["branch", "set", "--allow-backwards", "branch1"],
    );
    test_env.jj_cmd_ok(&workspace_root, &["branch", "untrack", "branch1@origin"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Warning: Non-tracking remote branch branch1@origin exists
    Hint: Run `jj branch track branch1@origin` to import the remote branch.
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_to_remote_named_git() {
    let (test_env, workspace_root) = set_up();
    let git_repo = {
        let mut git_repo_path = workspace_root.clone();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git2::Repository::open(&git_repo_path).unwrap()
    };
    git_repo.remote_rename("origin", "git").unwrap();

    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--all", "--remote=git"]);
    insta::assert_snapshot!(stderr, @r###"
    Branch changes to push to git:
      Add branch branch1 to 3571d60e8503
      Add branch branch2 to 132be02d5c96
    Error: Git remote named 'git' is reserved for local Git repository
    "###);
}

fn get_branch_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    // --quiet to suppress deleted branches hint
    test_env.jj_cmd_success(repo_path, &["branch", "list", "--all-remotes", "--quiet"])
}
