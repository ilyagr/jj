// Copyright 2023 The Jujutsu Authors
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

use insta::assert_snapshot;

use crate::common::TestEnvironment;

#[test]
fn test_util_config_schema() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "config-schema"]);
    // Validate partial snapshot, redacting any lines nested 2+ indent levels.
    insta::with_settings!({filters => vec![(r"(?m)(^        .*$\r?\n)+", "        [...]\n")]}, {
        assert_snapshot!(output, @r#"
        {
          "$schema": "http://json-schema.org/draft-04/schema",
          "$comment": "`taplo` and the corresponding VS Code plugins only support version draft-04 of JSON Schema, see <https://taplo.tamasfe.dev/configuration/developing-schemas.html>. draft-07 is mostly compatible with it, newer versions may not be.",
          "title": "Jujutsu config",
          "type": "object",
          "description": "User configuration for Jujutsu VCS. See https://jj-vcs.github.io/jj/latest/config/ for details",
          "properties": {
            "user": {
              "type": "object",
              "description": "Settings about the user",
              "properties": {
                [...]
              }
            },
            "operation": {
              "type": "object",
              "description": "Metadata to be attached to jj operations (shown in jj op log)",
              "properties": {
                [...]
              }
            },
            "ui": {
              "type": "object",
              "description": "UI settings",
              "definitions": {
                [...]
              },
              "properties": {
                [...]
              }
            },
            "fsmonitor": {
              "type": "object",
              "description": "External filesystem monitor settings, useful for large repos",
              "properties": {
                [...]
              }
            },
            "colors": {
              "type": "object",
              "description": "Mapping from jj formatter labels to colors",
              "definitions": {
                [...]
              },
              "additionalProperties": {
                [...]
              }
            },
            "diff": {
              "type": "object",
              "description": "Builtin diff formats settings",
              "properties": {
                [...]
              }
            },
            "git": {
              "type": "object",
              "description": "Settings for git behavior (when using git backend)",
              "properties": {
                [...]
              }
            },
            "gerrit": {
              "type": "object",
              "description": "Settings for interacting with Gerrit",
              "properties": {
                [...]
              }
            },
            "merge": {
              "type": "object",
              "description": "Merge settings",
              "properties": {
                [...]
              }
            },
            "merge-tools": {
              "type": "object",
              "description": "Tables of custom options to pass to the given merge tool (selected in ui.merge-editor)",
              "additionalProperties": {
                [...]
              }
            },
            "revsets": {
              "type": "object",
              "description": "Revset expressions used by various commands",
              "properties": {
                [...]
              },
              "additionalProperties": {
                [...]
              }
            },
            "revset-aliases": {
              "type": "object",
              "description": "Custom symbols/function aliases that can used in revset expressions",
              "properties": {
                [...]
              },
              "additionalProperties": {
                [...]
              }
            },
            "template-aliases": {
              "type": "object",
              "description": "Custom symbols/function aliases that can used in templates",
              "additionalProperties": {
                [...]
              }
            },
            "aliases": {
              "type": "object",
              "description": "Custom subcommand aliases to be supported by the jj command",
              "additionalProperties": {
                [...]
              }
            },
            "snapshot": {
              "type": "object",
              "description": "Parameters governing automatic capture of files into the working copy commit",
              "properties": {
                [...]
              }
            },
            "experimental-advance-branches": {
              "type": "object",
              "description": "Settings controlling the 'advance-branches' feature which moves bookmarks forward when new commits are created.",
              "properties": {
                [...]
              }
            },
            "signing": {
              "type": "object",
              "description": "Settings for verifying and creating cryptographic commit signatures",
              "properties": {
                [...]
              }
            },
            "fix": {
              "type": "object",
              "description": "Settings for jj fix",
              "properties": {
                [...]
              }
            },
            "split": {
              "type": "object",
              "description": "Settings for jj split",
              "properties": {
                [...]
              }
            },
            "hints": {
              "type": "object",
              "description": "Various hints in jj's UI that can be disabled",
              "additionalProperties": {
                [...]
              }
            },
            "templates": {
              "type": "object",
              "description": "Definitions for the templates that various jj commands use",
              "properties": {
                [...]
              },
              "additionalProperties": {
                [...]
              }
            },
            "working-copy": {
              "type": "object",
              "description": "Working copy settings",
              "properties": {
                [...]
              }
            },
            "--when": {
              "type": "object",
              "description": "Conditions restriction the application of the configuration",
              "properties": {
                [...]
              }
            },
            "--scope": {
              "type": "array",
              "description": "Scoped tables for conditional configuration",
              "items": {
                [...]
              }
            }
          }
        }
        [EOF]
        "#);
    });
}

#[test]
fn test_gc_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["util", "gc"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["util", "gc", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot garbage collect from a non-head operation
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["util", "gc", "--expire=foobar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --expire only accepts 'now'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gc_operation_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create an operation.
    work_dir.write_file("file", "a change\n");
    work_dir.run_jj(["commit", "-m", "a change"]).success();
    let op_to_remove = work_dir.current_operation_id();

    // Make another operation the head.
    work_dir.write_file("file", "another change\n");
    work_dir
        .run_jj(["commit", "-m", "another change"])
        .success();

    // This works before the operation is removed.
    work_dir
        .run_jj(["debug", "object", "operation", &op_to_remove])
        .success();

    // Remove some operations.
    work_dir.run_jj(["operation", "abandon", "..@-"]).success();
    work_dir.run_jj(["util", "gc", "--expire=now"]).success();

    // Now this doesn't work.
    let output = work_dir.run_jj(["debug", "object", "operation", &op_to_remove]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Object b50d0a8f111a9d30d45d429d62c8e54016cc7c891706921a6493756c8074e883671cf3dac0ac9f94ef0fa8c79738a3dfe38c3e1f6c5e1a4a4d0857d266ef2040 of type operation not found
    2: Cannot access $TEST_ENV/repo/.jj/repo/op_store/operations/b50d0a8f111a9d30d45d429d62c8e54016cc7c891706921a6493756c8074e883671cf3dac0ac9f94ef0fa8c79738a3dfe38c3e1f6c5e1a4a4d0857d266ef2040
    [EOF]
    [exit status: 255]
    ");
}

#[test]
fn test_shell_completions() {
    #[track_caller]
    fn test(shell: &str) {
        let test_env = TestEnvironment::default();
        // Use the local backend because GitBackend::gc() depends on the git CLI.
        let output = test_env
            .run_jj_in(".", ["util", "completion", shell])
            .success();
        // Ensures only stdout contains text
        assert!(
            !output.stdout.is_empty() && output.stderr.is_empty(),
            "{output}"
        );
    }

    test("bash");
    test("fish");
    test("nushell");
    test("zsh");
}

#[test]
fn test_util_exec() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--append",
            "hello",
        ],
    );
    // Ensures only stdout contains text
    insta::assert_snapshot!(output, @"hello[EOF]");
}

#[test]
fn test_util_exec_fail() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "exec", "--", "jj-test-missing-program"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to execute external command 'jj-test-missing-program'
    [EOF]
    [exit status: 1]
    ");
}
