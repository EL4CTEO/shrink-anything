//! Checks what's installed: the parsers that double-check an edit, package managers, and agents.

use crate::lang::SPECS;
use crate::{ui, util};
use anyhow::Result;

fn found(names: &str) -> Option<String> {
    names.split('|').find(|n| util::which(n).is_some()).map(String::from)
}

fn row(ok: bool, name: &str, note: &str) {
    let mark = if ok { ui::green("✓") } else { ui::dim("·") };
    ui::line(format!("  {mark} {name:<12} {}", ui::dim(note)));
}

pub fn run() -> Result<()> {
    ui::line(ui::bold("  languages"));
    let names: Vec<&str> = SPECS.iter().map(|s| s.name).collect();
    ui::line(format!("  {}", ui::dim(format!("dead code and duplicates: {}", names.join(", ")))));
    ui::line(format!("  {}", ui::dim("dead files: Python, JavaScript, TypeScript, PHP, Ruby, Dart, Lua, Luau")));
    ui::line(format!(
        "  {}",
        ui::dim("dependencies: npm, pnpm, yarn, bun, Cargo, pip (requirements, pyproject), Go modules")
    ));

    ui::line("");
    ui::line(ui::bold("  parsers (every edited file is parsed before the check runs)"));
    for spec in SPECS.iter().filter(|s| !s.syntax.is_empty()) {
        match found(spec.syntax[0]) {
            Some(program) => row(true, spec.name, &program),
            None => row(
                false,
                spec.name,
                &format!("{} not found: edits are proven by your check alone", spec.syntax[0].replace('|', " / ")),
            ),
        }
    }
    ui::line(ui::dim("    other languages: their compiler in your check is the parser"));

    ui::line("");
    ui::line(ui::bold("  package managers (to reinstall after a dependency goes)"));
    for pm in ["npm", "pnpm", "yarn", "bun", "cargo", "go"] {
        row(util::which(pm).is_some(), pm, "");
    }

    ui::line("");
    ui::line(ui::bold("  agents (for duplicates and what needs a look)"));
    let mut any = false;
    for agent in ["claude", "codex"] {
        let ok = util::which(agent).is_some();
        any |= ok;
        row(ok, agent, if ok { "--agent" } else { "not found" });
    }
    if !any {
        ui::line(ui::dim("    any command that reads a prompt on stdin works: --agent \"my-agent --flag\""));
    }
    Ok(())
}
