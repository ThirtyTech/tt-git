//! `tt-git __test-plugin <steps…>`: a scriptable stand-in for a real plugin,
//! so the test suite runs the same on every OS without shell scripts.
//!
//! Steps run in order:
//! - `record <file>`   append `{"args", "cwd", "env": {TTGIT_*…}}` as a JSON line
//! - `echo <text>` / `stderr <text>`
//! - `big <bytes>`     print that many `x`s
//! - `sleep <ms>`
//! - `wait-file <path>` block until the file exists (max 30s)
//! - `call <method> <params-json>`  request over `$TTGIT_SOCKET_PATH`, print result
//! - `cli <args…>`     run `$TTGIT_BIN_PATH <args…>` (consumes the rest)
//! - `interactive <text>` print text, then echo typed lines until `q`
//! - `exit <code>`

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};
use serde_json::{Value, json};

pub fn run(steps: &[String]) -> Result<()> {
    let all: Vec<String> = steps.to_vec();
    let mut i = 0;
    while i < all.len() {
        let step = all[i].as_str();
        i += 1;
        let mut arg = || {
            let a = all.get(i).cloned().ok_or_else(|| eyre!("{step}: missing argument"));
            i += 1;
            a
        };
        match step {
            "record" => {
                let file = arg()?;
                let env: serde_json::Map<String, Value> = std::env::vars()
                    .filter(|(k, _)| k.starts_with("TTGIT_"))
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                let line = json!({
                    "args": all,
                    "cwd": std::env::current_dir()?.display().to_string(),
                    "env": env,
                });
                // One write per record so concurrent hook processes appending
                // to the same file never interleave within a line.
                let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
                f.write_all(format!("{line}\n").as_bytes())?;
            }
            "echo" => println!("{}", arg()?),
            "stderr" => eprintln!("{}", arg()?),
            "big" => {
                let n: usize = arg()?.parse()?;
                let mut out = std::io::stdout().lock();
                out.write_all(&vec![b'x'; n])?;
            }
            "sleep" => std::thread::sleep(Duration::from_millis(arg()?.parse()?)),
            "wait-file" => {
                let path = arg()?;
                let start = Instant::now();
                while !std::path::Path::new(&path).exists() {
                    if start.elapsed() > Duration::from_secs(30) {
                        return Err(eyre!("timed out waiting for {path}"));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            "call" => {
                let method = arg()?;
                let params: Value = serde_json::from_str(&arg()?)?;
                let address = std::env::var("TTGIT_SOCKET_PATH")?;
                let result = tt_git_plugin::socket::call(&address, &method, params)?;
                println!("{result}");
            }
            "cli" => {
                let bin = std::env::var("TTGIT_BIN_PATH")?;
                let status = std::process::Command::new(bin).args(&all[i..]).status()?;
                if !status.success() {
                    return Err(eyre!("cli step failed: {status}"));
                }
                return Ok(());
            }
            "interactive" => {
                println!("{}", arg()?);
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let line = line?;
                    if line.trim() == "q" {
                        break;
                    }
                    println!("you typed: {line}");
                }
            }
            "exit" => std::process::exit(arg()?.parse()?),
            other => return Err(eyre!("unknown test-plugin step {other:?}")),
        }
    }
    Ok(())
}
