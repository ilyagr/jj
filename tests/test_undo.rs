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
            "192424fd0add239fbc9d8ddd3cf2314c7e09efa5e8688886a87e4fc160c588483d96c4bc4f6f5c34186914a4e64c4d1eef8a54bde24292e124da17baa1178035",
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
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "v3"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "4aedbd1115b10209ced227185b7c445598f88a6d94bbc9a9c3bc1465f2c9b85fb00ada810921c4cf02bf4a2423c577237657b74992dad63ca4243c4ad6d6c36c",
        ),
        parents: [
            OperationId(
                "9c51d2e874e28bacdcf1938514e92020ef840168c5f2c65f3f192adada3848fd154d5fbb5f46ec6e67797514fb6f803318a7f17d5c9f32b6e9a071bb2f99ba0f",
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
    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_debug_op(&test_env, &repo_path), @r###"
    Operation {
        view_id: ViewId(
            "4aedbd1115b10209ced227185b7c445598f88a6d94bbc9a9c3bc1465f2c9b85fb00ada810921c4cf02bf4a2423c577237657b74992dad63ca4243c4ad6d6c36c",
        ),
        parents: [
            OperationId(
                "9c51d2e874e28bacdcf1938514e92020ef840168c5f2c65f3f192adada3848fd154d5fbb5f46ec6e67797514fb6f803318a7f17d5c9f32b6e9a071bb2f99ba0f",
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
    main: 29f0efc9eb74 v3
      @origin (ahead by 1 commits, behind by 1 commits): 367d4f2f6deb v1
    "###);
}

fn get_branch_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["branch", "list"])
}

fn get_debug_op(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["debug", "operation"])
}
