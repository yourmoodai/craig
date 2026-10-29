mod prepare;
use savior::{MAX_TEXT_BYTES, ReplacementMode, RuntimeDetector, rewrite};
use serde::Deserialize;
use std::io::{self, Read};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    text: String,
}
fn run() -> Result<serde_json::Value, String> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    if ["inspect", "traces"].contains(&command.as_str()) {
        let executable = std::env::current_exe().map_err(|_| "inspector_unavailable")?;
        let worker = executable
            .parent()
            .ok_or("inspector_unavailable")?
            .join(if cfg!(windows) {
                "savior-review.exe"
            } else {
                "savior-review"
            });
        let status = std::process::Command::new(worker)
            .args(std::env::args().skip(1))
            .status()
            .map_err(|_| "inspector_unavailable")?;
        std::process::exit(status.code().unwrap_or(1));
    }
    if command == "prepare" && std::env::args().count() == 2 {
        return prepare::prepare()
            .map(|path| serde_json::json!({"version":1,"status":"ready","model_dir":path}));
    }
    if command == "help" || command == "--help" {
        return Ok(
            serde_json::json!({"commands":["rewrite", "doctor", "prepare", "inspect", "traces"]}),
        );
    }
    if !["rewrite", "doctor"].contains(&command.as_str()) || std::env::args().count() != 2 {
        return Err("invalid_command".into());
    }
    let dir = std::env::var("SAVIOR_MODEL_DIR").map_err(|_| "model_not_configured")?;
    let policy = std::env::var("SAVIOR_POLICY")
        .unwrap_or_else(|_| "offensive_spans".into())
        .parse()
        .map_err(|_| "invalid_options")?;
    let kind = std::env::var("SAVIOR_DETECTOR").unwrap_or_else(|_| "bert".into());
    let mut model =
        RuntimeDetector::load(dir, &kind, policy, Default::default()).map_err(|e| e.to_string())?;
    let threshold = std::env::var("SAVIOR_THRESHOLD")
        .unwrap_or_else(|_| model.threshold().to_string())
        .parse::<f32>()
        .map_err(|_| "invalid_options")?;
    if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
        return Err("invalid_options".into());
    }
    let strategy =
        std::env::var("SAVIOR_REPLACEMENT_STRATEGY").unwrap_or_else(|_| "fixed_phrase".into());
    let marker = std::env::var("SAVIOR_REPLACEMENT").ok();
    let replacement = ReplacementMode::new(Some(&strategy), marker).map_err(|e| e.to_string())?;
    if command == "doctor" {
        return Ok(
            serde_json::json!({"version":1,"status":"ready","detector":kind,"model":if kind == "bert" { savior::MODEL_ID } else { "local-lexical" },"revision":if kind == "bert" { Some(savior::MODEL_REVISION) } else { None },"policy":policy.name(),"threshold":threshold}),
        );
    }
    let mut input = String::new();
    io::stdin()
        .take((MAX_TEXT_BYTES * 8 + 1) as u64)
        .read_to_string(&mut input)
        .map_err(|_| "invalid_request")?;
    if input.len() > MAX_TEXT_BYTES * 8 {
        return Err("invalid_request".into());
    }
    let request: Request = serde_json::from_str(&input).map_err(|_| "invalid_request")?;
    if request.version != 1 {
        return Err("invalid_request".into());
    }
    let result =
        rewrite(&mut model, &request.text, threshold, &replacement).map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"version":1,"status":if result.replacement_count == 0 {"unchanged"} else {"rewritten"},"text":result.text,"replacement_count":result.replacement_count}),
    )
}
fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(code) => {
            println!(
                "{}",
                serde_json::json!({"version":1,"status":"error","code":code})
            );
            if std::env::var("LOG_LEVEL").as_deref() != Ok("silent") {
                eprintln!("savior: {code}");
            }
            std::process::exit(1);
        }
    }
}
