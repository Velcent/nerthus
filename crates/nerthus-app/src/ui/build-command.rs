//! Literal arguments for the native build command and batch script.
//!
//! Gaea emits each variable as `-v name=value`. Quote the complete
//! assignment, including numeric ranges, rather than testing for letters.
//! Windows prompt commands assume cmd's default disabled delayed expansion
//! (`/V:OFF`); generated batch scripts explicitly disable it in their header.

#[derive(Clone, Copy)]
pub enum CommandKind {
    Prompt,
    Batch,
}

/// An executable's path. Windows parses this before the native argv.
pub fn quote_program(text: &str, kind: CommandKind) -> String {
    #[cfg(windows)]
    return escape_cmd(text, kind);

    #[cfg(not(windows))]
    quote_argument(text, kind)
}

/// One argument, with shell expansion disabled and its contents preserved.
pub fn quote_argument(text: &str, kind: CommandKind) -> String {
    #[cfg(windows)]
    return quote_cmd_argument(text, kind);

    #[cfg(not(windows))]
    {
        let _ = kind;
        let escaped = text.replace('\'', "'\\''");
        format!("'{escaped}'")
    }
}

/// Both the variable name and its value belong to a single argv entry.
pub fn variable_argument(name: &str, value: &str, kind: CommandKind) -> String {
    let assignment = format!("{name}={value}");
    let quoted = quote_argument(&assignment, kind);
    format!("-v {quoted}")
}

/// First quote for the native Windows argv parser, doubling backslashes
/// before embedded quotes and the closing quote. Then protect the command
/// interpreter's metacharacters, including those quote marks themselves.
#[cfg(any(windows, test))]
fn quote_cmd_argument(text: &str, kind: CommandKind) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in text.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
        } else {
            quoted.extend(std::iter::repeat_n('\\', backslashes));
        }
        quoted.push(character);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    escape_cmd(&quoted, kind)
}

/// These quotes reach the executable rather than changing cmd's quoting
/// state. Batch files additionally double `%`; their header disables
/// delayed expansion so literal `!` is preserved too.
#[cfg(any(windows, test))]
fn escape_cmd(text: &str, kind: CommandKind) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        if character == '%' && matches!(kind, CommandKind::Batch) {
            escaped.push_str("%%");
            continue;
        }
        if character.is_whitespace() || "()[]%!^\"`<>&|;, *?".contains(character) {
            escaped.push('^');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUES: [&str; 10] = ["0, 1", "123 456", "123\t456", "", "Ridge", "two words", "a'b\"c", "$(printf expanded)`printf expanded`$HOME", "&|;<>()[]*?^!%PATH%", "C:\\a folder\\"];

    #[test]
    fn windows_arguments_escape_both_native_quotes_and_shell_characters() {
        assert_eq!(quote_cmd_argument("0, 1", CommandKind::Prompt), "^\"0^,^ 1^\"");
        assert_eq!(quote_cmd_argument("a\\\"&b\\", CommandKind::Batch), "^\"a\\\\\\^\"^&b\\\\^\"");
        assert_eq!(quote_cmd_argument("%PATH%!", CommandKind::Batch), "^\"%%PATH%%^!^\"");
    }

    #[cfg(unix)]
    #[test]
    fn shell_execution_preserves_every_variable_assignment() {
        for kind in [CommandKind::Prompt, CommandKind::Batch] {
            let mut script = String::from("printf '%s\\0'");
            let mut expected = Vec::new();
            for value in VALUES {
                let name = "Variable name";
                script.push(' ');
                script.push_str(&variable_argument(name, value, kind));
                expected.push("-v".to_string());
                expected.push(format!("{name}={value}"));
            }
            let output = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let arguments: Vec<&str> = output.stdout.split(|byte| *byte == 0).filter(|argument| !argument.is_empty()).map(|argument| std::str::from_utf8(argument).unwrap()).collect();
            assert_eq!(arguments, expected);
        }
    }

    /// A native argv recorder avoids testing cmd's built-in `echo` parser
    /// in place of the parser used by `nerthus-build.exe`.
    #[cfg(windows)]
    #[test]
    fn command_prompt_and_batch_execution_preserve_variable_assignments() {
        use std::io::Write;

        let directory = std::env::temp_dir().join(format!("nerthus command arguments {}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("arguments.rs");
        let program = directory.join("arguments.exe");
        let recorded = directory.join("arguments.bin");
        let probe = r#"fn main() {
            let file = std::env::var_os("NERTHUS_COMMAND_ARGUMENTS_OUTPUT").unwrap();
            let mut output = Vec::new();
            for argument in std::env::args().skip(1) {
                output.extend_from_slice(argument.as_bytes());
                output.push(0);
            }
            std::fs::write(file, output).unwrap();
        }"#;
        std::fs::write(&source, probe).unwrap();
        let status = std::process::Command::new("rustc").arg(&source).arg("-o").arg(&program).status().unwrap();
        assert!(status.success());

        for kind in [CommandKind::Prompt, CommandKind::Batch] {
            if recorded.exists() {
                std::fs::remove_file(&recorded).unwrap();
            }
            let mut line = quote_program(&program.to_string_lossy(), kind);
            let mut expected = Vec::new();
            for value in VALUES {
                line.push(' ');
                line.push_str(&variable_argument("Variable name", value, kind));
                expected.extend_from_slice(b"-v\0Variable name=");
                expected.extend_from_slice(value.as_bytes());
                expected.push(0);
            }
            let mut command = std::process::Command::new("cmd");
            command.args(["/D", "/Q", "/V:OFF"]);
            command.env("NERTHUS_COMMAND_ARGUMENTS_OUTPUT", &recorded);
            let output = match kind {
                CommandKind::Prompt => {
                    command.stdin(std::process::Stdio::piped());
                    command.stdout(std::process::Stdio::piped());
                    command.stderr(std::process::Stdio::piped());
                    let mut child = command.spawn().unwrap();
                    writeln!(child.stdin.take().unwrap(), "{line}\r\nexit\r").unwrap();
                    child.wait_with_output().unwrap()
                }
                CommandKind::Batch => {
                    let script = directory.join("commands.bat");
                    std::fs::write(&script, format!("@setlocal DisableDelayedExpansion\r\n{line}\r\n")).unwrap();
                    command.arg("/C").arg(&script);
                    command.output().unwrap()
                }
            };
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(std::fs::read(&recorded).unwrap(), expected);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
