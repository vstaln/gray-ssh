//! gray-ssh — `ssh_run` / `ssh_copy` tools delegating to remote hosts.
//!
//! Port of pi's `ssh` extension. pi rewired its read/write/edit/bash tools to
//! a remote over SSH; gray's wire can't swap builtin tool ops, so the remote
//! surface shrinks to two explicit tools: `ssh_run` runs a command via
//! `ssh -o BatchMode=yes -o ConnectTimeout=8 <host> <command>` (output capped
//! at 32KB, nonzero exit → is_error) and `ssh_copy` moves a file with `scp`.
//!
//! Deny by default: only the hosts listed in `~/.gray/ssh/hosts` (one per
//! line, `#` comments ok) may be used. No list means no host.
//!
//! Wire methods: plugin/manifest, tool/call, plugin/shutdown.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// ssh_run's default timeout when `timeout_secs` isn't passed.
const RUN_DEFAULT: Duration = Duration::from_secs(60);
/// scp gets longer — file transfers legitimately outlive shell commands.
const COPY_DEFAULT: Duration = Duration::from_secs(300);
/// Combined stdout+stderr returned to the model, max bytes.
const OUT_CAP: usize = 32 * 1024;

fn manifest() -> Value {
    json!({
        "name": "ssh",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "1.1",
        "tools": [
            {
                "name": "ssh_run",
                "description": "Run a shell command on a remote host over SSH (`ssh -o BatchMode=yes -o ConnectTimeout=8`). Requires key-based auth — no password prompts are possible. Returns stdout+stderr (capped at 32KB); nonzero exit is an error.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": { "type": "string", "description": "SSH target, e.g. user@host or an ~/.ssh/config alias." },
                        "command": { "type": "string", "description": "Command line run by the remote shell." },
                        "timeout_secs": { "type": "integer", "description": "Kill after this many seconds (default 60)." }
                    },
                    "required": ["host", "command"]
                }
            },
            {
                "name": "ssh_copy",
                "description": "Copy a file between local and remote with scp. direction \"up\" sends `local` to `host:remote`; \"down\" fetches `host:remote` to `local`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "host": { "type": "string", "description": "SSH target, e.g. user@host or an ~/.ssh/config alias." },
                        "local": { "type": "string", "description": "Local path." },
                        "remote": { "type": "string", "description": "Remote path." },
                        "direction": { "type": "string", "enum": ["up", "down"], "description": "\"up\": local → host. \"down\": host → local." },
                        "timeout_secs": { "type": "integer", "description": "Kill after this many seconds (default 300)." }
                    },
                    "required": ["host", "local", "remote", "direction"]
                }
            }
        ],
        "commands": [],
    })
}

/// `~/.gray/ssh`, honoring `$GRAY_HOME` first.
fn state_dir() -> PathBuf {
    std::env::var_os("GRAY_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".gray")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ssh")
}

/// Deny by default. Lines in `~/.gray/ssh/hosts` are exact host matches
/// (`user@host` or bare `host`); blanks and `#` comments skip. A missing list
/// allows nothing.
fn host_allowed(dir: &Path, host: &str) -> bool {
    let path = dir.join("hosts");
    let Ok(list) = std::fs::read_to_string(&path) else { return false };
    list.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .any(|l| l == host)
}

/// Run `prog args…` with a timeout; returns combined stdout+stderr capped at
/// OUT_CAP. `Err` covers spawn failure, timeout (child is killed), and a
/// nonzero exit — callers map it to `is_error`.
fn run_capped(prog: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = std::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{prog} failed to start: {e}"))?;
    let mut out_pipe = child.stdout.take().expect("stdout piped");
    let mut err_pipe = child.stderr.take().expect("stderr piped");
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out_pipe.read_to_end(&mut b);
        b
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err_pipe.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_t.join();
                let _ = err_t.join();
                return Err(format!("{prog} timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{prog} wait failed: {e}")),
        }
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    let mut text = stdout.trim_end().to_string();
    if !stderr.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("stderr: {}", stderr.trim()));
    }
    if text.is_empty() {
        text = "(no output)".into();
    }
    if text.len() > OUT_CAP {
        let mut end = OUT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n… truncated at 32KB");
    }
    if status.success() {
        Ok(text)
    } else {
        Err(format!("{prog} exited {}: {text}", status.code().unwrap_or(-1)))
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(format!("missing required argument: {key}"))
}

fn timeout_arg(args: &Value, default: Duration) -> Duration {
    args.get("timeout_secs")
        .and_then(Value::as_u64)
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
        .unwrap_or(default)
}

/// A leading `-` would let a host or path smuggle ssh/scp options in.
fn no_dash(s: &str, what: &str) -> Result<(), String> {
    if s.starts_with('-') {
        Err(format!("{what} must not start with '-'"))
    } else {
        Ok(())
    }
}

fn check_host(dir: &Path, host: &str) -> Result<(), String> {
    no_dash(host, "host")?;
    if host_allowed(dir, host) {
        Ok(())
    } else if dir.join("hosts").exists() {
        Err(format!("host {host:?} not in the ~/.gray/ssh/hosts allowlist"))
    } else {
        Err(format!(
            "ssh is deny-by-default: create ~/.gray/ssh/hosts listing {host:?} (one host per line)"
        ))
    }
}

fn ssh_run(dir: &Path, args: &Value) -> Result<String, String> {
    let host = arg(args, "host")?;
    let command = arg(args, "command")?;
    check_host(dir, host)?;
    run_capped(
        "ssh",
        &["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host, command],
        timeout_arg(args, RUN_DEFAULT),
    )
}

fn ssh_copy(dir: &Path, args: &Value) -> Result<String, String> {
    let host = arg(args, "host")?;
    let local = arg(args, "local")?;
    let remote = arg(args, "remote")?;
    let direction = arg(args, "direction")?;
    check_host(dir, host)?;
    no_dash(local, "local")?;
    let remote_spec = format!("{host}:{remote}");
    let (src, dst) = match direction {
        "up" => (local.to_string(), remote_spec),
        "down" => (remote_spec, local.to_string()),
        other => return Err(format!("unknown direction {other:?} — \"up\" or \"down\"")),
    };
    run_capped(
        "scp",
        &["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", &src, &dst],
        timeout_arg(args, COPY_DEFAULT),
    )
    .map(|_| format!("copied {src} → {dst}"))
}

fn call_tool(dir: &Path, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "ssh_run" => ssh_run(dir, args),
        "ssh_copy" => ssh_copy(dir, args),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// One request → `Some(reply)`, or `None` for notifications. The bool asks
/// the loop to exit after writing the reply.
fn handle(req: &Value) -> (Option<Value>, bool) {
    let dir = state_dir();
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = id else {
        return (None, method == "plugin/shutdown");
    };
    let result = match method {
        "plugin/manifest" => manifest(),
        "tool/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("args").cloned().unwrap_or(Value::Null);
            match call_tool(&dir, name, &args) {
                Ok(text) => json!({ "content": text }),
                Err(text) => json!({ "content": text, "is_error": true }),
            }
        }
        "plugin/shutdown" => return (Some(json!({ "id": id, "result": {} })), true),
        _ => {
            let error = json!({ "code": -32601, "message": "method not found" });
            return (Some(json!({ "id": id, "error": error })), false);
        }
    };
    (Some(json!({ "id": id, "result": result })), false)
}

fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("manifest") {
        println!("{}", manifest());
        return Ok(());
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let (reply, exit) = handle(&req);
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
        if exit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        handle(&json!({ "id": 1, "method": method, "params": params }))
            .0
            .unwrap()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gray-ssh-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn manifest_declares_two_tools() {
        let m = call("plugin/manifest", Value::Null)["result"].clone();
        assert_eq!(m["name"], "ssh");
        let names: Vec<&str> = m["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(names, ["ssh_run", "ssh_copy"]);
    }

    #[test]
    fn missing_args_are_errors() {
        let dir = tmpdir("args");
        assert!(ssh_run(&dir, &json!({})).unwrap_err().contains("host"));
        assert!(ssh_run(&dir, &json!({"host": "h"})).unwrap_err().contains("command"));
        assert!(ssh_copy(&dir, &json!({"host": "h", "local": "l", "remote": "r"}))
            .unwrap_err()
            .contains("direction"));
    }

    #[test]
    fn no_list_means_no_host() {
        let dir = tmpdir("allow");
        assert!(!host_allowed(&dir, "anything"));
        let err = ssh_run(&dir, &json!({"host": "anything", "command": "true"})).unwrap_err();
        assert!(err.contains("deny-by-default"), "got: {err}");
    }

    #[test]
    fn allowlist_admits_only_listed_hosts() {
        let dir = tmpdir("allow");
        std::fs::write(dir.join("hosts"), "good\n# comment\n\nuser@also-good\n").unwrap();
        assert!(host_allowed(&dir, "good"));
        assert!(host_allowed(&dir, "user@also-good"));
        assert!(!host_allowed(&dir, "bad"));
        let err = ssh_run(&dir, &json!({"host": "bad", "command": "true"})).unwrap_err();
        assert!(err.contains("allowlist"), "got: {err}");
    }

    #[test]
    fn option_injection_is_refused() {
        let dir = tmpdir("dash");
        let err = ssh_run(&dir, &json!({"host": "-oProxyCommand=x", "command": "true"})).unwrap_err();
        assert!(err.contains("'-'"), "got: {err}");
    }

    #[test]
    fn run_capped_kills_on_timeout() {
        let t = Instant::now();
        let err = run_capped("sleep", &["5"], Duration::from_secs(1)).unwrap_err();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(t.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn run_capped_flags_nonzero() {
        let err = run_capped("false", &[], Duration::from_secs(5)).unwrap_err();
        assert!(err.contains("exited"), "got: {err}");
        assert!(run_capped("echo", &["hi"], Duration::from_secs(5)).unwrap().contains("hi"));
    }

    #[test]
    fn shutdown_replies_then_exits() {
        let (reply, exit) = handle(&json!({ "id": 2, "method": "plugin/shutdown" }));
        assert!(reply.is_some() && exit);
    }
}
