//! TUI application state and rendering.

use libnetwork_daemon::{
    InterfaceInfo, KnownNetwork, ScanResult, SupplicantStatus,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListState, Paragraph, Row, Table,
        TableState, Widget,
    },
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

/// The kind of an editable field in a modal form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// yes/no toggle.
    Bool,
    /// free text.
    Text,
    /// pick one of `options` via a popup.
    Selection { options: Vec<String> },
}

/// A single row in a modal edit form.
#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub kind: FieldKind,
    /// children shown when the field is expanded (e.g. static-IP fields under
    /// a "手动" IPv4 selection).
    pub children: Vec<Field>,
    /// whether `children` are displayed.
    pub expanded: bool,
    /// hide input as `*` (passwords).
    pub password: bool,
}

impl Field {
    pub fn bool(label: &str, value: bool) -> Field {
        Field {
            label: label.to_string(),
            value: if value { "yes".into() } else { "no".into() },
            kind: FieldKind::Bool,
            children: Vec::new(),
            expanded: false,
            password: false,
        }
    }
    pub fn text(label: &str, value: &str) -> Field {
        Field {
            label: label.to_string(),
            value: value.to_string(),
            kind: FieldKind::Text,
            children: Vec::new(),
            expanded: false,
            password: false,
        }
    }
    pub fn password(label: &str) -> Field {
        Field {
            label: label.to_string(),
            value: String::new(),
            kind: FieldKind::Text,
            children: Vec::new(),
            expanded: false,
            password: true,
        }
    }
    pub fn selection(label: &str, options: &[&str], value: &str) -> Field {
        Field {
            label: label.to_string(),
            value: value.to_string(),
            kind: FieldKind::Selection {
                options: options.iter().map(|s| s.to_string()).collect(),
            },
            children: Vec::new(),
            expanded: false,
            password: false,
        }
    }
    pub fn fixed(label: &str, value: &str) -> Field {
        Field {
            label: label.to_string(),
            value: value.to_string(),
            kind: FieldKind::Text,
            children: Vec::new(),
            expanded: false,
            password: false,
        }
    }
}

/// A modal form: top-level rows, focus, and an open selection popup.
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
    /// Index of the field whose selection popup is open.
    pub selection_open: Option<usize>,
    /// Cursor within an open selection popup.
    pub selection_cursor: usize,
    /// true when the OK/Cancel footer has focus.
    pub on_footer: bool,
    /// 0 = OK, 1 = Cancel.
    pub footer_sel: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        Self {
            fields,
            focus: 0,
            selection_open: None,
            selection_cursor: 0,
            on_footer: false,
            footer_sel: 0,
        }
    }
    /// All visible rows (flattening expanded children), with an index.
    pub fn flat_rows(&self) -> Vec<(usize, &Field)> {
        let mut out = Vec::new();
        for (i, f) in self.fields.iter().enumerate() {
            out.push((i, f));
            if f.expanded {
                for c in &f.children {
                    out.push((i, c));
                }
            }
        }
        out
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
        /// 0 = OK, 1 = Cancel (footer focus).
        footer_sel: usize,
    },
    /// "Connecting..." while a Wi-Fi association is in flight.
    Connecting { ssid: String },
    /// Edit an interface (DHCP / SLAAC / static IP) or a saved AP.
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
    /// Build the interface table rows.
    pub fn interface_rows(&self) -> Vec<Vec<Span<'static>>> {
        self.interfaces
            .iter()
            .map(|i| {
                let mark = if i.state.is_online() { "●" } else { "○" };
                vec![
                    Span::styled(
                        format!("{mark} {:<8}", i.name),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(
                        "{:<12}",
                        format!("{:?}", i.interface_type)
                    )),
                    Span::raw(format!("{:<14}", i.state.to_string())),
                    Span::raw(format!(
                        "{:<20}",
                        i.ipv4_addrs
                            .first()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "-".into())
                    )),
                    Span::raw(format!(
                        "{:<22}",
                        i.ipv6_addrs
                            .first()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "-".into())
                    )),
                ]
            })
            .collect()
    }

    /// Build known-network table rows.
    pub fn network_rows(&self) -> Vec<Vec<Span<'static>>> {
        if self.known_networks.is_empty() {
            return vec![vec![Span::raw("(no known networks — e to add)")]];
        }
        self.known_networks
            .iter()
            .map(|n| {
                vec![
                    Span::styled(
                        format!("{:<28}", n.ssid),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(format!("{:<8}", n.security.to_string())),
                    Span::raw(format!(
                        "{:<18}",
                        n.bssid
                            .map(|b| b.to_string())
                            .unwrap_or_else(|| "any".into())
                    )),
                    Span::raw(format!("{:?}", n.state)),
                ]
            })
            .collect()
    }

    /// Wi-Fi view: a connected-AP header line plus scan-result rows.
    pub fn wifi_rows(&self) -> Vec<Vec<Span<'static>>> {
        let mut out = Vec::new();
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
                let connected = vec![
                    Span::styled("● ", Style::default().fg(Color::Green)),
                    Span::styled(
                        format!("{ssid:<28}"),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(format!(" {:<20}", state)),
                    Span::raw(format!(" signal {signal}")),
                ];
                out.push(connected);
            }
            None => out.push(vec![Span::raw("(no Wi-Fi connection)")]),
        }
        if self.scan_results.is_empty() {
            out.push(vec![Span::raw("(no scan results — press s to scan)")]);
            return out;
        }
        out.extend(self.scan_results.iter().map(|r| {
            vec![
                Span::styled(
                    format!("{:<28}", r.ssid),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(format!(" {:<8}", r.security.to_string())),
                Span::raw(format!(" ch {:<3}", r.channel())),
                Span::raw(format!(" {:>4} dBm", r.signal)),
                Span::raw(format!(" {:>18}", r.bssid.to_string())),
            ]
        }));
        out
    }

    /// Build the interface-edit form.
    pub fn interface_form(iface: &InterfaceInfo) -> Form {
        let dhcp = iface.dhcpv4_enabled;
        let mut ipv4 = Field::selection(
            "IPv4 配置",
            &["DHCP", "手动"],
            if dhcp { "DHCP" } else { "手动" },
        );
        let static_ip = iface
            .ipv4_addrs
            .first()
            .map(|a| a.to_string())
            .unwrap_or_default();
        ipv4.children = vec![
            Field::text("IP 地址", ""),
            Field::text("网关", ""),
            Field::text("子网掩码", ""),
        ];
        // Prefill static IP when the interface isn't on DHCP.
        if !dhcp && let Some(v) = static_ip.split('/').next() {
            ipv4.children[0].value = v.to_string();
        }
        Form::new(vec![
            ipv4,
            Field::bool("SLAAC (IPv6 自动)", iface.slaac_enabled),
        ])
    }

    /// Build the edit-saved-AP form for a known network.
    pub fn ap_form(net: &KnownNetwork) -> Form {
        let security = net.security.to_string();
        Form::new(vec![
            Field::fixed("SSID", &net.ssid),
            Field::selection("安全", &["Open", "Psk", "Eap"], &security),
            Field::text("BSSID (留空=自动)", ""),
            Field::password("密码 (PSK)"),
            Field::bool("自动连接", net.is_enabled()),
        ])
    }

    /// Build an edit-AP form from a scan result (network not yet saved).
    pub fn ap_form_from_scan(scan: &ScanResult) -> Form {
        let security = scan.security.to_string();
        Form::new(vec![
            Field::fixed("SSID", &scan.ssid),
            Field::selection("安全", &["Open", "Psk", "Eap"], &security),
            Field::text("BSSID (留空=自动)", ""),
            Field::password("密码 (PSK)"),
        ])
    }
}

impl App {
    /// Render the whole UI (stateful, so selection is highlighted).
    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
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
        f.render_widget(
            Paragraph::new(Line::from(status_text)).style(status_style),
            chunks[0],
        );

        // Body.
        self.draw_body(f, chunks[1]);

        // Footer hint.
        let footer = Line::from(vec![
            Span::raw(" q=quit  i=ifaces  w=wifi  n=networks  r=refresh  "),
            Span::styled(
                "↑/↓ 移动  Enter=操作  e=编辑  s=扫描  Esc=返回",
                Style::default().fg(Color::Cyan),
            ),
        ]);
        f.render_widget(
            Paragraph::new(footer)
                .block(Block::default().borders(Borders::TOP).title("提示")),
            chunks[2],
        );

        // Modal overlay.
        if !matches!(self.modal, Modal::None) {
            self.draw_modal(f, area);
        }
    }

    fn draw_body(&mut self, f: &mut Frame, area: Rect) {
        let sel = Style::default()
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        match self.view {
            View::Interfaces => {
                let header = ["接口", "类型", "状态", "IPv4 地址", "IPv6 地址"];
                let table = Table::new(
                    self.interface_rows().into_iter().map(|row| {
                        Row::new(
                            row.into_iter().map(Cell::from).collect::<Vec<_>>(),
                        )
                    }),
                    [
                        Constraint::Length(14),
                        Constraint::Length(14),
                        Constraint::Length(15),
                        Constraint::Length(24),
                        Constraint::Min(20),
                    ],
                )
                .header(
                    Row::new(
                        header
                            .iter()
                            .map(|h| Cell::from(Line::from(*h)))
                            .collect::<Vec<_>>(),
                    )
                    .style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .row_highlight_style(sel)
                .block(Block::default().borders(Borders::ALL).title("接口"));
                let mut state = TableState::default();
                state.select(Some(self.interface_selected));
                f.render_stateful_widget(table, area, &mut state);
            }
            View::Wifi => {
                let list = List::new(
                    self.wifi_rows()
                        .into_iter()
                        .map(Line::from)
                        .collect::<Vec<_>>(),
                )
                .highlight_style(sel)
                .block(
                    Block::default().borders(Borders::ALL).title("Wi-Fi 网络"),
                );
                // Row 0 is the connected-AP line; scan results start at row 1.
                let mut state = ListState::default();
                state.select(Some(self.network_selected + 1));
                f.render_stateful_widget(&list, area, &mut state);
            }
            View::Networks => {
                let header = ["SSID", "安全", "BSSID", "状态"];
                let table = Table::new(
                    self.network_rows().into_iter().map(|row| {
                        Row::new(
                            row.into_iter().map(Cell::from).collect::<Vec<_>>(),
                        )
                    }),
                    [
                        Constraint::Length(30),
                        Constraint::Length(10),
                        Constraint::Length(22),
                        Constraint::Min(12),
                    ],
                )
                .header(
                    Row::new(
                        header
                            .iter()
                            .map(|h| Cell::from(Line::from(*h)))
                            .collect::<Vec<_>>(),
                    )
                    .style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .row_highlight_style(sel)
                .block(
                    Block::default().borders(Borders::ALL).title("已保存网络"),
                );
                let mut state = TableState::default();
                state.select(Some(self.network_selected));
                f.render_stateful_widget(table, area, &mut state);
            }
        }
    }
}

impl App {
    fn draw_modal(&self, f: &mut Frame, area: Rect) {
        match &self.modal {
            Modal::None => {}
            Modal::Message { title, message } => {
                self.popup(
                    f,
                    area,
                    title.clone(),
                    vec![Line::from(message.clone())],
                    0,
                );
            }
            Modal::Connecting { ssid } => {
                self.popup(
                    f,
                    area,
                    "连接中".into(),
                    vec![Line::from(format!("正在连接 {ssid}…"))],
                    0,
                );
            }
            Modal::Password {
                ssid, input, footer_sel, ..
            } => {
                self.popup(
                    f,
                    area,
                    format!("{ssid} 的密码"),
                    vec![Line::from(vec![
                        Span::raw("密码: "),
                        Span::styled(
                            mask(input),
                            Style::default().fg(Color::Yellow),
                        ),
                    ])],
                    *footer_sel,
                );
            }
            Modal::Edit { title, iface, form } => {
                let rows = form.flat_rows();
                let mut lines: Vec<Line> = Vec::new();
                for (flat, (top, field)) in rows.iter().enumerate() {
                    let is_child = *top != flat;
                    let cursor = if !is_child && flat == form.focus {
                        "▶"
                    } else {
                        "  "
                    };
                    let pad = if is_child { "    " } else { "" };
                    let shown = if field.password {
                        mask(&field.value)
                    } else {
                        field.value.clone()
                    };
                    let marker = match field.kind {
                        FieldKind::Selection { .. } => " ▾",
                        FieldKind::Bool => "",
                        FieldKind::Text => "",
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(
                                "{cursor}{pad}{:<18}{}",
                                field.label, marker
                            ),
                            if !is_child && flat == form.focus {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::raw(format!(" {shown}")),
                    ]));
                }
                self.popup(
                    f,
                    area,
                    format!("{title} — {iface}"),
                    lines,
                    form.footer_sel,
                );
                // Selection sub-popup.
                if let Some(top) = form.selection_open
                    && let Some(field) = form.fields.get(top)
                    && let FieldKind::Selection { options } = &field.kind
                {
                    self.draw_selection_popup(
                        f,
                        area,
                        options,
                        form.selection_cursor,
                    );
                }
            }
        }
    }

    fn draw_selection_popup(
        &self,
        f: &mut Frame,
        area: Rect,
        options: &[String],
        cursor: usize,
    ) {
        let lines: Vec<Line> = options
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mark = if i == cursor { "▶" } else { "  " };
                Line::from(vec![Span::styled(
                    format!("{mark} {o}"),
                    if i == cursor {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    },
                )])
            })
            .collect();
        let w = 30u16;
        let h = options.len() as u16 + 4;
        let rect = Rect {
            x: area.x + area.width.saturating_sub(w).saturating_div(2) + 12,
            y: area.y + area.height.saturating_sub(h).saturating_div(2),
            width: w,
            height: h,
        };
        Clear.render(rect, f.buffer_mut());
        f.render_widget(
            List::new(lines)
                .block(Block::default().borders(Borders::ALL))
                .highlight_style(
                    Style::default().add_modifier(Modifier::REVERSED),
                ),
            rect,
        );
    }

    fn popup(
        &self,
        f: &mut Frame,
        area: Rect,
        title: String,
        lines: Vec<Line>,
        footer_sel: usize,
    ) {
        // Height: content + a hint line + an OK/Cancel footer line + borders.
        let height = lines.len() as u16 + 5;
        let width = 56;
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width).saturating_div(2),
            y: area.y + area.height.saturating_sub(height).saturating_div(2),
            width,
            height,
        };
        f.render_widget(Clear, rect);
        let sel = Style::default().add_modifier(Modifier::REVERSED);
        let ok = if footer_sel == 0 { "[ OK ]".to_string() } else { " OK  ".to_string() };
        let cancel = if footer_sel == 1 { "[CANCEL]" } else { " Cancel " };
        let mut content = lines;
        content.push(Line::from(" ↑/↓=字段  Tab/←/→=OK/Cancel  Enter=确认  Esc=放弃 ".to_string()));
        content.push(Line::from(vec![
            Span::styled(
                format!(" {ok}   {cancel} "),
                if footer_sel == 0 { sel } else { Style::default() },
            ),
        ]));
        f.render_widget(
            Paragraph::new(content)
                .style(Style::default().bg(Color::DarkGray))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .border_style(Style::default().fg(Color::Cyan)),
                ),
            rect,
        );
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
    use libnetwork_daemon::{ConnectionState, InterfaceType, Security};

    fn iface(name: &str, state: ConnectionState) -> InterfaceInfo {
        let mut i = InterfaceInfo::new(1, name);
        i.state = state;
        i.interface_type = InterfaceType::Ethernet;
        i
    }

    #[test]
    fn interface_rows_format_columns() {
        let app = App {
            interfaces: vec![iface("eth0", ConnectionState::Connected)],
            ..Default::default()
        };
        let rows = app.interface_rows();
        assert_eq!(rows.len(), 1);
        let joined = rows[0]
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(joined.contains("eth0"));
        assert!(joined.contains("Ethernet"));
        assert!(joined.contains("Connected"));
    }

    #[test]
    fn wifi_rows_show_connected_ap() {
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
        let rows = app.wifi_rows();
        let joined = rows
            .iter()
            .flat_map(|r| r.iter().map(|s| s.content.as_ref()))
            .collect::<String>();
        assert!(joined.contains("Home"));
        assert!(joined.contains("signal -50 dBm"));
    }

    #[test]
    fn interface_form_option_default() {
        // A DHCP interface defaults the IPv4 selection to DHCP and keeps
        // children collapsed.
        let mut info = InterfaceInfo::new(1, "eth0");
        info.dhcpv4_enabled = true;
        let form = App::interface_form(&info);
        assert_eq!(form.fields[0].value, "DHCP");
        assert_eq!(
            form.fields[0].kind,
            FieldKind::Selection {
                options: vec!["DHCP".into(), "手动".into()]
            }
        );
        assert!(!form.fields[0].expanded);
    }

    #[test]
    fn flat_rows_expands_children() {
        let mut form = Form::new(vec![Field::selection(
            "IPv4 配置",
            &["DHCP", "手动"],
            "手动",
        )]);
        form.fields[0].children = vec![
            Field::text("IP 地址", "10.0.0.2"),
            Field::text("网关", "10.0.0.1"),
        ];
        form.fields[0].expanded = true;
        let rows = form.flat_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].1.label, "IPv4 配置");
        assert_eq!(rows[1].1.label, "IP 地址");
        assert_eq!(rows[2].1.label, "网关");
    }
}
