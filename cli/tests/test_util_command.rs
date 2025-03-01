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
            "$comment": "`taplo` and the corresponding VS Code plugins only support draft-04 verstion of JSON Schema, see <https://taplo.tamasfe.dev/configuration/developing-schemas.html>. draft-07 is mostly compatible with it, newer versions may not be.",
            "title": "Jujutsu config",
            "type": "object",
            "description": "User configuration for Jujutsu VCS. See https://jj-vcs.github.io/jj/latest/config/ for details",
            "properties": {
                [...]
            }
        }
        [EOF]
        "#);
    });
}

#[test]
fn test_gc_args() {
    let test_env = TestEnvironment::default();
    // Use the local backend because GitBackend::gc() depends on the git CLI.
    test_env
        .run_jj_in(
            ".",
            [
                "toy-backend",
                "init",
                "repo",
                "--config=ui.allow-init-native=true",
            ],
        )
        .success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["util", "gc"]);
    insta::assert_snapshot!(output, @"");

    let output = test_env.run_jj_in(&repo_path, ["util", "gc", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot garbage collect from a non-head operation
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["util", "gc", "--expire=foobar"]);
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
    // Use the local backend because GitBackend::gc() depends on the git CLI.
    test_env
        .run_jj_in(
            ".",
            [
                "toy-backend",
                "init",
                "repo",
                "--config=ui.allow-init-native=true",
            ],
        )
        .success();
    let repo_path = test_env.env_root().join("repo");

    // Create an operation.
    std::fs::write(repo_path.join("file"), "a change\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "a change"])
        .success();
    let op_to_remove = test_env.current_operation_id(&repo_path);

    // Make another operation the head.
    std::fs::write(repo_path.join("file"), "another change\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "another change"])
        .success();

    // This works before the operation is removed.
    test_env
        .run_jj_in(&repo_path, ["debug", "operation", &op_to_remove])
        .success();

    // Remove some operations.
    test_env
        .run_jj_in(&repo_path, ["operation", "abandon", "..@-"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["util", "gc", "--expire=now"])
        .success();

    // Now this doesn't work.
    let output = test_env.run_jj_in(&repo_path, ["debug", "operation", &op_to_remove]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: No operation ID matching "8382f401329617b0c91a63354b86ca48fc28dee8d7a916fdad5310030f9a1260e969c43ed2b13d1d48eaf38f6f45541ecf593bcb6105495d514d21b3b6a98846"
    [EOF]
    [exit status: 1]
    "#);
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
