mod prepare;
use craig::{FixedMarker, MAX_TEXT_BYTES, RuntimeDetector, rewrite};
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
    if command == "prepare" && std::env::args().count() == 2 {
        return prepare::prepare()
            .map(|path| serde_json::json!({"version":1,"status":"ready","model_dir":path}));
    }
    if command == "help" || command == "--help" {
        return Ok(serde_json::json!({"commands":["rewrite", "doctor"]}));
    }
    if !["rewrite", "doctor"].contains(&command.as_str()) || std::env::args().count() != 2 {
        return Err("invalid_command".into());
    }
    let dir = std::env::var("CRAIG_MODEL_DIR").map_err(|_| "model_not_configured")?;
    let policy = std::env::var("CRAIG_POLICY")
        .unwrap_or_else(|_| "offensive_spans".into())
        .parse()
        .map_err(|_| "invalid_options")?;
    let kind = std::env::var("CRAIG_DETECTOR").unwrap_or_else(|_| "bert".into());
    let mut model =
        RuntimeDetector::load(dir, &kind, policy, Default::default()).map_err(|e| e.to_string())?;
    let threshold = std::env::var("CRAIG_THRESHOLD")
        .unwrap_or_else(|_| model.threshold().to_string())
        .parse::<f32>()
        .map_err(|_| "invalid_options")?;
    if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
        return Err("invalid_options".into());
    }
    let marker =
        FixedMarker::new(std::env::var("CRAIG_REPLACEMENT").unwrap_or_else(|_| "ya ya ya".into()))
            .map_err(|e| e.to_string())?;
    if command == "doctor" {
        return Ok(
            serde_json::json!({"version":1,"status":"ready","detector":kind,"model":if kind == "bert" { craig::MODEL_ID } else { "local-lexical" },"revision":if kind == "bert" { Some(craig::MODEL_REVISION) } else { None },"policy":policy.name(),"threshold":threshold}),
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
        rewrite(&mut model, &request.text, threshold, &marker).map_err(|e| e.to_string())?;
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
                eprintln!("craig: {code}");
            }
            std::process::exit(1);
        }
    }
}
