//! Persistent prompt history.

use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result};

/// Reads NUL-delimited prompts from persistent storage.
pub(crate) fn read(path: &Path) -> Result<Vec<String>> {
    match fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("read prompt history {}", path.display())),
        Ok(contents) => contents
            .split(|byte| *byte == 0)
            .filter(|prompt| !prompt.is_empty())
            .map(|prompt| {
                String::from_utf8(prompt.to_vec()).context("prompt history contains invalid UTF-8")
            })
            .collect(),
    }
}

/// Adds a prompt once, preserving multiline input without shell parsing.
pub(crate) fn record(path: &Path, prompt: &str) -> Result<()> {
    if read(path)?.iter().any(|entry| entry == prompt) {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create prompt history directory {}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open prompt history {}", path.display()))?;
    file.write_all(prompt.as_bytes())
        .and_then(|_| file.write_all(&[0]))
        .with_context(|| format!("write prompt history {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn records_each_prompt_once() {
        let temp = tempfile_path();
        record(&temp, "first\nsecond").unwrap();
        record(&temp, "first\nsecond").unwrap();

        assert_eq!(read(&temp).unwrap(), ["first\nsecond"]);
        fs::remove_file(temp).unwrap();
    }

    fn tempfile_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shell-ai-history-{}", std::process::id()))
    }
}
