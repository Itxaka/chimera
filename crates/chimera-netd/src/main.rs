mod netops;

use std::collections::HashMap;
use std::process::exit;

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            m.insert(key.to_string(), args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    m
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: chimera-netd <create-tap|delete-tap> [flags]");
        exit(2);
    }
    let sub = args[0].as_str();
    let flags = parse_flags(&args[1..]);

    let result = match sub {
        "create-tap" => {
            let tap = require(&flags, "tap");
            let bridge = require(&flags, "bridge");
            let user = require(&flags, "user");
            netops::run_cmds(netops::create_tap_cmds(&tap, &bridge, &user))
        }
        "delete-tap" => {
            let tap = require(&flags, "tap");
            netops::run_cmds(netops::delete_tap_cmds(&tap))
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("chimera-netd: {e}");
        exit(1);
    }
}

fn require(flags: &HashMap<String, String>, key: &str) -> String {
    match flags.get(key) {
        Some(v) => v.clone(),
        None => {
            eprintln!("missing required flag --{key}");
            exit(2);
        }
    }
}
