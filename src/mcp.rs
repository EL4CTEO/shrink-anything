//! MCP server on stdio (JSON-RPC 2.0, one message per line). Lets any agent drive shrink-anything.

use crate::candidate::{CandKind, Candidate};
use crate::state::{self, Ctx};
use crate::{agent, cut, report, scan, ui};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "shrink-anything makes a codebase smaller and proves nothing broke. It finds dead code, dead files, \
unused dependencies and duplicated code in any mix of languages, deletes what it can prove dead, and keeps a deletion only if the \
project's own check (its tests or build) still passes with as many tests as before. Loop: shrink_scan -> shrink_cut with the test \
command -> shrink_task for what needs judgment (duplicates, public members) -> make those edits yourself -> shrink_verify after \
each group of edits -> shrink_report. shrink_undo takes a cut back. Never edit tests to make the check pass.";

fn tools() -> Value {
    let path = json!({ "type": "string", "description": "Project folder (default: the server's working folder)" });
    let check = json!({ "type": "array", "items": { "type": "string" },
        "description": "The command that proves the project works, e.g. [\"npm\", \"test\"] or [\"cargo\", \"test\"]. One item with spaces is run as a shell line. Remembered once given." });
    let library = json!({ "type": "boolean", "description": "true: it's a library, keep public names. false: it's an app. Default: guessed per language." });
    let tool = |name: &str, description: &str, properties: Value, required: Value| {
        json!({ "name": name, "description": description,
                "inputSchema": { "type": "object", "properties": properties, "required": required } })
    };
    json!([
        tool(
            "shrink_scan",
            "List what could go, without changing anything: dead code, dead files, unused dependencies (tier safe or risky) and duplicated code.",
            json!({ "path": path, "library": library }),
            json!([])
        ),
        tool(
            "shrink_cut",
            "Delete the safe candidates and run the check: what passes stays, what breaks is put back. Reversible with shrink_undo.",
            json!({ "check": check, "path": path, "library": library,
                    "aggressive": { "type": "boolean", "description": "Also cut risky candidates (public members, library exports)" },
                    "only": { "type": "array", "items": { "type": "string", "enum": ["code", "file", "dep", "asset"] } },
                    "max_runs": { "type": "integer", "default": 40, "description": "Budget of check runs" } }),
            json!([])
        ),
        tool(
            "shrink_task",
            "What needs judgment after the cut: duplicated code to merge and risky candidates to review. Do these edits yourself, then call shrink_verify.",
            json!({ "path": path, "library": library }),
            json!([])
        ),
        tool(
            "shrink_verify",
            "Run the check on the project as it is now. Fails if it exits non-zero or fewer tests run than before the first cut.",
            json!({ "check": check, "path": path }),
            json!([])
        ),
        tool(
            "shrink_undo",
            "Take back the last cut (or every cut). Refuses when a file changed since, unless forced.",
            json!({ "path": path, "all": { "type": "boolean" }, "force": { "type": "boolean" } }),
            json!([])
        ),
        tool(
            "shrink_report",
            "Before and after as markdown, plus .shrink/report.md and a shareable .shrink/report.svg card.",
            json!({ "path": path, "library": library }),
            json!([])
        ),
        tool(
            "shrink_weigh",
            "Where the weight of a build is: bytes by kind of file, the heaviest files, source maps, and files that are in there twice.",
            json!({ "path": path, "paths": { "type": "array", "items": { "type": "string" }, "description": "Build outputs to weigh, e.g. [\"dist\"]. Default: the usual build folders that exist." } }),
            json!([])
        ),
        tool(
            "shrink_status",
            "The check command, the numbers before the first cut, and every run.",
            json!({ "path": path }),
            json!([])
        ),
    ])
}

fn brief(c: &Candidate) -> Value {
    json!({ "id": c.id, "kind": c.kind, "tier": c.tier, "name": c.name, "path": c.path, "line": c.line, "lines": c.lines, "bytes": c.bytes, "why": c.why })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn call(params: &Value) -> Result<Value> {
    let args = &params["arguments"];
    let root = crate::util::absolute(&args["path"].as_str().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")));
    let library = args["library"].as_bool();
    let mut ctx = Ctx::open(&root)?;
    let check = strings(&args["check"]);
    if !check.is_empty() {
        ctx.state.check = check;
    }
    Ok(match params["name"].as_str().unwrap_or_default() {
        "shrink_scan" => {
            let s = scan::scan(&root, library)?;
            json!({ "metrics": s.metrics, "tokens": s.metrics.tokens(), "languages": s.languages, "library": s.library,
                    "candidates": s.candidates.iter().take(200).map(brief).collect::<Vec<_>>(),
                    "more_candidates": s.candidates.len().saturating_sub(200),
                    "duplicates": s.duplicates.iter().take(60).collect::<Vec<_>>() })
        }
        "shrink_cut" => {
            let only: Vec<CandKind> = strings(&args["only"])
                .iter()
                .filter_map(|k| match k.as_str() {
                    "code" => Some(CandKind::Symbol),
                    "file" => Some(CandKind::File),
                    "dep" => Some(CandKind::Dep),
                    "asset" => Some(CandKind::Asset),
                    _ => None,
                })
                .collect();
            let opts = cut::Opts {
                aggressive: args["aggressive"].as_bool().unwrap_or(false),
                only,
                max_runs: args["max_runs"].as_u64().map_or(40, |n| n as usize),
                library,
                ..cut::Opts::default()
            };
            let o = cut::cut(&mut ctx, &opts)?;
            json!({ "changed": o.run.is_some(), "before": o.before, "after": o.after, "tests": o.tests,
                    "kept": o.kept.iter().map(brief).collect::<Vec<_>>(),
                    "put_back": o.put_back.iter().map(|p| json!({ "id": p.candidate.id, "reason": p.reason })).collect::<Vec<_>>(),
                    "untested": o.untested, "check_runs": o.check_runs + 1, "go_mod_tidy_removed": o.tidied,
                    "size_before": o.size_before, "size_after": o.size_after })
        }
        "shrink_task" => {
            let s = scan::scan(&root, library)?;
            match agent::task(&s, &ctx.state.check) {
                Some(task) => json!({ "task": task }),
                None => json!({ "task": null, "note": "nothing left that needs judgment" }),
            }
        }
        "shrink_verify" => {
            ctx.save()?;
            let v = cut::verify(&ctx)?;
            json!({ "ok": v.ok, "tests": v.tests, "tests_before_first_cut": v.base_tests, "seconds": v.secs, "output": v.output })
        }
        "shrink_undo" => {
            let runs = state::undo(
                &mut ctx,
                args["all"].as_bool().unwrap_or(false),
                args["force"].as_bool().unwrap_or(false),
            )?;
            json!({ "undone": runs.iter().map(|r| json!({ "run": r.id, "files": r.files.len() })).collect::<Vec<_>>() })
        }
        "shrink_report" => serde_json::to_value(report::report(&ctx, library)?)?,
        "shrink_weigh" => serde_json::to_value(crate::weigh::weigh(&root, &strings(&args["paths"]))?)?,
        "shrink_status" => json!({ "check": ctx.state.check, "baseline": ctx.state.baseline,
                                   "runs": ctx.live_runs().map(|r| json!({ "run": r.id, "kind": r.kind, "cuts": r.cuts.len(), "before": r.before, "after": r.after })).collect::<Vec<_>>() }),
        other => anyhow::bail!("unknown tool {other}"),
    })
}

/// Sends each step of a long tool call as a progress notification, for clients that asked with a progressToken.
fn progress_notifier(token: Value) -> Box<dyn FnMut(&str) + Send> {
    let mut progress = 0u64;
    Box::new(move |message: &str| {
        progress += 1;
        let note = progress_note(&token, progress, message);
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{note}");
        let _ = stdout.flush();
    })
}

fn progress_note(token: &Value, progress: u64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/progress",
            "params": { "progressToken": token, "progress": progress, "message": message } })
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let params = &msg["params"];
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "shrink-anything", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let token = &params["_meta"]["progressToken"];
            if token.is_string() || token.is_number() {
                ui::set_progress(Some(progress_notifier(token.clone())));
            }
            let result = call(params);
            ui::set_progress(None);
            match result {
                Ok(v) => {
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }] })
                }
                Err(e) => json!({ "content": [{ "type": "text", "text": format!("{e:#}") }], "isError": true }),
            }
        }
        method => {
            return Some(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("unknown method {method}") } }),
            );
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

pub fn serve() -> Result<()> {
    ui::set_quiet(true);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(e) => {
                Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }))
            }
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_listing() {
        let init = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-11-25" } })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        let list = handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 8);
        assert!(tools.iter().all(|t| t["name"].as_str().unwrap().starts_with("shrink_")));
        assert!(handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let bad = handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" })).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
    }

    #[test]
    fn a_tool_error_is_a_result_not_a_crash() {
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "shrink_scan", "arguments": { "path": "/definitely/not/here" } } }))
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
    }
}
