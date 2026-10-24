//! The systemd unit `cargo xtask master install` copies into /etc/systemd/system.
//! This binary writes the `ExecStart` line that starts itself, so the unit and the
//! `serve` flags cannot drift apart; the channel alone picks the port.

use crate::args::UnitSettings;

/// The unit file text, ending in a newline.
pub fn render(settings: &UnitSettings) -> String {
    let UnitSettings {
        channel,
        exec,
        cert,
        key,
        user,
        group,
    } = settings;
    let start = format!(
        "{} serve --bind 0.0.0.0:{} --cert {} --key {}",
        exec.display(),
        channel.port(),
        cert.display(),
        key.display(),
    );
    let sections: [(&str, Vec<(&str, String)>); 3] = [
        (
            "Unit",
            vec![
                ("Description", format!("BO2MP {channel} master and relay")),
                ("After", "network-online.target".into()),
                ("Wants", "network-online.target".into()),
            ],
        ),
        (
            "Service",
            vec![
                ("Type", "simple".into()),
                ("User", user.clone()),
                ("Group", group.clone()),
                ("ExecStart", start),
                ("Restart", "on-failure".into()),
                ("RestartSec", "2".into()),
                // Lock the service out of everything it does not need.
                ("NoNewPrivileges", "true".into()),
                ("PrivateDevices", "true".into()),
                ("PrivateTmp", "true".into()),
                ("ProtectHome", "true".into()),
                ("ProtectSystem", "strict".into()),
            ],
        ),
        ("Install", vec![("WantedBy", "multi-user.target".into())]),
    ];
    let blocks: Vec<String> = sections
        .iter()
        .map(|(title, entries)| {
            let mut block = format!("[{title}]\n");
            for (name, value) in entries {
                block.push_str(&format!("{name}={value}\n"));
            }
            block
        })
        .collect();
    blocks.join("\n")
}
