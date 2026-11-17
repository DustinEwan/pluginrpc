//! Run the six core rules against one plugin, from the command line.
//!
//! This is the part of §12 the mechanism can do for anybody: given a profile spec and a plugin
//! manifest, it reports `spawn`, `protocol-version`, `handshake-schema`, `unknown-method`,
//! `framing-intact`, and `process-alive`, with no profile code in between. A plugin author in any
//! language runs this before writing a plugin's own suite, and a host that has its own runner can
//! diff against it.
//!
//! The profile's own rules are not run here: they belong to the profile's suite program (§12
//! item 6), which this command does not invoke.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use plugin_rpc::conformance::{core, CoreCheck, Manifest, ProfileSpec};

const USAGE: &str = "usage: prpc-core <profile.json> <manifest.json> [--env NAME=value] [--cwd DIR] [--deadline-ms N]";

fn main() -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut cwd: Option<PathBuf> = None;
    let mut deadline_ms: Option<u64> = None;
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
    let report = core::run(&CoreCheck {
        profile: &profile,
        command: manifest.command.clone(),
        env,
        cwd,
        deadline,
    });
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
