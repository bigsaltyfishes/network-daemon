//! TUI application state and rendering.

use libnetwork_daemon::{
    InterfaceInfo, KnownNetwork, ScanResult, SupplicantStatus,
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, Paragraph, Widget},
};

/// Which main view the user is on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum View {
    #[default]
    Interfaces,
    /// Wi-Fi: connected AP header + scan results for a wlan interface.
    Wifi,
    /// Saved known networks.
    Networks,
}

/// A single editable field in a modal form.
#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub value: String,
    /// Whether the field is a value slot vs a fixed toggle.
    pub editable: bool,
    /// Hide input as `*` (passwords).
    pub password: bool,
}

impl Field {
    pub fn toggle_enabled() -> Field {
        Field {
            label: "Enabled".into(),
            value: "yes".into(),
            editable: true,
            password: false,
        }
    }
    pub fn text(label: &str, value: &str) -> Field {
        Field {
            label: label.to_string(),
            value: value.to_string(),
            editable: true,
            password: false,
        }
    }
    pub fn password(label: &str) -> Field {
        Field {
            label: label.to_string(),
            value: String::new(),
            editable: true,
            password: true,
        }
    }
    pub fn fixed(label: &str, value: &str) -> Field {
        Field {
            label: label.to_string(),
            value: value.to_string(),
            editable: false,
            password: false,
        }
    }
}

/// A modal form: a list of fields with a cursor.
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        Self { fields, focus: 0 }
    }
    pub fn current(&mut self) -> &mut Field {
        let i = self.focus.min(self.fields.len().saturating_sub(1));
        &mut self.fields[i]
    }
}

/// A modal popup shown over the main view.
#[derive(Debug, Default)]
pub enum Modal {
    #[default]
    None,
    /// An informational popup (dismiss with Enter/Esc).
    Message { title: String, message: String },
    /// Password prompt for a PSK network.
    Password {
        ssid: String,
        bssid: Option<String>,
        iface: String,
        input: String,
    },
    /// "Connecting..." while a Wi-Fi association is in flight.
    Connecting { ssid: String },
    /// Edit an interface (DHCP / SLAAC / static IP / DNS) or a saved AP.
    Edit {
        title: String,
        iface: String,
        form: Form,
    },
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub interfaces: Vec<InterfaceInfo>,
    pub scan_results: Vec<ScanResult>,
    pub known_networks: Vec<KnownNetwork>,
    /// Current supplicant status for the selected wlan interface.
    pub wifi_status: Option<SupplicantStatus>,
    pub interface_selected: usize,
    pub network_selected: usize,
    pub modal: Modal,
    pub status: String,
    pub error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            view: View::default(),
            interfaces: Vec::new(),
            scan_results: Vec::new(),
            known_networks: Vec::new(),
            wifi_status: None,
            interface_selected: 0,
            network_selected: 0,
            modal: Modal::None,
            status: "Connected. Select an interface below.".into(),
            error: None,
        }
    }
}

impl App {
    /// Render the interface list as ratatui line items.
    pub fn interface_lines(&self) -> Vec<Line<'static>> {
        self.interfaces
            .iter()
            .map(|i| {
                let state = i.state.to_string();
                let ip = i
                    .ipv4_addrs
                    .first()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "-".into());
                let iftype = format!("{:?}", i.interface_type);
                let mark = if i.state.is_online() { "●" } else { "○" };
                let sel = if i.is_wlan() {
                    ", wifi: Enter"
                } else {
                    ", Enter: toggle"
                };
                Line::from(vec![
                    Span::styled(
                        format!("{mark} {:<8}", i.name),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(" {:<12}", iftype)),
                    Span::raw(format!(" {:<14}", state)),
                    Span::raw(format!(" {:<20}", ip)),
                    Span::raw(format!(
                        " {:<22}",
                        i.ipv6_addrs
                            .first()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "-".into())
                    )),
                    Span::raw(sel),
                ])
            })
            .collect()
    }

    /// Render known networks as list items.
    pub fn network_lines(&self) -> Vec<Line<'static>> {
        if self.known_networks.is_empty() {
            return vec![Line::from("(no known networks — e to add)")];
        }
        self.known_networks
            .iter()
            .map(|n| {
                let security = n.security.to_string();
                let state = format!("{:?}", n.state);
                let bssid = n
                    .bssid
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "any".into());
                Line::from(vec![
                    Span::styled(
                        format!("{:<28}", n.ssid),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(format!(" {:<8}", security)),
                    Span::raw(format!(" {:<18}", bssid)),
                    Span::raw(format!(" {}", state)),
                ])
            })
            .collect()
    }

    /// Wi-Fi view: a connected-AP header line plus scan results.
    pub fn wifi_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        // Connected AP header.
        match &self.wifi_status {
            Some(s) => {
                let ssid = s.ssid.clone().unwrap_or_else(|| "-".into());
                let state = s
                    .state
                    .map(|st| format!("{st:?}"))
                    .unwrap_or_else(|| "-".into());
                let signal = self
                    .scan_results
                    .iter()
                    .find(|r| Some(r.ssid.as_str()) == s.ssid.as_deref())
                    .map(|r| format!("{} dBm", r.signal))
                    .unwrap_or_else(|| "-".into());
                out.push(Line::from(vec![
                    Span::styled(
                        "Connected: ",
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(format!("{ssid:<28}")),
                    Span::raw(format!(" {state}")),
                    Span::raw(format!("  signal {signal}")),
                ]));
                out.push(Line::from(""));
            }
            None => out.push(Line::from("(no Wi-Fi connection)")),
        }
        // Scan results.
        if self.scan_results.is_empty() {
            out.push(Line::from("(no scan results — press s to scan)"));
            return out;
        }
        out.extend(self.scan_results.iter().map(|r| {
            let signal = format!("{:>4} dBm", r.signal);
            let channel = format!("{:>2}", r.channel());
            Line::from(vec![
                Span::styled(
                    format!("{:<28}", r.ssid),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(format!(" {:<8}", r.security.to_string())),
                Span::raw(format!(" ch {:<3}", channel)),
                Span::raw(format!(" {}", signal)),
                Span::raw(format!(" {:>18}", r.bssid.to_string())),
            ])
        }));
        out
    }

    /// Build the interface-edit form for a selected interface.
    pub fn interface_form(iface: &InterfaceInfo) -> Form {
        let v4 = iface
            .ipv4_addrs
            .first()
            .map(|a| a.to_string())
            .unwrap_or_default();
        Form::new(vec![
            Field {
                label: "IPv4 (blank = DHCP)".into(),
                value: if iface.dhcpv4_enabled {
                    String::new()
                } else {
                    v4
                },
                editable: true,
                password: false,
            },
            Field {
                label: "SLAAC (IPv6 auto)".into(),
                value: if iface.slaac_enabled {
                    "yes".into()
                } else {
                    "no".into()
                },
                editable: true,
                password: false,
            },
            Field {
                label: "DHCPv4".into(),
                value: if iface.dhcpv4_enabled {
                    "yes".into()
                } else {
                    "no".into()
                },
                editable: true,
                password: false,
            },
        ])
    }

    /// Build an edit-AP form from a scan result (network not yet saved).
    pub fn ap_form_from_scan(scan: &ScanResult) -> Form {
        Form::new(vec![
            Field {
                label: "SSID".into(),
                value: scan.ssid.clone(),
                editable: false,
                password: false,
            },
            Field {
                label: "BSSID (any = auto)".into(),
                value: scan.bssid.to_string(),
                editable: true,
                password: false,
            },
            Field {
                label: "Password (PSK)".into(),
                value: String::new(),
                editable: true,
                password: true,
            },
        ])
    }

    /// Build the edit-saved-AP form for a known network.
    pub fn ap_form(net: &KnownNetwork) -> Form {
        Form::new(vec![
            Field {
                label: "SSID".into(),
                value: net.ssid.clone(),
                editable: false,
                password: false,
            },
            Field {
                label: "BSSID (any = auto)".into(),
                value: net.bssid.map(|b| b.to_string()).unwrap_or_default(),
                editable: true,
                password: false,
            },
            Field {
                label: "Password (PSK)".into(),
                value: String::new(),
                editable: true,
                password: true,
            },
            Field {
                label: "Auto-connect".into(),
                value: if net.is_enabled() {
                    "yes".into()
                } else {
                    "no".into()
                },
                editable: true,
                password: false,
            },
        ])
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header / status
                Constraint::Min(8),    // body
                Constraint::Length(3), // footer hint
            ])
            .split(area);

        // Header status bar.
        let status_style = if self.error.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let status_text =
            self.error.clone().unwrap_or_else(|| self.status.clone());
        let header =
            Paragraph::new(Line::from(status_text)).style(status_style);
        buf.set_style(chunks[0], status_style);
        header.render(chunks[0], buf);

        // Body.
        let body = match self.view {
            View::Interfaces => List::new(self.interface_lines())
                .block(
                    Block::default().borders(Borders::ALL).title("Interfaces"),
                )
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("> "),
            View::Wifi => List::new(self.wifi_lines())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Wi-Fi Networks"),
                )
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("> "),
            View::Networks => List::new(self.network_lines())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Known Networks"),
                )
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("> "),
        };
        body.render(chunks[1], buf);

        // Footer hint.
        let footer = Line::from(vec![
            Span::raw(" q=quit  i=ifaces  w=wifi  n=networks  r=refresh  "),
            Span::styled(
                "↑/↓ move  Enter=act  e=edit  s=scan  Esc=back",
                Style::default().fg(Color::Cyan),
            ),
        ]);
        Paragraph::new(footer)
            .block(Block::default().borders(Borders::TOP).title("Hint"))
            .render(chunks[2], buf);

        // Modal overlay.
        if !matches!(self.modal, Modal::None) {
            self.render_modal(area, buf);
        }
    }
}

impl App {
    /// Render the currently active modal as a centered popup.
    fn render_modal(&self, area: Rect, buf: &mut Buffer) {
        let (title, lines, input_hint) = match &self.modal {
            Modal::None => return,
            Modal::Message { title, message } => (
                title.clone(),
                vec![message.clone()],
                String::from(" Enter/Esc to close "),
            ),
            Modal::Connecting { ssid } => (
                "Connecting…".to_string(),
                vec![format!("Connecting to {ssid}")],
                " please wait ".into(),
            ),
            Modal::Password { ssid, input, .. } => (
                format!("Password for {ssid}"),
                vec![format!("Password: {}", mask(input))],
                " Enter=connect  Esc=cancel ".into(),
            ),
            Modal::Edit { title, iface, form } => {
                let mut l = Vec::new();
                for (i, f) in form.fields.iter().enumerate() {
                    let cursor = if i == form.focus { ">" } else { " " };
                    let shown = if f.password {
                        mask(&f.value)
                    } else if f.value.is_empty() {
                        " ".to_string()
                    } else {
                        f.value.clone()
                    };
                    l.push(format!("{cursor} {:<18} {}", f.label, shown));
                }
                (
                    format!("{title} — {iface}"),
                    l,
                    " ↑/↓ move  Enter=save  Esc=cancel ".into(),
                )
            }
        };

        // Center the popup.
        let height = lines.len() as u16 + 4;
        let width = 52;
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width).saturating_div(2),
            y: area.y + area.height.saturating_sub(height).saturating_div(2),
            width,
            height,
        };
        buf.set_style(popup, Style::default().bg(Color::DarkGray));
        Clear.render(popup, buf);

        let mut content = lines
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect::<Vec<_>>();
        content.push(Line::from(input_hint));
        Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .render(popup, buf);
    }
}

/// Replace all chars with `*` (for password inputs).
fn mask(s: &str) -> String {
    if s.is_empty() {
        " ".to_string()
    } else {
        "*".repeat(s.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnetwork_daemon::{ConnectionState, Security};

    fn iface(name: &str, state: ConnectionState) -> InterfaceInfo {
        let mut i = InterfaceInfo::new(1, name);
        i.state = state;
        i
    }

    #[test]
    fn interface_lines_formats_state_and_name() {
        let app = App {
            interfaces: vec![iface("eth0", ConnectionState::Connected)],
            ..Default::default()
        };
        let lines = app.interface_lines();
        assert_eq!(lines.len(), 1);
        let rendered = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(rendered.contains("eth0"));
        assert!(rendered.contains("Connected"));
    }

    #[test]
    fn wifi_lines_shows_connected_ap() {
        let mut app = App {
            wifi_status: Some(SupplicantStatus {
                ssid: Some("Home".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        app.scan_results.push(ScanResult {
            bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
            freq: 2412,
            signal: -50,
            flags: Default::default(),
            ssid: "Home".into(),
            security: Security::Psk,
        });
        let lines = app.wifi_lines();
        let joined = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<String>();
        assert!(joined.contains("Connected:"));
        assert!(joined.contains("Home"));
        assert!(joined.contains("signal -50 dBm"));
    }

    #[test]
    fn network_lines_empty_state() {
        let app = App::default();
        let lines = app.network_lines();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|s| s.content.contains("no known networks"))
        );
    }

    #[test]
    fn scan_lines_empty_state() {
        // wifi_lines empty-scan message.
        let app = App::default();
        let lines = app.wifi_lines();
        assert!(lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.content.contains("no scan results"))
        }));
    }

    #[test]
    fn mask_hides_password() {
        assert_eq!(mask("abc"), "***");
        assert_eq!(mask(""), " ");
    }
}
