mod corpus;
mod evaluation;
mod judge;
mod setup;
mod training;

use clap::{Parser, Subcommand};
use savior::{ModelDetector, Policy, ReplacementMode, artifact_digest};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::Instant,
};

type LabResult<T> = Result<T, String>;
fn fail<T>(code: &str) -> LabResult<T> {
    Err(code.into())
}
fn new_file(path: &Path) -> LabResult<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "output_exists_or_unavailable".into())
}
fn write_json(path: &Path, value: &impl Serialize) -> LabResult<()> {
    serde_json::to_writer_pretty(new_file(path)?, value).map_err(|_| "write_failed".into())
}
#[derive(Parser)]
#[command(about = "offline model evaluation and classifier training")]
struct Args {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    FetchJudge {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        server_binary: PathBuf,
    },
    Bootstrap {
        #[arg(long)]
        output: PathBuf,
    },
    Evaluate {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long, default_value = "offensive_spans")]
        policy: String,
        #[arg(long, default_value = "test")]
        split: String,
        #[arg(long)]
        threshold: Option<f32>,
        #[arg(long, default_value = "fixed_phrase")]
        replacement_strategy: String,
        #[arg(long)]
        marker: Option<String>,
        #[arg(long)]
        judge: Option<PathBuf>,
        #[arg(long, default_value_t = 100)]
        sample: usize,
        #[arg(long)]
        baseline: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    Calibrate {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long, default_value = "offensive_spans")]
        policy: String,
        #[arg(long)]
        output: PathBuf,
    },
    Train {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        policy: String,
        #[arg(long, default_value_t = 10)]
        epochs: usize,
        #[arg(long)]
        output: PathBuf,
    },
    Package {
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
fn hash(path: &Path) -> LabResult<String> {
    artifact_digest(path).map_err(|_| "artifact_unavailable".into())
}
fn identity(model: &Path) -> LabResult<serde_json::Value> {
    let mut files = BTreeMap::new();
    for name in training::BASE_FILES {
        files.insert(name.to_string(), hash(&model.join(name))?);
    }
    if model.join("manifest.json").exists() {
        files.insert("manifest.json".into(), hash(&model.join("manifest.json"))?);
        files.insert("training.json".into(), hash(&model.join("training.json"))?);
        files.insert(
            "head.safetensors".into(),
            hash(&model.join("head.safetensors"))?,
        );
    }
    Ok(
        serde_json::json!({"base_model":savior::MODEL_ID,"base_revision":savior::MODEL_REVISION,"files":files}),
    )
}
fn read_json(path: &Path) -> LabResult<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).map_err(|_| "report_unavailable")?)
        .map_err(|_| "invalid_report".into())
}
fn rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // Safety: getrusage writes to valid storage; assume_init requires a successful return.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let bytes = unsafe { usage.assume_init() }.ru_maxrss as u64;
            return Some(if cfg!(target_os = "macos") {
                bytes
            } else {
                bytes * 1024
            });
        }
    }
    None
}
fn compare(report: &serde_json::Value, baseline: &serde_json::Value) -> serde_json::Value {
    let compatible = [
        "version",
        "corpus_sha256",
        "policy",
        "split",
        "replacement_strategy",
        "machine",
        "runner_sha256",
        "rubric_sha256",
    ]
    .iter()
    .all(|k| report[k] == baseline[k]);
    if !compatible {
        return serde_json::json!({"status":"inconclusive","reason":"baseline_mismatch"});
    }
    let current = &report["summary"]["overall"];
    let previous = &baseline["summary"]["overall"];
    let number = |v: &serde_json::Value| v.as_f64().unwrap_or(f64::NAN);
    let improved = number(&current["span_f1"]) > number(&previous["span_f1"]);
    let clean = number(&current["counts"]["unnecessary_replacements"])
        <= number(&previous["counts"]["unnecessary_replacements"]);
    let performance = number(&report["summary"]["warm_p95_ms"])
        <= 1.1 * number(&baseline["summary"]["warm_p95_ms"])
        && number(&report["detector_peak_rss_bytes"])
            <= 1.1 * number(&baseline["detector_peak_rss_bytes"]);
    let deterministic = current["counts"]["deterministic_failures"] == 0
        && current["counts"]["unexpected_errors"] == 0;
    let reviewed = report["labels_reviewed"] == true && baseline["labels_reviewed"] == true;
    let evidence = report["machine_declared"] == true
        && baseline["machine_declared"] == true
        && report["optimized"] == true
        && baseline["optimized"] == true
        && report["judge_calibration"]["status"] == "calibrated_on_this_corpus"
        && number(&current["counts"]["examples"]) >= 30.0;
    let status = if !deterministic || !clean {
        "fail"
    } else if !reviewed || !improved || !performance || !evidence || report["split"] != "test" {
        "inconclusive"
    } else {
        "eligible_for_release_review"
    };
    serde_json::json!({"status":status,"held_out_f1_improved":improved,"clean_instruction_nonregression":clean,"performance_within_tolerance":performance,
        "deterministic_checks_pass":deterministic,"labels_reviewed":reviewed,"requires_existing_regressions_and_reference_parity":true})
}
fn run(args: Args) -> LabResult<()> {
    match args.command {
        Commands::FetchJudge {
            directory,
            server_binary,
        } => {
            setup::fetch_judge(&directory, &server_binary)?;
            println!("judge_artifact_verified");
        }
        Commands::Bootstrap { output } => {
            corpus::bootstrap(&output)?;
            println!("corpus_created");
        }
        Commands::Train {
            model,
            corpus,
            policy,
            epochs,
            output,
        } => {
            let rows = corpus::read(&corpus)?;
            let report = training::train(
                &model,
                &rows,
                &hash(&corpus)?,
                policy.parse().map_err(|_| "invalid_policy")?,
                &output,
                epochs,
            )?;
            println!(
                "{}",
                serde_json::to_string(&report).map_err(|_| "write_failed")?
            );
        }
        Commands::Calibrate {
            model,
            corpus,
            policy,
            output,
        } => {
            let policy = policy.parse().map_err(|_| "invalid_policy")?;
            let rows = corpus::read(&corpus)?;
            // The offensive baseline permits directed-abuse calibration without a trained head.
            let mut detector = ModelDetector::load_policy(
                &model,
                if model.join("manifest.json").exists() {
                    policy
                } else {
                    Policy::OffensiveSpans
                },
            )
            .map_err(|_| "model_unavailable")?;
            let mut report = evaluation::calibrate(&mut detector, &rows, policy)?;
            report["model"] = identity(&model)?;
            report["corpus_sha256"] = hash(&corpus)?.into();
            write_json(&output, &report)?;
            println!("calibration_written");
        }
        Commands::Evaluate {
            model,
            corpus,
            policy,
            split,
            threshold,
            replacement_strategy,
            marker,
            judge: judge_config,
            sample,
            baseline,
            output,
        } => {
            if !["train", "validation", "test"].contains(&split.as_str()) || sample > 100 {
                return fail("invalid_evaluation_options");
            }
            let policy: Policy = policy.parse().map_err(|_| "invalid_policy")?;
            let replacement = ReplacementMode::new(Some(&replacement_strategy), marker)
                .map_err(|_| "invalid_options")?;
            let runner_hash = hash(&std::env::current_exe().map_err(|_| "runner_unavailable")?)?;
            let corpus_hash = hash(&corpus)?;
            let rows = corpus::read(&corpus)?;
            let selected: Vec<_> = rows.iter().filter(|r| r.split == split).collect();
            if selected.is_empty() {
                return fail("empty_evaluation_split");
            }
            let model_identity = identity(&model)?;
            let start = Instant::now();
            let baseline_only =
                !model.join("manifest.json").exists() && policy == Policy::DirectedAbuse;
            let mut detector = ModelDetector::load_policy(
                &model,
                if baseline_only {
                    Policy::OffensiveSpans
                } else {
                    policy
                },
            )
            .map_err(|_| "model_unavailable")?;
            let load_ms = start.elapsed().as_secs_f64() * 1000.0;
            let threshold = threshold.unwrap_or(detector.threshold());
            if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
                return fail("invalid_threshold");
            }
            use savior::Detector;
            detector
                .detect("Please preserve the test.")
                .map_err(|_| "warmup_failed")?;
            let outcomes =
                evaluation::run(&mut detector, &selected, policy, threshold, &replacement)?;
            let peak = rss_bytes();
            drop(detector);
            let sampled = judge::sample(&outcomes, sample);
            let mut judgments = Vec::new();
            let mut review = Vec::new();
            let mut judge_provenance = serde_json::Value::Null;
            let mut judge_error = None;
            let mut judge = match judge_config.as_ref().map(|p| judge::Judge::start(p)) {
                Some(Ok(j)) => {
                    judge_provenance = j.provenance.clone();
                    Some(j)
                }
                Some(Err(e)) => {
                    judge_error = Some(e);
                    None
                }
                None => None,
            };
            let mut reviewed_count = 0;
            let mut agreements = 0;
            let mut reviewed_failures = 0;
            let mut judge_errors = 0;
            let mut cohorts: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
            for (row, outcome) in selected.iter().zip(&outcomes) {
                let representative = sampled.contains(&outcome.id);
                let failure = outcome.metrics.exact_outcomes == 0
                    || outcome.metrics.deterministic_failures > 0;
                if !representative && !failure {
                    continue;
                }
                let verdict = judge.as_mut().map(|j| j.score(row, outcome, policy));
                if judge_config.is_some() && !matches!(&verdict, Some(Ok(_))) {
                    judge_errors += 1;
                }
                let unresolved = match &verdict {
                    Some(Ok(v)) => {
                        v.policy == judge::Verdict::Uncertain
                            || v.preservation == judge::Verdict::Uncertain
                    }
                    _ => true,
                };
                let judge_pass = matches!(&verdict,Some(Ok(v)) if v.policy==judge::Verdict::Pass && v.preservation==judge::Verdict::Pass);
                let bucket = cohorts
                    .entry(if representative {
                        "representative"
                    } else {
                        "failure_enriched"
                    })
                    .or_default();
                bucket[if unresolved {
                    2
                } else if judge_pass {
                    0
                } else {
                    1
                }] += 1;
                if row.reviewed && representative && !unresolved {
                    reviewed_count += 1;
                    reviewed_failures += usize::from(failure);
                    agreements += usize::from(judge_pass == !failure);
                }
                if failure || unresolved || !judge_pass {
                    review.push(serde_json::json!({"id":row.id,"deterministic_failure":outcome.metrics.deterministic_failures>0,"reference_disagreement":failure,"judge_unresolved":unresolved}));
                }
                judgments.push(serde_json::json!({"id":row.id,"cohort":if representative {"representative"}else{"failure_enriched"},
                    "verdict":match verdict {Some(Ok(v))=>serde_json::to_value(v).map_err(|_|"write_failed")?,Some(Err(code))=>serde_json::json!({"status":"unresolved","code":code}),None=>serde_json::json!({"status":"unresolved","code":"judge_not_available"})}}));
            }
            let machine = std::env::var("SAVIOR_BENCH_MACHINE")
                .unwrap_or_else(|_| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH));
            let mut report = serde_json::json!({"version":1,"kind":"empirical_evaluation","status":"inconclusive","policy":policy,"split":split,"replacement_strategy":replacement.name(),"threshold":threshold,
                "threshold_only_baseline":baseline_only,"model":model_identity,"corpus_sha256":corpus_hash,"seed":42,"sample_limit":sample,
                "machine":machine,"machine_declared":std::env::var("SAVIOR_BENCH_MACHINE").is_ok(),"runner_sha256":runner_hash,
                "optimized":!cfg!(debug_assertions),"labels_reviewed":selected.iter().all(|r|r.reviewed),"cold_load_ms":load_ms,"detector_peak_rss_bytes":peak,
                "summary":evaluation::summary(&outcomes),"outcomes":outcomes,"judge":judge_provenance,"judge_error":judge_error,
                "rubric_sha256":format!("{:x}",Sha256::digest(judge::RUBRIC.as_bytes())),"judgments":judgments,"review_queue":review,"judge_cohorts":cohorts.into_iter().map(|(k,v)|(k,serde_json::json!({"pass":v[0],"fail":v[1],"unresolved":v[2]}))).collect::<BTreeMap<_,_>>(),
                "judge_calibration":{"reviewed_representative_examples":reviewed_count,"reviewed_failures":reviewed_failures,"agreements":agreements,"agreement_wilson_95":evaluation::wilson(agreements,reviewed_count),"status":if reviewed_count>=30 && reviewed_failures>=5 && reviewed_count-reviewed_failures>=5 && evaluation::wilson(agreements,reviewed_count)[0]>=0.8 {"calibrated_on_this_corpus"}else{"inconclusive"}}});
            if let Some(baseline) = baseline {
                report["comparison"] = compare(&report, &read_json(&baseline)?);
            }
            let failed = outcomes
                .iter()
                .any(|o| o.metrics.deterministic_failures > 0 || o.metrics.unexpected_errors > 0);
            write_json(&output, &report)?;
            println!("evaluation_written");
            if failed {
                return fail("evaluation_failed");
            }
            if judge_errors > 0 || judge_config.is_some() && judge.is_none() {
                return fail("judge_evaluation_incomplete");
            }
        }
        Commands::Package {
            model,
            report,
            output,
        } => {
            let evaluation = read_json(&report)?;
            if evaluation["model"] != identity(&model)? || evaluation["split"] != "test" {
                return fail("package_report_mismatch");
            }
            if evaluation["summary"]["overall"]["counts"]["deterministic_failures"] != 0
                || evaluation["summary"]["overall"]["counts"]["unexpected_errors"] != 0
            {
                return fail("package_failed_evaluation");
            }
            let manifest = savior::ModelManifest::read(&model).map_err(|_| "invalid_candidate")?;
            if evaluation["policy"]
                != serde_json::to_value(manifest.policy).map_err(|_| "invalid_policy")?
                || evaluation["threshold"]
                    .as_f64()
                    .is_none_or(|t| (t - manifest.threshold as f64).abs() > 0.000001)
            {
                return fail("package_policy_mismatch");
            }
            ModelDetector::load_policy(&model, manifest.policy).map_err(|_| "invalid_candidate")?;
            let training = read_json(&model.join("training.json"))?;
            if training["encoder_unchanged"] != true
                || training["classifier_changed"] != true
                || training["export_probability_max_error"]
                    .as_f64()
                    .is_none_or(|n| !n.is_finite() || n > 0.0001)
            {
                return fail("missing_training_evidence");
            }
            std::fs::create_dir(&output).map_err(|_| "output_exists_or_unavailable")?;
            for name in training::BASE_FILES.into_iter().chain([
                "head.safetensors",
                "manifest.json",
                "training.json",
            ]) {
                std::fs::copy(model.join(name), output.join(name))
                    .map_err(|_| "package_copy_failed")?;
            }
            std::fs::copy("models/LICENSE-APACHE-2.0", output.join("LICENSE"))
                .map_err(|_| "missing_model_license")?;
            std::fs::copy("models/NOTICE", output.join("NOTICE"))
                .map_err(|_| "missing_model_notice")?;
            std::fs::copy(report, output.join("evaluation.json"))
                .map_err(|_| "package_copy_failed")?;
            write_json(
                &output.join("release.json"),
                &serde_json::json!({"status":"candidate_not_promoted","policy":manifest.policy,
                "modifications":"classifier head fine-tuned; encoder and tokenizer unchanged","license":"Apache-2.0","comparison":evaluation["comparison"],
                "requires_reviewed_labels_and_existing_regression_checks":true}),
            )?;
            println!("candidate_packaged_not_promoted");
        }
    }
    Ok(())
}
fn main() {
    if let Err(code) = run(Args::parse()) {
        eprintln!("savior-lab: {code}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_evidence_cannot_qualify_a_release() {
        assert_ne!(
            compare(&serde_json::json!({}), &serde_json::json!({}))["status"],
            "eligible_for_release_review"
        );
    }
    #[test]
    fn outputs_do_not_overwrite_previous_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        write_json(&path, &serde_json::json!({"version":1})).unwrap();
        assert!(write_json(&path, &serde_json::json!({"version":2})).is_err());
    }
}
