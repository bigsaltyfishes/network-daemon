use libnetwork_daemon::{
    DaemonCommand, InterfaceManagerAction, InterfaceType, LinkOptions,
    WlanLinkOptions,
};
use schemars::schema_for;

fn main() {
    let _schema = schema_for!(DaemonCommand);
    // To regenerate the command schema, uncomment:
    // println!("{}", serde_json::to_string_pretty(&_schema).unwrap());

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
