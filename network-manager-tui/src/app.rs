//! nmtui-style application state and rendering.

use std::{
    cmp::Reverse,
    time::{Duration, Instant},
};

use libnetwork_daemon::{
    ConnectionState, DaemonCommand, DaemonResponse, GlobalDaemonAction,
    GlobalDaemonResponse, InterfaceInfo, InterfaceManagerAction,
    InterfaceResponse, InterfaceType, KnownNetwork, MacAddr, Modification,
    PrefixedIpv4Addr, PrefixedIpv6Addr, ScanResult, Security, SupplicantStatus,
    WiFiManagerAction, WiFiManagerResponse, WpaState,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};

use crate::client::DaemonClient;

const MAIN_OPTIONS: [&str; 4] = [
    "Edit a connection",
    "Activate a connection",
    "Set system hostname",
    "Quit",
];
const NEW_CONNECTION_TYPES: [&str; 5] =
    ["Wi-Fi", "Bond", "Bridge", "Team", "VLAN"];

/// Top-level nmtui activity screen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    #[default]
    MainMenu,
    EditConnections,
    ActivateConnections,
    Hostname,
}

/// A modal layered over a list or form screen.
#[derive(Debug, Default)]
pub enum Popup {
    #[default]
    None,
    Message(MessagePopup),
    NewConnection(NewConnectionPopup),
    Password(PasswordPopup),
    Connecting(ConnectingPopup),
    Editor(Box<ConnectionEditor>),
}

#[derive(Debug, Clone)]
pub(crate) struct MessagePopup {
    title: String,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewFocus {
    List,
    Cancel,
    Create,
}

#[derive(Debug, Clone)]
pub(crate) struct NewConnectionPopup {
    selected: usize,
    focus: NewFocus,
}

#[derive(Debug, Clone)]
struct WifiTarget {
    iface: String,
    ssid: String,
    bssid: Option<MacAddr>,
    security: Security,
    hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordFocus {
    Input,
    Show,
    Cancel,
    Connect,
}

#[derive(Debug, Clone)]
pub(crate) struct PasswordPopup {
    target: WifiTarget,
    input: String,
    show: bool,
    focus: PasswordFocus,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectingPopup {
    target: WifiTarget,
    last_poll: Instant,
}

#[derive(Debug, Clone)]
enum EditTarget {
    Interface(String),
    Network(KnownNetwork),
}

#[derive(Debug, Clone)]
struct EditRow {
    label: String,
    target: Option<EditTarget>,
}

#[derive(Debug, Clone)]
enum ActivationTarget {
    Interface(String),
    Wifi(WifiTarget),
}

#[derive(Debug, Clone)]
struct ActivationRow {
    label: String,
    signal: Option<i32>,
    target: Option<ActivationTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpMode {
    Automatic,
    Manual,
    Ignore,
}

impl IpMode {
    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Manual => "Manual",
            Self::Ignore => "Ignore",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorMode {
    Ethernet,
    Wifi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorField {
    ProfileName,
    Device,
    Ssid,
    Bssid,
    Security,
    Identity,
    Password,
    Hidden,
    Ipv4Mode,
    Ipv4Address,
    Ipv4Gateway,
    Ipv4Dns,
    Ipv4Search,
    Ipv4Routing,
    Ipv4NeverDefault,
    Ipv4IgnoreRoutes,
    Ipv4IgnoreDns,
    Ipv4Required,
    Ipv6Mode,
    Slaac,
    Ipv6Address,
    Ipv6Gateway,
    Ipv6Dns,
    Ipv6Search,
    Ipv6Routing,
    Ipv6NeverDefault,
    Ipv6IgnoreRoutes,
    Ipv6IgnoreDns,
    Ipv6Required,
    Autoconnect,
    AvailableUsers,
}

impl EditorField {
    fn is_routing(self) -> bool {
        matches!(self, Self::Ipv4Routing | Self::Ipv6Routing)
    }

    fn is_text(self) -> bool {
        matches!(
            self,
            Self::ProfileName
                | Self::Device
                | Self::Ssid
                | Self::Bssid
                | Self::Identity
                | Self::Password
                | Self::Ipv4Address
                | Self::Ipv4Gateway
                | Self::Ipv4Dns
                | Self::Ipv4Search
                | Self::Ipv6Address
                | Self::Ipv6Gateway
                | Self::Ipv6Dns
                | Self::Ipv6Search
        )
    }

    fn is_bool(self) -> bool {
        matches!(
            self,
            Self::Hidden
                | Self::Slaac
                | Self::Ipv4NeverDefault
                | Self::Ipv4IgnoreRoutes
                | Self::Ipv4IgnoreDns
                | Self::Ipv4Required
                | Self::Ipv6NeverDefault
                | Self::Ipv6IgnoreRoutes
                | Self::Ipv6IgnoreDns
                | Self::Ipv6Required
                | Self::Autoconnect
                | Self::AvailableUsers
        )
    }
}

#[derive(Debug, Clone)]
struct ChoicePopup {
    field: EditorField,
    options: Vec<String>,
    selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingFamily {
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingFocus {
    List,
    Add,
    Delete,
    Cancel,
    Ok,
}

impl RoutingFocus {
    fn index(self) -> usize {
        match self {
            Self::List => 0,
            Self::Add => 1,
            Self::Delete => 2,
            Self::Cancel => 3,
            Self::Ok => 4,
        }
    }
}

#[derive(Debug, Clone)]
struct RoutingPopup {
    family: RoutingFamily,
    routes: Vec<String>,
    selected: usize,
    focus: RoutingFocus,
    editing: bool,
    input: String,
    error: Option<String>,
}

impl RoutingPopup {
    fn new(family: RoutingFamily, routes: Vec<String>) -> Self {
        Self {
            family,
            routes,
            selected: 0,
            focus: RoutingFocus::List,
            editing: false,
            input: String::new(),
            error: None,
        }
    }

    fn title(&self) -> &'static str {
        match self.family {
            RoutingFamily::Ipv4 => "Edit IPv4 Routes",
            RoutingFamily::Ipv6 => "Edit IPv6 Routes",
        }
    }

    fn placeholder(&self) -> &'static str {
        "destination/prefix gateway"
    }

    fn begin_add(&mut self) {
        self.editing = true;
        self.input.clear();
        self.error = None;
    }

    fn add_route(&mut self) -> Result<(), String> {
        let mut parts = self.input.split_whitespace();
        let destination = parts.next().unwrap_or_default();
        let gateway = parts.next();
        if destination.is_empty() {
            return Err("Route must not be empty".to_string());
        }
        if parts.next().is_some() {
            return Err(
                "Route accepts one destination and one gateway".to_string()
            );
        }
        match self.family {
            RoutingFamily::Ipv4 => {
                let address: PrefixedIpv4Addr = serde_json::from_value(
                    serde_json::Value::String(destination.to_string()),
                )
                .map_err(|_| "Invalid IPv4 destination/prefix".to_string())?;
                if address.prefix_len > 32 {
                    return Err(
                        "IPv4 prefix must be between 0 and 32".to_string()
                    );
                }
                if let Some(gateway) = gateway {
                    gateway
                        .parse::<std::net::Ipv4Addr>()
                        .map_err(|_| "Invalid IPv4 gateway".to_string())?;
                }
            }
            RoutingFamily::Ipv6 => {
                let address: PrefixedIpv6Addr = serde_json::from_value(
                    serde_json::Value::String(destination.to_string()),
                )
                .map_err(|_| "Invalid IPv6 destination/prefix".to_string())?;
                if address.prefix_len > 128 {
                    return Err(
                        "IPv6 prefix must be between 0 and 128".to_string()
                    );
                }
                if let Some(gateway) = gateway {
                    gateway
                        .parse::<std::net::Ipv6Addr>()
                        .map_err(|_| "Invalid IPv6 gateway".to_string())?;
                }
            }
        }
        self.routes.push(self.input.trim().to_string());
        self.selected = self.routes.len().saturating_sub(1);
        self.editing = false;
        self.input.clear();
        self.error = None;
        Ok(())
    }

    fn delete_selected(&mut self) {
        if self.routes.is_empty() {
            return;
        }
        self.routes.remove(self.selected);
        self.selected = self.selected.min(self.routes.len().saturating_sub(1));
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            RoutingFocus::List => {
                if !self.routes.is_empty() {
                    self.selected = (self.selected as isize + delta)
                        .rem_euclid(self.routes.len() as isize)
                        as usize;
                }
            }
            _ => {
                let order = [
                    RoutingFocus::Add,
                    RoutingFocus::Delete,
                    RoutingFocus::Cancel,
                    RoutingFocus::Ok,
                ];
                let current = self.focus.index().saturating_sub(1);
                self.focus = order[(current as isize + delta)
                    .rem_euclid(order.len() as isize)
                    as usize];
            }
        }
    }

    fn tab(&mut self, backwards: bool) {
        let order = [
            RoutingFocus::List,
            RoutingFocus::Add,
            RoutingFocus::Delete,
            RoutingFocus::Cancel,
            RoutingFocus::Ok,
        ];
        let current = self.focus.index();
        let delta = if backwards { -1 } else { 1 };
        self.focus = order[(current as isize + delta)
            .rem_euclid(order.len() as isize)
            as usize];
    }
}

/// Connection editor data and focus state.
#[derive(Debug, Clone)]
pub struct ConnectionEditor {
    mode: EditorMode,
    profile_name: String,
    device: String,
    ssid: String,
    bssid: String,
    security: Security,
    identity: String,
    password: String,
    hidden: bool,
    ipv4_mode: IpMode,
    ipv4_address: String,
    ipv4_gateway: String,
    ipv4_dns: String,
    ipv4_search: String,
    ipv4_routes: Vec<String>,
    ipv4_never_default: bool,
    ipv4_ignore_routes: bool,
    ipv4_ignore_dns: bool,
    ipv4_required: bool,
    ipv6_mode: IpMode,
    slaac: bool,
    ipv6_address: String,
    ipv6_gateway: String,
    ipv6_dns: String,
    ipv6_search: String,
    ipv6_routes: Vec<String>,
    ipv6_never_default: bool,
    ipv6_ignore_routes: bool,
    ipv6_ignore_dns: bool,
    ipv6_required: bool,
    autoconnect: bool,
    available_users: bool,
    existing_network: Option<(String, Option<MacAddr>)>,
    focus: usize,
    footer: bool,
    footer_selected: usize,
    choice: Option<ChoicePopup>,
    routing: Option<RoutingPopup>,
    scroll: u16,
}

impl ConnectionEditor {
    fn from_interface(iface: &InterfaceInfo) -> Self {
        let ipv4_address = iface
            .ipv4_addrs
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        let ipv6_address = iface
            .ipv6_addrs
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        Self {
            mode: EditorMode::Ethernet,
            profile_name: iface.name.clone(),
            device: iface.name.clone(),
            ssid: String::new(),
            bssid: String::new(),
            security: Security::Open,
            identity: String::new(),
            password: String::new(),
            hidden: false,
            ipv4_mode: if iface.dhcpv4_enabled {
                IpMode::Automatic
            } else {
                IpMode::Manual
            },
            ipv4_address,
            ipv4_gateway: iface
                .gateway_ipv4
                .as_ref()
                .map(|address| address.addr.to_string())
                .unwrap_or_default(),
            ipv4_dns: String::new(),
            ipv4_search: String::new(),
            ipv4_routes: Vec::new(),
            ipv4_never_default: false,
            ipv4_ignore_routes: false,
            ipv4_ignore_dns: false,
            ipv4_required: false,
            ipv6_mode: if iface.slaac_enabled {
                IpMode::Automatic
            } else if iface.ipv6_addrs.is_empty() {
                IpMode::Ignore
            } else {
                IpMode::Manual
            },
            slaac: iface.slaac_enabled,
            ipv6_address,
            ipv6_gateway: iface
                .gateway_ipv6
                .as_ref()
                .map(|address| address.addr.to_string())
                .unwrap_or_default(),
            ipv6_dns: String::new(),
            ipv6_search: String::new(),
            ipv6_routes: Vec::new(),
            ipv6_never_default: false,
            ipv6_ignore_routes: false,
            ipv6_ignore_dns: false,
            ipv6_required: false,
            autoconnect: true,
            available_users: true,
            existing_network: None,
            focus: 0,
            footer: false,
            footer_selected: 0,
            choice: None,
            routing: None,
            scroll: 0,
        }
    }

    fn from_wifi(
        iface: &str,
        scan: Option<&ScanResult>,
        known: Option<&KnownNetwork>,
    ) -> Self {
        let ssid = scan
            .map(|network| network.ssid.clone())
            .or_else(|| known.map(|network| network.ssid.clone()))
            .unwrap_or_default();
        let security = scan
            .map(|network| network.security)
            .or_else(|| known.map(|network| network.security))
            .unwrap_or(Security::Open);
        let bssid = scan
            .map(|network| network.bssid.to_string())
            .or_else(|| {
                known
                    .and_then(|network| network.bssid)
                    .map(|address| address.to_string())
            })
            .unwrap_or_default();
        Self {
            mode: EditorMode::Wifi,
            profile_name: ssid.clone(),
            device: iface.to_string(),
            ssid,
            bssid,
            security,
            identity: known
                .and_then(|network| network.identity.clone())
                .unwrap_or_default(),
            password: known
                .and_then(|network| network.password.clone())
                .unwrap_or_default(),
            hidden: known.is_some_and(|network| network.hidden),
            ipv4_mode: IpMode::Automatic,
            ipv4_address: String::new(),
            ipv4_gateway: String::new(),
            ipv4_dns: String::new(),
            ipv4_search: String::new(),
            ipv4_routes: Vec::new(),
            ipv4_never_default: false,
            ipv4_ignore_routes: false,
            ipv4_ignore_dns: false,
            ipv4_required: false,
            ipv6_mode: IpMode::Automatic,
            slaac: true,
            ipv6_address: String::new(),
            ipv6_gateway: String::new(),
            ipv6_dns: String::new(),
            ipv6_search: String::new(),
            ipv6_routes: Vec::new(),
            ipv6_never_default: false,
            ipv6_ignore_routes: false,
            ipv6_ignore_dns: false,
            ipv6_required: false,
            autoconnect: known.is_none_or(KnownNetwork::is_enabled),
            available_users: true,
            existing_network: known
                .map(|network| (network.ssid.clone(), network.bssid)),
            focus: 0,
            footer: false,
            footer_selected: 0,
            choice: None,
            routing: None,
            scroll: 0,
        }
    }

    fn fields(&self) -> Vec<EditorField> {
        let mut fields = vec![EditorField::ProfileName, EditorField::Device];
        if self.mode == EditorMode::Wifi {
            fields.extend([
                EditorField::Ssid,
                EditorField::Bssid,
                EditorField::Security,
            ]);
            if self.security == Security::Eap {
                fields.push(EditorField::Identity);
            }
            if self.security != Security::Open {
                fields.push(EditorField::Password);
            }
            fields.push(EditorField::Hidden);
        }
        fields.extend([
            EditorField::Ipv4Mode,
            EditorField::Ipv4Address,
            EditorField::Ipv4Gateway,
            EditorField::Ipv4Dns,
            EditorField::Ipv4Search,
            EditorField::Ipv4Routing,
            EditorField::Ipv4NeverDefault,
            EditorField::Ipv4IgnoreRoutes,
            EditorField::Ipv4IgnoreDns,
            EditorField::Ipv4Required,
            EditorField::Ipv6Mode,
            EditorField::Slaac,
            EditorField::Ipv6Address,
            EditorField::Ipv6Gateway,
            EditorField::Ipv6Dns,
            EditorField::Ipv6Search,
            EditorField::Ipv6Routing,
            EditorField::Ipv6NeverDefault,
            EditorField::Ipv6IgnoreRoutes,
            EditorField::Ipv6IgnoreDns,
            EditorField::Ipv6Required,
            EditorField::Autoconnect,
            EditorField::AvailableUsers,
        ]);
        fields
    }

    fn focused(&self, field: EditorField) -> bool {
        self.fields().get(self.focus).copied() == Some(field)
    }

    fn field_label(field: EditorField) -> &'static str {
        match field {
            EditorField::ProfileName => "Profile name",
            EditorField::Device => "Device",
            EditorField::Ssid => "SSID",
            EditorField::Bssid => "BSSID",
            EditorField::Security => "Security",
            EditorField::Identity => "Identity",
            EditorField::Password => "Password",
            EditorField::Hidden => "Hidden network",
            EditorField::Ipv4Mode => "IPv4 CONFIGURATION",
            EditorField::Ipv4Address => "Addresses",
            EditorField::Ipv4Gateway => "Gateway",
            EditorField::Ipv4Dns => "DNS servers",
            EditorField::Ipv4Search => "Search domains",
            EditorField::Ipv4Routing => "Routing",
            EditorField::Ipv4NeverDefault => {
                "Never use this network for default route"
            }
            EditorField::Ipv4IgnoreRoutes => {
                "Ignore automatically obtained routes"
            }
            EditorField::Ipv4IgnoreDns => {
                "Ignore automatically obtained DNS parameters"
            }
            EditorField::Ipv4Required => {
                "Require IPv4 addressing for this connection"
            }
            EditorField::Ipv6Mode => "IPv6 CONFIGURATION",
            EditorField::Slaac => "Accept IPv6 router advertisements (SLAAC)",
            EditorField::Ipv6Address => "Addresses",
            EditorField::Ipv6Gateway => "Gateway",
            EditorField::Ipv6Dns => "DNS servers",
            EditorField::Ipv6Search => "Search domains",
            EditorField::Ipv6Routing => "Routing",
            EditorField::Ipv6NeverDefault => {
                "Never use this network for default route"
            }
            EditorField::Ipv6IgnoreRoutes => {
                "Ignore automatically obtained routes"
            }
            EditorField::Ipv6IgnoreDns => {
                "Ignore automatically obtained DNS parameters"
            }
            EditorField::Ipv6Required => {
                "Require IPv6 addressing for this connection"
            }
            EditorField::Autoconnect => "Automatically connect",
            EditorField::AvailableUsers => "Available to all users",
        }
    }

    fn text_value(&self, field: EditorField) -> &str {
        match field {
            EditorField::ProfileName => &self.profile_name,
            EditorField::Device => &self.device,
            EditorField::Ssid => &self.ssid,
            EditorField::Bssid => &self.bssid,
            EditorField::Identity => &self.identity,
            EditorField::Password => &self.password,
            EditorField::Ipv4Address => &self.ipv4_address,
            EditorField::Ipv4Gateway => &self.ipv4_gateway,
            EditorField::Ipv4Dns => &self.ipv4_dns,
            EditorField::Ipv4Search => &self.ipv4_search,
            EditorField::Ipv6Address => &self.ipv6_address,
            EditorField::Ipv6Gateway => &self.ipv6_gateway,
            EditorField::Ipv6Dns => &self.ipv6_dns,
            EditorField::Ipv6Search => &self.ipv6_search,
            _ => "",
        }
    }

    fn text_value_mut(&mut self, field: EditorField) -> Option<&mut String> {
        match field {
            EditorField::ProfileName => Some(&mut self.profile_name),
            EditorField::Device => Some(&mut self.device),
            EditorField::Ssid => Some(&mut self.ssid),
            EditorField::Bssid => Some(&mut self.bssid),
            EditorField::Identity => Some(&mut self.identity),
            EditorField::Password => Some(&mut self.password),
            EditorField::Ipv4Address => Some(&mut self.ipv4_address),
            EditorField::Ipv4Gateway => Some(&mut self.ipv4_gateway),
            EditorField::Ipv4Dns => Some(&mut self.ipv4_dns),
            EditorField::Ipv4Search => Some(&mut self.ipv4_search),
            EditorField::Ipv6Address => Some(&mut self.ipv6_address),
            EditorField::Ipv6Gateway => Some(&mut self.ipv6_gateway),
            EditorField::Ipv6Dns => Some(&mut self.ipv6_dns),
            EditorField::Ipv6Search => Some(&mut self.ipv6_search),
            _ => None,
        }
    }

    fn bool_value(&self, field: EditorField) -> bool {
        match field {
            EditorField::Hidden => self.hidden,
            EditorField::Slaac => self.slaac,
            EditorField::Ipv4NeverDefault => self.ipv4_never_default,
            EditorField::Ipv4IgnoreRoutes => self.ipv4_ignore_routes,
            EditorField::Ipv4IgnoreDns => self.ipv4_ignore_dns,
            EditorField::Ipv4Required => self.ipv4_required,
            EditorField::Ipv6NeverDefault => self.ipv6_never_default,
            EditorField::Ipv6IgnoreRoutes => self.ipv6_ignore_routes,
            EditorField::Ipv6IgnoreDns => self.ipv6_ignore_dns,
            EditorField::Ipv6Required => self.ipv6_required,
            EditorField::Autoconnect => self.autoconnect,
            EditorField::AvailableUsers => self.available_users,
            _ => false,
        }
    }

    fn toggle(&mut self, field: EditorField) {
        match field {
            EditorField::Hidden => self.hidden = !self.hidden,
            EditorField::Slaac => self.slaac = !self.slaac,
            EditorField::Ipv4NeverDefault => {
                self.ipv4_never_default = !self.ipv4_never_default
            }
            EditorField::Ipv4IgnoreRoutes => {
                self.ipv4_ignore_routes = !self.ipv4_ignore_routes
            }
            EditorField::Ipv4IgnoreDns => {
                self.ipv4_ignore_dns = !self.ipv4_ignore_dns
            }
            EditorField::Ipv4Required => {
                self.ipv4_required = !self.ipv4_required
            }
            EditorField::Ipv6NeverDefault => {
                self.ipv6_never_default = !self.ipv6_never_default
            }
            EditorField::Ipv6IgnoreRoutes => {
                self.ipv6_ignore_routes = !self.ipv6_ignore_routes
            }
            EditorField::Ipv6IgnoreDns => {
                self.ipv6_ignore_dns = !self.ipv6_ignore_dns
            }
            EditorField::Ipv6Required => {
                self.ipv6_required = !self.ipv6_required
            }
            EditorField::Autoconnect => self.autoconnect = !self.autoconnect,
            EditorField::AvailableUsers => {
                self.available_users = !self.available_users
            }
            _ => {}
        }
    }

    fn choice_options(field: EditorField) -> Option<Vec<String>> {
        match field {
            EditorField::Security => Some(
                ["None", "WPA & WPA2 Personal", "WPA & WPA2 Enterprise"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            EditorField::Ipv4Mode | EditorField::Ipv6Mode => Some(
                ["Automatic", "Manual", "Ignore"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            _ => None,
        }
    }

    fn choice_selected(&self, field: EditorField, options: &[String]) -> usize {
        let value = match field {
            EditorField::Security => match self.security {
                Security::Open => "None",
                Security::Psk => "WPA & WPA2 Personal",
                Security::Eap => "WPA & WPA2 Enterprise",
                Security::Unknown => "None",
            },
            EditorField::Ipv4Mode => self.ipv4_mode.label(),
            EditorField::Ipv6Mode => self.ipv6_mode.label(),
            _ => "",
        };
        options
            .iter()
            .position(|option| option == value)
            .unwrap_or(0)
    }

    fn apply_choice(&mut self) {
        let Some(choice) = self.choice.take() else {
            return;
        };
        match choice.field {
            EditorField::Security => {
                self.security = match choice.selected {
                    1 => Security::Psk,
                    2 => Security::Eap,
                    _ => Security::Open,
                };
            }
            EditorField::Ipv4Mode => {
                self.ipv4_mode = match choice.selected {
                    1 => IpMode::Manual,
                    2 => IpMode::Ignore,
                    _ => IpMode::Automatic,
                };
            }
            EditorField::Ipv6Mode => {
                self.ipv6_mode = match choice.selected {
                    1 => IpMode::Manual,
                    2 => IpMode::Ignore,
                    _ => IpMode::Automatic,
                };
            }
            _ => {}
        }
    }

    fn type_char(&mut self, character: char) {
        let Some(field) = self.fields().get(self.focus).copied() else {
            return;
        };
        if field.is_text()
            && let Some(value) = self.text_value_mut(field)
        {
            value.push(character);
        }
    }

    fn backspace(&mut self) {
        let Some(field) = self.fields().get(self.focus).copied() else {
            return;
        };
        if field.is_text()
            && let Some(value) = self.text_value_mut(field)
        {
            value.pop();
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let length = self.fields().len();
        if length == 0 {
            return;
        }
        self.focus = (self.focus as isize + delta).clamp(0, length as isize - 1)
            as usize;
        self.choice = None;
        self.routing = None;
        self.scroll = self.focus.saturating_sub(8) as u16;
    }

    fn tab_focus(&mut self, backwards: bool) {
        let length = self.fields().len();
        if length == 0 {
            return;
        }
        if self.footer {
            if backwards {
                if self.footer_selected == 0 {
                    self.footer = false;
                    self.focus = length - 1;
                    self.scroll = self.focus.saturating_sub(8) as u16;
                } else {
                    self.footer_selected = 0;
                }
            } else if self.footer_selected == 1 {
                self.footer = false;
                self.focus = 0;
                self.scroll = 0;
            } else {
                self.footer_selected = 1;
            }
            return;
        }
        if backwards {
            if self.focus == 0 {
                self.footer = true;
                self.footer_selected = 1;
            } else {
                self.move_focus(-1);
            }
        } else if self.focus + 1 >= length {
            self.footer = true;
            self.footer_selected = 0;
        } else {
            self.move_focus(1);
        }
    }

    fn open_choice(&mut self) {
        let Some(field) = self.fields().get(self.focus).copied() else {
            return;
        };
        let Some(options) = Self::choice_options(field) else {
            return;
        };
        let selected = self.choice_selected(field, &options);
        self.choice = Some(ChoicePopup {
            field,
            options,
            selected,
        });
    }

    fn open_routing(&mut self, field: EditorField) {
        let (family, routes) = match field {
            EditorField::Ipv4Routing => {
                (RoutingFamily::Ipv4, self.ipv4_routes.clone())
            }
            EditorField::Ipv6Routing => {
                (RoutingFamily::Ipv6, self.ipv6_routes.clone())
            }
            _ => return,
        };
        self.routing = Some(RoutingPopup::new(family, routes));
    }

    fn apply_routing(&mut self, popup: RoutingPopup) {
        match popup.family {
            RoutingFamily::Ipv4 => self.ipv4_routes = popup.routes,
            RoutingFamily::Ipv6 => self.ipv6_routes = popup.routes,
        }
        self.routing = None;
    }

    fn field_line(&self, field: EditorField) -> Line<'static> {
        let label = Self::field_label(field);
        let focused = self.focused(field);
        let value = self.text_value(field);
        let display = if field == EditorField::Password {
            if value.is_empty() {
                String::new()
            } else {
                "*".repeat(value.chars().count())
            }
        } else {
            value.to_string()
        };
        let display = if display.is_empty()
            && matches!(
                field,
                EditorField::Ipv4Address
                    | EditorField::Ipv4Dns
                    | EditorField::Ipv4Search
                    | EditorField::Ipv6Address
                    | EditorField::Ipv6Dns
                    | EditorField::Ipv6Search
            ) {
            "<Add...>".to_string()
        } else {
            display
        };
        let value_style = if focused {
            selected_style()
        } else {
            field_style()
        };
        Line::from(vec![
            Span::styled(format!("{label:<25}"), label_style()),
            Span::styled(format!(" {display:<38}"), value_style),
        ])
    }

    fn selection_line(&self, field: EditorField, value: &str) -> Line<'static> {
        let focused = self.focused(field);
        let style = if focused {
            selected_style()
        } else {
            field_style()
        };
        Line::from(vec![
            Span::styled(
                format!("{:<25}", Self::field_label(field)),
                label_style(),
            ),
            Span::styled(format!(" <{value}>"), style),
        ])
    }

    fn checkbox_line(&self, field: EditorField) -> Line<'static> {
        let checked = if self.bool_value(field) { "X" } else { " " };
        let style = if self.focused(field) {
            selected_style()
        } else {
            panel_style()
        };
        Line::from(Span::styled(
            format!("[{checked}] {}", Self::field_label(field)),
            style,
        ))
    }

    fn routing_line(
        &self,
        field: EditorField,
        routes: &[String],
    ) -> Line<'static> {
        let summary = if routes.is_empty() {
            "No custom routes".to_string()
        } else if routes.len() == 1 {
            "1 custom route".to_string()
        } else {
            format!("{} custom routes", routes.len())
        };
        let action_style = if self.focused(field) {
            selected_style()
        } else {
            panel_style().add_modifier(Modifier::BOLD)
        };
        Line::from(vec![
            Span::styled(format!("{:<25}", "Routing"), label_style()),
            Span::styled(format!("({summary}) "), panel_style()),
            Span::styled("<Edit...>", action_style),
        ])
    }

    fn form_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("")];
        lines.push(self.field_line(EditorField::ProfileName));
        lines.push(self.field_line(EditorField::Device));
        if self.mode == EditorMode::Wifi {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("- WI-FI", section_style())));
            lines.push(self.field_line(EditorField::Ssid));
            lines.push(self.field_line(EditorField::Bssid));
            lines.push(self.selection_line(
                EditorField::Security,
                match self.security {
                    Security::Open => "None",
                    Security::Psk => "WPA & WPA2 Personal",
                    Security::Eap => "WPA & WPA2 Enterprise",
                    Security::Unknown => "None",
                },
            ));
            if self.security == Security::Eap {
                lines.push(self.field_line(EditorField::Identity));
            }
            if self.security != Security::Open {
                lines.push(self.field_line(EditorField::Password));
            }
            lines.push(self.checkbox_line(EditorField::Hidden));
        } else {
            lines.push(Line::from(Span::styled("- ETHERNET", section_style())));
        }

        lines.push(Line::from(""));
        lines.push(
            self.selection_line(EditorField::Ipv4Mode, self.ipv4_mode.label()),
        );
        lines.push(self.field_line(EditorField::Ipv4Address));
        lines.push(self.field_line(EditorField::Ipv4Gateway));
        lines.push(self.field_line(EditorField::Ipv4Dns));
        lines.push(self.field_line(EditorField::Ipv4Search));
        lines.push(
            self.routing_line(EditorField::Ipv4Routing, &self.ipv4_routes),
        );
        lines.push(self.checkbox_line(EditorField::Ipv4NeverDefault));
        lines.push(self.checkbox_line(EditorField::Ipv4IgnoreRoutes));
        lines.push(self.checkbox_line(EditorField::Ipv4IgnoreDns));
        lines.push(self.checkbox_line(EditorField::Ipv4Required));

        lines.push(Line::from(""));
        lines.push(
            self.selection_line(EditorField::Ipv6Mode, self.ipv6_mode.label()),
        );
        lines.push(self.checkbox_line(EditorField::Slaac));
        lines.push(self.field_line(EditorField::Ipv6Address));
        lines.push(self.field_line(EditorField::Ipv6Gateway));
        lines.push(self.field_line(EditorField::Ipv6Dns));
        lines.push(self.field_line(EditorField::Ipv6Search));
        lines.push(
            self.routing_line(EditorField::Ipv6Routing, &self.ipv6_routes),
        );
        lines.push(self.checkbox_line(EditorField::Ipv6NeverDefault));
        lines.push(self.checkbox_line(EditorField::Ipv6IgnoreRoutes));
        lines.push(self.checkbox_line(EditorField::Ipv6IgnoreDns));
        lines.push(self.checkbox_line(EditorField::Ipv6Required));
        lines
    }

    fn footer_lines(&self) -> Vec<Line<'static>> {
        let auto = if self.autoconnect { "X" } else { " " };
        let users = if self.available_users { "X" } else { " " };
        vec![
            Line::from(format!("[{auto}] Automatically connect")),
            Line::from(format!("[{users}] Available to all users")),
            Line::from(vec![
                Span::raw(" "),
                button_span("Cancel", self.footer && self.footer_selected == 0),
                Span::raw(" "),
                button_span("OK", self.footer && self.footer_selected == 1),
            ]),
        ]
    }

    fn ipv4_value(&self) -> Result<Modification<PrefixedIpv4Addr>, String> {
        match self.ipv4_mode {
            IpMode::Automatic | IpMode::Ignore => Ok(Modification::Clear),
            IpMode::Manual => {
                let value = self.ipv4_address.trim();
                let address: PrefixedIpv4Addr = serde_json::from_value(
                    serde_json::Value::String(value.to_string()),
                )
                .map_err(|_| format!("invalid IPv4 address: {value}"))?;
                if address.prefix_len > 32 {
                    return Err(
                        "IPv4 prefix must be between 0 and 32".to_string()
                    );
                }
                Ok(Modification::Replace(address))
            }
        }
    }

    fn ipv6_value(&self) -> Result<Modification<PrefixedIpv6Addr>, String> {
        match self.ipv6_mode {
            IpMode::Automatic => Ok(Modification::NoChange),
            IpMode::Ignore => Ok(Modification::Clear),
            IpMode::Manual => {
                let value = self.ipv6_address.trim();
                let address: PrefixedIpv6Addr = serde_json::from_value(
                    serde_json::Value::String(value.to_string()),
                )
                .map_err(|_| format!("invalid IPv6 address: {value}"))?;
                if address.prefix_len > 128 {
                    return Err(
                        "IPv6 prefix must be between 0 and 128".to_string()
                    );
                }
                Ok(Modification::Replace(address))
            }
        }
    }

    fn interface_action(
        &self,
        name: String,
    ) -> Result<InterfaceManagerAction, String> {
        Ok(InterfaceManagerAction::ModLink {
            name,
            ipv4: self.ipv4_value()?,
            ipv6: self.ipv6_value()?,
            oper_state: Modification::NoChange,
            slaac: Modification::Replace(self.slaac),
            dhcpv4: Modification::Replace(self.ipv4_mode == IpMode::Automatic),
        })
    }

    fn wifi_values(
        &self,
    ) -> Result<(String, Option<MacAddr>, Security), String> {
        let ssid = self.ssid.trim().to_string();
        if ssid.is_empty() {
            return Err("SSID must not be empty".to_string());
        }
        let bssid =
            if self.bssid.trim().is_empty() {
                None
            } else {
                Some(self.bssid.trim().parse().map_err(|_| {
                    "BSSID must be aa:bb:cc:dd:ee:ff".to_string()
                })?)
            };
        match self.security {
            Security::Psk if !(8..=63).contains(&self.password.len()) => {
                return Err(
                    "WPA-PSK password must contain 8-63 bytes".to_string()
                );
            }
            Security::Eap if self.identity.trim().is_empty() => {
                return Err("EAP identity must not be empty".to_string());
            }
            Security::Eap if self.password.is_empty() => {
                return Err("EAP password must not be empty".to_string());
            }
            _ => {}
        }
        Ok((ssid, bssid, self.security))
    }
}

/// Complete client-side state for the nmtui-like flow.
pub struct App {
    pub screen: Screen,
    pub interfaces: Vec<InterfaceInfo>,
    pub scan_results: Vec<ScanResult>,
    pub known_networks: Vec<KnownNetwork>,
    pub wifi_status: Option<SupplicantStatus>,
    pub main_selected: usize,
    pub main_focus: usize,
    pub edit_selected: usize,
    pub edit_focus: usize,
    pub activation_selected: usize,
    pub activation_focus: usize,
    pub popup: Popup,
    pub hostname: String,
    pub hostname_focus: usize,
    wifi_iface: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::MainMenu,
            interfaces: Vec::new(),
            scan_results: Vec::new(),
            known_networks: Vec::new(),
            wifi_status: None,
            main_selected: 0,
            main_focus: 0,
            edit_selected: 0,
            edit_focus: 0,
            activation_selected: 0,
            activation_focus: 0,
            popup: Popup::None,
            hostname: String::new(),
            hostname_focus: 0,
            wifi_iface: None,
        }
    }
}

impl App {
    /// Refresh interfaces once before entering the activity picker.
    pub fn refresh_interfaces(&mut self, client: &mut DaemonClient) {
        let command = DaemonCommand::InterfaceManager {
            action: InterfaceManagerAction::GetAllInterfaces,
        };
        match client.request(&command) {
            Ok(DaemonResponse::InterfaceManager {
                response: InterfaceResponse::InfoList(mut list),
            }) => {
                list.sort_by_key(|interface| interface.id);
                self.interfaces = list;
                self.clamp_interface_state();
            }
            Ok(response) => {
                self.show_error(
                    "Interface refresh failed",
                    format!("{response:?}"),
                );
            }
            Err(error) => self.show_error("Interface refresh failed", error),
        }
    }

    fn clamp_interface_state(&mut self) {
        if let Some(name) = self
            .interfaces
            .iter()
            .find(|interface| interface.is_wlan())
            .map(|interface| interface.name.clone())
        {
            self.wifi_iface = Some(name);
        } else if self.wifi_iface.as_ref().is_some_and(|name| {
            !self
                .interfaces
                .iter()
                .any(|interface| interface.name == *name && interface.is_wlan())
        }) {
            self.wifi_iface = None;
        }
    }

    fn request(
        &mut self,
        client: &mut DaemonClient,
        command: &DaemonCommand,
    ) -> Result<DaemonResponse, String> {
        match client.request(command) {
            Ok(DaemonResponse::Error(error)) => Err(error.to_string()),
            Ok(DaemonResponse::Global {
                response: GlobalDaemonResponse::Error { message },
            }) => Err(message),
            Ok(DaemonResponse::Global {
                response: GlobalDaemonResponse::WiFiInterfaceNotFound { iface },
            }) => Err(format!("Wi-Fi interface not found: {iface}")),
            Ok(response) => Ok(response),
            Err(error) => Err(error.to_string()),
        }
    }

    fn show_error<T: std::fmt::Display>(&mut self, title: &str, error: T) {
        self.popup = Popup::Message(MessagePopup {
            title: title.to_string(),
            message: error.to_string(),
        });
    }

    fn show_message(&mut self, title: &str, message: impl Into<String>) {
        self.popup = Popup::Message(MessagePopup {
            title: title.to_string(),
            message: message.into(),
        });
    }

    fn first_wifi(&self) -> Option<String> {
        self.wifi_iface.clone().or_else(|| {
            self.interfaces
                .iter()
                .find(|interface| interface.is_wlan())
                .map(|interface| interface.name.clone())
        })
    }

    fn refresh_wifi(&mut self, client: &mut DaemonClient, scan: bool) {
        let Some(iface) = self.first_wifi() else {
            return;
        };
        self.wifi_iface = Some(iface.clone());
        if scan {
            let command = DaemonCommand::WiFiManager {
                iface: iface.clone(),
                action: WiFiManagerAction::Scan,
            };
            let _ = self.request(client, &command);
        }
        let scan_command = DaemonCommand::WiFiManager {
            iface: iface.clone(),
            action: WiFiManagerAction::ScanResults,
        };
        if let Ok(DaemonResponse::WiFiManager {
            response: WiFiManagerResponse::ScanResults(mut results),
            ..
        }) = self.request(client, &scan_command)
        {
            results.sort_by_key(|result| Reverse(result.signal));
            self.scan_results = results;
        }
        let status_command = DaemonCommand::WiFiManager {
            iface: iface.clone(),
            action: WiFiManagerAction::Status,
        };
        if let Ok(DaemonResponse::WiFiManager {
            response: WiFiManagerResponse::Status(status),
            ..
        }) = self.request(client, &status_command)
        {
            self.wifi_status = Some(status);
        }
        let known_command = DaemonCommand::WiFiManager {
            iface,
            action: WiFiManagerAction::KnownNetworks,
        };
        if let Ok(DaemonResponse::WiFiManager {
            response: WiFiManagerResponse::KnownNetworks(mut networks),
            ..
        }) = self.request(client, &known_command)
        {
            networks.sort_by_key(|network| Reverse(network.priority));
            self.known_networks = networks;
        }
    }

    fn edit_rows(&self) -> Vec<EditRow> {
        let mut rows = vec![EditRow {
            label: "Ethernet".to_string(),
            target: None,
        }];
        rows.extend(
            self.interfaces
                .iter()
                .filter(|interface| {
                    interface.interface_type != InterfaceType::Loopback
                        && !interface.is_wlan()
                })
                .map(|interface| EditRow {
                    label: format!(
                        "  {}{}",
                        if interface.state.is_online() {
                            "* "
                        } else {
                            ""
                        },
                        interface.name
                    ),
                    target: Some(EditTarget::Interface(interface.name.clone())),
                }),
        );
        rows.push(EditRow {
            label: "Wi-Fi".to_string(),
            target: None,
        });
        if self.known_networks.is_empty() {
            rows.push(EditRow {
                label: "  (no saved connections)".to_string(),
                target: None,
            });
        } else {
            rows.extend(self.known_networks.iter().map(|network| EditRow {
                label: format!(
                    "  {}{}",
                    if network.is_current() { "* " } else { "" },
                    network.ssid
                ),
                target: Some(EditTarget::Network(network.clone())),
            }));
        }
        rows.push(EditRow {
            label: "Bluetooth".to_string(),
            target: None,
        });
        rows.push(EditRow {
            label: "  (no saved connections)".to_string(),
            target: None,
        });
        rows
    }

    fn activation_rows(&self) -> Vec<ActivationRow> {
        let mut rows = vec![ActivationRow {
            label: "Ethernet".to_string(),
            signal: None,
            target: None,
        }];
        rows.extend(
            self.interfaces
                .iter()
                .filter(|interface| {
                    interface.interface_type != InterfaceType::Loopback
                        && !interface.is_wlan()
                })
                .map(|interface| ActivationRow {
                    label: format!(
                        "  {}Wired connection 1 ({})",
                        if interface.state.is_online() {
                            "* "
                        } else {
                            ""
                        },
                        interface.name
                    ),
                    signal: None,
                    target: Some(ActivationTarget::Interface(
                        interface.name.clone(),
                    )),
                }),
        );
        rows.push(ActivationRow {
            label: "Wi-Fi".to_string(),
            signal: None,
            target: None,
        });
        if self.scan_results.is_empty() {
            rows.push(ActivationRow {
                label: "  (no wireless networks found)".to_string(),
                signal: None,
                target: None,
            });
        } else if let Some(iface) = self.first_wifi() {
            rows.extend(self.scan_results.iter().map(|network| {
                ActivationRow {
                    label: format!(
                        "  {}{}  {}",
                        if self.is_current_wifi(network) {
                            "* "
                        } else {
                            ""
                        },
                        network.ssid,
                        network.security
                    ),
                    signal: Some(network.signal),
                    target: Some(ActivationTarget::Wifi(WifiTarget {
                        iface: iface.clone(),
                        ssid: network.ssid.clone(),
                        bssid: Some(network.bssid),
                        security: network.security,
                        hidden: false,
                    })),
                }
            }));
        }
        rows.push(ActivationRow {
            label: "Bluetooth".to_string(),
            signal: None,
            target: None,
        });
        rows.push(ActivationRow {
            label: "  (no Bluetooth connections)".to_string(),
            signal: None,
            target: None,
        });
        rows
    }

    fn is_current_wifi(&self, network: &ScanResult) -> bool {
        self.wifi_status
            .as_ref()
            .and_then(|status| status.ssid.as_ref())
            .is_some_and(|ssid| ssid == &network.ssid)
    }

    fn is_current_wifi_target(&self, target: &WifiTarget) -> bool {
        self.wifi_status.as_ref().is_some_and(|status| {
            status.state == Some(WpaState::Completed)
                && status.ssid.as_deref() == Some(target.ssid.as_str())
        })
    }

    fn move_main(&mut self, delta: isize) {
        let length = MAIN_OPTIONS.len();
        self.main_selected = (self.main_selected as isize + delta)
            .clamp(0, length as isize - 1)
            as usize;
    }

    fn move_edit(&mut self, delta: isize) {
        let rows = self.edit_rows();
        let mut index = self.edit_selected as isize;
        for _ in 0..rows.len() {
            index = (index + delta).rem_euclid(rows.len() as isize);
            if rows[index as usize].target.is_some() {
                self.edit_selected = index as usize;
                return;
            }
        }
    }

    fn move_activation(&mut self, delta: isize) {
        let rows = self.activation_rows();
        if rows.is_empty() {
            return;
        }
        let mut index = self.activation_selected as isize;
        for _ in 0..rows.len() {
            index = (index + delta).rem_euclid(rows.len() as isize);
            if rows[index as usize].target.is_some() {
                self.activation_selected = index as usize;
                return;
            }
        }
    }

    fn first_edit_target(&self) -> Option<usize> {
        self.edit_rows().iter().position(|row| row.target.is_some())
    }

    fn first_activation_target(&self) -> Option<usize> {
        self.activation_rows()
            .iter()
            .position(|row| row.target.is_some())
    }

    fn open_main_choice(&mut self, client: &mut DaemonClient) -> bool {
        match self.main_selected {
            0 => {
                self.refresh_interfaces(client);
                self.screen = Screen::EditConnections;
                self.edit_selected = self.first_edit_target().unwrap_or(0);
                self.edit_focus = 0;
                false
            }
            1 => {
                self.refresh_interfaces(client);
                self.refresh_wifi(client, true);
                self.screen = Screen::ActivateConnections;
                self.activation_selected =
                    self.first_activation_target().unwrap_or(0);
                self.activation_focus = 0;
                false
            }
            2 => {
                self.screen = Screen::Hostname;
                self.hostname_focus = 0;
                self.load_hostname(client);
                false
            }
            3 => true,
            _ => false,
        }
    }

    fn load_hostname(&mut self, client: &mut DaemonClient) {
        let command = DaemonCommand::Global {
            action: GlobalDaemonAction::GetHostname,
        };
        match self.request(client, &command) {
            Ok(DaemonResponse::Global {
                response: GlobalDaemonResponse::Hostname(name),
            }) => self.hostname = name,
            Ok(response) => {
                self.show_error(
                    "Hostname read failed",
                    format!("{response:?}"),
                );
            }
            Err(error) => self.show_error("Hostname read failed", error),
        }
    }

    fn save_hostname(&mut self, client: &mut DaemonClient) {
        let name = self.hostname.trim().to_string();
        if name.is_empty() {
            self.show_message("Invalid hostname", "Hostname must not be empty");
            return;
        }
        let command = DaemonCommand::Global {
            action: GlobalDaemonAction::SetHostname { name },
        };
        match self.request(client, &command) {
            Ok(DaemonResponse::Global {
                response: GlobalDaemonResponse::Hostname(name),
            }) => {
                self.hostname = name;
                self.screen = Screen::MainMenu;
            }
            Ok(response) => {
                self.show_error(
                    "Hostname update failed",
                    format!("{response:?}"),
                );
            }
            Err(error) => self.show_error("Hostname update failed", error),
        }
    }

    fn open_edit(&mut self) {
        let rows = self.edit_rows();
        let Some(target) = rows
            .get(self.edit_selected)
            .and_then(|row| row.target.clone())
        else {
            return;
        };
        match target {
            EditTarget::Interface(name) => {
                if let Some(interface) = self
                    .interfaces
                    .iter()
                    .find(|interface| interface.name == name)
                {
                    self.popup = Popup::Editor(Box::new(
                        ConnectionEditor::from_interface(interface),
                    ));
                }
            }
            EditTarget::Network(network) => {
                let Some(iface) = self.first_wifi() else {
                    self.show_message(
                        "Cannot edit connection",
                        "No Wi-Fi interface is available",
                    );
                    return;
                };
                self.popup = Popup::Editor(Box::new(
                    ConnectionEditor::from_wifi(&iface, None, Some(&network)),
                ));
            }
        }
    }

    fn delete_edit(&mut self, client: &mut DaemonClient) {
        let rows = self.edit_rows();
        let Some(EditTarget::Network(network)) = rows
            .get(self.edit_selected)
            .and_then(|row| row.target.clone())
        else {
            self.show_message(
                "Delete connection",
                "Only saved Wi-Fi connections can be deleted here",
            );
            return;
        };
        let command = DaemonCommand::WiFiManager {
            iface: self.first_wifi().unwrap_or_default(),
            action: WiFiManagerAction::RemoveNetwork {
                ssid: network.ssid,
                bssid: network.bssid,
            },
        };
        match self.request(client, &command) {
            Ok(_) => {
                self.refresh_wifi(client, false);
                self.edit_selected = self.edit_selected.saturating_sub(1);
            }
            Err(error) => self.show_error("Delete connection failed", error),
        }
    }

    fn open_new_connection(&mut self) {
        self.popup = Popup::NewConnection(NewConnectionPopup {
            selected: 0,
            focus: NewFocus::List,
        });
    }

    fn create_connection(&mut self, selected: usize) {
        match selected {
            0 => {
                let Some(iface) = self.first_wifi() else {
                    self.show_message(
                        "New Wi-Fi connection",
                        "No Wi-Fi interface is available",
                    );
                    return;
                };
                self.popup = Popup::Editor(Box::new(
                    ConnectionEditor::from_wifi(&iface, None, None),
                ));
            }
            _ => self.show_message(
                "Connection type unavailable",
                format!(
                    "{} connections are not supported by this daemon yet",
                    NEW_CONNECTION_TYPES[selected]
                ),
            ),
        }
    }

    fn interface_action(&mut self, client: &mut DaemonClient, name: String) {
        let Some(interface) = self
            .interfaces
            .iter()
            .find(|interface| interface.name == name)
        else {
            return;
        };
        if interface.interface_type == InterfaceType::Loopback {
            self.show_message(
                "Activate a connection",
                "The loopback interface cannot be activated",
            );
            return;
        }
        let up = !matches!(
            interface.state,
            ConnectionState::Connected | ConnectionState::Up
        );
        let command = DaemonCommand::InterfaceManager {
            action: InterfaceManagerAction::ModLink {
                name,
                ipv4: Modification::NoChange,
                ipv6: Modification::NoChange,
                oper_state: Modification::Replace(up),
                slaac: Modification::NoChange,
                dhcpv4: Modification::NoChange,
            },
        };
        match self.request(client, &command) {
            Ok(_) => self.refresh_interfaces(client),
            Err(error) => self.show_error("Interface activation failed", error),
        }
    }

    fn activate_selected(&mut self, client: &mut DaemonClient) {
        let rows = self.activation_rows();
        let Some(target) = rows
            .get(self.activation_selected)
            .and_then(|row| row.target.clone())
        else {
            return;
        };
        match target {
            ActivationTarget::Interface(name) => {
                self.interface_action(client, name)
            }
            ActivationTarget::Wifi(target) => {
                if self.is_current_wifi_target(&target) {
                    self.disconnect_wifi(client, target.iface);
                } else {
                    self.start_wifi(client, target, None);
                }
            }
        }
    }

    fn disconnect_wifi(&mut self, client: &mut DaemonClient, iface: String) {
        let command = DaemonCommand::WiFiManager {
            iface,
            action: WiFiManagerAction::Disconnect,
        };
        match self.request(client, &command) {
            Ok(_) => self.refresh_wifi(client, false),
            Err(error) => self.show_error("Disconnect failed", error),
        }
    }

    fn activation_button_label(&self) -> &'static str {
        let rows = self.activation_rows();
        match rows
            .get(self.activation_selected)
            .and_then(|row| row.target.as_ref())
        {
            Some(ActivationTarget::Interface(name))
                if self.interfaces.iter().any(|interface| {
                    interface.name == *name
                        && matches!(
                            interface.state,
                            ConnectionState::Connected | ConnectionState::Up
                        )
                }) =>
            {
                "Deactivate"
            }
            Some(ActivationTarget::Wifi(target))
                if self.is_current_wifi_target(target) =>
            {
                "Deactivate"
            }
            _ => "Activate",
        }
    }

    fn start_wifi(
        &mut self,
        client: &mut DaemonClient,
        target: WifiTarget,
        password: Option<String>,
    ) {
        let known = self.known_networks.iter().find(|network| {
            network.ssid == target.ssid
                && (network.bssid.is_none() || network.bssid == target.bssid)
        });
        if password.is_none()
            && known.is_none()
            && target.security != Security::Open
        {
            self.popup = Popup::Password(PasswordPopup {
                target,
                input: String::new(),
                show: false,
                focus: PasswordFocus::Input,
            });
            return;
        }
        if known.is_none() {
            let add_command = DaemonCommand::WiFiManager {
                iface: target.iface.clone(),
                action: WiFiManagerAction::AddNetwork {
                    ssid: target.ssid.clone(),
                    bssid: target.bssid,
                    security: target.security,
                    password,
                    identity: if target.security == Security::Eap {
                        Some("user".to_string())
                    } else {
                        None
                    },
                    hidden: target.hidden,
                },
            };
            if let Err(error) = self.request(client, &add_command) {
                self.show_error("Add connection failed", error);
                return;
            }
        }
        let connect_command = DaemonCommand::WiFiManager {
            iface: target.iface.clone(),
            action: WiFiManagerAction::Connect {
                ssid: target.ssid.clone(),
                bssid: target.bssid,
            },
        };
        match self.request(client, &connect_command) {
            Ok(_) => {
                self.popup = Popup::Connecting(ConnectingPopup {
                    target,
                    last_poll: Instant::now() - Duration::from_secs(1),
                });
            }
            Err(error) => self.show_error("Connection failed", error),
        }
    }

    fn save_editor(
        &mut self,
        client: &mut DaemonClient,
        editor: ConnectionEditor,
    ) {
        match editor.mode {
            EditorMode::Ethernet => {
                let name = editor.device.trim().to_string();
                if name.is_empty() {
                    self.show_message(
                        "Invalid connection",
                        "Device must not be empty",
                    );
                    return;
                }
                match editor.interface_action(name) {
                    Ok(action) => {
                        let command =
                            DaemonCommand::InterfaceManager { action };
                        match self.request(client, &command) {
                            Ok(_) => {
                                self.popup = Popup::None;
                                self.refresh_interfaces(client);
                            }
                            Err(error) => {
                                self.show_error("Save connection failed", error)
                            }
                        }
                    }
                    Err(error) => {
                        self.show_message("Invalid connection", error)
                    }
                }
            }
            EditorMode::Wifi => {
                let (ssid, bssid, security) = match editor.wifi_values() {
                    Ok(values) => values,
                    Err(error) => {
                        self.show_message("Invalid connection", error);
                        return;
                    }
                };
                let iface = editor.device.trim().to_string();
                if iface.is_empty() {
                    self.show_message(
                        "Invalid connection",
                        "Device must not be empty",
                    );
                    return;
                }
                if let Some((old_ssid, old_bssid)) =
                    editor.existing_network.clone()
                {
                    let remove = DaemonCommand::WiFiManager {
                        iface: iface.clone(),
                        action: WiFiManagerAction::RemoveNetwork {
                            ssid: old_ssid,
                            bssid: old_bssid,
                        },
                    };
                    if let Err(error) = self.request(client, &remove) {
                        self.show_error("Update connection failed", error);
                        return;
                    }
                }
                let add = DaemonCommand::WiFiManager {
                    iface: iface.clone(),
                    action: WiFiManagerAction::AddNetwork {
                        ssid,
                        bssid,
                        security,
                        password: if editor.password.is_empty() {
                            None
                        } else {
                            Some(editor.password)
                        },
                        identity: if editor.identity.is_empty() {
                            None
                        } else {
                            Some(editor.identity)
                        },
                        hidden: editor.hidden,
                    },
                };
                match self.request(client, &add) {
                    Ok(_) => {
                        self.popup = Popup::None;
                        self.refresh_wifi(client, false);
                    }
                    Err(error) => {
                        self.show_error("Save connection failed", error)
                    }
                }
            }
        }
    }

    /// Handle one key press. Returns true when the event loop should quit.
    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        if !matches!(self.popup, Popup::None) {
            return self.handle_popup_key(key, client);
        }
        match self.screen {
            Screen::MainMenu => self.handle_main_key(key, client),
            Screen::EditConnections => self.handle_edit_key(key, client),
            Screen::ActivateConnections => {
                self.handle_activation_key(key, client)
            }
            Screen::Hostname => self.handle_hostname_key(key, client),
        }
    }

    fn handle_main_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        use crossterm::event::KeyCode;
        if self.main_focus == 0 {
            match key {
                KeyCode::Up => self.move_main(-1),
                KeyCode::Down => self.move_main(1),
                KeyCode::Tab | KeyCode::BackTab => self.main_focus = 1,
                KeyCode::Enter => return self.open_main_choice(client),
                KeyCode::Esc | KeyCode::Char('q') => return true,
                _ => {}
            }
        } else {
            match key {
                KeyCode::Tab | KeyCode::BackTab => self.main_focus = 0,
                KeyCode::Enter => return self.open_main_choice(client),
                KeyCode::Esc | KeyCode::Char('q') => return true,
                _ => {}
            }
        }
        false
    }

    fn handle_edit_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        use crossterm::event::KeyCode;
        if self.edit_focus == 0 {
            match key {
                KeyCode::Up => self.move_edit(-1),
                KeyCode::Down => self.move_edit(1),
                KeyCode::Enter | KeyCode::Char('e') => self.open_edit(),
                KeyCode::Char('a') | KeyCode::Char('n') => {
                    self.open_new_connection()
                }
                KeyCode::Char('d') => self.delete_edit(client),
                KeyCode::Tab => self.edit_focus = 1,
                KeyCode::BackTab => self.edit_focus = 4,
                KeyCode::Esc => self.screen = Screen::MainMenu,
                _ => {}
            }
        } else {
            match key {
                KeyCode::Up => {
                    self.edit_focus = if self.edit_focus == 1 {
                        4
                    } else {
                        self.edit_focus - 1
                    }
                }
                KeyCode::Down => {
                    self.edit_focus = if self.edit_focus == 4 {
                        1
                    } else {
                        self.edit_focus + 1
                    }
                }
                KeyCode::Tab => {
                    self.edit_focus = if self.edit_focus == 4 {
                        0
                    } else {
                        self.edit_focus + 1
                    }
                }
                KeyCode::BackTab => {
                    self.edit_focus = if self.edit_focus == 1 {
                        0
                    } else {
                        self.edit_focus - 1
                    }
                }
                KeyCode::Enter => match self.edit_focus {
                    1 => self.open_new_connection(),
                    2 => self.open_edit(),
                    3 => self.delete_edit(client),
                    4 => self.screen = Screen::MainMenu,
                    _ => {}
                },
                KeyCode::Esc => self.screen = Screen::MainMenu,
                _ => {}
            }
        }
        false
    }

    fn handle_activation_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        use crossterm::event::KeyCode;
        if self.activation_focus == 0 {
            match key {
                KeyCode::Up => self.move_activation(-1),
                KeyCode::Down => self.move_activation(1),
                KeyCode::Enter => self.activate_selected(client),
                KeyCode::Char('r') => self.refresh_wifi(client, true),
                KeyCode::Tab => self.activation_focus = 1,
                KeyCode::BackTab => self.activation_focus = 2,
                KeyCode::Esc => self.screen = Screen::MainMenu,
                _ => {}
            }
        } else {
            match key {
                KeyCode::Up => {
                    self.activation_focus =
                        if self.activation_focus == 1 { 2 } else { 1 }
                }
                KeyCode::Down => {
                    self.activation_focus =
                        if self.activation_focus == 2 { 1 } else { 2 }
                }
                KeyCode::Tab => {
                    self.activation_focus = if self.activation_focus == 2 {
                        0
                    } else {
                        self.activation_focus + 1
                    }
                }
                KeyCode::BackTab => {
                    self.activation_focus = if self.activation_focus == 1 {
                        0
                    } else {
                        self.activation_focus - 1
                    }
                }
                KeyCode::Enter => match self.activation_focus {
                    1 => self.activate_selected(client),
                    2 => self.screen = Screen::MainMenu,
                    _ => {}
                },
                KeyCode::Esc => self.screen = Screen::MainMenu,
                _ => {}
            }
        }
        false
    }

    fn handle_hostname_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        use crossterm::event::KeyCode;
        match key {
            KeyCode::Tab => self.hostname_focus = (self.hostname_focus + 1) % 3,
            KeyCode::BackTab => {
                self.hostname_focus = (self.hostname_focus + 2) % 3
            }
            KeyCode::Up => {
                self.hostname_focus = self.hostname_focus.saturating_sub(1)
            }
            KeyCode::Down => {
                self.hostname_focus = (self.hostname_focus + 1).min(2)
            }
            KeyCode::Char(character) if self.hostname_focus == 0 => {
                self.hostname.push(character)
            }
            KeyCode::Backspace if self.hostname_focus == 0 => {
                self.hostname.pop();
            }
            KeyCode::Enter => match self.hostname_focus {
                0 => self.hostname_focus = 2,
                1 => self.screen = Screen::MainMenu,
                2 => self.save_hostname(client),
                _ => {}
            },
            KeyCode::Esc => self.screen = Screen::MainMenu,
            _ => {}
        }
        false
    }

    fn handle_popup_key(
        &mut self,
        key: crossterm::event::KeyCode,
        client: &mut DaemonClient,
    ) -> bool {
        use crossterm::event::KeyCode;
        match &mut self.popup {
            Popup::Message(_) => {
                if matches!(key, KeyCode::Enter | KeyCode::Esc) {
                    self.popup = Popup::None;
                }
            }
            Popup::Connecting(_) => {
                if key == KeyCode::Esc {
                    self.popup = Popup::None;
                }
            }
            Popup::NewConnection(popup) => {
                let mut create = None;
                let mut cancel = false;
                match key {
                    KeyCode::Up if popup.focus == NewFocus::List => {
                        popup.selected = popup.selected.saturating_sub(1);
                    }
                    KeyCode::Down if popup.focus == NewFocus::List => {
                        popup.selected = (popup.selected + 1)
                            .min(NEW_CONNECTION_TYPES.len() - 1);
                    }
                    KeyCode::Tab => {
                        popup.focus = match popup.focus {
                            NewFocus::List => NewFocus::Cancel,
                            NewFocus::Cancel => NewFocus::Create,
                            NewFocus::Create => NewFocus::List,
                        };
                    }
                    KeyCode::BackTab => {
                        popup.focus = match popup.focus {
                            NewFocus::List => NewFocus::Create,
                            NewFocus::Cancel => NewFocus::List,
                            NewFocus::Create => NewFocus::Cancel,
                        };
                    }
                    KeyCode::Left | KeyCode::Right
                        if matches!(
                            popup.focus,
                            NewFocus::Cancel | NewFocus::Create
                        ) =>
                    {
                        popup.focus = match popup.focus {
                            NewFocus::Cancel => NewFocus::Create,
                            NewFocus::Create => NewFocus::Cancel,
                            NewFocus::List => NewFocus::List,
                        };
                    }
                    KeyCode::Enter => match popup.focus {
                        NewFocus::List => popup.focus = NewFocus::Create,
                        NewFocus::Cancel => cancel = true,
                        NewFocus::Create => create = Some(popup.selected),
                    },
                    KeyCode::Esc => cancel = true,
                    _ => {}
                }
                if cancel {
                    self.popup = Popup::None;
                } else if let Some(selected) = create {
                    self.popup = Popup::None;
                    self.create_connection(selected);
                }
            }
            Popup::Password(popup) => {
                let mut submit = None;
                let mut cancel = false;
                match key {
                    KeyCode::Tab => {
                        popup.focus = match popup.focus {
                            PasswordFocus::Input => PasswordFocus::Show,
                            PasswordFocus::Show => PasswordFocus::Cancel,
                            PasswordFocus::Cancel => PasswordFocus::Connect,
                            PasswordFocus::Connect => PasswordFocus::Input,
                        };
                    }
                    KeyCode::BackTab => {
                        popup.focus = match popup.focus {
                            PasswordFocus::Input => PasswordFocus::Connect,
                            PasswordFocus::Show => PasswordFocus::Input,
                            PasswordFocus::Cancel => PasswordFocus::Show,
                            PasswordFocus::Connect => PasswordFocus::Cancel,
                        };
                    }
                    KeyCode::Left | KeyCode::Right
                        if matches!(
                            popup.focus,
                            PasswordFocus::Cancel | PasswordFocus::Connect
                        ) =>
                    {
                        popup.focus = match popup.focus {
                            PasswordFocus::Cancel => PasswordFocus::Connect,
                            PasswordFocus::Connect => PasswordFocus::Cancel,
                            PasswordFocus::Input | PasswordFocus::Show => {
                                popup.focus
                            }
                        };
                    }
                    KeyCode::Char(character)
                        if popup.focus == PasswordFocus::Input =>
                    {
                        popup.input.push(character);
                    }
                    KeyCode::Backspace
                        if popup.focus == PasswordFocus::Input =>
                    {
                        popup.input.pop();
                    }
                    KeyCode::Char(' ')
                        if popup.focus == PasswordFocus::Show =>
                    {
                        popup.show = !popup.show
                    }
                    KeyCode::Enter => match popup.focus {
                        PasswordFocus::Input => {
                            popup.focus = PasswordFocus::Connect
                        }
                        PasswordFocus::Show => popup.show = !popup.show,
                        PasswordFocus::Cancel => cancel = true,
                        PasswordFocus::Connect => {
                            submit = Some((
                                popup.target.clone(),
                                popup.input.clone(),
                            ));
                        }
                    },
                    KeyCode::Esc => cancel = true,
                    _ => {}
                }
                if cancel {
                    self.popup = Popup::None;
                } else if let Some((target, password)) = submit {
                    self.popup = Popup::None;
                    self.start_wifi(client, target, Some(password));
                }
            }
            Popup::Editor(editor) => {
                let mut save = false;
                let mut cancel = false;
                let mut close_routing = false;
                let mut apply_routing = false;
                if let Some(choice) = &mut editor.choice {
                    match key {
                        KeyCode::Up => {
                            choice.selected = choice.selected.saturating_sub(1)
                        }
                        KeyCode::Down => {
                            choice.selected = (choice.selected + 1)
                                .min(choice.options.len().saturating_sub(1));
                        }
                        KeyCode::Enter => editor.apply_choice(),
                        KeyCode::Esc => editor.choice = None,
                        _ => {}
                    }
                } else if editor.routing.is_some() {
                    {
                        let routing = editor
                            .routing
                            .as_mut()
                            .expect("routing popup exists");
                        if routing.editing {
                            match key {
                                KeyCode::Char(character) => {
                                    routing.input.push(character)
                                }
                                KeyCode::Backspace => {
                                    routing.input.pop();
                                }
                                KeyCode::Enter => {
                                    if let Err(error) = routing.add_route() {
                                        routing.error = Some(error);
                                    }
                                }
                                KeyCode::Esc => {
                                    routing.editing = false;
                                    routing.input.clear();
                                    routing.error = None;
                                }
                                _ => {}
                            }
                        } else {
                            match key {
                                KeyCode::Up => routing.move_cursor(-1),
                                KeyCode::Down => routing.move_cursor(1),
                                KeyCode::Tab => routing.tab(false),
                                KeyCode::BackTab => routing.tab(true),
                                KeyCode::Left | KeyCode::Right
                                    if matches!(
                                        routing.focus,
                                        RoutingFocus::Cancel | RoutingFocus::Ok
                                    ) =>
                                {
                                    routing.focus = match routing.focus {
                                        RoutingFocus::Cancel => {
                                            RoutingFocus::Ok
                                        }
                                        RoutingFocus::Ok => {
                                            RoutingFocus::Cancel
                                        }
                                        _ => routing.focus,
                                    };
                                }
                                KeyCode::Enter => match routing.focus {
                                    RoutingFocus::Add => routing.begin_add(),
                                    RoutingFocus::Delete => {
                                        routing.delete_selected()
                                    }
                                    RoutingFocus::Cancel => {
                                        close_routing = true
                                    }
                                    RoutingFocus::Ok => apply_routing = true,
                                    RoutingFocus::List => {}
                                },
                                KeyCode::Esc => close_routing = true,
                                _ => {}
                            }
                        }
                    }
                    if close_routing {
                        editor.routing = None;
                    } else if apply_routing {
                        let routing = editor
                            .routing
                            .take()
                            .expect("routing popup exists");
                        editor.apply_routing(routing);
                    }
                } else if editor.footer {
                    match key {
                        KeyCode::Tab => editor.tab_focus(false),
                        KeyCode::BackTab => editor.tab_focus(true),
                        KeyCode::Left | KeyCode::Right => {
                            editor.footer_selected = 1 - editor.footer_selected
                        }
                        KeyCode::Up => editor.footer = false,
                        KeyCode::Enter if editor.footer_selected == 0 => {
                            cancel = true
                        }
                        KeyCode::Enter => save = true,
                        KeyCode::Esc => cancel = true,
                        _ => {}
                    }
                } else {
                    match key {
                        KeyCode::Up => editor.move_focus(-1),
                        KeyCode::Down => editor.move_focus(1),
                        KeyCode::Tab => editor.tab_focus(false),
                        KeyCode::BackTab => editor.tab_focus(true),
                        KeyCode::Enter => {
                            if let Some(field) =
                                editor.fields().get(editor.focus).copied()
                            {
                                if field.is_routing() {
                                    editor.open_routing(field);
                                } else if ConnectionEditor::choice_options(
                                    field,
                                )
                                .is_some()
                                {
                                    editor.open_choice();
                                } else if field.is_bool() {
                                    editor.toggle(field);
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(field) =
                                editor.fields().get(editor.focus).copied()
                                && field.is_bool()
                            {
                                editor.toggle(field);
                            } else {
                                editor.type_char(' ');
                            }
                        }
                        KeyCode::Char(character) => editor.type_char(character),
                        KeyCode::Backspace => editor.backspace(),
                        KeyCode::Esc => cancel = true,
                        _ => {}
                    }
                }
                if cancel {
                    self.popup = Popup::None;
                } else if save {
                    let popup = std::mem::replace(&mut self.popup, Popup::None);
                    if let Popup::Editor(editor) = popup {
                        self.save_editor(client, *editor);
                    }
                }
            }
            Popup::None => {}
        }
        false
    }

    /// Poll an in-flight Wi-Fi connection without blocking the event loop on every frame.
    pub fn tick(&mut self, client: &mut DaemonClient) {
        let target = match &mut self.popup {
            Popup::Connecting(popup)
                if popup.last_poll.elapsed() >= Duration::from_millis(500) =>
            {
                popup.last_poll = Instant::now();
                Some(popup.target.clone())
            }
            _ => None,
        };
        let Some(target) = target else {
            return;
        };
        let command = DaemonCommand::WiFiManager {
            iface: target.iface.clone(),
            action: WiFiManagerAction::Status,
        };
        match self.request(client, &command) {
            Ok(DaemonResponse::WiFiManager {
                response: WiFiManagerResponse::Status(status),
                ..
            }) if status.state == Some(WpaState::Completed) => {
                self.wifi_status = Some(status);
                self.popup = Popup::None;
                self.refresh_interfaces(client);
            }
            Ok(DaemonResponse::WiFiManager {
                response: WiFiManagerResponse::Status(status),
                ..
            }) if matches!(
                status.state,
                Some(WpaState::Disconnected)
                    | Some(WpaState::InterfaceDisabled)
            ) =>
            {
                self.popup = Popup::Message(MessagePopup {
                    title: "Connection failed".to_string(),
                    message: format!("{}: {:?}", target.ssid, status.state),
                });
            }
            Ok(_) | Err(_) => {}
        }
    }

    /// Render the current screen and any popup into the terminal frame.
    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(screen_style()), area);
        match self.screen {
            Screen::MainMenu => self.draw_main(frame, area),
            Screen::EditConnections => self.draw_edit_connections(frame, area),
            Screen::ActivateConnections => {
                self.draw_activate_connections(frame, area)
            }
            Screen::Hostname => self.draw_hostname(frame, area),
        }
        match &self.popup {
            Popup::None => {}
            Popup::Message(popup) => self.draw_message(frame, area, popup),
            Popup::NewConnection(popup) => {
                self.draw_new_connection(frame, area, popup)
            }
            Popup::Password(popup) => self.draw_password(frame, area, popup),
            Popup::Connecting(popup) => {
                self.draw_connecting(frame, area, popup)
            }
            Popup::Editor(editor) => self.draw_editor(frame, area, editor),
        }
    }

    fn draw_main(&self, frame: &mut Frame, area: Rect) {
        let rect = centered(area, 34, 17);
        draw_frame(frame, rect, "NetworkManager TUI");
        let body = inner(rect);
        let instruction = Rect {
            x: body.x + 2,
            y: body.y + 1,
            width: body.width.saturating_sub(4),
            height: 2,
        };
        frame.render_widget(
            Paragraph::new("Please select an option")
                .style(instruction_style()),
            instruction,
        );
        let list_rect = Rect {
            x: body.x + 2,
            y: body.y + 3,
            width: body.width.saturating_sub(4),
            height: 7,
        };
        let items = vec![
            ListItem::new(MAIN_OPTIONS[0]),
            ListItem::new(MAIN_OPTIONS[1]),
            ListItem::new(MAIN_OPTIONS[2]),
            ListItem::new(""),
            ListItem::new(MAIN_OPTIONS[3]),
        ];
        let mut state = ListState::default();
        let selected_row = if self.main_selected >= 3 {
            self.main_selected + 1
        } else {
            self.main_selected
        };
        state.select(Some(selected_row));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(selected_style())
                .highlight_symbol(""),
            list_rect,
            &mut state,
        );
        let footer = Rect {
            x: body.x + 2,
            y: body.y + body.height.saturating_sub(3),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(button_span("OK", self.main_focus == 1)))
                .alignment(Alignment::Right),
            footer,
        );
    }

    fn draw_edit_connections(&self, frame: &mut Frame, area: Rect) {
        let rect = centered(area, 72, 25);
        draw_frame(frame, rect, "Edit Connections");
        let body = inner(rect);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(72),
                Constraint::Percentage(28),
            ])
            .split(body);
        let rows = self.edit_rows();
        let items = rows
            .iter()
            .map(|row| {
                let style = if row.target.is_none() {
                    section_style()
                } else {
                    panel_style()
                };
                ListItem::new(Line::from(Span::styled(
                    row.label.clone(),
                    style,
                )))
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if rows.iter().any(|row| row.target.is_some()) {
            state.select(Some(
                self.edit_selected.min(rows.len().saturating_sub(1)),
            ));
        }
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default().borders(Borders::ALL).style(panel_style()),
                )
                .highlight_style(selected_style())
                .highlight_symbol(""),
            columns[0],
            &mut state,
        );
        draw_scrollbar(frame, columns[0], rows.len(), state.offset());
        self.draw_buttons(
            frame,
            columns[1],
            &["Add", "Edit", "Delete", "Back"],
            self.edit_focus,
        );
    }

    fn draw_activate_connections(&self, frame: &mut Frame, area: Rect) {
        let rect = centered(area, 76, 26);
        draw_frame(frame, rect, "Activate a Connection");
        let body = inner(rect);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(75),
                Constraint::Percentage(25),
            ])
            .split(body);
        let rows = self.activation_rows();
        let items = rows
            .iter()
            .map(|row| {
                let signal = row.signal.map(signal_bars).unwrap_or_default();
                let mut line = Line::from(row.label.clone());
                if !signal.is_empty() {
                    line.spans.push(Span::styled(
                        format!(" {signal}"),
                        panel_style(),
                    ));
                }
                if row.target.is_none() {
                    line = Line::from(Span::styled(
                        row.label.clone(),
                        section_style(),
                    ));
                }
                ListItem::new(line)
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if rows.iter().any(|row| row.target.is_some()) {
            state.select(Some(
                self.activation_selected.min(rows.len().saturating_sub(1)),
            ));
        }
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default().borders(Borders::ALL).style(panel_style()),
                )
                .highlight_style(selected_style())
                .highlight_symbol(""),
            columns[0],
            &mut state,
        );
        draw_scrollbar(frame, columns[0], rows.len(), state.offset());
        self.draw_buttons(
            frame,
            columns[1],
            &[self.activation_button_label(), "Back"],
            self.activation_focus,
        );
    }

    fn draw_hostname(&self, frame: &mut Frame, area: Rect) {
        let rect = centered(area, 60, 11);
        draw_frame(frame, rect, "Set System Hostname");
        let body = inner(rect);
        let input = Rect {
            x: body.x + 2,
            y: body.y + 2,
            width: body.width.saturating_sub(4),
            height: 1,
        };
        let style = if self.hostname_focus == 0 {
            selected_style()
        } else {
            field_style()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Hostname                 ", label_style()),
                Span::styled(format!(" {:<42}", self.hostname), style),
            ])),
            input,
        );
        let buttons = Rect {
            x: body.x + 2,
            y: body.y + body.height.saturating_sub(2),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                button_span("Cancel", self.hostname_focus == 1),
                Span::raw(" "),
                button_span("OK", self.hostname_focus == 2),
            ]))
            .alignment(Alignment::Right),
            buttons,
        );
    }

    fn draw_buttons(
        &self,
        frame: &mut Frame,
        area: Rect,
        labels: &[&str],
        focus: usize,
    ) {
        for (index, label) in labels.iter().enumerate() {
            let rect = Rect {
                x: area.x,
                y: area.y + index as u16 * 2 + 1,
                width: area.width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(button_span(
                    label,
                    focus == index + 1,
                ))),
                rect,
            );
        }
    }

    fn draw_message(
        &self,
        frame: &mut Frame,
        area: Rect,
        popup: &MessagePopup,
    ) {
        let rect = centered(area, 62, 9);
        draw_frame(frame, rect, &popup.title);
        let body = inner(rect);
        let message = Rect {
            x: body.x + 2,
            y: body.y + 2,
            width: body.width.saturating_sub(4),
            height: body.height.saturating_sub(4),
        };
        frame.render_widget(
            Paragraph::new(popup.message.clone())
                .style(panel_style())
                .wrap(ratatui::widgets::Wrap { trim: false }),
            message,
        );
        let footer = Rect {
            x: body.x + 2,
            y: body.bottom().saturating_sub(2),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(button_span("OK", true)))
                .alignment(Alignment::Right),
            footer,
        );
    }

    fn draw_new_connection(
        &self,
        frame: &mut Frame,
        area: Rect,
        popup: &NewConnectionPopup,
    ) {
        let rect = centered(area, 62, 16);
        draw_frame(frame, rect, "New Connection");
        let body = inner(rect);
        frame.render_widget(
            Paragraph::new("Select the type of connection you wish to create.")
                .style(panel_style()),
            Rect {
                x: body.x + 2,
                y: body.y + 1,
                width: body.width.saturating_sub(4),
                height: 1,
            },
        );
        let list_rect = Rect {
            x: body.x + 20,
            y: body.y + 3,
            width: body.width.saturating_sub(28),
            height: 6,
        };
        let items = NEW_CONNECTION_TYPES
            .iter()
            .map(|item| ListItem::new(*item))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if popup.focus == NewFocus::List {
            state.select(Some(popup.selected));
        }
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(selected_style())
                .highlight_symbol(""),
            list_rect,
            &mut state,
        );
        let arrows = Rect {
            x: list_rect.right().saturating_add(2),
            y: list_rect.y,
            width: 2,
            height: list_rect.height,
        };
        frame.render_widget(
            Paragraph::new("↑\n\n\n\n\n↓").style(disabled_style()),
            arrows,
        );
        let footer = Rect {
            x: body.x + 2,
            y: body.bottom().saturating_sub(2),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                button_span("Cancel", popup.focus == NewFocus::Cancel),
                Span::raw(" "),
                button_span("Create", popup.focus == NewFocus::Create),
            ]))
            .alignment(Alignment::Right),
            footer,
        );
    }

    fn draw_password(
        &self,
        frame: &mut Frame,
        area: Rect,
        popup: &PasswordPopup,
    ) {
        let rect = centered(area, 62, 12);
        draw_frame(frame, rect, "Password Required");
        let body = inner(rect);
        frame.render_widget(
            Paragraph::new(format!("Password for {}", popup.target.ssid))
                .style(panel_style()),
            Rect {
                x: body.x + 2,
                y: body.y + 1,
                width: body.width.saturating_sub(4),
                height: 1,
            },
        );
        let password = if popup.show {
            popup.input.clone()
        } else {
            "*".repeat(popup.input.chars().count())
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Password                 ", label_style()),
                Span::styled(
                    format!(" {password:<38}"),
                    if popup.focus == PasswordFocus::Input {
                        selected_style()
                    } else {
                        field_style()
                    },
                ),
            ])),
            Rect {
                x: body.x + 2,
                y: body.y + 3,
                width: body.width.saturating_sub(4),
                height: 1,
            },
        );
        let checked = if popup.show { "X" } else { " " };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("[{checked}] Show password"),
                if popup.focus == PasswordFocus::Show {
                    selected_style()
                } else {
                    panel_style()
                },
            ))),
            Rect {
                x: body.x + 2,
                y: body.y + 5,
                width: body.width.saturating_sub(4),
                height: 1,
            },
        );
        let footer = Rect {
            x: body.x + 2,
            y: body.bottom().saturating_sub(2),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                button_span("Cancel", popup.focus == PasswordFocus::Cancel),
                Span::raw(" "),
                button_span("Connect", popup.focus == PasswordFocus::Connect),
            ]))
            .alignment(Alignment::Right),
            footer,
        );
    }

    fn draw_connecting(
        &self,
        frame: &mut Frame,
        area: Rect,
        popup: &ConnectingPopup,
    ) {
        let rect = centered(area, 52, 8);
        draw_frame(frame, rect, "Activate a Connection");
        let body = inner(rect);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(format!("Connecting to {}...", popup.target.ssid)),
                Line::from("Please wait for the connection to complete."),
            ])
            .style(panel_style()),
            Rect {
                x: body.x + 2,
                y: body.y + 1,
                width: body.width.saturating_sub(4),
                height: 3,
            },
        );
        let footer = Rect {
            x: body.x + 2,
            y: body.bottom().saturating_sub(2),
            width: body.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(button_span("Cancel", true)))
                .alignment(Alignment::Right),
            footer,
        );
    }

    fn draw_editor(
        &self,
        frame: &mut Frame,
        area: Rect,
        editor: &ConnectionEditor,
    ) {
        let rect = centered(area, 80, 30);
        draw_frame(frame, rect, "Edit Connection");
        let body = inner(rect);
        let footer_height = 4;
        let content = Rect {
            x: body.x + 1,
            y: body.y,
            width: body.width.saturating_sub(2),
            height: body.height.saturating_sub(footer_height),
        };
        let form_lines = editor.form_lines();
        let max_scroll =
            form_lines.len().saturating_sub(content.height as usize) as u16;
        frame.render_widget(
            Paragraph::new(form_lines)
                .style(panel_style())
                .scroll((editor.scroll.min(max_scroll), 0)),
            content,
        );
        let footer = Rect {
            x: body.x + 1,
            y: body.bottom().saturating_sub(footer_height),
            width: body.width.saturating_sub(2),
            height: footer_height,
        };
        for (index, line) in editor.footer_lines().into_iter().enumerate() {
            frame.render_widget(
                Paragraph::new(line).style(panel_style()),
                Rect {
                    x: footer.x,
                    y: footer.y + index as u16,
                    width: footer.width,
                    height: 1,
                },
            );
        }
        if let Some(choice) = &editor.choice {
            self.draw_choice(frame, area, choice);
        } else if let Some(routing) = &editor.routing {
            self.draw_routing(frame, area, routing);
        }
    }

    fn draw_choice(&self, frame: &mut Frame, area: Rect, choice: &ChoicePopup) {
        let width = 34;
        let height = (choice.options.len() as u16 + 4)
            .min(area.height.saturating_sub(2));
        let rect = centered(area, width, height);
        draw_frame(frame, rect, "Select");
        let body = inner(rect);
        let items = choice
            .options
            .iter()
            .map(|option| ListItem::new(option.clone()))
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        state.select(Some(choice.selected));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(selected_style())
                .highlight_symbol(""),
            body,
            &mut state,
        );
    }

    fn draw_routing(
        &self,
        frame: &mut Frame,
        area: Rect,
        popup: &RoutingPopup,
    ) {
        let rect = centered(area, 72, 18);
        draw_frame(frame, rect, popup.title());
        let body = inner(rect);
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(5),
                Constraint::Length(if popup.editing { 3 } else { 0 }),
            ])
            .split(body);
        frame.render_widget(
            Paragraph::new(format!("Enter routes as {}.", popup.placeholder()))
                .style(panel_style()),
            vertical[0],
        );
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(72),
                Constraint::Percentage(28),
            ])
            .split(vertical[1]);
        let items = if popup.routes.is_empty() {
            vec![ListItem::new(Span::styled(
                "  (no custom routes)",
                disabled_style(),
            ))]
        } else {
            popup
                .routes
                .iter()
                .map(|route| ListItem::new(format!("  {route}")))
                .collect()
        };
        let mut state = ListState::default();
        if popup.focus == RoutingFocus::List && !popup.routes.is_empty() {
            state.select(Some(popup.selected));
        }
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default().borders(Borders::ALL).style(panel_style()),
                )
                .highlight_style(selected_style())
                .highlight_symbol(""),
            columns[0],
            &mut state,
        );
        self.draw_buttons(
            frame,
            columns[1],
            &["Add", "Delete", "Cancel", "OK"],
            popup.focus.index(),
        );
        if popup.editing {
            let input = Line::from(vec![
                Span::styled("Route ", label_style()),
                Span::styled(format!(" {:<52}", popup.input), selected_style()),
            ]);
            let hint = popup.error.as_deref().map_or_else(
                || "Enter to add, Esc to cancel".to_string(),
                str::to_string,
            );
            frame.render_widget(
                Paragraph::new(vec![
                    input,
                    Line::from(Span::styled(
                        hint,
                        if popup.error.is_some() {
                            panel_style().fg(Color::Red)
                        } else {
                            disabled_style()
                        },
                    )),
                ])
                .style(panel_style()),
                vertical[2],
            );
        }
    }
}

fn screen_style() -> Style {
    Style::default().fg(Color::White).bg(Color::Blue)
}

fn panel_style() -> Style {
    Style::default().fg(Color::Black).bg(Color::Gray)
}

fn label_style() -> Style {
    panel_style().fg(Color::Blue)
}

fn instruction_style() -> Style {
    panel_style().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

fn section_style() -> Style {
    panel_style().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

fn field_style() -> Style {
    Style::default().fg(Color::White).bg(Color::Blue)
}

fn selected_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD)
}

fn disabled_style() -> Style {
    panel_style().fg(Color::DarkGray)
}

fn button_span(label: &str, selected: bool) -> Span<'static> {
    let value = format!("<{label}>");
    if selected {
        Span::styled(value, selected_style())
    } else {
        Span::styled(value, panel_style().add_modifier(Modifier::BOLD))
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn inner(rect: Rect) -> Rect {
    Rect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    }
}

fn draw_frame(frame: &mut Frame, rect: Rect, title: &str) {
    let shadow = Rect {
        x: rect.x.saturating_add(2),
        y: rect.y.saturating_add(2),
        width: rect.width,
        height: rect.height,
    };
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        shadow,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(panel_style().fg(Color::Black))
            .style(panel_style())
            .title(Line::from(Span::styled(
                format!(" {title} "),
                panel_style().fg(Color::Red).add_modifier(Modifier::BOLD),
            )))
            .title_alignment(Alignment::Center),
        rect,
    );
}

fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    content_length: usize,
    position: usize,
) {
    let viewport = area.height.saturating_sub(2) as usize;
    if content_length <= viewport || viewport == 0 {
        return;
    }
    let mut state = ScrollbarState::new(content_length)
        .position(position)
        .viewport_content_length(viewport);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓"))
        .thumb_symbol("█")
        .track_symbol(Some("│"))
        .style(disabled_style());
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

fn signal_bars(signal: i32) -> &'static str {
    match signal.clamp(-90, -30) {
        -45..=-30 => "▂▄▆█",
        -60..=-46 => "▂▄▆ ",
        -75..=-61 => "▂▄  ",
        _ => "▂   ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(0);

    fn ethernet(name: &str, state: ConnectionState) -> InterfaceInfo {
        let mut info = InterfaceInfo::new(1, name);
        info.interface_type = InterfaceType::Ethernet;
        info.state = state;
        info
    }

    fn render(app: &App) -> (String, bool) {
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let mut highlighted = false;
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| {
                highlighted |= cell.modifier.contains(Modifier::BOLD)
                    && cell.bg == Color::Red;
                cell.symbol()
            })
            .collect::<String>();
        (text, highlighted)
    }

    fn test_client() -> DaemonClient {
        let socket = std::env::temp_dir().join(format!(
            "network-daemon-tui-app-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(
                    b"{\"subsystem\":\"Global\",\"response\":\"Established\"}\n",
                )
                .unwrap();
        });
        let client = DaemonClient::connect(&socket).unwrap();
        let _ = std::fs::remove_file(socket);
        client
    }

    #[test]
    fn main_menu_matches_nmtui_activity_model() {
        let app = App::default();
        let (text, highlighted) = render(&app);
        assert!(text.contains("NetworkManager TUI"));
        assert!(text.contains("Please select an option"));
        assert!(text.contains("Edit a connection"));
        assert!(text.contains("Activate a connection"));
        assert!(text.contains("Set system hostname"));
        assert!(text.contains("Quit"));
        assert!(text.contains("<OK>"));
        assert!(highlighted);
    }

    #[test]
    fn edit_screen_groups_profiles_and_buttons() {
        let app = App {
            screen: Screen::EditConnections,
            interfaces: vec![ethernet("em0", ConnectionState::Connected)],
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("Edit Connections"));
        assert!(text.contains("Ethernet"));
        assert!(text.contains("em0"));
        assert!(text.contains("Wi-Fi"));
        assert!(text.contains("<Add>"));
        assert!(text.contains("<Back>"));
    }

    #[test]
    fn activation_screen_renders_groups_and_signal_bars() {
        let scan = ScanResult {
            freq: 2412,
            signal: -42,
            bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
            ssid: "Home".to_string(),
            security: Security::Psk,
            flags: Default::default(),
        };
        let mut wlan = ethernet("wlan0", ConnectionState::Disconnected);
        wlan.interface_type = InterfaceType::Wlan;
        let app = App {
            screen: Screen::ActivateConnections,
            interfaces: vec![wlan],
            scan_results: vec![scan],
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("Activate a Connection"));
        assert!(text.contains("Ethernet"));
        assert!(text.contains("Wi-Fi"));
        assert!(text.contains("Home"));
        assert!(text.contains("▂▄▆█"));
        assert!(text.contains("Bluetooth"));
        assert!(text.contains("<Activate>"));
    }

    #[test]
    fn activation_screen_uses_deactivate_for_current_wifi() {
        let scan = ScanResult {
            freq: 2412,
            signal: -42,
            bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
            ssid: "Home".to_string(),
            security: Security::Psk,
            flags: Default::default(),
        };
        let mut wlan = ethernet("wlan0", ConnectionState::Connected);
        wlan.interface_type = InterfaceType::Wlan;
        let app = App {
            screen: Screen::ActivateConnections,
            interfaces: vec![wlan],
            scan_results: vec![scan],
            wifi_status: Some(SupplicantStatus {
                state: Some(WpaState::Completed),
                ssid: Some("Home".to_string()),
                ..SupplicantStatus::default()
            }),
            activation_selected: 2,
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("<Deactivate>"));
    }

    #[test]
    fn activation_screen_uses_deactivate_for_active_ethernet() {
        let app = App {
            screen: Screen::ActivateConnections,
            interfaces: vec![ethernet("eth0", ConnectionState::Connected)],
            activation_selected: 1,
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("* Wired connection 1 (eth0)"));
        assert!(text.contains("<Deactivate>"));
        assert!(!text.contains("<Activate>"));
    }

    #[test]
    fn tab_cycles_list_and_button_regions_without_arrow_aliases() {
        let mut main = App::default();
        let mut edit = App {
            screen: Screen::EditConnections,
            interfaces: vec![ethernet("eth0", ConnectionState::Disconnected)],
            ..App::default()
        };
        let mut client = test_client();
        main.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        assert_eq!(main.main_focus, 1);
        main.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        assert_eq!(main.main_focus, 0);

        for expected in [1, 2, 3, 4, 0] {
            edit.handle_key(crossterm::event::KeyCode::Tab, &mut client);
            assert_eq!(edit.edit_focus, expected);
        }
        edit.handle_key(crossterm::event::KeyCode::Right, &mut client);
        assert_eq!(edit.edit_focus, 0);
        edit.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        edit.handle_key(crossterm::event::KeyCode::Up, &mut client);
        assert_eq!(edit.edit_focus, 4);

        let mut activate = App {
            screen: Screen::ActivateConnections,
            interfaces: vec![ethernet("eth0", ConnectionState::Disconnected)],
            activation_selected: 1,
            ..App::default()
        };
        for expected in [1, 2, 0] {
            activate.handle_key(crossterm::event::KeyCode::Tab, &mut client);
            assert_eq!(activate.activation_focus, expected);
        }
        activate.handle_key(crossterm::event::KeyCode::Left, &mut client);
        assert_eq!(activate.activation_focus, 0);

        let editor = ConnectionEditor::from_interface(&ethernet(
            "eth0",
            ConnectionState::Disconnected,
        ));
        let last_field = editor.fields().len() - 1;
        let mut editor_app = App {
            screen: Screen::EditConnections,
            popup: Popup::Editor(Box::new(editor.clone())),
            ..App::default()
        };
        for _ in 0..last_field {
            editor_app.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        }
        let Popup::Editor(editor) = &editor_app.popup else {
            panic!("expected editor popup");
        };
        assert_eq!(editor.focus, last_field);
        assert!(!editor.footer);
        editor_app.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        let Popup::Editor(editor) = &editor_app.popup else {
            panic!("expected editor popup");
        };
        assert!(editor.footer);
        assert_eq!(editor.footer_selected, 0);
        editor_app.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        editor_app.handle_key(crossterm::event::KeyCode::Tab, &mut client);
        let Popup::Editor(editor) = &editor_app.popup else {
            panic!("expected editor popup");
        };
        assert!(!editor.footer);
        assert_eq!(editor.focus, 0);
    }

    #[test]
    fn new_connection_popup_matches_reference_picker() {
        let app = App {
            screen: Screen::EditConnections,
            popup: Popup::NewConnection(NewConnectionPopup {
                selected: 3,
                focus: NewFocus::List,
            }),
            ..App::default()
        };
        let (text, highlighted) = render(&app);
        assert!(text.contains("New Connection"));
        assert!(
            text.contains("Select the type of connection you wish to create.")
        );
        assert!(text.contains("Wi-Fi"));
        assert!(text.contains("Team"));
        assert!(text.contains("<Cancel>"));
        assert!(text.contains("<Create>"));
        assert!(highlighted);
    }

    #[test]
    fn editor_renders_ipv4_ipv6_sections_and_footer() {
        let app = App {
            screen: Screen::EditConnections,
            popup: Popup::Editor(Box::new(ConnectionEditor::from_interface(
                &ethernet("em0", ConnectionState::Connected),
            ))),
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("Edit Connection"));
        assert!(text.contains("IPv4 CONFIGURATION"));
        assert!(text.contains("IPv6 CONFIGURATION"));
        assert!(text.contains("Accept IPv6 router advertisements"));
        assert!(text.contains("Automatically connect"));
        assert!(text.contains("<Cancel>"));
        assert!(text.contains("<OK>"));
    }

    #[test]
    fn editor_scroll_follows_focus_in_both_directions() {
        let mut editor = ConnectionEditor::from_interface(&ethernet(
            "eth0",
            ConnectionState::Disconnected,
        ));
        for _ in 0..16 {
            editor.move_focus(1);
        }
        let down_scroll = editor.scroll;
        assert!(down_scroll > 0);

        editor.move_focus(-1);
        assert!(editor.scroll < down_scroll);

        editor.move_focus(-100);
        assert_eq!(editor.focus, 0);
        assert_eq!(editor.scroll, 0);
    }

    #[test]
    fn routing_is_a_focusable_editor_field_with_an_edit_popup() {
        let mut editor = ConnectionEditor::from_interface(&ethernet(
            "eth0",
            ConnectionState::Disconnected,
        ));
        let fields = editor.fields();
        let routing_index = fields
            .iter()
            .position(|field| *field == EditorField::Ipv4Routing)
            .unwrap();
        editor.focus = routing_index;

        let app = App {
            screen: Screen::EditConnections,
            popup: Popup::Editor(Box::new(editor.clone())),
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("Routing"));
        assert!(text.contains("<Edit...>"));

        let mut app = App {
            screen: Screen::EditConnections,
            popup: Popup::Editor(Box::new(editor)),
            ..App::default()
        };
        let mut client = test_client();
        app.handle_key(crossterm::event::KeyCode::Enter, &mut client);
        let Popup::Editor(editor) = &app.popup else {
            panic!("expected editor popup");
        };
        assert!(matches!(
            editor.routing.as_ref().map(|popup| popup.family),
            Some(RoutingFamily::Ipv4)
        ));
        let (text, _) = render(&app);
        assert!(text.contains("Edit IPv4 Routes"));
        assert!(text.contains("<Add>"));
        assert!(text.contains("<OK>"));
    }

    #[test]
    fn routing_popup_validates_and_applies_routes() {
        let mut editor = ConnectionEditor::from_interface(&ethernet(
            "eth0",
            ConnectionState::Disconnected,
        ));
        editor.open_routing(EditorField::Ipv4Routing);
        let popup = editor.routing.as_mut().unwrap();
        popup.begin_add();
        popup.input = "10.0.0.0/24 192.168.1.1".to_string();
        popup.add_route().unwrap();
        assert_eq!(popup.routes, ["10.0.0.0/24 192.168.1.1"]);
        assert_eq!(popup.selected, 0);

        let popup = editor.routing.take().unwrap();
        editor.apply_routing(popup);
        assert_eq!(editor.ipv4_routes, ["10.0.0.0/24 192.168.1.1"]);
        assert!(editor.routing.is_none());
    }

    #[test]
    fn hostname_activity_renders_current_value_and_actions() {
        let app = App {
            screen: Screen::Hostname,
            hostname: "router".to_string(),
            ..App::default()
        };
        let (text, _) = render(&app);
        assert!(text.contains("Set System Hostname"));
        assert!(text.contains("Hostname"));
        assert!(text.contains("router"));
        assert!(text.contains("<Cancel>"));
        assert!(text.contains("<OK>"));
    }

    #[test]
    fn editor_validates_wifi_credentials() {
        let editor = ConnectionEditor::from_wifi("wlan0", None, None);
        assert!(editor.wifi_values().is_err());
    }

    #[test]
    fn editor_focus_fields_match_visible_wifi_controls() {
        let mut editor = ConnectionEditor::from_wifi("wlan0", None, None);
        let fields = editor.fields();
        assert_eq!(
            fields
                .iter()
                .filter(|field| **field == EditorField::ProfileName)
                .count(),
            1
        );
        assert_eq!(
            fields
                .iter()
                .filter(|field| **field == EditorField::Device)
                .count(),
            1
        );
        assert!(!fields.contains(&EditorField::Identity));
        assert!(!fields.contains(&EditorField::Password));

        editor.security = Security::Psk;
        let fields = editor.fields();
        assert!(!fields.contains(&EditorField::Identity));
        assert!(fields.contains(&EditorField::Password));

        editor.security = Security::Eap;
        let fields = editor.fields();
        assert!(fields.contains(&EditorField::Identity));
        assert!(fields.contains(&EditorField::Password));
    }

    #[test]
    fn signal_bars_have_stable_quality_buckets() {
        assert_eq!(signal_bars(-35), "▂▄▆█");
        assert_eq!(signal_bars(-55), "▂▄▆ ");
        assert_eq!(signal_bars(-70), "▂▄  ");
        assert_eq!(signal_bars(-85), "▂   ");
    }
}
