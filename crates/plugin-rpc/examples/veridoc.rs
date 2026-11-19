//! The pluginrpc harness: one call into the claim library.
//!
//! veridoc builds this target itself — `cargo build --release --example veridoc` — and asks it what
//! the project's claim modules declare, so a project never runs it by hand. For each claim module
//! the project declares, name it here and add its claims to the list.

#[path = "../src/framing.doc.rs"]
mod framing_doc;

fn main() -> std::process::ExitCode {
    veridoc_claims::harness(veridoc_claims::merge([framing_doc::claims()]))
}
