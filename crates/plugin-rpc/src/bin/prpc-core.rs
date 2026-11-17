//! Run the six core rules against one plugin, from the command line.
//!
//! This is the part of §12 the mechanism can do for anybody: given a profile spec and a plugin
//! manifest, it reports `spawn`, `protocol-version`, `handshake-schema`, `unknown-method`,
//! `framing-intact`, and `process-alive`, with no profile code in between. A plugin author in any
//! language runs this before writing a plugin's own suite, and a host that has its own runner can
//! diff against it.
//!
//! With `--suite`, the command also runs the profile's own suite program (§12 item 6): it writes
//! the profile spec, the manifest, and the schemas the spec names into a directory, runs
//! `<argv> --out <dir>`, and reads the verdicts back. The two halves print as one report, core
//! rules first.
//!
//! With `--suite`, every rule the profile gives a scripted defect gets that defect run (§12 item
//! 4): the plugin is started again with the defect in place, and the rule keeps its unverified
//! mark unless the rules the defect names really do report a violation.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use plugin_rpc::conformance::{
    core, run_defect, run_suite, CoreCheck, DefectRun, Manifest, ProfileSpec,
};

const USAGE: &str = "usage: prpc-core <profile.json> <manifest.json> [--suite] [--env NAME=value] [--cwd DIR] [--deadline-ms N]";

fn main() -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut cwd: Option<PathBuf> = None;
    let mut deadline_ms: Option<u64> = None;
    let mut suite = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--env" => {
                let Some(pair) = args.next() else {
                    return fail("--env wants NAME=value");
                };
                let Some((name, value)) = pair.split_once('=') else {
                    return fail(&format!("`{pair}` is not NAME=value"));
                };
                env.push((name.to_string(), value.to_string()));
            }
            "--cwd" => {
                let Some(dir) = args.next() else {
                    return fail("--cwd wants a directory");
                };
                cwd = Some(PathBuf::from(dir));
            }
            "--deadline-ms" => {
                let Some(ms) = args.next() else {
                    return fail("--deadline-ms wants a number");
                };
                match ms.parse() {
                    Ok(ms) => deadline_ms = Some(ms),
                    Err(_) => return fail(&format!("`{ms}` is not a number of milliseconds")),
                }
            }
            "--suite" => suite = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => return fail(&format!("unknown option `{other}`")),
            other => positional.push(other.to_string()),
        }
    }
    if positional.len() != 2 {
        return fail("a profile spec and a manifest are both required");
    }

    let profile = match ProfileSpec::load(&positional[0]) {
        Ok(profile) => profile,
        Err(e) => return fail(&format!("{}: {e}", positional[0])),
    };
    let manifest = match Manifest::load(&positional[1]) {
        Ok(manifest) => manifest,
        Err(e) => return fail(&format!("{}: {e}", positional[1])),
    };
    let deadline =
        Duration::from_millis(deadline_ms.unwrap_or_else(|| profile.deadline_ms(Some(&manifest))));
    let check = CoreCheck {
        profile: &profile,
        command: manifest.command.clone(),
        env,
        cwd,
        deadline,
    };
    let mut report = core::run(&check);
    if suite {
        let spec_dir = PathBuf::from(&positional[0])
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let out_dir = std::env::temp_dir().join(format!("prpc-suite-{}", std::process::id()));
        eprintln!("suite directory: {}", out_dir.display());
        match run_suite(&profile, &manifest, &spec_dir, &out_dir, &check.env) {
            Ok(ran) => report.verdicts.extend(ran.verdicts),
            Err(e) => report.suite_error = Some(e.to_string()),
        }
        // §12 item 4: the profile's `expectViolation` is a claim about a run nobody else has
        // done, so this does the run. A rule whose defect does not fire is reported as unverified,
        // the same mark a profile gets for admitting it has no defect, because a pass on it is
        // not proof that the rule can detect the failure it targets.
        for rule in &profile.conformance.rules {
            // The suite reports a scripted rule as verified on the profile's word alone, so this
            // is where the word is either made good or taken back.
            let verdict = report
                .verdicts
                .iter_mut()
                .find(|verdict| verdict.rule == rule.id);
            match run_defect(rule, &profile, &manifest, &spec_dir, &out_dir, &check) {
                DefectRun::Fired => {
                    if let Some(verdict) = verdict {
                        verdict.unverified = false;
                    }
                }
                DefectRun::NotScripted => {}
                DefectRun::Silent { missing } => {
                    eprintln!(
                        "note  {}: its scripted defect left {} still passing",
                        rule.id,
                        missing.join(", ")
                    );
                    if let Some(verdict) = verdict {
                        verdict.unverified = true;
                    }
                }
                DefectRun::Failed { detail } => {
                    eprintln!(
                        "note  {}: its scripted defect could not be run: {detail}",
                        rule.id
                    );
                    if let Some(verdict) = verdict {
                        verdict.unverified = true;
                    }
                }
            }
        }
    }
    print!("{}", report.render());
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}\n{USAGE}");
    ExitCode::FAILURE
}
