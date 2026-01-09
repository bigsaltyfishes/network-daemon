use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use libnetwork_daemon::{InterfaceInfo, PrefixedIpv4Addr, ignore};

pub struct BackupRouteTable<I>
where
    I: Eq + Hash + Clone,
{
    gateways: HashMap<u32, I>,
    id_to_net: HashMap<u32, HashSet<I>>,
    net_to_id: HashMap<I, HashSet<u32>>,
}

impl<I> BackupRouteTable<I>
where
    I: Eq + Hash + Clone,
{
    pub fn new() -> Self {
        BackupRouteTable {
            gateways: HashMap::new(),
            id_to_net: HashMap::new(),
            net_to_id: HashMap::new(),
        }
    }
}

impl<I> BackupRouteTable<I>
where
    I: Eq + Hash + Clone,
{
    pub fn insert(
        &mut self,
        if_idx: u32,
        gateway: I,
        nets: impl Iterator<Item = I>,
    ) {
        self.gateways.insert(if_idx, gateway.clone());

        nets.into_iter().for_each(|net| {
            self.net_to_id
                .entry(net.clone())
                .or_default()
                .insert(if_idx);

            self.id_to_net.entry(if_idx).or_default().insert(net);
        });
    }

    pub fn remove(&mut self, if_idx: u32) {
        ignore!(self.gateways.remove(&if_idx));
        if let Some(nets) = self.id_to_net.remove(&if_idx) {
            nets.into_iter().for_each(|net| {
                if let Some(id_set) = self.net_to_id.get_mut(&net) {
                    id_set.remove(&if_idx);
                    if id_set.is_empty() {
                        self.net_to_id.remove(&net);
                    }
                }
            });
        }
    }

    pub fn available_gateways(&self) -> impl Iterator<Item = (I, u32)> + '_ {
        self.gateways.iter().map(|(k, v)| (v.clone(), *k))
    }

    pub fn net_outputs(
        &self,
        net: &I,
    ) -> Option<impl Iterator<Item = &u32> + '_> {
        self.net_to_id.get(net).map(|set| set.iter())
    }
}
