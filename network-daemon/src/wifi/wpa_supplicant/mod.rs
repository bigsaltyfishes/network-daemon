mod control;
mod handle;

use std::{
    collections::HashMap,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::Duration,
};

use control::{WpaCtrl, WpaEventListener};
use futures_lite::stream;
use kameo::{
    Actor,
    actor::{ActorRef, WeakActorRef},
    error::{ActorStopReason, PanicError},
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    KnownNetwork, KnownNetworkState, MacAddr, ScanResult, Security,
    SupplicantStatus, WiFiManagerAction, WiFiManagerEvent, WiFiManagerResponse,
    WpaCommand, WpaEvent, ensure,
    error::{WifiError, WpaCtrlError},
    ignore,
};
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use crate::wifi::{
    PSK_MAX_LEN, PSK_MIN_LEN, WifiManager, WifiManagerBackend,
    wpa_supplicant::handle::WpaSupplicantInstanceHandle,
};

#[derive(Default, Debug, Clone)]
struct KnownSsidEntry {
    no_bssid: Option<KnownNetwork>,
    by_bssid: HashMap<MacAddr, KnownNetwork>,
}

pub struct WpaSupplicant {
    // Management interface
    iface: String,

    // WPA control interface
    ctrl_cmd: WpaCtrl,

    // wpa_supplicant instance handle
    handle: Option<WpaSupplicantInstanceHandle>,

    // Cached scan results (ordered as reported by wpa_supplicant)
    scan_results: Vec<ScanResult>,

    // Known networks keyed by SSID, then optional BSSID bucket
    known_networks: HashMap<String, KnownSsidEntry>,

    // Channels for notifying subscribers
    supervisor: ActorRef<WifiManager<Self>>,

    // Working directory for sockets
    workdir: PathBuf,

    // Whether initial status broadcast has been sent
    status_init: bool,
}

impl WpaSupplicant {
    /// Create a new WiFi manager with the given control interface
    pub async fn new<P, S>(
        workdir: P,
        iface: S,
        supervisor: ActorRef<WifiManager<Self>>,
    ) -> Result<Self, WifiError>
    where
        P: AsRef<Path> + Send + Sync,
        S: AsRef<str> + Send + Sync,
    {
        let instance = WpaSupplicantInstanceHandle::new(
            "/var/run/wpa_supplicant",
            iface.as_ref(),
        )
        .await?;
        let ctrl_cmd = WpaCtrl::open(
            workdir
                .as_ref()
                .join(format!("wpa_ctrl_{}", iface.as_ref())),
            format!("/var/run/wpa_supplicant/{}", iface.as_ref()),
        )
        .await?;

        Ok(Self {
            iface: iface.as_ref().to_string(),
            ctrl_cmd,
            handle: Some(instance),
            scan_results: Vec::new(),
            known_networks: HashMap::new(),
            supervisor,
            workdir: workdir.as_ref().to_path_buf(),
            status_init: false,
        })
    }

    /// Start a scan
    ///
    /// This initiates a scan but does not wait for results.
    /// Use `scan_and_wait()` to scan and wait for results.
    pub async fn scan(&mut self) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::Scan)
            .await
            .map_err(|_| WifiError::ScanFailed)
    }

    /// Set passive scan mode
    ///
    /// # Arguments
    ///
    /// * `enable` - true for passive scan, false for active scan
    #[allow(dead_code)] // control method; not yet bound to a WiFiManagerAction
    pub async fn set_passive_scan(
        &mut self,
        enable: bool,
    ) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::Set {
                key: "passive_scan".to_string(),
                value: if enable {
                    "1".to_string()
                } else {
                    "0".to_string()
                },
            })
            .await
            .map_err(WifiError::from)
    }

    /// Get scan results
    ///
    /// Returns the most recent scan results. Call `scan()` or
    /// `scan_and_wait()` first to populate results.
    pub async fn get_scan_results(
        &mut self,
    ) -> Result<Vec<ScanResult>, WifiError> {
        let response = self
            .ctrl_cmd
            .request(WpaCommand::ScanResults)
            .await
            .map_err(|_| WifiError::GetScanResultsFailed)?;

        let results: Vec<ScanResult> = response
            .lines()
            .skip(1) // Skip header line
            .filter_map(ScanResult::parse)
            .collect();

        Ok(results)
    }

    /// Get all known/saved networks (from in-memory store)
    pub async fn get_known_networks(
        &self,
    ) -> Result<Vec<KnownNetwork>, WifiError> {
        let mut networks = Vec::new();
        for entry in self.known_networks.values() {
            if let Some(n) = &entry.no_bssid {
                networks.push(n.clone());
            }
            networks.extend(entry.by_bssid.values().cloned());
        }

        Ok(networks)
    }

    /// Get a specific network value
    #[allow(dead_code)] // helper; used by later phases to read network config
    async fn get_network_value(
        &mut self,
        nwid: i32,
        key: &str,
    ) -> Result<String, WifiError> {
        let response = self
            .ctrl_cmd
            .request(&WpaCommand::GetNetwork {
                id: nwid,
                key: key.to_string(),
            })
            .await?;

        if response == "FAIL" {
            return Err(WifiError::NetworkNotFound { nwid });
        }

        Ok(response)
    }

    /// Set a network value
    async fn set_network_value(
        &mut self,
        nwid: i32,
        key: &str,
        value: &str,
    ) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::SetNetwork {
                id: nwid,
                key: key.to_string(),
                value: value.to_string(),
            })
            .await
            .map_err(|_| WifiError::ConfigureNetworkFailed { nwid })
    }

    /// Add a new network
    ///
    /// # Returns
    ///
    /// Network ID of the newly created network
    pub async fn add_network(&mut self) -> Result<i32, WifiError> {
        let response = self
            .ctrl_cmd
            .request(&WpaCommand::AddNetwork)
            .await
            .map_err(|_| WifiError::AddNetworkFailed)?;

        response
            .trim()
            .parse()
            .map_err(|_| WifiError::AddNetworkFailed)
    }

    /// Remove a network
    pub async fn remove_network(&mut self, nwid: i32) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::RemoveNetwork { id: nwid })
            .await
            .map_err(|_| WifiError::RemoveNetworkFailed { nwid })
    }

    /// Enable a network
    pub async fn enable_network(&mut self, nwid: i32) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::EnableNetwork { id: nwid })
            .await
            .map_err(|_| WifiError::ConfigureNetworkFailed { nwid })
    }

    /// Disable a network
    #[allow(dead_code)] // control method; used for network dis/abling
    pub async fn disable_network(
        &mut self,
        nwid: i32,
    ) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::DisableNetwork { id: nwid })
            .await
            .map_err(|_| WifiError::ConfigureNetworkFailed { nwid })
    }

    /// Set auto-connect for a network
    ///
    /// # Arguments
    ///
    /// * `nwid` - Network ID
    /// * `enable` - true to enable auto-connect, false to disable
    #[allow(dead_code)] // control method; not yet bound to a WiFiManagerAction
    pub async fn set_autoconnect(
        &mut self,
        nwid: i32,
        enable: bool,
    ) -> Result<(), WifiError> {
        if enable {
            self.enable_network(nwid).await
        } else {
            self.disable_network(nwid).await
        }
    }

    /// Set network priority
    ///
    /// # Arguments
    ///
    /// * `nwid` - Network ID
    /// * `priority` - Priority value (higher = preferred)
    #[allow(dead_code)] // control method; used by later TUI phase
    pub async fn set_priority(
        &mut self,
        nwid: i32,
        priority: i32,
    ) -> Result<(), WifiError> {
        self.set_network_value(nwid, "priority", &priority.to_string())
            .await
    }

    /// Configure SSID for a network
    pub async fn configure_ssid(
        &mut self,
        nwid: i32,
        ssid: &str,
    ) -> Result<(), WifiError> {
        // SSID must be quoted
        self.set_network_value(nwid, "ssid", &format!("\"{}\"", ssid))
            .await
    }

    /// Configure BSSID for a network
    pub async fn configure_bssid(
        &mut self,
        nwid: i32,
        bssid: &MacAddr,
    ) -> Result<(), WifiError> {
        self.set_network_value(nwid, "bssid", &bssid.to_string())
            .await
    }

    /// Configure PSK (WPA/WPA2-Personal) password
    pub async fn configure_psk(
        &mut self,
        nwid: i32,
        psk: &str,
    ) -> Result<(), WifiError> {
        // Validate password length
        if psk.len() < PSK_MIN_LEN || psk.len() > PSK_MAX_LEN {
            return Err(WifiError::InvalidPasswordLength {
                min: PSK_MIN_LEN,
                max: PSK_MAX_LEN,
            });
        }

        self.set_network_value(nwid, "psk", &format!("\"{}\"", psk))
            .await
    }

    /// Configure EAP (enterprise) authentication
    pub async fn configure_eap(
        &mut self,
        nwid: i32,
        identity: &str,
        password: &str,
    ) -> Result<(), WifiError> {
        self.set_network_value(nwid, "key_mgmt", "WPA-EAP").await?;
        self.set_network_value(nwid, "eap", "PEAP").await?;
        self.set_network_value(nwid, "identity", &format!("\"{}\"", identity))
            .await?;
        self.set_network_value(nwid, "password", &format!("\"{}\"", password))
            .await?;
        self.set_network_value(nwid, "phase2", "\"auth=MSCHAPV2\"")
            .await
    }

    /// Configure open network (no encryption)
    pub async fn configure_open(&mut self, nwid: i32) -> Result<(), WifiError> {
        self.set_network_value(nwid, "key_mgmt", "NONE").await
    }

    /// Configure network as hidden (requires active probing)
    pub async fn configure_hidden(
        &mut self,
        nwid: i32,
    ) -> Result<(), WifiError> {
        self.set_network_value(nwid, "scan_ssid", "1").await
    }

    /// Add and configure a new network
    ///
    /// # Arguments
    ///
    /// * `ssid` - Network SSID
    /// * `security` - Security type
    /// * `password` - Password (required for PSK)
    /// * `identity` - Identity (required for EAP)
    /// * `hidden` - Whether the network is hidden
    ///
    /// # Returns
    ///
    /// Network ID of the configured network
    pub async fn add_and_configure_network(
        &mut self,
        ssid: &str,
        bssid: Option<MacAddr>,
        security: Security,
        password: Option<&str>,
        identity: Option<&str>,
        hidden: bool,
    ) -> Result<i32, WifiError> {
        let nwid = self.add_network().await?;

        // Configure SSID
        if let Err(e) = self.configure_ssid(nwid, ssid).await {
            let _ = self.remove_network(nwid).await;
            return Err(e);
        }

        if let Some(bssid) = bssid
            && let Err(e) = self.configure_bssid(nwid, &bssid).await
        {
            let _ = self.remove_network(nwid).await;
            return Err(e);
        }

        // Configure security
        let result = match security {
            Security::Open => self.configure_open(nwid).await,
            Security::Psk => {
                let psk = password.ok_or(WifiError::InvalidPasswordLength {
                    min: PSK_MIN_LEN,
                    max: PSK_MAX_LEN,
                })?;
                self.configure_psk(nwid, psk).await
            }
            Security::Eap => {
                let id = identity.ok_or(WifiError::NotSupported(
                    "EAP requires identity".to_string(),
                ))?;
                let pwd = password.unwrap_or("");
                self.configure_eap(nwid, id, pwd).await
            }
            Security::Unknown => Err(WifiError::NotSupported(
                "unknown security type".to_string(),
            )),
        };

        if let Err(e) = result {
            let _ = self.remove_network(nwid).await;
            return Err(e);
        }

        // Configure hidden if needed
        if hidden && let Err(e) = self.configure_hidden(nwid).await {
            let _ = self.remove_network(nwid).await;
            return Err(e);
        }

        // Enable the network. Newly configured networks are always enabled.
        self.enable_network(nwid).await?;

        Ok(nwid)
    }

    /// Unregister a known network from wpa_supplicant
    async fn unregister_wpa_network(&mut self, known: &KnownNetwork) {
        if let Some(nwid) = known.id
            && let Err(e) = self.remove_network(nwid).await
        {
            warn!(
                "failed to remove stale wpa_supplicant network {}: {}",
                nwid, e
            );
        }
    }

    /// Register a known network
    async fn register_known_network(
        &mut self,
        ssid: String,
        bssid: Option<MacAddr>,
        security: Security,
        password: Option<String>,
        identity: Option<String>,
        hidden: bool,
    ) -> Result<(), WifiError> {
        // Validate credentials early to avoid storing unusable entries.
        match security {
            Security::Psk => {
                let pwd = password.as_ref().ok_or(
                    WifiError::InvalidPasswordLength {
                        min: PSK_MIN_LEN,
                        max: PSK_MAX_LEN,
                    },
                )?;
                if pwd.len() < PSK_MIN_LEN || pwd.len() > PSK_MAX_LEN {
                    return Err(WifiError::InvalidPasswordLength {
                        min: PSK_MIN_LEN,
                        max: PSK_MAX_LEN,
                    });
                }
            }
            Security::Eap if identity.is_none() => {
                return Err(WifiError::NotSupported(
                    "EAP requires identity during registration".to_string(),
                ));
            }
            Security::Eap => {}
            _ => {}
        }

        let mut known = KnownNetwork {
            id: None,
            priority: 0,
            hidden,
            security,
            state: KnownNetworkState::Enabled,
            ssid: ssid.clone(),
            bssid,
            password: password.clone(),
            identity: identity.clone(),
        };

        let prev = {
            let entry = self.known_networks.entry(ssid.clone()).or_default();
            match bssid {
                Some(mac) => entry.by_bssid.remove(&mac),
                None => entry.no_bssid.take(),
            }
        };

        if let Some(prev) = prev {
            self.unregister_wpa_network(&prev).await;
        }

        let entry = self.known_networks.entry(ssid).or_default();
        match bssid {
            Some(mac) => {
                known.bssid = Some(mac);
                entry.by_bssid.insert(mac, known);
            }
            None => {
                entry.no_bssid = Some(known);
            }
        }

        Ok(())
    }

    async fn remove_known_network(
        &mut self,
        ssid: &str,
        bssid: Option<MacAddr>,
    ) -> Result<(), WifiError> {
        let removed = {
            let entry = self.known_networks.get_mut(ssid).ok_or_else(|| {
                WifiError::KnownNetworkNotFound {
                    ssid: ssid.to_string(),
                    bssid,
                }
            })?;

            match bssid {
                Some(mac) => entry.by_bssid.remove(&mac),
                None => entry.no_bssid.take(),
            }
        }
        .ok_or_else(|| WifiError::KnownNetworkNotFound {
            ssid: ssid.to_string(),
            bssid,
        })?;

        if let Some(nwid) = removed.id {
            self.remove_network(nwid).await?;
        }

        let should_prune = self
            .known_networks
            .get(ssid)
            .map(|entry| entry.no_bssid.is_none() && entry.by_bssid.is_empty())
            .unwrap_or(false);

        if should_prune {
            self.known_networks.remove(ssid);
        }

        Ok(())
    }

    fn select_known_candidate(
        &self,
        ssid: &str,
        requested_bssid: Option<MacAddr>,
    ) -> Result<(MacAddr, KnownNetwork), WifiError> {
        let entry = self.known_networks.get(ssid).ok_or_else(|| {
            WifiError::KnownNetworkNotFound {
                ssid: ssid.to_string(),
                bssid: requested_bssid,
            }
        })?;

        match requested_bssid {
            Some(mac) => entry
                .by_bssid
                .get(&mac)
                .cloned()
                .map(|n| (mac, n))
                .ok_or_else(|| WifiError::KnownNetworkNotFound {
                    ssid: ssid.to_string(),
                    bssid: Some(mac),
                }),
            None => {
                let scans: Vec<(usize, &ScanResult)> = self
                    .scan_results
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.ssid == ssid)
                    .collect();

                if scans.is_empty() {
                    return Err(WifiError::SsidUnavailable {
                        ssid: ssid.to_string(),
                    });
                }

                if entry.by_bssid.is_empty() {
                    if scans.len() == 1 {
                        let (_, scan) = scans[0];
                        // Use the sole BSSID even though it was not
                        // pre-registered.
                        let mut known =
                            entry.no_bssid.clone().ok_or_else(|| {
                                WifiError::KnownNetworkNotFound {
                                    ssid: ssid.to_string(),
                                    bssid: None,
                                }
                            })?;
                        let bssid = scan.bssid;
                        known.bssid = Some(bssid);
                        return Ok((bssid, known));
                    }

                    return Err(WifiError::BssidRequired {
                        ssid: ssid.to_string(),
                    });
                }

                // Pick strongest known BSSID; tie-break by scan order.
                let mut best: Option<(i32, usize, MacAddr, KnownNetwork)> =
                    None;
                for (idx, scan) in scans {
                    if let Some(known) = entry.by_bssid.get(&scan.bssid) {
                        match &best {
                            Some((best_signal, best_idx, _, _))
                                if *best_signal > scan.signal
                                    || (*best_signal == scan.signal
                                        && *best_idx <= idx) => {}
                            _ => {
                                best = Some((
                                    scan.signal,
                                    idx,
                                    scan.bssid,
                                    known.clone(),
                                ))
                            }
                        }
                    }
                }

                if let Some((_, _, bssid, known)) = best {
                    Ok((bssid, known))
                } else {
                    Err(WifiError::SsidUnavailable {
                        ssid: ssid.to_string(),
                    })
                }
            }
        }
    }

    /// Select and connect to a network
    ///
    /// # Arguments
    ///
    /// * `nwid` - Network ID, or -1 for any available network
    /// * `freq` - Optional frequency hint
    pub async fn select_network(
        &mut self,
        nwid: i32,
        freq: Option<i32>,
    ) -> Result<(), WifiError> {
        // Optionally set frequency hint
        if let Some(f) = freq {
            let _ = self
                .set_network_value(nwid, "freq_list", &f.to_string())
                .await;
        }

        self.ctrl_cmd
            .request_ok(WpaCommand::SelectNetwork {
                id: if nwid >= 0 { Some(nwid) } else { None },
            })
            .await
            .map_err(|_| WifiError::ConnectFailed)
    }

    async fn connect_known(
        &mut self,
        ssid: String,
        bssid: Option<MacAddr>,
    ) -> Result<(), WifiError> {
        let (target_bssid, mut known) =
            self.select_known_candidate(&ssid, bssid)?;

        // Remove existing entry in supplicant if present before reconnecting
        self.unregister_wpa_network(&known).await;

        let nwid = self
            .add_and_configure_network(
                &known.ssid,
                Some(target_bssid),
                known.security,
                known.password.as_deref(),
                known.identity.as_deref(),
                known.hidden,
            )
            .await?;

        self.select_network(nwid, None).await?;

        known.id = Some(nwid);
        known.state = KnownNetworkState::Current;

        let entry = self.known_networks.entry(ssid).or_default();
        entry.by_bssid.insert(target_bssid, known);

        Ok(())
    }

    /// Reconnect to the current network
    pub async fn reconnect(&mut self) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::Reconnect)
            .await
            .map_err(|_| WifiError::ConnectFailed)
    }

    /// Disconnect from current network
    pub async fn disconnect(&mut self) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::Disconnect)
            .await
            .map_err(|_| WifiError::DisconnectFailed)
    }

    /// Get current supplicant status
    pub async fn get_status(&mut self) -> Result<SupplicantStatus, WifiError> {
        let response = self.ctrl_cmd.request(WpaCommand::Status).await?;
        let mut status = SupplicantStatus::parse(&response);

        // If freq is missing or 0, try to get it from BSS info
        if status.freq.unwrap_or(0) == 0
            && let Some(bssid) = &status.bssid
            && let Ok(Some(freq)) = self.get_bss_freq(bssid).await
        {
            status.freq = Some(freq);
        }

        Ok(status)
    }

    /// Get BSS frequency for a BSSID
    pub async fn get_bss_freq(
        &mut self,
        bssid: &str,
    ) -> Result<Option<i32>, WifiError> {
        let response = self
            .ctrl_cmd
            .request(WpaCommand::Bss {
                addr: bssid.to_string(),
            })
            .await?;

        for line in response.lines() {
            if let Some(freq_str) = line.strip_prefix("freq=") {
                return Ok(freq_str.parse().ok());
            }
        }

        Ok(None)
    }

    /// Save configuration to file
    #[allow(dead_code)] // control method; needed for config persistence
    pub async fn save_config(&mut self) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(&WpaCommand::SaveConfig)
            .await
            .map_err(|_| WifiError::SaveConfigFailed)
    }

    /// Reload configuration from file
    #[allow(dead_code)] // control method; reloads supplicant config
    pub async fn reconfigure(&mut self) -> Result<(), WifiError> {
        self.ctrl_cmd
            .request_ok(WpaCommand::Reconfigure)
            .await
            .map_err(WifiError::from)
    }
}

impl Message<StreamMessage<Result<WpaEvent, WpaCtrlError>, (), ()>>
    for WpaSupplicant
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<Result<WpaEvent, WpaCtrlError>, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let actor_ref = ctx.actor_ref();
        match msg {
            StreamMessage::Next(ev) => {
                match ev {
                    // Broadcast event to subscribers
                    Ok(event) => {
                        ignore!(
                            actor_ref
                                .tell(WiFiManagerAction::WpaEvent { event })
                                .await
                        );
                    }
                    Err(e) => {
                        error!("WPA event listener error: {:?}", e);
                        actor_ref.kill();
                    }
                }
            }
            StreamMessage::Finished(()) => {
                error!("WPA event listener stream finished");
                actor_ref.kill();
            }
            _ => {}
        }
    }
}

impl Message<StreamMessage<WiFiManagerAction, (), ()>> for WpaSupplicant {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<WiFiManagerAction, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let actor_ref = ctx.actor_ref();
        if let StreamMessage::Next(action) = msg
            && let Err(e) = actor_ref.tell(action).await
        {
            error!("Failed to forward action to self: {}", e);
        }
    }
}

impl Message<WiFiManagerAction> for WpaSupplicant {
    type Reply = Result<WiFiManagerResponse, WifiError>;

    async fn handle(
        &mut self,
        msg: WiFiManagerAction,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        Ok(match msg {
            WiFiManagerAction::Scan => {
                self.scan().await?;
                WiFiManagerResponse::Success(())
            }
            WiFiManagerAction::ScanResults => {
                WiFiManagerResponse::ScanResults(self.scan_results.clone())
            }
            WiFiManagerAction::KnownNetworks => {
                self.get_known_networks().await.map(|v| {
                    let v = v
                        .into_iter()
                        .map(|mut n| {
                            n.password = None;
                            n
                        })
                        .collect();
                    WiFiManagerResponse::KnownNetworks(v)
                })?
            }
            WiFiManagerAction::Status => {
                let resp =
                    self.get_status().await.map(WiFiManagerResponse::Status)?;

                if !self.status_init {
                    info!(
                        "Broadcasting initial WiFi status for iface {}",
                        self.iface
                    );
                    self.status_init = true;
                    // Broadcast initial status
                    if let WiFiManagerResponse::Status(status) = &resp {
                        let _ = self
                            .supervisor
                            .tell(WiFiManagerEvent::StatusUpdated {
                                iface: self.iface.clone(),
                                status: Box::new(status.clone()),
                            })
                            .send()
                            .await;
                    } else {
                        warn!(
                            "BUG: Failed to get initial status for broadcast"
                        );
                    }
                }

                resp
            }
            WiFiManagerAction::AddNetwork {
                ssid,
                bssid,
                security,
                password,
                identity,
                hidden,
            } => self
                .register_known_network(
                    ssid, bssid, security, password, identity, hidden,
                )
                .await
                .map(WiFiManagerResponse::Success)?,
            WiFiManagerAction::RemoveNetwork { ssid, bssid } => self
                .remove_known_network(&ssid, bssid)
                .await
                .map(WiFiManagerResponse::Success)?,
            WiFiManagerAction::Connect { ssid, bssid } => self
                .connect_known(ssid, bssid)
                .await
                .map(WiFiManagerResponse::Success)?,
            WiFiManagerAction::Disconnect => {
                self.disconnect().await.map(WiFiManagerResponse::Success)?
            }
            WiFiManagerAction::Reconnect => {
                self.reconnect().await.map(WiFiManagerResponse::Success)?
            }
            WiFiManagerAction::WpaEvent { event } => {
                match event {
                    WpaEvent::ScanResults => {
                        self.scan_results =
                            self.get_scan_results().await.unwrap_or_default();
                        let _ = self
                            .supervisor
                            .tell(WiFiManagerEvent::ScanResultsAvailable)
                            .send()
                            .await;
                    }
                    WpaEvent::StateChange { .. }
                    | WpaEvent::Associated { .. }
                    | WpaEvent::Disconnected { .. }
                    | WpaEvent::Connected { .. } => {
                        if let Ok(status) = self.get_status().await {
                            let _ = self
                                .supervisor
                                .tell(WiFiManagerEvent::StatusUpdated {
                                    iface: self.iface.clone(),
                                    status: Box::new(status),
                                })
                                .send()
                                .await;
                        }
                    }
                    _ => {}
                }
                WiFiManagerResponse::Success(())
            }
            WiFiManagerAction::SubscribeEvents => {
                let rx = self
                    .supervisor
                    .ask(WiFiManagerAction::SubscribeEvents)
                    .await;
                match rx {
                    Ok(WiFiManagerResponse::EventReceiver(rx)) => {
                        WiFiManagerResponse::EventReceiver(rx)
                    }
                    Ok(r) => {
                        return Err(WifiError::ParseError(format!(
                            "Unexpected response from supervisor: {:?}",
                            r
                        )));
                    }
                    Err(e) => {
                        return Err(WifiError::ParseError(format!(
                            "Failed to ask supervisor: {:?}",
                            e
                        )));
                    }
                }
            }
        })
    }
}

impl Message<StreamMessage<std::process::ExitStatus, (), ()>>
    for WpaSupplicant
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<std::process::ExitStatus, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let actor_ref = ctx.actor_ref();
        if let StreamMessage::Next(status) = msg {
            error!(
                "WPA Supplicant for iface {} exited with status: {}",
                self.iface, status
            );
            actor_ref.kill();
        }
    }
}

impl Actor for WpaSupplicant {
    type Args = Self;

    type Error = WifiError;

    async fn on_start(
        mut args: Self::Args,
        actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        if let Err(e) =
            actor_ref.register(format!("WiFiBackend-{}", args.iface))
        {
            error!("Failed to register WiFiBackend actor: {}", e);
        }

        // Attach handle exit stream
        let handle = ensure!(args.handle.take());
        let exit_stream = Box::pin(handle);
        actor_ref.attach_stream(exit_stream, (), ());

        // WPA event listener stream
        let event_listener = Box::pin(
            WpaEventListener::open(
                args.workdir.join(format!("wpa_evt_{}", args.iface)),
                format!("/var/run/wpa_supplicant/{}", args.iface),
            )
            .await?,
        );

        // Periodic scan stream
        let scan_stream = Box::pin(
            stream::repeat(WiFiManagerAction::Scan)
                .throttle(Duration::from_secs(15)),
        );
        actor_ref.attach_stream(event_listener, (), ());
        actor_ref.attach_stream(scan_stream, (), ());

        // Obtain initial status
        actor_ref
            .tell(WiFiManagerAction::Status)
            .await
            .unwrap_or_else(|e| {
                error!("Failed to get initial WiFi status: {}", e)
            });

        Ok(args)
    }

    async fn on_panic(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        err: PanicError,
    ) -> Result<ControlFlow<ActorStopReason>, Self::Error> {
        error!("WiFiManager panicked: {:?}", err);
        Ok(ControlFlow::Break(ActorStopReason::Panicked(err)))
    }
}

impl WifiManagerBackend for WpaSupplicant {
    async fn new<P, S>(
        workdir: P,
        iface: S,
        supervisor: ActorRef<WifiManager<Self>>,
    ) -> Result<Self, WifiError>
    where
        P: AsRef<Path> + Send + Sync,
        S: AsRef<str> + Send + Sync,
    {
        Self::new(workdir, iface, supervisor).await
    }
}
