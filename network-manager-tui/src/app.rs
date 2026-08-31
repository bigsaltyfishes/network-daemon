//! TUI application state and rendering.

use libnetwork_daemon::{InterfaceInfo, KnownNetwork, ScanResult};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, Paragraph, Widget},
};

/// Which panel the user is focused on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum View {
    #[default]
    Interfaces,
    Networks,
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub interfaces: Vec<InterfaceInfo>,
    pub scan_results: Vec<ScanResult>,
    pub known_networks: Vec<KnownNetwork>,
    pub interface_selected: usize,
    pub network_selected: usize,
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
            interface_selected: 0,
            network_selected: 0,
            status: "Connected. Select an interface below.".into(),
            error: None,
        }
    }
}

impl App {
    /// Render the interface list as ratatui line items.
    fn interface_lines(&self) -> Vec<Line<'static>> {
        self.interfaces
            .iter()
            .map(|i| {
                let state = i.state.to_string();
                let ip = i
                    .ipv4_addrs
                    .first()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "-".into());
                let sec = if i.is_wlan() { " wifi" } else { "" };
                let mark = if i.state.is_online() { "●" } else { "○" };
                Line::from(vec![
                    Span::styled(
                        format!("{mark} {:<10}", i.name),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(" {:<14}", state)),
                    Span::raw(format!(" {:<20}", ip)),
                    Span::raw(sec.to_string()),
                ])
            })
            .collect()
    }

    /// Render known networks as list items.
    fn network_lines(&self) -> Vec<Line<'static>> {
        if self.known_networks.is_empty() {
            return vec![Line::from("(no known networks)")];
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
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header / status
                Constraint::Min(8),    // body
                Constraint::Length(3), // scan results footer
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

        // Body: interface list or known networks depending on view.
        let body = match self.view {
            View::Interfaces => List::new(self.interface_lines())
                .block(
                    Block::default().borders(Borders::ALL).title("Interfaces"),
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

        // Footer: a hint line inside the scan-results panel.
        let footer = Line::from(vec![
            Span::raw(" q=quit  i=interfaces  n=networks  "),
            Span::styled("s=scan  c=connect", Style::default().fg(Color::Cyan)),
        ]);
        Paragraph::new(footer)
            .block(Block::default().borders(Borders::TOP).title("Hint"))
            .render(chunks[2], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnetwork_daemon::ConnectionState;

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
    fn network_lines_empty_state() {
        let app = App::default();
        let lines = app.network_lines();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|s| s.content == "(no known networks)")
        );
    }
}
