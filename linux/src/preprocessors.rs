//! Save preprocessors: the same contract as the macOS shell. Each
//! command reads the document on stdin and writes the whole document
//! to stdout; the chain pipes one into the next, and any failure —
//! non-zero exit, empty output for a non-empty document, or a hang —
//! applies nothing. `{path}` and `{filename}` in a command expand to
//! the document's absolute path and bare name.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Failure {
    pub command: String,
    pub details: String,
}

/// How long one command may take before it counts as hung.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `commands` over `text` in order, in `directory` (the project
/// root, so tools pick up their own config files). Blocking.
pub fn run(
    commands: &[String],
    text: &str,
    directory: Option<&Path>,
    document_path: Option<&str>,
) -> Result<String, Failure> {
    let path = document_path.unwrap_or("");
    let filename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut current = text.to_owned();
    for command in commands {
        let words: Vec<String> = split(command)
            .into_iter()
            .map(|word| word.replace("{path}", path).replace("{filename}", &filename))
            .collect();
        let Some((program, arguments)) = words.split_first() else {
            continue;
        };
        match pipe(&current, program, arguments, directory) {
            Ok(output) => {
                if output.is_empty() && !current.is_empty() {
                    return Err(Failure {
                        command: command.clone(),
                        details: "the command produced no output; save preprocessors \
                                  must write the document to standard output"
                            .into(),
                    });
                }
                current = output;
            }
            Err(details) => {
                return Err(Failure {
                    command: command.clone(),
                    details,
                });
            }
        }
    }
    Ok(current)
}

fn pipe(
    input: &str,
    program: &str,
    arguments: &[String],
    directory: Option<&Path>,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = directory.filter(|d| d.is_dir()) {
        command.current_dir(directory);
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;

    // Feed on a thread: a document larger than the pipe buffer
    // deadlocks if written and read from the same one.
    let mut stdin = child.stdin.take().expect("stdin piped");
    let payload = input.as_bytes().to_vec();
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&payload);
    });
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        let mut errors = Vec::new();
        let _ = stderr.read_to_end(&mut errors);
        (output, errors)
    });

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = feeder.join();
                let _ = reader.join();
                return Err(format!("timed out after {}s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error.to_string()),
        }
    };
    let _ = feeder.join();
    let (output, errors) = reader.join().unwrap_or_default();

    if !status.success() {
        let tail = String::from_utf8_lossy(&errors);
        let tail = tail.trim();
        return Err(if tail.is_empty() {
            format!("exited with status {status}")
        } else {
            tail.chars().take(2000).collect()
        });
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Splits a command line into words, honoring single and double quotes
/// — enough shell for formatter invocations, without a shell.
fn split(command_line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_content = false;
    for character in command_line.chars() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
        } else if character == '"' || character == '\'' {
            quote = Some(character);
            has_content = true;
        } else if character == ' ' || character == '\t' {
            if has_content {
                words.push(std::mem::take(&mut current));
            }
            has_content = false;
        } else {
            current.push(character);
            has_content = true;
        }
    }
    if has_content {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_pipes_and_substitutes() {
        let commands = vec!["tr a-z A-Z".to_string(), "sed -e s|^|{filename}:|".to_string()];
        let result = run(&commands, "hello\n", None, Some("/tmp/Makefile")).unwrap();
        assert_eq!(result, "Makefile:HELLO\n");
    }

    #[test]
    fn failure_reports_the_command() {
        let commands = vec!["false".to_string()];
        let error = run(&commands, "x", None, None).unwrap_err();
        assert_eq!(error.command, "false");
    }

    #[test]
    fn quoted_words_stay_whole() {
        assert_eq!(split("a 'b c' d"), vec!["a", "b c", "d"]);
    }
}
