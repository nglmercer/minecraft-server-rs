//! A stand-in for the `java` launcher, used by the supervisor tests.
//!
//! This is a Rust binary rather than a shell script so the tests run on Windows
//! as well as Unix. A script would have needed a `.cmd` twin, and the two would
//! have drifted — leaving the platform with no CI the least tested.
//!
//! It validates `-jar` exactly as the real launcher does, then performs the
//! instructions that follow it. Those arrive through the server's own
//! `server_args`, so a test drives behaviour through the same configuration
//! path a real server uses:
//!
//! * `done` — log the line a Minecraft server prints once the world is loaded.
//! * `serve` — read stdin, echo each line, exit 0 on `stop`.
//! * `hang` — ignore stdin forever, like a deadlocked server.
//! * `spam:<n>` — print `n` lines, to exercise console trimming.
//! * `err:<text>` — write to stderr.
//! * `exit:<code>` — exit immediately with that status.

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // The real launcher fails this way, and the tests depend on it: a fake that
    // ignored its arguments could not notice a broken jar path.
    let jar = args
        .windows(2)
        .find(|pair| pair[0] == "-jar")
        .map(|pair| &pair[1]);

    let Some(jar) = jar else {
        eprintln!("Error: no -jar argument was passed");
        std::process::exit(1);
    };
    if !std::path::Path::new(jar).is_file() {
        eprintln!("Error: Unable to access jarfile {jar}");
        std::process::exit(1);
    }

    let instructions = args
        .iter()
        .skip_while(|arg| arg.as_str() != "-jar")
        .skip(2)
        .cloned()
        .collect::<Vec<_>>();

    for instruction in instructions {
        match instruction.split_once(':') {
            Some(("exit", code)) => std::process::exit(code.parse().unwrap_or(0)),
            Some(("err", text)) => eprintln!("{text}"),
            Some(("spam", count)) => {
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                for i in 0..count.parse().unwrap_or(0) {
                    let _ = writeln!(out, "line {i}");
                }
                let _ = out.flush();
            }
            _ => match instruction.as_str() {
                "done" => {
                    println!(r#"[11:05:24 INFO]: Done (1.234s)! For help, type "help""#);
                    let _ = std::io::stdout().flush();
                }
                "serve" => serve(),
                "hang" => loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                },
                other => eprintln!("fake-java: unknown instruction {other}"),
            },
        }
    }
}

/// Read commands until told to stop, the way a server console does.
fn serve() -> ! {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim() == "stop" {
            println!("Stopping server");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        println!("echoed: {}", line.trim());
        let _ = std::io::stdout().flush();
    }
    std::process::exit(0);
}
