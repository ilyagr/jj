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

use common::{get_stderr_string, get_stdout_string, TestEnvironment};

pub mod common;

#[test]
fn test_log_with_empty_revision() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let stderr = test_env.jj_cmd_cli_error(&repo_path, &["log", "-r="]);
    insta::assert_snapshot!(stderr, @r###"
    error: The argument '--revisions <REVISIONS>' requires a value but none was supplied

    For more information try '--help'
    "###);
}

#[test]
fn test_log_with_or_without_diff() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "add a file"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "a new commit"]);
    std::fs::write(repo_path.join("file1"), "foo\nbar\n").unwrap();

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description"]);
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    o add a file
    o (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "-p"]);
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | Modified regular file file1:
    |    1    1: foo
    |         2: bar
    o add a file
    | Added regular file file1:
    |         1: foo
    o (no description set)
    "###);

    // Test default log output format with prefixes
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["log", "--config-toml", "ui.unique-prefixes='underscore'"]), @r###"
    @ f_fdaa62087a test.user@example.com 2001-02-03 04:05:10.000 +07:00 7_89e536fd2e
    | a new commit
    o 9_a45c67d3e9 test.user@example.com 2001-02-03 04:05:08.000 +07:00 4_291e264ae9
    | add a file
    o 000000000000  1970-01-01 00:00:00.000 +00:00 000000000000
      (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "--no-graph"]);
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    add a file
    (no description set)
    "###);

    // `-p` for default diff output, `-s` for summary
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "-p", "-s"]);
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | M file1
    | Modified regular file file1:
    |    1    1: foo
    |         2: bar
    o add a file
    | A file1
    | Added regular file file1:
    |         1: foo
    o (no description set)
    "###);

    // `-s` for summary, `--git` for git diff (which implies `-p`)
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "-s", "--git"]);
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | M file1
    | diff --git a/file1 b/file1
    | index 257cc5642c...3bd1f0e297 100644
    | --- a/file1
    | +++ b/file1
    | @@ -1,1 +1,2 @@
    |  foo
    | +bar
    o add a file
    | A file1
    | diff --git a/file1 b/file1
    | new file mode 100644
    | index 0000000000..257cc5642c
    | --- /dev/null
    | +++ b/file1
    | @@ -1,0 +1,1 @@
    | +foo
    o (no description set)
    "###);

    // `-p` enables default "summary" output, so `-s` is noop
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            "description",
            "-p",
            "-s",
            "--config-toml=diff.format='summary'",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | M file1
    o add a file
    | A file1
    o (no description set)
    "###);

    // `-p` enables default "color-words" diff output, so `--color-words` is noop
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "-p", "--color-words"],
    );
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | Modified regular file file1:
    |    1    1: foo
    |         2: bar
    o add a file
    | Added regular file file1:
    |         1: foo
    o (no description set)
    "###);

    // `--git` enables git diff, so `-p` is noop
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "--no-graph", "-p", "--git"],
    );
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    diff --git a/file1 b/file1
    index 257cc5642c...3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    add a file
    diff --git a/file1 b/file1
    new file mode 100644
    index 0000000000..257cc5642c
    --- /dev/null
    +++ b/file1
    @@ -1,0 +1,1 @@
    +foo
    (no description set)
    "###);

    // Both formats enabled if `--git` and `--color-words` are explicitly specified
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            "description",
            "--no-graph",
            "-p",
            "--git",
            "--color-words",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    diff --git a/file1 b/file1
    index 257cc5642c...3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    Modified regular file file1:
       1    1: foo
            2: bar
    add a file
    diff --git a/file1 b/file1
    new file mode 100644
    index 0000000000..257cc5642c
    --- /dev/null
    +++ b/file1
    @@ -1,0 +1,1 @@
    +foo
    Added regular file file1:
            1: foo
    (no description set)
    "###);

    // `-s` with or without graph
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "-s"]);
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    | M file1
    o add a file
    | A file1
    o (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "--no-graph", "-s"],
    );
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    M file1
    add a file
    A file1
    (no description set)
    "###);

    // `--git` implies `-p`, with or without graph
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "-r", "@", "--git"],
    );
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    ~ diff --git a/file1 b/file1
      index 257cc5642c...3bd1f0e297 100644
      --- a/file1
      +++ b/file1
      @@ -1,1 +1,2 @@
       foo
      +bar
    "###);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "-r", "@", "--no-graph", "--git"],
    );
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    diff --git a/file1 b/file1
    index 257cc5642c...3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    "###);

    // `--color-words` implies `-p`, with or without graph
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "-r", "@", "--color-words"],
    );
    insta::assert_snapshot!(stdout, @r###"
    @ a new commit
    ~ Modified regular file file1:
         1    1: foo
              2: bar
    "###);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            "description",
            "-r",
            "@",
            "--no-graph",
            "--color-words",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    a new commit
    Modified regular file file1:
       1    1: foo
            2: bar
    "###);
}

#[test]
fn test_log_prefix_highlight() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let prefix_format = r#"
      "Change " change_id.short_underscore_prefix() " " description.first_line()
      " " commit_id.short_underscore_prefix() " " branches
    "#;

    std::fs::write(repo_path.join("file"), "original file\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "initial"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "c", "original"]);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "original", "-T", prefix_format]),
        @r###"
    @ Change 9_a45c67d3e9 initial b_a1a30916d2 original
    ~ 
    "###
    );
    for i in 1..50 {
        test_env.jj_cmd_success(&repo_path, &["new", "-m", &format!("commit{i}")]);
        std::fs::write(repo_path.join("file"), &format!("file {i}\n")).unwrap();
    }
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "original", "-T", prefix_format]),
        @r###"
    o Change 9a4_5c67d3e9 initial ba1_a30916d2 original
    ~ 
    "###
    );
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "@-----------..@", "-T", prefix_format]),
        @r###"
    @ Change 4c9_32da8013 commit49 d8_3437a2cef 
    o Change 0d_58f15eaba commit48 f3_abb4ea0ac 
    o Change fc_e6c2c5912 commit47 38e_891bea27 
    o Change d5_1defcac30 commit46 1c_04d947707 
    o Change 4f_13b1391d6 commit45 747_24ae22b1 
    o Change 6a_de2950a04 commit44 c7a_a67cf7bb 
    o Change 06c_482e452d commit43 8e_c99dfcb6c 
    o Change 392_beeb018e commit42 8f0_e60411b7 
    o Change a1_b73d3ff91 commit41 71_d6937a66c 
    o Change 708_8f461291 commit40 db_572049026 
    o Change c49_f7f006c7 commit39 d94_54fec8a6 
    ~ 
    "###
    );
}

#[test]
fn test_log_prefix_highlight_counts_hidden_commits() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let prefix_format = r#"
      "Change " change_id.short_underscore_prefix() " " description.first_line()
      " " commit_id.short_underscore_prefix() " " branches
    "#;

    std::fs::write(repo_path.join("file"), "original file\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "initial"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "c", "original"]);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()", "-T", prefix_format]),
        @r###"
    @ Change 9_a45c67d3e9 initial b_a1a30916d2 original
    o Change 000000000000 (no description set) 000000000000 
    "###
    );
    for i in 1..100 {
        test_env.jj_cmd_success(&repo_path, &["describe", "-m", &format!("commit{i}")]);
        std::fs::write(repo_path.join("file"), &format!("file {i}\n")).unwrap();
    }
    // The first commit still exists. Its unique prefix became longer.
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "ba1", "-T", prefix_format]),
        @r###"
    o Change 9a4_5c67d3e9 initial ba1_a30916d2 
    ~ 
    "###
    );
    // The first commit is no longer visible
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()", "-T", prefix_format]),
        @r###"
    @ Change 9a4_5c67d3e9 commit99 de_3177d2acf original
    o Change 000000000000 (no description set) 000000000000 
    "###
    );
    insta::assert_snapshot!(
        test_env.jj_cmd_failure(&repo_path, &["log", "-r", "d", "-T", prefix_format]),
        @r###"
    Error: Commit or change id prefix "d" is ambiguous
    "###
    );
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["log", "-r", "de", "-T", prefix_format]),
        @r###"
    @ Change 9a4_5c67d3e9 commit99 de_3177d2acf original
    ~ 
    "###
    );
}

#[test]
fn test_log_divergence() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "description 1"]);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            r#"description.first_line() if(divergent, " !divergence!")"#,
        ],
    );
    // No divergence
    insta::assert_snapshot!(stdout, @r###"
    @ description 1
    o (no description set)
    "###);

    // Create divergence
    test_env.jj_cmd_success(
        &repo_path,
        &["describe", "-m", "description 2", "--at-operation", "@-"],
    );
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            r#"description.first_line() if(divergent, " !divergence!")"#,
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    Concurrent modification detected, resolving automatically.
    o description 2 !divergence!
    | @ description 1 !divergence!
    |/  
    o (no description set)
    "###);
}

#[test]
fn test_log_reversed() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "first"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "second"]);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "--reversed"]);
    insta::assert_snapshot!(stdout, @r###"
    o (no description set)
    o first
    @ second
    "###);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "--reversed", "--no-graph"],
    );
    insta::assert_snapshot!(stdout, @r###"
    (no description set)
    first
    second
    "###);
}

#[test]
fn test_log_filtered_by_path() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "first"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "second"]);
    std::fs::write(repo_path.join("file1"), "foo\nbar\n").unwrap();
    std::fs::write(repo_path.join("file2"), "baz\n").unwrap();

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "file1"]);
    insta::assert_snapshot!(stdout, @r###"
    @ second
    o first
    ~ 
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "file2"]);
    insta::assert_snapshot!(stdout, @r###"
    @ second
    ~ 
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description", "-s", "file1"]);
    insta::assert_snapshot!(stdout, @r###"
    @ second
    | M file1
    o first
    ~ A file1
    "###);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "description", "-s", "file2", "--no-graph"],
    );
    insta::assert_snapshot!(stdout, @r###"
    second
    A file2
    "###);

    // file() revset doesn't filter the diff.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "-T",
            "description",
            "-s",
            "-rfile(file2)",
            "--no-graph",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    second
    M file1
    A file2
    "###);
}

#[test]
fn test_log_warn_path_might_be_revset() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();

    // Don't warn if the file actually exists.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", "file1", "-T", "description"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @r###"
    @ (no description set)
    ~ 
    "###);
    insta::assert_snapshot!(get_stderr_string(&assert), @"");

    // Warn for `jj log .` specifically, for former Mercurial users.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", ".", "-T", "description"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @r###"
    @ (no description set)
    ~ 
    "###);
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"warning: The argument "." is being interpreted as a path, but this is often not useful because all non-empty commits touch '.'.  If you meant to show the working copy commit, pass -r '@' instead."###);

    // ...but checking `jj log .` makes sense in a subdirectory.
    let subdir = repo_path.join("dir");
    std::fs::create_dir_all(&subdir).unwrap();
    let assert = test_env.jj_cmd(&subdir, &["log", "."]).assert().success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @"");

    // Warn for `jj log @` instead of `jj log -r @`.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", "@", "-T", "description"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    warning: The argument "@" is being interpreted as a path. To specify a revset, pass -r "@" instead.
    "###);

    // Warn when there's no path with the provided name.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", "file2", "-T", "description"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    warning: The argument "file2" is being interpreted as a path. To specify a revset, pass -r "file2" instead.
    "###);

    // If an explicit revision is provided, then suppress the warning.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", "@", "-r", "@", "-T", "description"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @"");
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    "###);
}

#[test]
fn test_default_revset() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "add a file"]);

    // Set configuration to only show the root commit.
    test_env.add_config(
        br#"[ui]
        default-revset = "root"
        "#,
    );

    // Log should only contain one line (for the root commit), and not show the
    // commit created above.
    assert_eq!(
        1,
        test_env
            .jj_cmd_success(&repo_path, &["log", "-T", "commit_id"])
            .lines()
            .count()
    );
}

#[test]
fn test_default_revset_per_repo() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "add a file"]);

    // Set configuration to only show the root commit.
    std::fs::write(
        repo_path.join(".jj/repo/config.toml"),
        r#"ui.default-revset = "root""#,
    )
    .unwrap();

    // Log should only contain one line (for the root commit), and not show the
    // commit created above.
    assert_eq!(
        1,
        test_env
            .jj_cmd_success(&repo_path, &["log", "-T", "commit_id"])
            .lines()
            .count()
    );
}
