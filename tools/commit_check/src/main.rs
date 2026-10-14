use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::{self, Command};

const TYPES: &[&str] = &["feat", "fix", "refactor", "chore", "docs", "test", "ci"];
const FORBIDDEN_ATTRIBUTION: &[&str] = &[
    "co-authored-by",
    "generated-by",
    "assisted-by",
    "reviewed with claude code",
];

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("{}", error);
        process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [] => {
            let mut message = String::new();
            io::stdin()
                .read_to_string(&mut message)
                .map_err(|error| format!("could not read stdin: {}", error))?;
            check_named("commit message", &message)
        }
        [flag, path] if flag == "--edit" => {
            let message = fs::read_to_string(path)
                .map_err(|error| format!("could not read {}: {}", path, error))?;
            check_named(path, &message)
        }
        [flag, from, to] if flag == "--range" => check_range(from, to),
        _ => Err("usage: commit_check [--edit <path> | --range <from> <to>]".to_owned()),
    }
}

fn check_range(from: &str, to: &str) -> Result<(), String> {
    let range = format!("{}..{}", from, to);
    let output = Command::new("git")
        .args(["log", "--no-merges", "-z", "--format=%H%x00%B", &range])
        .output()
        .map_err(|error| format!("could not run git log: {}", error))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    let fields: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();
    for record in fields.chunks(2) {
        if record.len() < 2 || record[0].is_empty() {
            continue;
        }
        let revision = String::from_utf8_lossy(record[0]);
        let message = String::from_utf8_lossy(record[1]);
        check_named(&revision, &message)?;
    }

    Ok(())
}

fn check_named(name: &str, message: &str) -> Result<(), String> {
    validate(message).map_err(|error| format!("{}: {}", name, error))
}

fn validate(message: &str) -> Result<(), String> {
    let normalized = message.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let header = lines.next().unwrap_or_default();

    if header.chars().count() > 50 {
        return Err("header exceeds 50 characters".to_owned());
    }

    let conventional = strip_ticket(header)?;
    if conventional
        .chars()
        .any(|character| character.is_uppercase())
    {
        return Err("header must be lowercase after an optional ticket prefix".to_owned());
    }

    let (prefix, subject) = conventional
        .split_once(": ")
        .ok_or_else(|| "header must match <type>[scope][!]: <subject>".to_owned())?;
    if subject.is_empty() {
        return Err("subject must not be empty".to_owned());
    }
    if subject.ends_with('.') {
        return Err("subject must not end with a period".to_owned());
    }

    validate_prefix(prefix)?;

    for (index, line) in lines.enumerate() {
        if line.chars().count() > 72 {
            return Err(format!("line {} exceeds 72 characters", index + 2));
        }
    }

    let lowercase = normalized.to_lowercase();
    if FORBIDDEN_ATTRIBUTION
        .iter()
        .any(|attribution| lowercase.contains(attribution))
        || normalized.contains('🤖')
    {
        return Err("attribution footers and signatures are not permitted".to_owned());
    }

    Ok(())
}

fn strip_ticket(header: &str) -> Result<&str, String> {
    if !header.starts_with('[') {
        return Ok(header);
    }

    let end = header
        .find("] ")
        .ok_or_else(|| "ticket prefix must end with `] `".to_owned())?;
    let ticket = &header[1..end];
    let (project, number) = ticket
        .rsplit_once('-')
        .ok_or_else(|| "ticket prefix must match [PROJECT-123]".to_owned())?;
    if project.is_empty()
        || !project
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit()))
        || number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("ticket prefix must match [PROJECT-123]".to_owned());
    }

    Ok(&header[end + 2..])
}

fn validate_prefix(prefix: &str) -> Result<(), String> {
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (kind, scope) = match prefix.split_once('(') {
        Some((kind, scope)) => {
            let scope = scope
                .strip_suffix(')')
                .ok_or_else(|| "scope must end with `)`".to_owned())?;
            if scope.is_empty()
                || !scope.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_/-".contains(&byte)
                })
            {
                return Err("scope contains an invalid character".to_owned());
            }
            (kind, Some(scope))
        }
        None => (prefix, None),
    };

    if scope.is_none() && (kind.contains(')') || kind.contains('(')) {
        return Err("scope must match `(scope)`".to_owned());
    }
    if !TYPES.contains(&kind) {
        return Err(format!("type must be one of: {}", TYPES.join(", ")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate;
    #[test]
    fn accepts_conventional_messages() {
        for message in [
            "feat: add local replacement",
            "fix(parser): retain offsets",
            "[PROJ-1] feat!: change transport\n\nBREAKING CHANGE: update the adapter",
        ] {
            assert!(validate(message).is_ok(), "{message}");
        }
    }
    #[test]
    fn rejects_invalid_headers_and_attribution() {
        for message in [
            "Initial commit",
            "feat: Uppercase",
            "fix: trailing period.",
            "[proj-1] feat: lowercase ticket",
            "fix(BAD): scope",
            "feat: ",
            "fix: retained text\n\nCo-authored-by: robot",
            "fix: retained text\n\nGenerated-By: tool",
            "fix: retained text 🤖",
        ] {
            assert!(validate(message).is_err(), "{message}");
        }
        assert!(validate(&format!("feat: {}", "x".repeat(45))).is_err());
        assert!(validate(&format!("feat: valid\n\n{}", "x".repeat(73))).is_err());
    }
}
