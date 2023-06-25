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
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "6d2a7fcfa1a307ed2d2fec3e74e0c10b0f497edf418171b3a4a55b8b711058b1d51d792861e3d58a538653b7c6b7c9e5a843086cb493c5b3b0d1e7f619b5aecb",
        ),
        parents: [
            OperationId(
                "2b46bcfe10cc5c46eea27e64cbef436dd4d058cf1cd0ca95d39d0031c9b97f725f11527226e3a3d8a1753c87e9554c278b97feacaf55b48a9b5fa0b1eb904690",
            ),
        ],
        metadata: OperationMetadata {
            start_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147912000,
                ),
                tz_offset: 420,
            },
            end_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147912000,
                ),
                tz_offset: 420,
            },
            description: "push current branch(es) to git remote origin",
            hostname: "host.example.com",
            username: "test-username",
            tags: {
                "args": "jj git push",
            },
        },
    }
    View {
        head_ids: {
            CommitId(
                "ebba8fecca7e65141a97f3fbc265451560aa8235",
            ),
        },
        public_head_ids: {
            CommitId(
                "0000000000000000000000000000000000000000",
            ),
        },
        branches: {
            "main": BranchTarget {
                local_target: Some(
                    Normal(
                        CommitId(
                            "ebba8fecca7e65141a97f3fbc265451560aa8235",
                        ),
                    ),
                ),
                remote_targets: {
                    "origin": Normal(
                        CommitId(
                            "ebba8fecca7e65141a97f3fbc265451560aa8235",
                        ),
                    ),
                },
            },
        },
        tags: {},
        git_refs: {
            "refs/remotes/origin/main": Normal(
                CommitId(
                    "ebba8fecca7e65141a97f3fbc265451560aa8235",
                ),
            ),
        },
        git_head: None,
        wc_commit_ids: {
            WorkspaceId(
                "default",
            ): CommitId(
                "ebba8fecca7e65141a97f3fbc265451560aa8235",
            ),
        },
    }
    "###);
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "f60b204c090433d6db21b6ff006d3b6005782534b5d190f1f19a3ade0b01f8f0d817e3fa8e59a895d03549a2999c62efcd6433a1b3dddcb925822cab51db97b8",
        ),
        parents: [
            OperationId(
                "387fcf88f4f9adfa4bb2af5af65ac214f006c1aa61e5daad43b677546d130f8b4d8c26d59234e5c5e977f3bf065966754a2acdf5344508d76340fba6a91d9cbc",
            ),
        ],
        metadata: OperationMetadata {
            start_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147914000,
                ),
                tz_offset: 420,
            },
            end_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147914000,
                ),
                tz_offset: 420,
            },
            description: "undo operation 387fcf88f4f9adfa4bb2af5af65ac214f006c1aa61e5daad43b677546d130f8b4d8c26d59234e5c5e977f3bf065966754a2acdf5344508d76340fba6a91d9cbc",
            hostname: "host.example.com",
            username: "test-username",
            tags: {
                "args": "jj undo",
            },
        },
    }
    View {
        head_ids: {
            CommitId(
                "ebba8fecca7e65141a97f3fbc265451560aa8235",
            ),
        },
        public_head_ids: {
            CommitId(
                "0000000000000000000000000000000000000000",
            ),
        },
        branches: {
            "main": BranchTarget {
                local_target: Some(
                    Normal(
                        CommitId(
                            "ebba8fecca7e65141a97f3fbc265451560aa8235",
                        ),
                    ),
                ),
                remote_targets: {
                    "origin": Normal(
                        CommitId(
                            "367d4f2f6deb0f71e3b45489f9a5bb28224562f9",
                        ),
                    ),
                },
            },
        },
        tags: {},
        git_refs: {
            "refs/remotes/origin/main": Normal(
                CommitId(
                    "367d4f2f6deb0f71e3b45489f9a5bb28224562f9",
                ),
            ),
        },
        git_head: None,
        wc_commit_ids: {
            WorkspaceId(
                "default",
            ): CommitId(
                "ebba8fecca7e65141a97f3fbc265451560aa8235",
            ),
        },
    }
    "###);
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "v3"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "39ba4fe829f857fe243c8f99647ee10603a7247c33457942c61c45e40a07a801993806f0ccff41afde02f5d8c67ddeb557cd3a9f94a57629e521e1c04d878a9f",
        ),
        parents: [
            OperationId(
                "0790c9c7e2ea8ed3208c556f62b226898aa0222820a61c58129f741f83c6bee8556384bb3e41be26ab8f394ee68fa6c2ba451cae760279fc4f8e0780f8507a33",
            ),
        ],
        metadata: OperationMetadata {
            start_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147916000,
                ),
                tz_offset: 420,
            },
            end_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147916000,
                ),
                tz_offset: 420,
            },
            description: "describe commit ebba8fecca7e65141a97f3fbc265451560aa8235",
            hostname: "host.example.com",
            username: "test-username",
            tags: {
                "args": "jj describe -m v3",
            },
        },
    }
    View {
        head_ids: {
            CommitId(
                "29f0efc9eb741adc923a96619e00ff6cf63b9573",
            ),
        },
        public_head_ids: {
            CommitId(
                "0000000000000000000000000000000000000000",
            ),
        },
        branches: {
            "main": BranchTarget {
                local_target: Some(
                    Normal(
                        CommitId(
                            "29f0efc9eb741adc923a96619e00ff6cf63b9573",
                        ),
                    ),
                ),
                remote_targets: {
                    "origin": Normal(
                        CommitId(
                            "367d4f2f6deb0f71e3b45489f9a5bb28224562f9",
                        ),
                    ),
                },
            },
        },
        tags: {},
        git_refs: {
            "refs/remotes/origin/main": Normal(
                CommitId(
                    "367d4f2f6deb0f71e3b45489f9a5bb28224562f9",
                ),
            ),
        },
        git_head: None,
        wc_commit_ids: {
            WorkspaceId(
                "default",
            ): CommitId(
                "29f0efc9eb741adc923a96619e00ff6cf63b9573",
            ),
        },
    }
    "###);
    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "b65bc23d2aa1cdf1131aa35c71105f7db1b7a3c8a7022949ffd5132d380347f20dd4c81e6c4c3c14669f8edb93d54e9ab503d00b783dad56f9c9cf4658d99d7b",
        ),
        parents: [
            OperationId(
                "0e3f168a3fcb7bef7e031966f5d9e1b93beda42eb07d8f3d8bbc63126b7e597e3c108ca128dcfd0aafc81a7ec532b0932396bdab9ecf8f65b0f09fc0943f0b1a",
            ),
        ],
        metadata: OperationMetadata {
            start_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147918000,
                ),
                tz_offset: 420,
            },
            end_time: Timestamp {
                timestamp: MillisSinceEpoch(
                    981147918000,
                ),
                tz_offset: 420,
            },
            description: "fetch from git remote(s) origin",
            hostname: "host.example.com",
            username: "test-username",
            tags: {
                "args": "jj git fetch",
            },
        },
    }
    View {
        head_ids: {
            CommitId(
                "ebba8fecca7e65141a97f3fbc265451560aa8235",
            ),
            CommitId(
                "29f0efc9eb741adc923a96619e00ff6cf63b9573",
            ),
        },
        public_head_ids: {
            CommitId(
                "0000000000000000000000000000000000000000",
            ),
        },
        branches: {
            "main": BranchTarget {
                local_target: Some(
                    Conflict {
                        removes: [
                            CommitId(
                                "367d4f2f6deb0f71e3b45489f9a5bb28224562f9",
                            ),
                        ],
                        adds: [
                            CommitId(
                                "29f0efc9eb741adc923a96619e00ff6cf63b9573",
                            ),
                            CommitId(
                                "ebba8fecca7e65141a97f3fbc265451560aa8235",
                            ),
                        ],
                    },
                ),
                remote_targets: {
                    "origin": Normal(
                        CommitId(
                            "ebba8fecca7e65141a97f3fbc265451560aa8235",
                        ),
                    ),
                },
            },
        },
        tags: {},
        git_refs: {
            "refs/remotes/origin/main": Normal(
                CommitId(
                    "ebba8fecca7e65141a97f3fbc265451560aa8235",
                ),
            ),
        },
        git_head: None,
        wc_commit_ids: {
            WorkspaceId(
                "default",
            ): CommitId(
                "29f0efc9eb741adc923a96619e00ff6cf63b9573",
            ),
        },
    }
    "###);
    // TODO: This should probably not be considered a conflict. It currently is
    // because the undo made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    main (conflicted):
      - 367d4f2f6deb v1
      + 29f0efc9eb74 v3
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
