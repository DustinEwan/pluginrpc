//! The scenario harness veridoc builds and runs for each scenario.
//!
//! The claims in `src/lib.claims.json` name this crate's own functions, so the harness is the one
//! place where a scenario request becomes a call. It answers the observables `veridoc.tree.json`
//! declares for the action it is asked to run, and an action it does not implement is a hard error
//! rather than a zero: a scenario that silently reported the wrong answer would mark a claim
//! supported.
//!
//! Contract, one line each way:
//!
//! ```text
//! stdin  {"scenarioId":"..","action":"..","args":[],"fixture":"<absolute dir>","deadlineMs":30000}
//! stdout {"observables":{"accepted":false,"reason":".."}}
//! ```
//!
//! The fixture directory also arrives as `argv[1]`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut request = String::new();
    std::io::stdin().read_to_string(&mut request)?;
    let request: Value = serde_json::from_str(&request)?;
    let action = request["action"].as_str().unwrap_or_default().to_string();
    let fixture = request["fixture"]
        .as_str()
        .map(str::to_string)
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_default();

    let observables = match action.as_str() {
        "parse-a-request" => {
            let line = read_line_of(&fixture, "line.txt")?;
            match plugin_rpc::parse_request(&line) {
                Ok(_) => json!({ "accepted": true, "reason": "" }),
                Err(reason) => json!({ "accepted": false, "reason": reason.to_string() }),
            }
        }
        "read-a-pipe" => {
            let bytes = fs::read(at(&fixture, "pipe.txt"))?;
            let mut reader = plugin_rpc::buffered(bytes.as_slice());
            let mut buf = String::new();
            let mut messages = 0usize;
            // `read_line` ends a stream by answering `None`, so that answer is the loop's only
            // exit: an `Err` leaves through `?` instead. `lost` is what the claim asserts about
            // the blank line the fixture holds.
            let lost = loop {
                match plugin_rpc::read_line(&mut reader, &mut buf)? {
                    Some(_) => messages += 1,
                    None => break true,
                }
            };
            json!({ "messages": messages, "lost": lost })
        }
        other => {
            return Err(format!("the harness implements no action {other:?}").into());
        }
    };

    println!("{}", json!({ "observables": observables }));
    Ok(())
}

fn at(fixture: &str, name: &str) -> PathBuf {
    Path::new(fixture).join(name)
}

/// The one line a fixture holds, without its line ending: the line ending belongs to the fixture
/// file, and a claim about a refused line is about its content.
fn read_line_of(fixture: &str, name: &str) -> Result<String, std::io::Error> {
    Ok(fs::read_to_string(at(fixture, name))?
        .trim_end_matches(['\n', '\r'])
        .to_string())
}
