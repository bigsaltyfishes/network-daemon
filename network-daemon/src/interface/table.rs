use std::collections::{HashMap, HashSet};

use libnetwork_daemon::{ConnectionState, InterfaceInfo, InterfaceType};

/// Unified interface table managing all interface data and indices
///
/// Provides bidirectional mapping between interface index (u32) and name
/// (String), as well as secondary indices for type, parent, and state queries.
#[derive(Debug, Default)]
pub struct InterfaceTable {
    /// Primary storage: name -> InterfaceInfo
    interfaces: HashMap<String, InterfaceInfo>,

    /// Bidirectional index mapping
    index_to_name: HashMap<u32, String>,
    name_to_index: HashMap<String, u32>,

    /// Secondary indices
    by_type: HashMap<InterfaceType, HashSet<String>>,
    by_parent: HashMap<String, HashSet<String>>,
    by_state: HashMap<ConnectionState, HashSet<String>>,
}

impl InterfaceTable {
    /// Create a new empty InterfaceTable
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new interface, updating all indices
    pub fn insert(&mut self, info: InterfaceInfo) {
        let name = info.name.clone();
        let index = info.id;

        // Update primary storage
        self.interfaces.insert(name.clone(), info.clone());

        // Update bidirectional index mapping
        // Clean up old mappings if index or name already exists
        if let Some(old_name) = self.index_to_name.get(&index)
            && old_name != &name
        {
            self.name_to_index.remove(old_name);
        }
        if let Some(old_index) = self.name_to_index.get(&name)
            && *old_index != index
        {
            self.index_to_name.remove(old_index);
        }
        self.index_to_name.insert(index, name.clone());
        self.name_to_index.insert(name.clone(), index);

        // Update secondary indices
        self.by_type
            .entry(info.interface_type)
            .or_default()
            .insert(name.clone());

        if let Some(parent) = &info.parent {
            self.by_parent
                .entry(parent.clone())
                .or_default()
                .insert(name.clone());
        }

        self.by_state.entry(info.state).or_default().insert(name);
    }

    /// Remove an interface by name, cleaning up all indices
    ///
    /// Returns the removed InterfaceInfo if it existed
    pub fn remove(&mut self, name: &str) -> Option<InterfaceInfo> {
        let info = self.interfaces.remove(name)?;

        // Clean up bidirectional index mapping
        self.index_to_name.remove(&info.id);
        self.name_to_index.remove(name);

        // Clean up secondary indices
        if let Some(set) = self.by_type.get_mut(&info.interface_type) {
            set.remove(name);
        }

        if let Some(parent) = &info.parent
            && let Some(set) = self.by_parent.get_mut(parent)
        {
            set.remove(name);
        }

        if let Some(set) = self.by_state.get_mut(&info.state) {
            set.remove(name);
        }

        Some(info)
    }

    /// Update an interface by removing the old entry and inserting the new one
    ///
    /// Returns the old InterfaceInfo if it existed
    pub fn update(&mut self, info: InterfaceInfo) -> Option<InterfaceInfo> {
        let old = self.remove(&info.name);
        self.insert(info);
        old
    }

    /// Get an interface by name
    pub fn get(&self, name: &str) -> Option<&InterfaceInfo> {
        self.interfaces.get(name)
    }

    /// Get an interface by ifindex
    pub fn get_by_index(&self, index: u32) -> Option<&InterfaceInfo> {
        self.index_to_name
            .get(&index)
            .and_then(|name| self.interfaces.get(name))
    }

    /// Get the ifindex for a given interface name
    #[allow(dead_code)] // used by TUI/client lookups
    pub fn get_index(&self, name: &str) -> Option<u32> {
        self.name_to_index.get(name).copied()
    }

    /// Check if an interface with the given name exists
    #[allow(dead_code)] // used by TUI/client
    pub fn contains(&self, name: &str) -> bool {
        self.interfaces.contains_key(name)
    }

    /// Check if an interface with the given ifindex exists
    #[allow(dead_code)] // used by TUI/client
    pub fn contains_index(&self, index: u32) -> bool {
        self.index_to_name.contains_key(&index)
    }

    /// List all interfaces
    pub fn all(&self) -> Vec<InterfaceInfo> {
        self.interfaces.values().cloned().collect()
    }

    /// List interfaces by type
    pub fn list_by_type(
        &self,
        interface_type: &InterfaceType,
    ) -> Vec<InterfaceInfo> {
        self.by_type
            .get(interface_type)
            .map(|set| {
                set.iter()
                    .filter_map(|name| self.interfaces.get(name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List interfaces by parent device
    pub fn list_by_parent(&self, parent: &str) -> Vec<InterfaceInfo> {
        self.by_parent
            .get(parent)
            .map(|set| {
                set.iter()
                    .filter_map(|name| self.interfaces.get(name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List interfaces by connection state
    pub fn list_by_state(&self, state: &ConnectionState) -> Vec<InterfaceInfo> {
        self.by_state
            .get(state)
            .map(|set| {
                set.iter()
                    .filter_map(|name| self.interfaces.get(name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clear all data and indices
    pub fn clear(&mut self) {
        self.interfaces.clear();
        self.index_to_name.clear();
        self.name_to_index.clear();
        self.by_type.clear();
        self.by_parent.clear();
        self.by_state.clear();
    }

    /// Get the number of interfaces
    #[allow(dead_code)] // used by TUI/client
    pub fn len(&self) -> usize {
        self.interfaces.len()
    }

    /// Check if the table is empty
    #[allow(dead_code)] // used by TUI/client
    pub fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_interface(
        name: &str,
        id: u32,
        iface_type: InterfaceType,
    ) -> InterfaceInfo {
        InterfaceInfo {
            id,
            name: name.to_string(),
            interface_type: iface_type,
            state: ConnectionState::Disconnected,
            parent: None,
            mac_addr: None,
            ipv4_addrs: vec![],
            ipv6_addrs: vec![],
            gateway_ipv4: None,
            gateway_ipv6: None,
            dhcpv4_enabled: false,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let mut table = InterfaceTable::new();
        let info = make_test_interface("eth0", 1, InterfaceType::Ethernet);

        table.insert(info.clone());

        assert_eq!(table.get("eth0").unwrap().id, 1);
        assert_eq!(table.get_by_index(1).unwrap().name, "eth0");
        assert_eq!(table.get_index("eth0"), Some(1));
    }

    #[test]
    fn test_remove() {
        let mut table = InterfaceTable::new();
        let info = make_test_interface("eth0", 1, InterfaceType::Ethernet);

        table.insert(info);
        assert!(table.contains("eth0"));

        let removed = table.remove("eth0");
        assert!(removed.is_some());
        assert!(!table.contains("eth0"));
        assert!(!table.contains_index(1));
    }

    #[test]
    fn test_update() {
        let mut table = InterfaceTable::new();
        let mut info = make_test_interface("eth0", 1, InterfaceType::Ethernet);

        table.insert(info.clone());
        assert_eq!(
            table.list_by_state(&ConnectionState::Disconnected).len(),
            1
        );

        info.state = ConnectionState::Connected;
        table.update(info);

        assert_eq!(
            table.list_by_state(&ConnectionState::Disconnected).len(),
            0
        );
        assert_eq!(table.list_by_state(&ConnectionState::Connected).len(), 1);
    }

    #[test]
    fn test_list_by_type() {
        let mut table = InterfaceTable::new();
        table.insert(make_test_interface("eth0", 1, InterfaceType::Ethernet));
        table.insert(make_test_interface("wlan0", 2, InterfaceType::Wlan));
        table.insert(make_test_interface("eth1", 3, InterfaceType::Ethernet));

        let ethernet = table.list_by_type(&InterfaceType::Ethernet);
        assert_eq!(ethernet.len(), 2);

        let wireless = table.list_by_type(&InterfaceType::Wlan);
        assert_eq!(wireless.len(), 1);
    }
}
