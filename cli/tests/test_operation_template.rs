// Copyright 2024 The Jujutsu Authors
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

use crate::common::TestEnvironment;

#[test]
fn test_op_log_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a few operations
    work_dir.run_jj(["describe", "-m", "op0"]).success();
    work_dir.run_jj(["describe", "-m", "op1"]).success();
    work_dir.run_jj(["describe", "-m", "op2"]).success();

    // Test operation.parents() and OperationList methods
    let template = r#"id ++ "\nP: " ++ parents.len() ++ " " ++ parents.map(|op| op.id()) ++ "\n""#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  f0fa265b7c6a982227120be99a82fe86d762ee8efb26e8fba8cc81669a820d7e76df6cf64bed649716bb13f9b3679222cbfa0fa262c5100e3790dc7fcbd9b3b5
    │  P: 1 a4bfccf0d4788bac603d8fb7effb351979ecd9bdc1020b0ad33cfa143233287e2f91b8daafbd8396e07461ae13a9b6a80d7e0796f4c4aa6d8339c5654ab5db46
    ○  a4bfccf0d4788bac603d8fb7effb351979ecd9bdc1020b0ad33cfa143233287e2f91b8daafbd8396e07461ae13a9b6a80d7e0796f4c4aa6d8339c5654ab5db46
    │  P: 1 ca312e6f542841937a55ef62ba5c7d494dcaf83f94512f8f764efc4c0a8c3955a52de7690f253fedd48b0f67f04a1bb7e089db8ee3c5679b816ef3d4b61c3f4d
    ○  ca312e6f542841937a55ef62ba5c7d494dcaf83f94512f8f764efc4c0a8c3955a52de7690f253fedd48b0f67f04a1bb7e089db8ee3c5679b816ef3d4b61c3f4d
    │  P: 1 8f47435a3990362feaf967ca6de2eb0a31c8b883dfcb66fba5c22200d12bbe61e3dc8bc855f1f6879285fcafaf85ac792f9a43bcc36e57d28737d18347d5e752
    ○  8f47435a3990362feaf967ca6de2eb0a31c8b883dfcb66fba5c22200d12bbe61e3dc8bc855f1f6879285fcafaf85ac792f9a43bcc36e57d28737d18347d5e752
    │  P: 1 00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    ○  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
       P: 0
    [EOF]
    ");

    // OperationList can be filtered
    let template = r#""P: " ++ parents.filter(|op| !op.root()).map(|op| op.id().short()) ++ "\n""#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  P: a4bfccf0d478
    ○  P: ca312e6f5428
    ○  P: 8f47435a3990
    ○  P:
    ○  P:
    [EOF]
    ");

    // OperationList map with argument
    let template = r#"parents.map(|op| op.id().shortest(4))"#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse template: Method `shortest` doesn't exist for type `OperationId`
    Caused by:  --> 1:26
      |
    1 | parents.map(|op| op.id().shortest(4))
      |                          ^------^
      |
      = Method `shortest` doesn't exist for type `OperationId`
    Hint: Did you mean `short`?
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_operation_list_methods() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "opA"]).success();
    work_dir.run_jj(["describe", "-m", "opB"]).success();

    // Test len and is_empty on OperationList
    let template =
        r#""parents.len=" ++ parents.len() ++ ", parents.is_empty=" ++ parents.is_empty() ++ "\n""#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Method `is_empty` doesn't exist for type `List<Operation>`
    Caused by:  --> 1:69
      |
    1 | "parents.len=" ++ parents.len() ++ ", parents.is_empty=" ++ parents.is_empty() ++ "\n"
      |                                                                     ^------^
      |
      = Method `is_empty` doesn't exist for type `List<Operation>`
    [EOF]
    [exit status: 1]
    "#);
}
