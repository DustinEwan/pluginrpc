//! The conformance procedure of `prpc/1` (spec §12), as a reference implementation.
//!
//! Two halves, and they are deliberately different shapes. The **core rules** are six checks that
//! name no method of any profile, so this module runs them for every profile from two files: the
//! profile's own spec and the plugin's manifest. The **profile's own rules** are a program the
//! profile ships, run under the contract of §12(6), because a profile's rules need folds across
//! responses, cross-response comparisons, and derived inputs — arbitrary logic — and the mechanism
//! is not a programming language. A host written in Go and a suite written in Node get the same
//! report because both read the same two files and honour the same `verdicts.json`.
//!
//! The normative text is `spec/spec.md`; the doc comments cite the section each rule implements.
//! Where this module checks a constraint by hand rather than by running a JSON Schema validator, the
//! comment says which schema the check stands for, because a schema the reference types do not
//! enforce is a schema that will drift (`crates/plugin-rpc/tests/envelope_parity.rs` is the same
//! argument for the envelopes).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod core;

pub use core::CoreCheck;

/// The mechanism revision a profile spec must name, or this loader refuses it (§13). A profile
/// spec for a revision this host does not implement is refused at load time rather than run and
/// misreported.
pub const MECHANISM: &str = "prpc/1";

/// The deadline one driven invocation is given when neither the profile spec nor the manifest says
/// (§9, §11).
pub const DEFAULT_DEADLINE_MS: u64 = 1000;

/// The core rules every profile inherits, read from `spec/core-rules.json` — the one file that
/// fixes their ids, their order, and the expectation each states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreRule {
    pub id: String,
    pub expectation: String,
}

/// The core rules, in the order a suite runs them (§12).
///
/// A suite reads them from the committed file rather than from a Rust array, because the file is
/// what a profile author reads and what a suite in any language must agree with. `PLUGIN_RPC_SPEC`
/// points at another `spec/` directory; the embedded copy is the fallback so a published crate is
/// self-contained.
///
/// That copy lives in this package rather than being read from `spec/`, because cargo ships only
/// files under a package root, and `spec/` is above two of them. CI compares the two byte for byte,
/// so the file the spec points at stays the one that ships.
pub fn core_rules() -> Vec<CoreRule> {
    #[derive(Deserialize)]
    struct File {
        rules: Vec<CoreRule>,
    }
    let text = match std::env::var("PLUGIN_RPC_SPEC") {
        Ok(dir) => std::fs::read_to_string(Path::new(&dir).join("core-rules.json"))
            .expect("PLUGIN_RPC_SPEC names a spec directory with core-rules.json in it"),
        Err(_) => EMBEDDED_CORE_RULES.to_string(),
    };
    serde_json::from_str::<File>(&text)
        .expect("spec/core-rules.json is readable")
        .rules
}

const EMBEDDED_CORE_RULES: &str = include_str!("../../core-rules.json");

/// The one file a profile publishes (spec §10). Its shape is `spec/schema/prpc-1/profile.json`,
/// which is normative; this type is the reference reader of it, and every field the meta-schema
/// forbids is refused here too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSpec {
    /// Must be [`MECHANISM`]; [`ProfileSpec::load`] refuses anything else.
    pub mechanism: String,
    /// The `<namespace>/<revision>` string the handshake must answer with (§6).
    pub protocol_version: String,
    /// The manifest's filename inside a plugin package (§11).
    pub manifest: String,
    /// The method set, with the committed schema for every request's params and every response's
    /// result (§10 item 2).
    pub methods: BTreeMap<String, Method>,
    /// The capability vocabulary, and the host's behavior if each is absent (§7, §10 item 3).
    pub capabilities: BTreeMap<String, Capability>,
    /// The mapping from the mechanism's loss reasons to domain meaning, and any reason the profile
    /// adds (§8, §10 item 4).
    pub loss: Loss,
    pub conformance: Conformance,
}

/// One method of the profile, and where its two normative schemas live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    /// Schema for the request's `params`. `None` when the method takes no parameters, in which
    /// case §3 omits `params` from the request rather than sending null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    pub result: String,
    /// When the handshake does not declare this capability, a rule that calls this method is not
    /// applicable rather than a violation (§12 item 3).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "requiresCapability"
    )]
    pub requires_capability: Option<String>,
    /// The params field carrying `deadlineMs` for this method, when it carries one (§9).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "deadlineField"
    )]
    pub deadline_field: Option<String>,
}

/// What the host does when a plugin does not declare one capability (§7). The sentence is also the
/// reason a rule is counted not applicable, so it appears in the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capability {
    pub if_absent: String,
}

/// Loss, as the profile maps it (§8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loss {
    pub mapping: LossMapping,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
}

/// The domain meaning of each reason the mechanism names. Every one is required: a host that
/// leaves a reason unmapped reports a lost plugin as no finding at all. A profile may add reasons
/// beside the five, and those arrive in [`LossMapping::added_reasons`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LossMapping {
    pub spawn: String,
    pub eof: String,
    pub exited: String,
    pub killed: String,
    pub no_answer: String,
    #[serde(flatten)]
    pub added_reasons: BTreeMap<String, String>,
}

/// What a profile fixes about how it is tested (§12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conformance {
    /// The profile's own suite program, run as `<argv> --out <dir>` (§12 item 6).
    pub suite: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    /// The profile's own rules, in the order the suite reports them (§10 item 6).
    pub rules: Vec<Rule>,
}

/// One rule of the profile's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub expectation: String,
    /// Rules whose violation makes this one impossible to run; they are counted not applicable
    /// here and never as passes (§12 item 3). A name may be a core rule's.
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "dependsOn")]
    pub depends_on: Vec<String>,
    pub defect: Defect,
}

/// How a rule is verified (§12 item 4): either the scripted defect that makes exactly that rule
/// report a violation, or an honest statement that no such defect exists yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Defect {
    Scripted {
        run: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
        #[serde(rename = "expectViolation")]
        expect_violation: Vec<String>,
    },
    Unverified {
        unverified: String,
    },
}

impl Defect {
    /// Whether anything has shown this rule can detect the failure it targets (§12 item 4).
    pub fn verified(&self) -> bool {
        matches!(self, Defect::Scripted { .. })
    }
}

impl ProfileSpec {
    /// Read and validate a profile spec. The meta-schema states the same rules; a profile spec that
    /// loads here is an instance of `schema/prpc-1/profile.json`, and the two checks are kept
    /// together by `tests/profile_spec.rs`.
    pub fn load(path: impl AsRef<Path>) -> Result<ProfileSpec, SpecError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| SpecError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let spec = ProfileSpec::parse(&text)?;
        // §10 item 2 resolves every schema path beside this file, so a name the reader cannot open
        // is refused here rather than becoming a rule that silently never ran.
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        for file in spec.schema_files() {
            if !dir.join(file).is_file() {
                return Err(SpecError::MissingSchema {
                    file: file.to_string(),
                    path: dir.join(file).display().to_string(),
                });
            }
        }
        Ok(spec)
    }

    /// Every schema file the method set names, each listed once: several methods share one request
    /// schema, and the mechanism copies a file once (§12 item 6).
    pub fn schema_files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = Vec::new();
        for method in self.methods.values() {
            for name in method.params.iter().chain(std::iter::once(&method.result)) {
                if !files.contains(&name.as_str()) {
                    files.push(name.as_str());
                }
            }
        }
        files
    }

    /// The same validation, from text already in hand.
    pub fn parse(text: &str) -> Result<ProfileSpec, SpecError> {
        let spec: ProfileSpec =
            serde_json::from_str(text).map_err(|e| SpecError::Decode(e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// The constraints the meta-schema states that a deserializer cannot state alone, plus the two
    /// the meta-schema leaves to the loader on purpose: the core rule ids, which live in
    /// `spec/core-rules.json` and must not be copied into a schema, and the order of `dependsOn`,
    /// which only a runner knows it needs.
    pub fn validate(&self) -> Result<(), SpecError> {
        if self.mechanism != MECHANISM {
            return Err(SpecError::Mechanism {
                found: self.mechanism.clone(),
                expected: MECHANISM.to_string(),
            });
        }
        if !crate::is_revision(&self.protocol_version) {
            return Err(SpecError::Version(self.protocol_version.clone()));
        }
        if self.methods.contains_key("handshake") {
            return Err(SpecError::HandshakeIsMine);
        }
        for name in self.methods.keys() {
            if !is_wire_name(name) {
                return Err(SpecError::BadMethodName {
                    method: name.clone(),
                });
            }
        }
        if self.conformance.suite.is_empty() {
            return Err(SpecError::EmptyArgv {
                what: "conformance.suite".to_string(),
            });
        }
        if self.conformance.deadline_ms == Some(0) {
            return Err(SpecError::DeadlineZero);
        }
        let mut added: Vec<&str> = Vec::new();
        for reason in &self.loss.added {
            if added.contains(&reason.as_str()) {
                return Err(SpecError::DuplicateLossReason {
                    reason: reason.clone(),
                });
            }
            added.push(reason.as_str());
        }
        let rules = core_rules();
        let core: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
        let mut declared: Vec<&str> = Vec::new();
        for rule in &self.conformance.rules {
            if !is_kebab(&rule.id) {
                return Err(SpecError::BadRuleId {
                    rule: rule.id.clone(),
                });
            }
            if core.contains(&rule.id.as_str()) {
                return Err(SpecError::CoreRuleRestated {
                    rule: rule.id.clone(),
                });
            }
            if declared.contains(&rule.id.as_str()) {
                return Err(SpecError::DuplicateRule {
                    rule: rule.id.clone(),
                });
            }
            for dependency in &rule.depends_on {
                let known =
                    core.contains(&dependency.as_str()) || declared.contains(&dependency.as_str());
                if !known {
                    return Err(SpecError::UnknownDependency {
                        rule: rule.id.clone(),
                        missing: dependency.clone(),
                    });
                }
            }
            if let Defect::Scripted {
                run,
                expect_violation,
                ..
            } = &rule.defect
            {
                if run.is_empty() {
                    return Err(SpecError::EmptyArgv {
                        what: format!("rule `{}` defect `run`", rule.id),
                    });
                }
                for target in expect_violation {
                    // A defect may fire a core rule — a plugin that swallows a malformed line
                    // violates `framing-intact`, which is the mechanism's — so both vocabularies
                    // are in scope, but the profile's own names must already be declared.
                    if !core.contains(&target.as_str())
                        && !declared.contains(&target.as_str())
                        && target != &rule.id
                    {
                        return Err(SpecError::UnknownDefectTarget {
                            rule: rule.id.clone(),
                            missing: target.clone(),
                        });
                    }
                }
            }
            declared.push(rule.id.as_str());
        }
        Ok(())
    }

    /// The deadline one driven invocation gets: the manifest's own value, then the profile's, then
    /// [`DEFAULT_DEADLINE_MS`] (§9, §11).
    pub fn deadline_ms(&self, manifest: Option<&Manifest>) -> u64 {
        manifest
            .and_then(|m| m.conformance.as_ref())
            .and_then(|c| c.deadline_ms)
            .or(self.conformance.deadline_ms)
            .unwrap_or(DEFAULT_DEADLINE_MS)
    }

    /// Whether the handshake's declared capabilities cover every capability the profile names. A
    /// profile rule gated on an absent capability is not applicable, so a host needs this before it
    /// runs the suite at all (§7, §12 item 3).
    pub fn missing_capabilities(&self, declared: &[String]) -> Vec<&str> {
        self.capabilities
            .keys()
            .filter(|capability| !declared.iter().any(|d| d == *capability))
            .map(String::as_str)
            .collect()
    }
}

/// kebab-case, the pattern the meta-schema puts on rule ids, because a report line and a rule id
/// have to be the same string. Method names and capability names are looser: see [`is_wire_name`].
fn is_kebab(name: &str) -> bool {
    let mut parts = name.split('-');
    let mut any = false;
    for part in parts.by_ref() {
        if part.is_empty()
            || !part.starts_with(|c: char| c.is_ascii_lowercase())
            || !part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return false;
        }
        any = true;
    }
    any && !name.starts_with('-') && !name.ends_with('-')
}

/// A single token, the pattern the meta-schema puts on a profile's method names and capability
/// names. The mechanism names no method (§1), so it constrains casing nowhere: `afz/1` ships
/// `coverageMap` and `sideEffects`, and a mechanism that forced kebab-case on them would be a
/// profile. Hand-rolled, as the rest of these checks are, so no regular-expression dependency.
fn is_wire_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A plugin's manifest (spec §11). Unknown fields are allowed, because a profile adds fields to
/// the manifest of its own plugins the way it adds fields to the handshake; `command` and the
/// `conformance` block are the mechanism's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub install: Vec<Vec<String>>,
    #[serde(default)]
    pub build: Vec<Vec<String>>,
    /// The plugin's argv, run verbatim.
    pub command: Vec<String>,
    #[serde(default)]
    pub conformance: Option<ManifestConformance>,
}

/// The manifest's `conformance` block (§12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestConformance {
    #[serde(default)]
    pub emitter: Option<Vec<String>>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
}

impl Manifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Manifest, SpecError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| SpecError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&text)
            .map_err(|e| SpecError::Decode(format!("{}: {e}", path.display())))
    }
}

/// One rule's outcome. Three, and only three (§12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    Violation,
    #[serde(rename = "not-applicable")]
    NotApplicable,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Pass => "pass",
            Outcome::Violation => "violation",
            Outcome::NotApplicable => "not-applicable",
        })
    }
}

/// One line of a conformance report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub rule: String,
    pub outcome: Outcome,
    /// On a violation: what the plugin did, written in schema terms where a schema was the check
    /// (§12 item 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// On a not-applicable: which gate stopped the rule (§12 item 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether anything has shown this rule can detect the failure it targets (§12 item 4). A pass
    /// on an unverified rule is not proof, and the report says so rather than printing a bare pass.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

impl Verdict {
    pub fn pass(rule: &str) -> Verdict {
        Verdict {
            rule: rule.to_string(),
            outcome: Outcome::Pass,
            note: None,
            reason: None,
            unverified: false,
        }
    }

    pub fn violation(rule: &str, note: impl Into<String>) -> Verdict {
        Verdict {
            rule: rule.to_string(),
            outcome: Outcome::Violation,
            note: Some(note.into()),
            reason: None,
            unverified: false,
        }
    }

    pub fn not_applicable(rule: &str, reason: impl Into<String>) -> Verdict {
        Verdict {
            rule: rule.to_string(),
            outcome: Outcome::NotApplicable,
            note: None,
            reason: Some(reason.into()),
            unverified: false,
        }
    }
}

/// The report: one verdict per rule, core rules first in the order of `spec/core-rules.json`, then
/// the profile's in the order of its own spec (§12 item 7).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub verdicts: Vec<Verdict>,
    /// Set when the suite itself could not run (§12 item 5). It is never a plugin violation, and it
    /// keeps `passed` false, because a run that tested nothing has not shown anything.
    pub suite_error: Option<String>,
}

impl Report {
    pub fn new(verdicts: Vec<Verdict>) -> Report {
        Report {
            verdicts,
            suite_error: None,
        }
    }

    pub fn suite_error(reason: impl Into<String>) -> Report {
        Report {
            verdicts: Vec::new(),
            suite_error: Some(reason.into()),
        }
    }

    /// The plugin conforms when every applicable rule passes (§12). A suite error is not a pass.
    pub fn passed(&self) -> bool {
        self.suite_error.is_none()
            && self
                .verdicts
                .iter()
                .all(|verdict| verdict.outcome != Outcome::Violation)
    }

    pub fn violations(&self) -> Vec<&Verdict> {
        self.verdicts
            .iter()
            .filter(|v| v.outcome == Outcome::Violation)
            .collect()
    }

    pub fn unverified(&self) -> Vec<&Verdict> {
        self.verdicts.iter().filter(|v| v.unverified).collect()
    }

    /// The report as an operator reads it: one line per rule, then the counts that say what the
    /// lines are worth.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for verdict in &self.verdicts {
            out.push_str(&format!(
                "{:>14}  {}\n",
                verdict.outcome.to_string(),
                verdict.rule
            ));
            if let Some(note) = &verdict.note {
                out.push_str(&format!("                  {note}\n"));
            }
            if let Some(reason) = &verdict.reason {
                out.push_str(&format!("                  {reason}\n"));
            }
            if verdict.unverified {
                out.push_str("                  unverified: no scripted defect fires this rule\n");
            }
        }
        let count = |outcome: Outcome| {
            self.verdicts
                .iter()
                .filter(|v| v.outcome == outcome)
                .count()
        };
        out.push_str(&format!(
            "{} pass, {} violation, {} not applicable, {} unverified\n",
            count(Outcome::Pass),
            count(Outcome::Violation),
            count(Outcome::NotApplicable),
            self.unverified().len()
        ));
        if let Some(error) = &self.suite_error {
            out.push_str(&format!("suite error: {error}\n"));
        }
        out
    }
}

/// One entry of the `verdicts.json` a profile's suite program writes (§12 item 6). It carries no
/// `unverified` field, because whether a rule is verified comes from the profile spec and is not
/// the suite program's to decide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteVerdict {
    pub rule: String,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Why the profile's suite program produced no usable report (§12 item 5). Every case here is the
/// suite's own failure and is never charged to the plugin.
#[derive(Debug, Error)]
pub enum SuiteError {
    #[error("cannot write the suite directory: {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the suite program `{program}` could not be started: {detail}")]
    Spawn { program: String, detail: String },
    #[error("the suite program exited with {status} and wrote no verdicts.json")]
    Failed { status: String },
    #[error("verdicts.json is not readable: {0}")]
    Decode(String),
    #[error("the suite program reported no verdict for `{rule}`; a missing verdict is never read as a pass")]
    Missing { rule: String },
    #[error("the suite program reported `{rule}`, which the profile spec does not declare")]
    Unknown { rule: String },
    #[error("the suite program reported `{rule}` twice")]
    Duplicate { rule: String },
}

/// Run the profile's own suite program and turn its `verdicts.json` into report lines (§12 item 6).
///
/// The mechanism writes `profile.json`, `manifest.json`, and every schema file the profile spec
/// names into `out_dir` — the last at the path the spec gives, copied from `spec_dir`, which is the
/// directory the spec was loaded from — then runs `<argv> --out <dir>`, and then requires exactly
/// one verdict for every rule the profile declares and none it does not. A rule the program did not
/// report is an error and not a pass, because a report that reads a missing verdict as a pass is the
/// one failure a conformance report must not have.
pub fn run_suite(
    profile: &ProfileSpec,
    manifest: &Manifest,
    spec_dir: &Path,
    out_dir: &Path,
    env: &[(String, String)],
) -> Result<Report, SuiteError> {
    std::fs::create_dir_all(out_dir).map_err(|source| SuiteError::Io {
        path: out_dir.display().to_string(),
        source,
    })?;
    write_json(&out_dir.join("profile.json"), profile)?;
    write_json(&out_dir.join("manifest.json"), manifest)?;
    for file in profile.schema_files() {
        let from = spec_dir.join(file);
        let dest = out_dir.join(file);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|source| SuiteError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        std::fs::copy(&from, &dest).map_err(|source| SuiteError::Io {
            path: from.display().to_string(),
            source,
        })?;
    }

    let argv = &profile.conformance.suite;
    let Some(program) = argv.first() else {
        return Err(SuiteError::Spawn {
            program: String::new(),
            detail: "the profile spec's `conformance.suite` is empty".to_string(),
        });
    };
    // The suite program starts the plugin, so the variables the runner was given for the plugin
    // have to reach the program too, or a scripted defect would be tested against a plugin that
    // never saw it.
    let status = std::process::Command::new(program)
        .args(argv.iter().skip(1))
        .arg("--out")
        .arg(out_dir)
        .envs(env.iter().cloned())
        .status()
        .map_err(|source| SuiteError::Spawn {
            program: program.clone(),
            detail: source.to_string(),
        })?;
    let text = match std::fs::read_to_string(out_dir.join("verdicts.json")) {
        Ok(text) => text,
        Err(source) => {
            return Err(if status.success() {
                SuiteError::Io {
                    path: out_dir.join("verdicts.json").display().to_string(),
                    source,
                }
            } else {
                SuiteError::Failed {
                    status: status.to_string(),
                }
            })
        }
    };
    let reported: Vec<SuiteVerdict> =
        serde_json::from_str(&text).map_err(|e| SuiteError::Decode(e.to_string()))?;

    let mut verdicts = Vec::new();
    for rule in &profile.conformance.rules {
        let mut found = reported.iter().filter(|v| v.rule == rule.id);
        let Some(first) = found.next() else {
            return Err(SuiteError::Missing {
                rule: rule.id.clone(),
            });
        };
        if found.next().is_some() {
            return Err(SuiteError::Duplicate {
                rule: rule.id.clone(),
            });
        }
        verdicts.push(Verdict {
            rule: rule.id.clone(),
            outcome: first.outcome,
            note: first.note.clone(),
            reason: first.reason.clone(),
            unverified: !rule.defect.verified(),
        });
    }
    for extra in &reported {
        if !profile
            .conformance
            .rules
            .iter()
            .any(|rule| rule.id == extra.rule)
        {
            return Err(SuiteError::Unknown {
                rule: extra.rule.clone(),
            });
        }
    }
    Ok(Report::new(verdicts))
}

/// What running one rule's scripted defect showed (§12 item 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefectRun {
    /// The profile states no defect for the rule, so it keeps the unverified mark.
    NotScripted,
    /// Every rule the defect names reported a violation, which is what makes this rule verified.
    Fired,
    /// The plugin answered and at least one named rule did not report a violation: the profile
    /// claims a proof this run did not produce.
    Silent { missing: Vec<String> },
    /// The defect's own run could not be carried out, which says nothing about the plugin.
    Failed { detail: String },
    /// The defect names a plugin other than the one this run is testing, so its outcome belongs to
    /// the profile's own run and says nothing about this plugin.
    OtherPlugin,
}

/// Run one rule's scripted defect and report what it showed (§12 item 4).
///
/// `expectViolation` is a claim about a run nobody has done, so this does the run: the core rules
/// against the plugin the defect starts, then the profile's suite program against that same
/// plugin, and the named rules looked for among the violations. A defect that names only core
/// rules skips the suite program, because nothing it reports is asked about.
///
/// Rules the profile did not name may report violations beside the named ones; a defect usually
/// breaks a neighbour on its way to the rule it targets. §12 item 4 asks whether the named rules
/// fail, not whether the rest of the protocol held while they did.
///
/// A defect names the plugin it was written against, and a profile's reference plugin is usually
/// not the plugin being tested: a harness checkout has no copy of the engine's test plugin. The
/// defect is then the profile's claim, proved where that plugin is built, and this run leaves the
/// rule's mark as the profile states it rather than calling it unverified for want of a binary.
pub fn run_defect(
    rule: &Rule,
    profile: &ProfileSpec,
    manifest: &Manifest,
    spec_dir: &Path,
    out_dir: &Path,
    base: &core::CoreCheck,
) -> DefectRun {
    let Defect::Scripted {
        run,
        env,
        expect_violation,
    } = &rule.defect
    else {
        return DefectRun::NotScripted;
    };
    if !same_program(run, &manifest.command) {
        return DefectRun::OtherPlugin;
    }
    let mut defect_env = base.env.clone();
    for pair in env {
        if let Some((key, value)) = pair.split_once('=') {
            defect_env.retain(|(name, _)| name != key);
            defect_env.push((key.to_string(), value.to_string()));
        }
    }
    let check = core::CoreCheck {
        profile,
        command: run.clone(),
        env: defect_env.clone(),
        cwd: base.cwd.clone(),
        deadline: base.deadline,
    };
    let report = core::run(&check);
    let core_ids: Vec<String> = core_rules().into_iter().map(|rule| rule.id).collect();
    let mut violated: Vec<String> = report
        .violations()
        .iter()
        .map(|verdict| verdict.rule.clone())
        .collect();
    if expect_violation.iter().any(|name| !core_ids.contains(name)) {
        let scripted = Manifest {
            command: run.clone(),
            ..manifest.clone()
        };
        match run_suite(profile, &scripted, spec_dir, out_dir, &defect_env) {
            Ok(ran) => violated.extend(ran.violations().iter().map(|verdict| verdict.rule.clone())),
            Err(e) => {
                return DefectRun::Failed {
                    detail: e.to_string(),
                }
            }
        }
    }
    let missing: Vec<String> = expect_violation
        .iter()
        .filter(|name| !violated.contains(name))
        .cloned()
        .collect();
    if missing.is_empty() {
        DefectRun::Fired
    } else {
        DefectRun::Silent { missing }
    }
}

/// Whether two argvs start the same program, judged element by element beside the working
/// directory. A manifest may name its plugin by absolute path while the profile names the same
/// binary relatively, and those are the same plugin; two plugins behind the same interpreter
/// (`/bin/sh demo.sh`, `/bin/sh other.sh`) are not, because the script is the plugin.
fn same_program(defect: &[String], command: &[String]) -> bool {
    defect.len() == command.len()
        && defect.iter().zip(command).all(|(defect, command)| {
            match (std::path::absolute(defect), std::path::absolute(command)) {
                (Ok(defect), Ok(command)) => defect == command,
                _ => defect == command,
            }
        })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), SuiteError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| SuiteError::Decode(e.to_string()))?;
    std::fs::write(path, text).map_err(|source| SuiteError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Why a profile spec is not usable. Each variant names what the author has to change.
#[derive(Debug, Error)]
pub enum SpecError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the profile spec is not an instance of schema/prpc-1/profile.json: {0}")]
    Decode(String),
    #[error("the profile spec names mechanism `{found}`; this host implements `{expected}`")]
    Mechanism { found: String, expected: String },
    #[error("`protocolVersion` must be `<namespace>/<revision>`, as in \"demo/1\"; the profile spec says `{0}`")]
    Version(String),
    #[error("the profile spec declares the method `handshake`, which the mechanism owns")]
    HandshakeIsMine,
    #[error("method `{method}` is not a single token of letters, digits, and dashes")]
    BadMethodName { method: String },
    #[error(
        "rule `{rule}` is not kebab-case; a report line and a rule id must be the same string"
    )]
    BadRuleId { rule: String },
    #[error("`{what}` is empty; §12 runs `<argv> --out <dir>`, and an argv with no program in it runs nothing")]
    EmptyArgv { what: String },
    #[error("`conformance.deadlineMs` is 0; a deadline of zero answers no request before it is over (§9)")]
    DeadlineZero,
    #[error("`loss.added` names `{reason}` twice; a report that lists one loss reason twice miscounts it (§8)")]
    DuplicateLossReason { reason: String },
    #[error(
        "`{file}` names no schema beside the profile spec; it resolves to `{path}` (§10 item 2)"
    )]
    MissingSchema { file: String, path: String },
    #[error(
        "`{rule}` is one of the core rules; a profile adds rules beside them and never repeats one"
    )]
    CoreRuleRestated { rule: String },
    #[error("two rules are both named `{rule}`; the report is keyed on the id")]
    DuplicateRule { rule: String },
    #[error(
        "rule `{rule}` depends on `{missing}`, which the profile spec does not declare before it"
    )]
    UnknownDependency { rule: String, missing: String },
    #[error("rule `{rule}` names a defect that must fire `{missing}`, which the profile spec does not declare before it")]
    UnknownDefectTarget { rule: String, missing: String },
}
