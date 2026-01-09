use libnetwork_daemon::{
    DaemonCommand, InterfaceManagerAction, InterfaceType, LinkOptions,
    WiFiManagerAction, WlanLinkOptions,
};
use schemars::schema_for;

fn main() {
    let schema = schema_for!(DaemonCommand);
    // println!("{}", serde_json::to_string_pretty(&schema).unwrap());
    let command = DaemonCommand::InterfaceManager {
        action: InterfaceManagerAction::AddLink {
            name: "wlan0".to_string(),
            kind: InterfaceType::Wlan,
            options: Some(LinkOptions::Wlan {
                parent: "iwlwifi0".to_string(),
                options: WlanLinkOptions {
                    ..Default::default()
                },
            }),
        },
    };
    println!("{}", serde_json::to_string(&command).unwrap());
}
