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

use crate::common::TestEnvironment;

pub mod common;

fn create_commit(test_env: &TestEnvironment, repo_path: &Path, name: &str, parents: &[&str]) {
    if parents.is_empty() {
        test_env.jj_cmd_ok(repo_path, &["new", "root()", "-m", name]);
    } else {
        let mut args = vec!["new", "-m", name];
        args.extend(parents);
        test_env.jj_cmd_ok(repo_path, &args);
    }
    std::fs::write(repo_path.join(name), format!("{name}\n")).unwrap();
    test_env.jj_cmd_ok(repo_path, &["branch", "create", name]);
}

#[test]
fn test_rebase_invalid() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &["a"]);

    // Missing destination
    let stderr = test_env.jj_cmd_cli_error(&repo_path, &["rebase"]);
    insta::assert_snapshot!(stderr, @r###"
    error: the following required arguments were not provided:
      --destination <DESTINATION>

    Usage: jj rebase --destination <DESTINATION>

    For more information, try '--help'.
    "###);

    // Both -r and -s
    let stderr =
        test_env.jj_cmd_cli_error(&repo_path, &["rebase", "-r", "a", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(stderr, @r###"
    error: the argument '--revision <REVISION>' cannot be used with '--source <SOURCE>'

    Usage: jj rebase --destination <DESTINATION> --revision <REVISION>

    For more information, try '--help'.
    "###);

    // Both -b and -s
    let stderr =
        test_env.jj_cmd_cli_error(&repo_path, &["rebase", "-b", "a", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(stderr, @r###"
    error: the argument '--branch <BRANCH>' cannot be used with '--source <SOURCE>'

    Usage: jj rebase --destination <DESTINATION> --branch <BRANCH>

    For more information, try '--help'.
    "###);

    // Both -r and --skip-empty
    let stderr = test_env.jj_cmd_cli_error(
        &repo_path,
        &["rebase", "-r", "a", "-d", "b", "--skip-empty"],
    );
    insta::assert_snapshot!(stderr, @r###"
    error: the argument '--revision <REVISION>' cannot be used with '--skip-empty'

    Usage: jj rebase --destination <DESTINATION> --revision <REVISION>

    For more information, try '--help'.
    "###);

    // Rebase onto self with -r
    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-r", "a", "-d", "a"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot rebase 2443ea76b0b1 onto itself
    "###);

    // Rebase root with -r
    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-r", "root()", "-d", "a"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: The root commit 000000000000 is immutable
    "###);

    // Rebase onto descendant with -s
    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot rebase 2443ea76b0b1 onto descendant 1394f625cbbd
    "###);
}

#[test]
fn test_rebase_branch() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &["a"]);
    create_commit(&test_env, &repo_path, "c", &["b"]);
    create_commit(&test_env, &repo_path, "d", &["b"]);
    create_commit(&test_env, &repo_path, "e", &["a"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  e
    │ ◉  d
    │ │ ◉  c
    │ ├─╯
    │ ◉  b
    ├─╯
    ◉  a
    ◉
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b", "c", "-d", "e"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  d
    │ ◉  c
    ├─╯
    ◉  b
    @  e
    ◉  a
    ◉
    "###);

    // Test rebasing multiple branches at once
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b=e", "-b=d", "-d=b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 2 commits
    Working copy now at: znkkpsqq 9ca2a154 e | e
    Parent commit      : zsuskuln 1394f625 b | b
    Added 1 files, modified 0 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  d
    │ @  e
    ├─╯
    │ ◉  c
    ├─╯
    ◉  b
    ◉  a
    ◉
    "###);

    // Same test but with more than one revision per argument
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-b=e|d", "-d=b"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Revset "e|d" resolved to more than one revision
    Hint: The revset "e|d" resolved to these revisions:
    znkkpsqq e52756c8 e | e
    vruxwmqv 514fa6b2 d | d
    Prefix the expression with 'all' to allow any number of revisions (i.e. 'all:e|d').
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b=all:e|d", "-d=b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 2 commits
    Working copy now at: znkkpsqq 817e3fb0 e | e
    Parent commit      : zsuskuln 1394f625 b | b
    Added 1 files, modified 0 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  d
    │ @  e
    ├─╯
    │ ◉  c
    ├─╯
    ◉  b
    ◉  a
    ◉
    "###);
}

#[test]
fn test_rebase_branch_with_merge() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &["a"]);
    create_commit(&test_env, &repo_path, "c", &[]);
    create_commit(&test_env, &repo_path, "d", &["c"]);
    create_commit(&test_env, &repo_path, "e", &["a", "d"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    e
    ├─╮
    │ ◉  d
    │ ◉  c
    │ │ ◉  b
    ├───╯
    ◉ │  a
    ├─╯
    ◉
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b", "d", "-d", "b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: znkkpsqq 391c91a7 e | e
    Parent commit      : vruxwmqv 1677f795 d | d
    Added 1 files, modified 0 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  e
    ◉  d
    ◉  c
    ◉  b
    ◉  a
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-d", "b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: znkkpsqq 040ae3a6 e | e
    Parent commit      : vruxwmqv 3d0f3644 d | d
    Added 1 files, modified 0 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  e
    ◉  d
    ◉  c
    ◉  b
    ◉  a
    ◉
    "###);
}

#[test]
fn test_rebase_single_revision() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &[]);
    create_commit(&test_env, &repo_path, "c", &["a", "b"]);
    create_commit(&test_env, &repo_path, "d", &["c"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  d
    ◉    c
    ├─╮
    │ ◉  b
    ◉ │  a
    ├─╯
    ◉
    "###);

    /* PROBLEM HERE
    // Descendants of the rebased commit "b" should be rebased onto parents. First
    // we test with a non-merge commit. Normally, the descendant "c" would still
    // have 2 parents afterwards: the parent of "b" -- the root commit -- and
    // "a". However, since the root commit is an ancestor of "a", we don't
    // actually want both to be parents of the same commit. So, only "a" becomes
    // a parent.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "b", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 2 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv 7e15b97a d | d
    Parent commit      : royxmykx 934236c8 c | c
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  b
    │ @  d
    │ ◉  c
    ├─╯
    ◉  a
    ◉
    "###);
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    */

    // Now, let's try moving the merge commit. After, both parents of "c" ("a" and
    // "b") should become parents of "d".
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "c", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 1 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv a37531e8 d | d
    Parent commit      : rlvkpnrz 2443ea76 a | a
    Parent commit      : zsuskuln d370aee1 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  c
    │ @    d
    │ ├─╮
    │ │ ◉  b
    ├───╯
    │ ◉  a
    ├─╯
    ◉
    "###);
}

#[test]
fn test_rebase_single_revision_merge_parent() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &[]);
    create_commit(&test_env, &repo_path, "c", &["b"]);
    create_commit(&test_env, &repo_path, "d", &["a", "c"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    d
    ├─╮
    │ ◉  c
    │ ◉  b
    ◉ │  a
    ├─╯
    ◉
    "###);

    // Descendants of the rebased commit should be rebased onto parents, and if
    // the descendant is a merge commit, it shouldn't forget its other parents.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "c", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 1 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv a37531e8 d | d
    Parent commit      : rlvkpnrz 2443ea76 a | a
    Parent commit      : zsuskuln d370aee1 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  c
    │ @  d
    ╭─┤
    │ ◉  b
    ◉ │  a
    ├─╯
    ◉
    "###);
}

#[test]
fn test_rebase_revision_onto_descendant() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[]);
    create_commit(&test_env, &repo_path, "a", &["base"]);
    create_commit(&test_env, &repo_path, "b", &["base"]);
    create_commit(&test_env, &repo_path, "merge", &["b", "a"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    merge
    ├─╮
    │ ◉  a
    ◉ │  b
    ├─╯
    ◉  base
    ◉
    "###);
    let setup_opid = test_env.current_operation_id(&repo_path);

    // Simpler example
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 3 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv bff4a4eb merge | merge
    Parent commit      : royxmykx c84e900d b | b
    Parent commit      : zsuskuln d57db87b a | a
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  merge
    ╭─┤
    ◉ │  a
    │ ◉  b
    ├─╯
    ◉
    "###);

    // Now, let's rebase onto the descendant merge
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Working copy now at: vruxwmqv b05964d1 merge | merge
    Parent commit      : royxmykx cea87a87 b | b
    Parent commit      : zsuskuln 2c5b7858 a | a
    Added 1 files, modified 0 files, removed 0 files
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "merge"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 3 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv 986b7a49 merge | merge
    Parent commit      : royxmykx c07c677c b | b
    Parent commit      : zsuskuln abc90087 a | a
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    @    merge
    ├─╮
    │ ◉  a
    ◉ │  b
    ├─╯
    ◉
    "###);

    // TODO(ilyagr): These will be good tests for `jj rebase --insert-after` and
    // `--insert-before`, once those are implemented.
}

#[test]
fn test_rebase_multiple_destinations() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &[]);
    create_commit(&test_env, &repo_path, "c", &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    │ ◉  b
    ├─╯
    │ ◉  a
    ├─╯
    ◉
    "###);

    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "a", "-d", "b", "-d", "c"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉    a
    ├─╮
    │ @  c
    ◉ │  b
    ├─╯
    ◉
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-r", "a", "-d", "b|c"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Revset "b|c" resolved to more than one revision
    Hint: The revset "b|c" resolved to these revisions:
    royxmykx fe2e8e8b c | c
    zsuskuln d370aee1 b | b
    Prefix the expression with 'all' to allow any number of revisions (i.e. 'all:b|c').
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "a", "-d", "all:b|c"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉    a
    ├─╮
    │ ◉  b
    @ │  c
    ├─╯
    ◉
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-r", "a", "-d", "b", "-d", "b"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: More than one revset resolved to revision d370aee184ba
    "###);

    // Same error with 'all:' if there is overlap.
    let stderr = test_env.jj_cmd_failure(
        &repo_path,
        &["rebase", "-r", "a", "-d", "all:b|c", "-d", "b"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: More than one revset resolved to revision d370aee184ba
    "###);

    let stderr = test_env.jj_cmd_failure(
        &repo_path,
        &["rebase", "-r", "a", "-d", "b", "-d", "root()"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot merge with root revision
    "###);
}

#[test]
fn test_rebase_with_descendants() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[]);
    create_commit(&test_env, &repo_path, "b", &[]);
    create_commit(&test_env, &repo_path, "c", &["a", "b"]);
    create_commit(&test_env, &repo_path, "d", &["c"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  d
    ◉    c
    ├─╮
    │ ◉  b
    ◉ │  a
    ├─╯
    ◉
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s", "b", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv 309336ff d | d
    Parent commit      : royxmykx 244fa794 c | c
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  d
    ◉  c
    ◉  b
    ◉  a
    ◉
    "###);

    // Rebase several subtrees at once.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s=c", "-s=d", "-d=a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 2 commits
    Working copy now at: vruxwmqv 92c2bc9a d | d
    Parent commit      : rlvkpnrz 2443ea76 a | a
    Added 0 files, modified 0 files, removed 2 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  d
    │ ◉  c
    ├─╯
    ◉  a
    │ ◉  b
    ├─╯
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    // Reminder of the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  d
    ◉    c
    ├─╮
    │ ◉  b
    ◉ │  a
    ├─╯
    ◉
    "###);

    // `d` was a descendant of `b`, and both are moved to be direct descendants of
    // `a`. `c` remains a descendant of `b`.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s=b", "-s=d", "-d=a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv f1e71cb7 d | d
    Parent commit      : rlvkpnrz 2443ea76 a | a
    Added 0 files, modified 0 files, removed 2 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  c
    ◉  b
    │ @  d
    ├─╯
    ◉  a
    ◉
    "###);

    // Same test as above, but with multiple commits per argument
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let stderr = test_env.jj_cmd_failure(&repo_path, &["rebase", "-s=b|d", "-d=a"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Revset "b|d" resolved to more than one revision
    Hint: The revset "b|d" resolved to these revisions:
    vruxwmqv df54a9fd d | d
    zsuskuln d370aee1 b | b
    Prefix the expression with 'all' to allow any number of revisions (i.e. 'all:b|d').
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s=all:b|d", "-d=a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv d17539f7 d | d
    Parent commit      : rlvkpnrz 2443ea76 a | a
    Added 0 files, modified 0 files, removed 2 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  c
    ◉  b
    │ @  d
    ├─╯
    ◉  a
    ◉
    "###);
}

fn get_log_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["log", "-T", "branches"])
}

// This behavior illustrates https://github.com/martinvonz/jj/issues/2600
// The behavior of `rebase -r A -d descendant_of_A` can also be affected or
// broken depending on how #2600 is fixed, so we test that as well.
#[test]
fn test_rebase_with_child_and_descendant_bug_2600() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[]);
    create_commit(&test_env, &repo_path, "a", &["base"]);
    create_commit(&test_env, &repo_path, "b", &["base", "a"]);
    create_commit(&test_env, &repo_path, "c", &["b"]);
    let setup_opid = test_env.current_operation_id(&repo_path);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉
    "###);

    // ===================== rebase -s tests =================
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["rebase", "-s", "base", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 4 commits
    Working copy now at: vruxwmqv bda47523 c | c
    Parent commit      : royxmykx caeef796 b | b
    "###);
    // This should be a no-op, but isn't.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉  b
    ◉  a
    ◉  base
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s", "a", "-d", "base"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv 2ce41b33 c | c
    Parent commit      : royxmykx f16045cb b | b
    "###);
    // This should be a no-op
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉  b
    ◉  a
    ◉  base
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-s", "a", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv 2b10f149 c | c
    Parent commit      : royxmykx 3b233bd8 b | b
    "###);
    // This works as expected
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ◉ │  base
    ├─╯
    ◉
    "###);

    // ===================== rebase -b tests =================
    // ====== Reminder of the setup =========
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b", "c", "-d", "base"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 3 commits
    Working copy now at: vruxwmqv 4c7dc623 c | c
    Parent commit      : royxmykx 5ea34bfd b | b
    "###);
    // This should be a no-op
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉  b
    ◉  a
    ◉  base
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b", "c", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 2 commits
    Working copy now at: vruxwmqv 2fc4ef73 c | c
    Parent commit      : royxmykx 9912ef4b b | b
    "###);
    // I'm unsure what the user would expect here, probably a no-op
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉  b
    ◉  a
    ◉  base
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-b", "a", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Rebased 4 commits
    Working copy now at: vruxwmqv 0a026b90 c | c
    Parent commit      : royxmykx d1b575a5 b | b
    "###);
    // I'm unsure what the user would expect here, probably a no-op
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉  b
    ◉  a
    ◉  base
    ◉
    "###);

    // ===================== rebase -r tests =================
    // ====== Reminder of the setup =========
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉
    "###);

    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv 57aaa944 c | c
    Parent commit      : royxmykx c8495a71 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // The user would expect unsimplified ancestry here.
    // ◉  base
    // │ @  c
    // │ ◉    b
    // │ ├─╮
    // │ │ ◉  a
    // │ ├─╯
    // ├─╯
    // ◉
    // However, this is NOT ALLOWED as the b would be a merge commit and a child of
    // the root commit.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    │ ◉  b
    │ ◉  a
    ├─╯
    ◉
    "###);

    // This tests the algorithm for rebasing onto descendants. The result should be
    // simplified if and only if it's simplified in the above case.
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv a72f0141 c | c
    Parent commit      : royxmykx 7033e775 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // Unsimlified ancestry would look like
    // @  c
    // │ ◉  base
    // ├─╯
    // ◉    b
    // ├─╮
    // │ ◉  a
    // ├─╯
    // ◉
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    ├─╯
    ◉  b
    ◉  a
    ◉
    "###);

    // This tests the algorithm for rebasing onto descendants. The result should be
    // simplified if and only if it's simplified in the above case.
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv 0b91d0eb c | c
    Parent commit      : royxmykx fb944989 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    │ ◉  b
    ├─╯
    ◉  a
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    // ====== Reminder of the setup =========
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "a", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 2 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv f366e099 c | c
    Parent commit      : royxmykx bfc7c538 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // In this case, it is unclear whether the user would always prefer unsimplified
    // ancestry (whether `b` should also be a direct child of the root commit).
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  a
    │ @  c
    │ ◉  b
    │ ◉  base
    ├─╯
    ◉
    "###);

    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "b", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 1 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv 9570ddf7 c | c
    Parent commit      : rlvkpnrz 0c61db1b base | base
    Parent commit      : zsuskuln 2c5b7858 a | a
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // This works like the user likely intended
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  b
    │ @    c
    │ ├─╮
    │ │ ◉  a
    │ ├─╯
    │ ◉  base
    ├─╯
    ◉
    "###);

    // This tests the algorithm for rebasing onto descendants. The result should be
    // simplified if and only if it's simplified in the above case.
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "b", "-d", "c"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 1 descendant commits onto parent of rebased commit
    Working copy now at: vruxwmqv c48b5170 c | c
    Parent commit      : rlvkpnrz 0c61db1b base | base
    Parent commit      : zsuskuln 2c5b7858 a | a
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  b
    @    c
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉
    "###);

    // In this test, the commit with weird ancestry is not rebased (neither directly
    // nor indirectly).
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "c", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Working copy now at: vruxwmqv e64d4b0d c | c
    Parent commit      : zsuskuln 2c5b7858 a | a
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    │ ◉  b
    ╭─┤
    ◉ │  a
    ├─╯
    ◉  base
    ◉
    "###);
}

#[test]
fn test_rebase_with_child_and_descendant_bug_2600_different_setup() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "not_root", &[]);
    create_commit(&test_env, &repo_path, "base", &["not_root"]);
    create_commit(&test_env, &repo_path, "a", &["base"]);
    create_commit(&test_env, &repo_path, "b", &["base", "a"]);
    create_commit(&test_env, &repo_path, "c", &["b"]);
    let setup_opid = test_env.current_operation_id(&repo_path);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @  c
    ◉    b
    ├─╮
    │ ◉  a
    ├─╯
    ◉  base
    ◉  not_root
    ◉
    "###);

    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "root()"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: znkkpsqq 0c381bbe c | c
    Parent commit      : vruxwmqv 01fc4063 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // The user would expect unsimplified ancestry here.
    // ◉  base
    // │ @  c
    // │ ◉    b
    // │ ├─╮
    // │ │ ◉  a
    // │ ├─╯
    // ├─╯
    // ◉
    // However, this is NOT ALLOWED as the b would be a merge commit and a child of
    // the root commit.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    │ ◉  b
    │ ◉  a
    │ ◉  not_root
    ├─╯
    ◉
    "###);

    // This tests the algorithm for rebasing onto descendants. The result should be
    // simplified if and only if it's simplified in the above case.
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: znkkpsqq c3283cba c | c
    Parent commit      : vruxwmqv 0248b4ba b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    // Unsimlified ancestry would look like
    // @  c
    // │ ◉  base
    // ├─╯
    // ◉    b
    // ├─╮
    // │ ◉  a
    // ├─╯
    // ◉
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    ├─╯
    ◉  b
    ◉  a
    ◉  not_root
    ◉
    "###);

    // This tests the algorithm for rebasing onto descendants. The result should be
    // simplified if and only if it's simplified in the above case.
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &setup_opid]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "base", "-d", "a"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Also rebased 4 descendant commits onto parent of rebased commit
    Working copy now at: znkkpsqq 9ecd9620 c | c
    Parent commit      : vruxwmqv 62bc8576 b | b
    Added 0 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    ◉  base
    │ @  c
    │ ◉  b
    ├─╯
    ◉  a
    ◉  not_root
    ◉
    "###);
}
