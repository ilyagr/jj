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

#[test]
fn test_undo_rewrite_with_child() {
    // Test that if we undo an operation that rewrote some commit, any descendants
    // after that will be rebased on top of the un-rewritten commit.
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "initial"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "modified"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    let op_id_hex = stdout[3..15].to_string();
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "child"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description"]);
    insta::assert_snapshot!(stdout, @r###"
    @  child
    ◉  modified
    ◉
    "###);
    test_env.jj_cmd_success(&repo_path, &["undo", &op_id_hex]);

    // Since we undid the description-change, the child commit should now be on top
    // of the initial commit
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description"]);
    insta::assert_snapshot!(stdout, @r###"
    @  child
    ◉  initial
    ◉
    "###);
}
#[test]
fn test_git_push_undo() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    git2::Repository::init_bare(git_repo_path).unwrap();
    test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["branch", "create", "main"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "v1"]);
    test_env.jj_cmd_success(&repo_path, &["git", "push"]);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "v2"]);
    test_env.jj_cmd_success(&repo_path, &["git", "push"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###""###);
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###""###);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "v3"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###""###);
    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###""###);
    // TODO: This should probably not be considered a conflict. It currently is
    // because the undo made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    main (conflicted):
      - 367d4f2f6deb v1
      + cb20e76758a0 v3
      + ebba8fecca7e v2
      @origin (behind by 1 commits): ebba8fecca7e v2
    "###);
}

fn get_branch_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["branch", "list"])
}

fn get_debug_op(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["debug", "operation"])
}
