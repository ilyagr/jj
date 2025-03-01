// Copyright 2025 The Jujutsu Authors
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

use crate::cli_util::CommandHelper;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Show a custom error to the user and quit
///
/// Ignores any extraneous arguments.
///
/// This is useful for handling deprecated commands:
///
/// ```toml
/// aliases.deprecated_command = [
///   "util",
///   "error",
///   "--hint=Use cool_command!",
///   "`deprecated_command` is deprecated",
///   "--"
/// ]
/// ```
///
/// Then, `jj deprecated-command`, `jj deprecated-command blah`, and even `jj
/// deprecated-command --help` will print the same error.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub struct UtilError {
    /// The text of the error, to be shown to the user
    error: String,
    /// A hint to print after the error
    #[arg(long)]
    hint: Option<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    _unused_args: Vec<String>,
}

pub fn cmd_util_error(
    _ui: &mut Ui,
    _command: &CommandHelper,
    args: &UtilError,
) -> Result<(), CommandError> {
    let UtilError {
        error,
        hint,
        _unused_args,
    } = args.clone();
    Err(match hint {
        None => user_error(error),
        Some(hint) => user_error_with_hint(error, hint),
    })
}
