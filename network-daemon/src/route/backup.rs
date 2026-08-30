use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use libnetwork_daemon::ignore;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_records_gateway_and_nets() {
        let mut table = BackupRouteTable::new();
        table.insert(1, 10, [100, 200].into_iter());
        table.insert(2, 20, [200, 300].into_iter());

        // available_gateways yields (gateway, if_idx).
        let mut gws: Vec<(u32, u32)> = table.available_gateways().collect();
        gws.sort();
        assert_eq!(gws, vec![(10, 1), (20, 2)]);

        // Shared network 200 is reachable via both interfaces.
        let outputs: Vec<&u32> = table.net_outputs(&200).unwrap().collect();
        assert_eq!(outputs.len(), 2);
        assert!(outputs.contains(&&1));
        assert!(outputs.contains(&&2));

        // Private network reachable only via its interface.
        assert_eq!(table.net_outputs(&100).unwrap().count(), 1);
        assert!(table.net_outputs(&999).is_none());
    }

    #[test]
    fn remove_cleans_all_mappings() {
        let mut table = BackupRouteTable::new();
        table.insert(1, 10, [100, 200].into_iter());
        table.insert(2, 20, [200].into_iter());

        table.remove(1);

        assert_eq!(table.available_gateways().count(), 1);
        assert!(table.net_outputs(&100).is_none());
        // Network 200 no longer claims interface 1.
        let outputs: Vec<&u32> = table.net_outputs(&200).unwrap().collect();
        assert_eq!(outputs, vec![&2]);
    }
}
