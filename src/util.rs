//! Small helpers: paths, processes, hashing, number formatting.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Finds a program on PATH (trying PATHEXT on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let exts: Vec<String> = if cfg!(windows) {
        let mut e = vec![String::new()];
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        e.extend(pathext.split(';').filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()));
        e
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A command from an argv. One argument with spaces in it is a shell line; npm-style .cmd shims go through cmd.exe.
pub fn command(argv: &[String]) -> Command {
    let shell_line = argv.len() == 1 && argv[0].contains(char::is_whitespace);
    if shell_line {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&argv[0]);
            return c;
        }
        let mut c = Command::new("sh");
        c.arg("-c").arg(&argv[0]);
        return c;
    }
    if cfg!(windows) {
        let shim = which(&argv[0])
            .and_then(|p| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()))
            .is_some_and(|e| e == "cmd" || e == "bat");
        if shim {
            let mut c = Command::new("cmd");
            c.arg("/C").args(argv);
            return c;
        }
    }
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    c
}

pub fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

pub fn secs(s: f64) -> String {
    if s >= 1.0 { format!("{s:.1}s") } else { format!("{:.0}ms", s * 1e3) }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 1234 -> "1.2k", 1234567 -> "1.2M".
pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

pub fn bytes(n: u64) -> String {
    match n {
        0..1_024 => format!("{n} B"),
        1_024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", n as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", n as f64 / 1_073_741_824.0),
    }
}

/// "-21%" for 100 -> 79. Empty when nothing changed.
pub fn delta(before: u64, after: u64) -> String {
    if before == 0 || before == after {
        return String::new();
    }
    let pct = (after as f64 - before as f64) / before as f64 * 100.0;
    if pct.abs() < 0.95 { format!("{pct:+.1}%") } else { format!("{pct:+.0}%") }
}

/// Size of a file, or of everything under a folder.
pub fn disk_size(p: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| disk_size(&e.path())).sum()).unwrap_or(0)
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(compact(980), "980");
        assert_eq!(compact(12_480), "12.5k");
        assert_eq!(bytes(3 * 1_048_576), "3.0 MB");
        assert_eq!(delta(100, 79), "-21%");
        assert_eq!(delta(100, 100), "");
        assert_eq!(delta(1000, 996), "-0.4%");
        assert_eq!(tail("a\nb\nc\n", 2), "b\nc");
    }

    #[test]
    fn shell_lines_and_argv() {
        let c = command(&["npm test && echo ok".to_string()]);
        assert!(matches!(c.get_program().to_str(), Some("cmd" | "sh")));
        let c = command(&["definitely-not-a-program".to_string(), "x".to_string()]);
        assert_eq!(c.get_program().to_str(), Some("definitely-not-a-program"));
    }
}
