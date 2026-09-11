//! Shell-independent prompt completion and external editing.

use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(super) fn edit_external(text: &str) -> Result<String> {
    let editor = std::env::var("EDITOR").context("set EDITOR to use external editing")?;
    let args = shell_words::split(&editor).context("EDITOR has invalid quoting")?;
    let tty = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")?;
    edit_with_command(text, &args, tty)
}

fn edit_with_command(text: &str, args: &[String], tty: fs::File) -> Result<String> {
    let Some(program) = args.first() else {
        bail!("EDITOR is empty")
    };
    let mut file = tempfile::Builder::new()
        .prefix("shell-ai-")
        .suffix(".txt")
        .tempfile()?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    // Do not interpret the editor command or the prompt with a shell.
    let status = Command::new(program)
        .args(&args[1..])
        .arg(file.path())
        .stdin(Stdio::from(tty.try_clone()?))
        .stdout(Stdio::from(tty.try_clone()?))
        .stderr(Stdio::from(tty))
        .status()
        .context("could not start EDITOR")?;
    if !status.success() {
        bail!("EDITOR exited with {status}; prompt was not changed")
    }
    fs::read_to_string(file.path()).context("could not read the edited prompt")
}

pub(super) fn complete(text: &str, cursor: usize) -> Result<Option<String>> {
    let cwd = std::env::current_dir()?;
    let paths: Vec<_> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    Ok(complete_in(text, cursor, &cwd, &paths, home.as_deref()))
}

fn complete_in(
    text: &str,
    cursor: usize,
    cwd: &Path,
    paths: &[std::path::PathBuf],
    home: Option<&Path>,
) -> Option<String> {
    let before = text.get(..cursor)?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let token = &before[start..];
    // Completion is literal, not shell evaluation. Do not guess inside shell expressions.
    if token.is_empty() || token.contains(['\'', '"', '`', '$', '\\']) {
        return None;
    }
    let mut candidates = BTreeSet::new();
    let (directory, prefix) = token
        .rsplit_once('/')
        .map_or(("", token), |(d, p)| (&token[..d.len() + 1], p));
    let base = if let Some(relative) = directory.strip_prefix("~/") {
        home?.join(relative)
    } else {
        cwd.join(directory)
    };
    add_entries(&base, prefix, directory, false, &mut candidates);
    if directory.is_empty() {
        for path in paths {
            add_entries(&cwd.join(path), token, "", true, &mut candidates);
        }
    }
    let mut iter = candidates.iter();
    let first = iter.next()?;
    let mut common = first.clone();
    for candidate in iter {
        let bytes = common
            .chars()
            .zip(candidate.chars())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum();
        common.truncate(bytes);
    }
    common
        .strip_prefix(token)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn add_entries(
    base: &Path,
    prefix: &str,
    directory: &str,
    commands: bool,
    candidates: &mut BTreeSet<String>,
) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.starts_with(prefix)
            || (name.starts_with('.') && !prefix.starts_with('.'))
            || name.chars().any(char::is_control)
        {
            continue;
        }
        let Ok(metadata) = fs::metadata(entry.path()) else {
            continue;
        };
        if commands {
            if !metadata.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
        }
        candidates.insert(format!(
            "{directory}{name}{}",
            if metadata.is_dir() { "/" } else { "" }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn editor_receives_a_private_file_and_arguments_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("editor script");
        let record = root.path().join("path");
        fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' \"$2\" > \"$1\"\nprintf 'changed\\ntext' > \"$2\"\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let args =
            shell_words::split(&format!("'{}' '{}'", script.display(), record.display())).unwrap();
        let tty = || {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")
                .unwrap()
        };
        assert_eq!(
            edit_with_command("original", &args, tty()).unwrap(),
            "changed\ntext"
        );
        let path = fs::read_to_string(&record).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(edit_with_command("original", &["/usr/bin/false".into()], tty()).is_err());
        assert!(edit_with_command("original", &["/missing/editor".into()], tty()).is_err());
        assert!(edit_with_command("original", &[], tty()).is_err());
    }

    #[test]
    fn completes_paths_common_prefix_and_unicode_without_touching_suffix() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();
        fs::write(root.path().join("résumé"), "").unwrap();
        fs::write(root.path().join("report-one"), "").unwrap();
        fs::write(root.path().join("report-two"), "").unwrap();
        assert_eq!(
            complete_in("open fol later", 8, root.path(), &[], None),
            Some("der/".into())
        );
        assert_eq!(
            complete_in("ré", 3, root.path(), &[], None),
            Some("sumé".into())
        );
        assert_eq!(
            complete_in("rep", 3, root.path(), &[], None),
            Some("ort-".into())
        );
        assert_eq!(
            complete_in("~/fol", 5, root.path(), &[], Some(root.path())),
            Some("der/".into())
        );
        assert_eq!(complete_in("$HOME/fol", 9, root.path(), &[], None), None);
        assert_eq!(complete_in("absent", 6, root.path(), &[], None), None);
        assert_eq!(complete_in("report-", 7, root.path(), &[], None), None);
        let absolute = format!("{}/fol", root.path().display());
        assert_eq!(
            complete_in(&absolute, absolute.len(), root.path(), &[], None),
            Some("der/".into())
        );
    }
    #[test]
    #[cfg(unix)]
    fn completes_only_executable_commands_from_path() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("command-test"), "").unwrap();
        fs::write(bin.join("command-data"), "").unwrap();
        fs::set_permissions(bin.join("command-test"), fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(bin.join("command-test"), bin.join("linked-command")).unwrap();
        std::os::unix::fs::symlink(&bin, root.path().join("linked-directory")).unwrap();
        assert_eq!(
            complete_in(
                "run command-",
                12,
                root.path(),
                std::slice::from_ref(&bin),
                None
            ),
            Some("test".into())
        );
        assert_eq!(
            complete_in("linked-c", 8, root.path(), &[bin], None),
            Some("ommand".into())
        );
        assert_eq!(
            complete_in("linked-d", 8, root.path(), &[], None),
            Some("irectory/".into())
        );
    }
}
