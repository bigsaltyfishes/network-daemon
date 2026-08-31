//! network-manager-tui: a terminal UI for the network daemon.
//!
//! Connects to the daemon's control socket (`/var/run/network-daemon/network-daemon.sock`)
//! and lets the user list interfaces, scan for Wi-Fi, and manage known networks.

mod app;
mod client;

use std::{
    io::{self, Stdout},
    path::PathBuf,
    time::Duration,
};

use app::{App, View};
use client::DaemonClient;
use crossterm::{
    ExecutableCommand,
    event::{Event, KeyCode, KeyEventKind, poll, read},
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use libnetwork_daemon::{
    DaemonCommand, InterfaceManagerAction, WiFiManagerAction,
};
use ratatui::Terminal;

fn main() {
    run().unwrap_or_else(|e| {
        eprintln!("network-manager-tui: {e}");
        std::process::exit(1);
    });
}

/// Run the TUI against the daemon socket.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let socket =
        PathBuf::from(std::env::var("NETWORK_DAEMON_SOCK").unwrap_or_else(
            |_| "/var/run/network-daemon/network-daemon.sock".to_string(),
        ));

    let mut client = DaemonClient::connect(&socket)?;

    // Pull initial data.
    let mut app = App::default();
    let interfaces = client.request(&DaemonCommand::InterfaceManager {
        action: InterfaceManagerAction::GetAllInterfaces,
    })?;
    if let libnetwork_daemon::DaemonResponse::InterfaceManager {
        response: libnetwork_daemon::InterfaceResponse::InfoList(list),
    } = interfaces
    {
        app.interfaces = list;
    }

    // Set the TUI into raw mode / alternate screen.
    let mut stdout: Stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Write a probe to stderr so it doesn't corrupt the TUI.
    eprintln!("network-manager-tui: connected to {:?}", socket);

    let result = event_loop(&mut terminal, &mut app, &mut client);
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    result
}

/// The main terminal event loop.
fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    app: &mut App,
    client: &mut DaemonClient,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|f| f.render_widget(&*app, f.area()))?;

        if !poll(Duration::from_millis(250))? {
            continue;
        }
        let event = read()?;
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('i') => app.view = View::Interfaces,
                KeyCode::Char('n') => app.view = View::Networks,
                KeyCode::Char('s') => {
                    if let Some(iface) =
                        app.interfaces.get(app.interface_selected)
                    {
                        let name = iface.name.clone();
                        on_scan(client, app, &name);
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(iface) =
                        app.interfaces.get(app.interface_selected)
                    {
                        let name = iface.name.clone();
                        on_connect(client, app, &name);
                    }
                }
                KeyCode::Down => match app.view {
                    View::Interfaces => {
                        app.interface_selected = (app.interface_selected + 1)
                            .min(app.interfaces.len().saturating_sub(1));
                    }
                    View::Networks => {
                        app.network_selected = (app.network_selected + 1)
                            .min(app.known_networks.len().saturating_sub(1));
                    }
                },
                KeyCode::Up => match app.view {
                    View::Interfaces => {
                        app.interface_selected =
                            app.interface_selected.saturating_sub(1);
                    }
                    View::Networks => {
                        app.network_selected =
                            app.network_selected.saturating_sub(1);
                    }
                },
                KeyCode::Char('r') => {
                    app.interfaces = match client.request(&DaemonCommand::InterfaceManager {
                            action: InterfaceManagerAction::GetAllInterfaces,
                        }) {
                            Ok(libnetwork_daemon::DaemonResponse::InterfaceManager {
                                response: libnetwork_daemon::InterfaceResponse::InfoList(l),
                            }) => l,
                            _ => app.interfaces.clone(),
                        };
                }
                _ => {}
            }
        }
    }
}

/// Trigger a Wi-Fi scan and cache results.
fn on_scan(client: &mut DaemonClient, app: &mut App, iface: &str) {
    app.status = format!("Scanning {}...", iface);
    let result = client.request(&DaemonCommand::WiFiManager {
        iface: iface.to_string(),
        action: WiFiManagerAction::ScanResults,
    });
    match result {
        Ok(libnetwork_daemon::DaemonResponse::WiFiManager {
            response:
                libnetwork_daemon::WiFiManagerResponse::ScanResults(results),
            ..
        }) => {
            app.scan_results = results;
            app.status =
                format!("{} results for {}", app.scan_results.len(), iface);
        }
        Ok(_) => {
            app.error = Some("Scan returned unexpected response".to_string())
        }
        Err(e) => app.error = Some(format!("Scan failed for {iface}: {e}")),
    }
}

/// Connect to a Wi-Fi network (by the selected scan result, if any).
fn on_connect(client: &mut DaemonClient, app: &mut App, iface: &str) {
    let Some(ssid) = app
        .scan_results
        .get(app.network_selected)
        .map(|r| r.ssid.clone())
    else {
        app.error = Some("No network selected to connect".into());
        return;
    };
    match client.request(&DaemonCommand::WiFiManager {
        iface: iface.to_string(),
        action: WiFiManagerAction::Connect { ssid, bssid: None },
    }) {
        Ok(libnetwork_daemon::DaemonResponse::WiFiManager {
            response: libnetwork_daemon::WiFiManagerResponse::Success(()),
            ..
        }) => {
            app.status = format!("Connecting on {iface}");
        }
        Ok(_) => app.error = Some(format!("Connect handled for {iface}")),
        Err(e) => app.error = Some(format!("Connect error: {e}")),
    }
}
