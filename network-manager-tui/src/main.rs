//! network-manager-tui: a terminal UI for the network daemon.
//!
//! Connects to the daemon's control socket and lets the user manage interfaces,
//! scan / connect to Wi-Fi, and edit saved networks — aligned with nmtui.

mod app;
mod client;

use std::{
    io::{self, Stdout},
    net::Ipv4Addr,
    path::PathBuf,
    time::Duration,
};

use app::{App, FieldKind, Form, Modal, View};
use client::DaemonClient;
use crossterm::ExecutableCommand;
use crossterm::event::{Event, KeyCode, KeyEventKind, poll, read};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use libnetwork_daemon::{
    DaemonCommand, DaemonResponse, InterfaceManagerAction, InterfaceResponse,
    MacAddr, Modification, PrefixedIpv4Addr, ScanResult, Security, WpaState,
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

    let mut client = match DaemonClient::connect(&socket) {
        Ok(c) => c,
        Err(e) => {
            use client::ClientError;
            let hint = match &e {
                ClientError::Connect(io)
                    if io.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    "\n  Hint: you must be a member of the 'network' group\n  to talk to the daemon. Add yourself, e.g.:\n    sudo pw groupmod network -m $USER\n  then log out/in (or reconnect SSH) for it to take effect."
                }
                _ => {
                    "\n  Hint: is the network-daemon running? (sudo service network-daemon start)"
                }
            };
            eprintln!(
                "failed to connect to daemon at {}: {}",
                socket.display(),
                e
            );
            eprintln!("{hint}");
            return Ok(());
        }
    };

    let mut app = App::default();
    refresh_interfaces(&mut client, &mut app);
    refresh_wifi(&mut client, &mut app);

    let mut stdout: Stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    eprintln!("network-manager-tui: connected to {:?}", socket);

    let result = event_loop(&mut terminal, &mut app, &mut client);
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    result
}

/// Main terminal event loop.
fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    app: &mut App,
    client: &mut DaemonClient,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|f| app.draw(f))?;

        if matches!(app.modal, Modal::Connecting { .. }) {
            poll_connection(client, app);
        }

        if !poll(Duration::from_millis(200))? {
            continue;
        }
        let event = read()?;
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            if !matches!(app.modal, Modal::None) {
                handle_modal_key(key.code, app, client);
            } else {
                handle_view_key(key.code, app, client);
            }
        }
    }
}

/// Key handling when no modal is open.
fn handle_view_key(key: KeyCode, app: &mut App, client: &mut DaemonClient) {
    match app.view {
        View::Interfaces => match key {
            KeyCode::Char('q') => std::process::exit(0),
            KeyCode::Down => {
                app.interface_selected = (app.interface_selected + 1)
                    .min(app.interfaces.len().saturating_sub(1));
            }
            KeyCode::Up => {
                app.interface_selected =
                    app.interface_selected.saturating_sub(1);
            }
            KeyCode::Enter => enter_interface(app, client),
            KeyCode::Char('r') => refresh_interfaces(client, app),
            KeyCode::Char('e') => edit_interface(app),
            KeyCode::Char('n') => app.view = View::Networks,
            _ => {}
        },
        View::Wifi => match key {
            KeyCode::Char('q') => std::process::exit(0),
            KeyCode::Char('w') | KeyCode::Esc => app.view = View::Interfaces,
            KeyCode::Char('n') => app.view = View::Networks,
            KeyCode::Down => {
                app.network_selected = (app.network_selected + 1)
                    .min(app.scan_results.len().saturating_sub(1));
            }
            KeyCode::Up => {
                app.network_selected = app.network_selected.saturating_sub(1);
            }
            KeyCode::Char('s') => scan(client, app),
            KeyCode::Enter => wifi_connect(app, client),
            KeyCode::Char('e') => edit_wifi_ap(app, client),
            KeyCode::Char('r') => refresh_wifi(client, app),
            _ => {}
        },
        View::Networks => match key {
            KeyCode::Char('q') => std::process::exit(0),
            KeyCode::Char('n') | KeyCode::Esc => app.view = View::Interfaces,
            KeyCode::Down => {
                app.network_selected = (app.network_selected + 1)
                    .min(app.known_networks.len().saturating_sub(1));
            }
            KeyCode::Up => {
                app.network_selected = app.network_selected.saturating_sub(1);
            }
            KeyCode::Char('e') => edit_known_ap(app, client),
            KeyCode::Char('r') => refresh_wifi(client, app),
            _ => {}
        },
    }
}

/// Key handling when a modal is open.
fn handle_modal_key(key: KeyCode, app: &mut App, client: &mut DaemonClient) {
    match &mut app.modal {
        Modal::Message { .. } => match key {
            KeyCode::Enter | KeyCode::Esc => app.modal = Modal::None,
            _ => {}
        },
        Modal::Connecting { .. } => {
            if key == KeyCode::Esc {
                app.modal = Modal::None;
            }
        }
        Modal::Password { input, .. } => match key {
            KeyCode::Char(c) => input.push(c),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => password_confirm(app, client),
            KeyCode::Esc => app.modal = Modal::None,
            _ => {}
        },
        Modal::Edit { .. } => {
            // Scope the mutable borrow of the form so we can call
            // `edit_confirm(app, ..)` after the borrow ends (avoids double
            // borrow of `app`).
            let mut save = false;
            {
                if let Modal::Edit { form, .. } = &mut app.modal {
                    if form.selection_open.is_some() {
                        // A selection popup is open — navigate/confirm options.
                        match key {
                            KeyCode::Down => {
                                let len = form
                                    .selection_open
                                    .and_then(|i| form.fields.get(i))
                                    .and_then(|f| match &f.kind {
                                        FieldKind::Selection { options } => {
                                            Some(options.len())
                                        }
                                        _ => None,
                                    })
                                    .unwrap_or(0);
                                form.selection_cursor =
                                    (form.selection_cursor + 1) % len.max(1);
                            }
                            KeyCode::Up => {
                                form.selection_cursor =
                                    form.selection_cursor.saturating_sub(1);
                            }
                            KeyCode::Enter => confirm_selection(form),
                            KeyCode::Esc => form.selection_open = None,
                            _ => {}
                        }
                    } else {
                        match key {
                            KeyCode::Down => move_focus(form, 1),
                            KeyCode::Up => move_focus(form, -1),
                            KeyCode::Enter => {
                                let n = form.flat_rows().len();
                                if n == 0 {
                                    return;
                                }
                                let flat = form.focus.min(n - 1);
                                let (top, _) = form.flat_rows()[flat];
                                match &form.fields[top].kind {
                                    FieldKind::Selection { .. } => {
                                        form.selection_open = Some(top);
                                        form.selection_cursor = 0;
                                    }
                                    _ => save = true,
                                }
                            }
                            KeyCode::Backspace => {
                                edit_field_key(form, KeyCode::Backspace)
                            }
                            KeyCode::Char(c) => {
                                edit_field_key(form, KeyCode::Char(c))
                            }
                            KeyCode::Esc => app.modal = Modal::None,
                            _ => {}
                        }
                    }
                }
            }
            if save {
                edit_confirm(app, client);
            }
        }
        Modal::None => {}
    }
}

/// Move the edit-menu focus across visible (flat) rows.
fn move_focus(form: &mut Form, delta: isize) {
    let n = form.flat_rows().len();
    if n == 0 {
        return;
    }
    let cur = form.focus as isize;
    let next = (cur + delta).clamp(0, n as isize - 1);
    form.focus = next as usize;
}

/// Edit the focused field for a non-Enter key.
fn edit_field_key(form: &mut Form, key: KeyCode) {
    let n = form.flat_rows().len();
    if n == 0 {
        return;
    }
    let flat = form.focus.min(n - 1);
    let (top, _) = form.flat_rows()[flat];
    let field = &mut form.fields[top];
    match &field.kind {
        FieldKind::Selection { .. } => {
            // Selection fields are changed via the popup; ignore text keys.
        }
        FieldKind::Bool => match key {
            KeyCode::Char('y') | KeyCode::Char(' ') => {
                field.value = if field.value == "yes" {
                    "no".into()
                } else {
                    "yes".into()
                };
            }
            _ => {}
        },
        FieldKind::Text => match key {
            KeyCode::Char(c) => field.value.push(c),
            KeyCode::Backspace => {
                field.value.pop();
            }
            _ => {}
        },
    }
}

/// Confirm a selection-popup choice.
fn confirm_selection(form: &mut Form) {
    let Some(top) = form.selection_open else {
        return;
    };
    let Some(field) = form.fields.get_mut(top) else {
        return;
    };
    if let FieldKind::Selection { options } = &field.kind
        && let Some(chosen) = options.get(form.selection_cursor)
    {
        field.value = chosen.clone();
        // Expand child config fields when "手动" (first non-DHCP option).
        field.expanded = chosen != "DHCP" && !field.children.is_empty();
    }
    form.selection_open = None;
}

/// Pressing Enter on an interface: go into Wi-Fi for wlan, else toggle up/down.
fn enter_interface(app: &mut App, client: &mut DaemonClient) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    if iface.is_wlan() {
        app.view = View::Wifi;
        refresh_wifi(client, app);
        return;
    }
    let up = iface.state.is_up();
    let action = InterfaceManagerAction::ModLink {
        name: iface.name.clone(),
        ipv4: Modification::NoChange,
        ipv6: Modification::NoChange,
        oper_state: Modification::Replace(!up),
        slaac: Modification::NoChange,
    };
    send_interface_action(client, action);
    refresh_interfaces(client, app);
}

/// Open the interface-edit modal for the selected interface.
fn edit_interface(app: &mut App) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    let form = App::interface_form(&iface);
    app.modal = Modal::Edit {
        title: "Edit Interface".into(),
        iface: iface.name.clone(),
        form,
    };
}

/// Scan the selected wlan interface.
fn scan(client: &mut DaemonClient, app: &mut App) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        app.error = Some("select a wlan interface first".into());
        return;
    };
    app.status = format!("Scanning {}...", iface.name);
    if let Err(e) = client.request(&DaemonCommand::WiFiManager {
        iface: iface.name.clone(),
        action: libnetwork_daemon::WiFiManagerAction::Scan,
    }) {
        app.error = Some(format!("Scan failed: {e}"));
        return;
    }
    fetch_scan_results(client, app, &iface.name);
}

/// Refresh wlan status + scan results + known networks.
fn refresh_wifi(client: &mut DaemonClient, app: &mut App) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    if !iface.is_wlan() {
        return;
    }
    fetch_scan_results(client, app, &iface.name);
    fetch_status(client, app, &iface.name);
    fetch_known_networks(client, app, &iface.name);
}

fn fetch_scan_results(client: &mut DaemonClient, app: &mut App, iface: &str) {
    if let Ok(DaemonResponse::WiFiManager {
        response: libnetwork_daemon::WiFiManagerResponse::ScanResults(results),
        ..
    }) = client.request(&DaemonCommand::WiFiManager {
        iface: iface.to_string(),
        action: libnetwork_daemon::WiFiManagerAction::ScanResults,
    }) {
        app.scan_results = results;
    }
}

fn fetch_status(client: &mut DaemonClient, app: &mut App, iface: &str) {
    if let Ok(DaemonResponse::WiFiManager {
        response: libnetwork_daemon::WiFiManagerResponse::Status(status),
        ..
    }) = client.request(&DaemonCommand::WiFiManager {
        iface: iface.to_string(),
        action: libnetwork_daemon::WiFiManagerAction::Status,
    }) {
        app.wifi_status = Some(status);
    }
}

fn fetch_known_networks(client: &mut DaemonClient, app: &mut App, iface: &str) {
    if let Ok(DaemonResponse::WiFiManager {
        response: libnetwork_daemon::WiFiManagerResponse::KnownNetworks(nets),
        ..
    }) = client.request(&DaemonCommand::WiFiManager {
        iface: iface.to_string(),
        action: libnetwork_daemon::WiFiManagerAction::KnownNetworks,
    }) {
        app.known_networks = nets;
    }
}

/// Connect to the selected scan result.
fn wifi_connect(app: &mut App, client: &mut DaemonClient) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    let Some(scan) = app.scan_results.get(app.network_selected).cloned() else {
        app.error = Some("no network selected".into());
        return;
    };

    let is_known = app.known_networks.iter().any(|n| {
        n.ssid == scan.ssid
            && n.bssid.map(|b| b.to_string()) == Some(scan.bssid.to_string())
    });
    if is_known {
        connect(app, client, iface.name, scan.ssid, Some(scan.bssid), None);
        return;
    }
    if scan.security == Security::Open {
        add_and_connect(app, client, iface.name, scan, Security::Open, None);
        return;
    }
    app.modal = Modal::Password {
        ssid: scan.ssid.clone(),
        bssid: Some(scan.bssid.to_string()),
        iface: iface.name,
        input: String::new(),
    };
}

/// Submit a password from the modal.
fn password_confirm(app: &mut App, client: &mut DaemonClient) {
    let (ssid, bssid_str, iface, pwd) = match &app.modal {
        Modal::Password {
            ssid,
            bssid,
            iface,
            input,
        } => (ssid.clone(), bssid.clone(), iface.clone(), input.clone()),
        _ => return,
    };
    let scan = app
        .scan_results
        .iter()
        .find(|r| {
            r.ssid == ssid
                && r.bssid.to_string() == bssid_str.clone().unwrap_or_default()
        })
        .cloned();
    match scan {
        Some(s) => {
            let security = s.security;
            add_and_connect(app, client, iface, s, security, Some(pwd))
        }
        None => {
            app.modal = Modal::Message {
                title: "Error".into(),
                message: "network not in scan results".into(),
            }
        }
    }
}

/// Start a connection (known network).
fn connect(
    app: &mut App,
    client: &mut DaemonClient,
    iface: String,
    ssid: String,
    bssid: Option<MacAddr>,
    _pwd: Option<String>,
) {
    let name = ssid.clone();
    match client.request(&DaemonCommand::WiFiManager {
        iface: iface.clone(),
        action: libnetwork_daemon::WiFiManagerAction::Connect { ssid, bssid },
    }) {
        Ok(_) => {
            app.modal = Modal::Connecting { ssid: name.clone() };
            app.status = format!("Connecting {name}...");
        }
        Err(e) => {
            app.modal = Modal::Message {
                title: "Connect failed".into(),
                message: format!("{e}"),
            }
        }
    }
}

/// Add a network then connect.
fn add_and_connect(
    app: &mut App,
    client: &mut DaemonClient,
    iface: String,
    scan: ScanResult,
    _security: Security,
    password: Option<String>,
) {
    let bssid = Some(scan.bssid);
    let _ = client.request(&DaemonCommand::WiFiManager {
        iface: iface.clone(),
        action: libnetwork_daemon::WiFiManagerAction::AddNetwork {
            ssid: scan.ssid.clone(),
            bssid,
            security: scan.security,
            password,
            identity: None,
            hidden: false,
        },
    });
    let _ = client.request(&DaemonCommand::WiFiManager {
        iface: iface.clone(),
        action: libnetwork_daemon::WiFiManagerAction::Connect {
            ssid: scan.ssid.clone(),
            bssid,
        },
    });
    app.modal = Modal::Connecting {
        ssid: scan.ssid.clone(),
    };
    app.status = format!("Connecting {}...", scan.ssid);
}

/// Poll supplicant status while Connecting, closing modal when done/failed.
fn poll_connection(client: &mut DaemonClient, app: &mut App) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    if !iface.is_wlan() {
        return;
    }
    fetch_status(client, app, &iface.name);
    match &app.wifi_status {
        Some(s) if matches!(s.state, Some(WpaState::Completed)) => {
            app.modal = Modal::None;
            app.status = "Connected".into();
        }
        Some(s)
            if matches!(
                s.state,
                Some(WpaState::Disconnected)
                    | Some(WpaState::InterfaceDisabled)
            ) =>
        {
            let msg = format!(
                "Connection failed (state {:?})",
                s.state.unwrap_or(WpaState::Unknown)
            );
            app.modal = Modal::Message {
                title: "Connect failed".into(),
                message: msg,
            };
        }
        _ => {}
    }
}

/// Open the edit-AP modal for a scan result (from the Wi-Fi view).
fn edit_wifi_ap(app: &mut App, client: &mut DaemonClient) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    let Some(scan) = app.scan_results.get(app.network_selected).cloned() else {
        app.error = Some("no network to edit".into());
        return;
    };
    let form = App::ap_form_from_scan(&scan);
    app.modal = Modal::Edit {
        title: "Edit Network".into(),
        iface: iface.name,
        form,
    };
    let _ = client;
}

/// Open the edit-AP modal for a known network (Networks view).
fn edit_known_ap(app: &mut App, client: &mut DaemonClient) {
    let Some(iface) = app.interfaces.get(app.interface_selected).cloned()
    else {
        return;
    };
    let Some(net) = app.known_networks.get(app.network_selected).cloned()
    else {
        app.error = Some("no known network to edit".into());
        return;
    };
    let form = App::ap_form(&net);
    app.modal = Modal::Edit {
        title: "Edit Network".into(),
        iface: iface.name,
        form,
    };
    let _ = client;
}

/// Submit an edit form (interface or AP).
fn edit_confirm(app: &mut App, client: &mut DaemonClient) {
    let (title, iface, form) = match std::mem::take(&mut app.modal) {
        Modal::Edit { title, iface, form } => (title, iface, form),
        _ => return,
    };
    if title.starts_with("Edit Interface") {
        apply_interface_edit(client, iface, form);
    } else {
        apply_ap_edit(client, iface, form);
    }
}

/// Apply an interface-edit form (DHCP / SLAAC / static IPv4).
fn apply_interface_edit(client: &mut DaemonClient, iface: String, form: Form) {
    let v4 = form.fields[0].value.trim().to_string();
    let slaac = form.fields[1].value == "yes";

    let ipv4 = if v4.is_empty() {
        Modification::NoChange
    } else {
        match parse_prefixed_v4(&v4) {
            Some(p) => Modification::Replace(p),
            None => Modification::NoChange,
        }
    };

    let action = InterfaceManagerAction::ModLink {
        name: iface,
        ipv4,
        ipv6: Modification::NoChange,
        oper_state: Modification::NoChange,
        slaac: Modification::Replace(slaac),
    };
    send_interface_action(client, action);
}

/// Apply an AP-edit form (BSSID / password / auto-connect).
fn apply_ap_edit(client: &mut DaemonClient, iface: String, form: Form) {
    let ssid = form.fields[0].value.clone();
    let bssid_val = form.fields[1].value.trim().to_string();
    let pwd = form.fields[2].value.clone();
    let bssid = MacAddr::parse(&bssid_val);
    let _ = client.request(&DaemonCommand::WiFiManager {
        iface,
        action: libnetwork_daemon::WiFiManagerAction::AddNetwork {
            ssid,
            bssid,
            security: if pwd.is_empty() {
                Security::Open
            } else {
                Security::Psk
            },
            password: if pwd.is_empty() { None } else { Some(pwd) },
            identity: None,
            hidden: false,
        },
    });
}

/// Parse "a.b.c.d/prefix" into a PrefixedIpv4Addr.
fn parse_prefixed_v4(s: &str) -> Option<PrefixedIpv4Addr> {
    let (ip, prefix) = s.split_once('/')?;
    let addr: Ipv4Addr = ip.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    Some(PrefixedIpv4Addr::new(addr, prefix))
}

fn send_interface_action(
    client: &mut DaemonClient,
    action: InterfaceManagerAction,
) {
    if let Err(e) = client.request(&DaemonCommand::InterfaceManager { action })
    {
        eprintln!("interface action failed: {e}");
    }
}

/// Refresh the interface list into the app.
fn refresh_interfaces(client: &mut DaemonClient, app: &mut App) {
    if let Ok(DaemonResponse::InterfaceManager {
        response: InterfaceResponse::InfoList(list),
    }) = client.request(&DaemonCommand::InterfaceManager {
        action: InterfaceManagerAction::GetAllInterfaces,
    }) {
        app.interfaces = list;
    }
}
