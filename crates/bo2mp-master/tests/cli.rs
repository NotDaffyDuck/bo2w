//! The command line as `cargo xtask master` and the deploy scripts use it: the
//! systemd unit text, and the refusals for missing or unknown arguments.

use std::process;

fn run(argv: &[&str]) -> process::Output {
    process::Command::new(env!("CARGO_BIN_EXE_bo2mp-master"))
        .args(argv)
        .output()
        .expect("launch bo2mp-master")
}

fn refused_with(args: &[&str], wanted: &str) {
    let out = run(args);
    assert!(!out.status.success(), "{args:?} should fail");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains(wanted), "{args:?}: stderr {said:?} lacks {wanted:?}");
}

#[test]
fn unit_for_the_dev_channel() {
    let out = run(&[
        "print-unit",
        "--channel",
        "dev",
        "--exec",
        "/srv/bo2mp/bin/bo2mp-master-dev",
        "--cert",
        "/etc/bo2mp/dev.pem",
        "--key",
        "/etc/bo2mp/dev.key",
    ]);
    assert!(out.status.success());
    let expected = "[Unit]\n\
        Description=BO2MP dev master and relay\n\
        After=network-online.target\n\
        Wants=network-online.target\n\
        \n\
        [Service]\n\
        Type=simple\n\
        User=bo2mp\n\
        Group=bo2mp\n\
        ExecStart=/srv/bo2mp/bin/bo2mp-master-dev serve --bind 0.0.0.0:4434 --cert /etc/bo2mp/dev.pem --key /etc/bo2mp/dev.key\n\
        Restart=on-failure\n\
        RestartSec=2\n\
        NoNewPrivileges=true\n\
        PrivateDevices=true\n\
        PrivateTmp=true\n\
        ProtectHome=true\n\
        ProtectSystem=strict\n\
        \n\
        [Install]\n\
        WantedBy=multi-user.target\n";
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn unit_for_prod_with_its_own_account() {
    let out = run(&[
        "print-unit",
        "--user",
        "relay",
        "--group",
        "games",
        "--channel",
        "prod",
        "--exec",
        "/x/m",
        "--cert",
        "/x/c",
        "--key",
        "/x/k",
    ]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("Description=BO2MP prod master and relay\n"));
    assert!(text.contains("User=relay\nGroup=games\n"));
    assert!(text.contains("ExecStart=/x/m serve --bind 0.0.0.0:4433 --cert /x/c --key /x/k\n"));
}

#[test]
fn missing_arguments_are_named() {
    refused_with(&[], "usage: bo2mp-master serve|status|list|print-unit");
    refused_with(&["serve"], "serve requires --bind HOST:PORT");
    refused_with(&["serve", "--bind", "127.0.0.1:1"], "serve requires --cert PATH");
    refused_with(
        &["serve", "--bind", "127.0.0.1:1", "--cert", "c"],
        "serve requires --key PATH",
    );
    refused_with(&["status"], "command requires --connect HOST:PORT");
    refused_with(
        &["list", "--connect", "127.0.0.1:4433"],
        "command requires --server-name NAME",
    );
    refused_with(&["print-unit"], "print-unit requires --channel prod|dev");
    refused_with(&["print-unit", "--channel", "dev"], "print-unit requires --exec PATH");
    refused_with(
        &["print-unit", "--channel", "dev", "--exec", "e"],
        "print-unit requires --cert PATH",
    );
    refused_with(
        &["print-unit", "--channel", "dev", "--exec", "e", "--cert", "c"],
        "print-unit requires --key PATH",
    );
}

#[test]
fn bad_input_is_refused() {
    refused_with(&["frobnicate"], "unknown command frobnicate");
    refused_with(&["status", "--colour", "red"], "unknown option --colour");
    refused_with(&["status", "--connect"], "missing value for --connect");
    let out = run(&["print-unit", "--channel", "staging"]);
    assert!(!out.status.success());
    let out = run(&["serve", "--bind", "not-an-address"]);
    assert!(!out.status.success());
}
