use std::{
    collections::{BTreeSet, HashMap, HashSet},
    net::IpAddr,
};

use libnetwork_daemon::{PrefixedIpAddr, PrefixedIpv4Addr, PrefixedIpv6Addr};
use route_manager::Route;

#[derive(Debug)]
pub struct RouteTable {
    v4: HashMap<PrefixedIpv4Addr, BTreeSet<Route>>,
    v6: HashMap<PrefixedIpv6Addr, BTreeSet<Route>>,

    by_oif: HashMap<u32, HashSet<PrefixedIpAddr>>,
}

impl RouteTable {
    pub fn new() -> Self {
        RouteTable {
            v4: HashMap::new(),
            v6: HashMap::new(),

            by_oif: HashMap::new(),
        }
    }

    pub fn insert(&mut self, route: Route) {
        match &route.destination() {
            IpAddr::V4(addr) => {
                let oif = route.if_index().unwrap();
                let addr = PrefixedIpv4Addr::new(*addr, route.prefix());
                self.v4.entry(addr.clone()).or_default().insert(route);
                self.by_oif
                    .entry(oif)
                    .or_default()
                    .insert(PrefixedIpAddr::V4(addr));
            }
            IpAddr::V6(addr) => {
                let oif = route.if_index().unwrap();
                let addr = PrefixedIpv6Addr::new(*addr, route.prefix());
                self.v6.entry(addr.clone()).or_default().insert(route);
                self.by_oif
                    .entry(oif)
                    .or_default()
                    .insert(PrefixedIpAddr::V6(addr));
            }
        }
    }

    pub fn remove(&mut self, addr: &PrefixedIpAddr) -> Option<Route> {
        match addr {
            PrefixedIpAddr::V4(inner) => {
                if let Some(rt_set) = self.v4.get_mut(inner)
                    && let Some(rt) = rt_set.iter().next().cloned()
                {
                    let oif = rt.if_index().unwrap();
                    rt_set.remove(&rt);
                    if let Some(set) = self.by_oif.get_mut(&oif) {
                        set.remove(addr);
                        if set.is_empty() {
                            self.by_oif.remove(&oif);
                        }
                    }

                    if self.v4.get(inner).unwrap().is_empty() {
                        self.v4.remove(inner);
                    }

                    return Some(rt);
                }
            }
            PrefixedIpAddr::V6(inner) => {
                if let Some(rt_set) = self.v6.get_mut(inner)
                    && let Some(rt) = rt_set.iter().next().cloned()
                {
                    let oif = rt.if_index().unwrap();
                    rt_set.remove(&rt);
                    if let Some(set) = self.by_oif.get_mut(&oif) {
                        set.remove(addr);
                        if set.is_empty() {
                            self.by_oif.remove(&oif);
                        }
                    }

                    if self.v6.get(inner).unwrap().is_empty() {
                        self.v6.remove(inner);
                    }

                    return Some(rt);
                }
            }
        }

        None
    }

    #[allow(dead_code)] // used by RouteManager dead-route cleanup
    pub fn remove_by_oif(
        &mut self,
        oif: u32,
    ) -> Option<impl Iterator<Item = PrefixedIpAddr>> {
        if let Some(addrs) = self.by_oif.remove(&oif) {
            for addr in &addrs {
                self.remove(addr);
            }

            return Some(addrs.into_iter());
        }

        None
    }

    #[allow(dead_code)] // used by default-route handling
    pub fn remove_default_route(&mut self, v6: bool) -> Option<Route> {
        if v6 {
            self.remove(&PrefixedIpAddr::V6(PrefixedIpv6Addr::UNSPECIFIED))
        } else {
            self.remove(&PrefixedIpAddr::V4(PrefixedIpv4Addr::UNSPECIFIED))
        }
    }

    pub fn find(
        &self,
        addr: &PrefixedIpAddr,
    ) -> Option<impl Iterator<Item = &Route>> {
        match addr {
            PrefixedIpAddr::V4(addr) => self.v4.get(addr).map(|set| set.iter()),
            PrefixedIpAddr::V6(addr) => self.v6.get(addr).map(|set| set.iter()),
        }
    }

    pub fn find_default(
        &self,
        v6: bool,
    ) -> Option<impl Iterator<Item = &Route>> {
        if v6 {
            self.v6
                .get(&PrefixedIpv6Addr::UNSPECIFIED)
                .map(|set| set.iter())
        } else {
            self.v4
                .get(&PrefixedIpv4Addr::UNSPECIFIED)
                .map(|set| set.iter())
        }
    }

    pub fn find_by_oif(
        &self,
        oif: u32,
    ) -> Option<impl Iterator<Item = &Route>> {
        self.by_oif.get(&oif).map(|addrs| {
            addrs.iter().filter_map(move |addr| {
                self.find(addr)
                    .map(|mut set| set.find(|rt| rt.if_index() == Some(oif)))?
            })
        })
    }

    #[allow(dead_code)] // route enumeration; Phase D test
    pub fn v4_routes(&self) -> impl Iterator<Item = &Route> {
        self.v4.values().flatten()
    }

    #[allow(dead_code)] // route enumeration; Phase D test
    pub fn v6_routes(&self) -> impl Iterator<Item = &Route> {
        self.v6.values().flatten()
    }

    #[allow(dead_code)] // route enumeration; TUI/status
    pub fn all_routes(&self) -> impl Iterator<Item = &Route> {
        self.v4.values().flatten().chain(self.v6.values().flatten())
    }

    #[allow(dead_code)] // table reset
    pub fn clear(&mut self) {
        self.v4.clear();
        self.v6.clear();
        self.by_oif.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn v4_route(ip: [u8; 4], prefix: u8, oif: u32) -> Route {
        Route::new(
            IpAddr::V4(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])),
            prefix,
        )
        .with_if_index(oif)
    }

    fn v4_prefixed(ip: [u8; 4], prefix: u8) -> PrefixedIpAddr {
        PrefixedIpAddr::V4(PrefixedIpv4Addr::new(
            Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
            prefix,
        ))
    }

    #[test]
    fn insert_and_find() {
        let mut table = RouteTable::new();
        table.insert(v4_route([10, 0, 0, 0], 8, 1));

        let found: Vec<&Route> = table
            .find(&v4_prefixed([10, 0, 0, 0], 8))
            .unwrap()
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].destination(),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0))
        );
    }

    #[test]
    fn insert_tracks_by_oif() {
        let mut table = RouteTable::new();
        table.insert(v4_route([10, 0, 0, 0], 8, 1));
        table.insert(v4_route([10, 1, 0, 0], 16, 1));
        table.insert(v4_route([172, 16, 0, 0], 12, 2));

        let routes: Vec<&Route> = table.find_by_oif(1).unwrap().collect();
        assert_eq!(routes.len(), 2);
        let routes2: Vec<&Route> = table.find_by_oif(2).unwrap().collect();
        assert_eq!(routes2.len(), 1);
        assert!(table.find_by_oif(3).is_none());
    }

    #[test]
    fn remove_drops_from_all_indices() {
        let mut table = RouteTable::new();
        table.insert(v4_route([10, 0, 0, 0], 8, 1));

        let removed = table.remove(&v4_prefixed([10, 0, 0, 0], 8)).unwrap();
        assert_eq!(
            removed.destination(),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0))
        );
        assert!(table.find(&v4_prefixed([10, 0, 0, 0], 8)).is_none());
        // Removing the last route on an oif should clear the by_oif bucket.
        assert!(table.find_by_oif(1).is_none());
    }

    #[test]
    fn default_route_crud() {
        let mut table = RouteTable::new();

        // No default route initially.
        assert!(table.find_default(false).is_none());
        assert!(table.find_default(true).is_none());

        table.insert(v4_route([0, 0, 0, 0], 0, 1));
        assert!(table.find_default(false).is_some());

        let removed = table.remove_default_route(false).unwrap();
        assert_eq!(removed.destination(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert!(table.find_default(false).is_none());
    }

    #[test]
    fn remove_by_oif_removes_all() {
        let mut table = RouteTable::new();
        table.insert(v4_route([10, 0, 0, 0], 8, 1));
        table.insert(v4_route([10, 1, 0, 0], 16, 1));
        table.insert(v4_route([192, 168, 1, 0], 24, 2));

        let removed: Vec<PrefixedIpAddr> =
            table.remove_by_oif(1).unwrap().collect();
        assert_eq!(removed.len(), 2);
        assert!(table.find_by_oif(1).is_none());
        // The other interface's route is untouched.
        assert!(table.find_by_oif(2).is_some());
    }

    #[test]
    fn all_routes_round_trip() {
        let mut table = RouteTable::new();
        table.insert(v4_route([10, 0, 0, 0], 8, 1));
        assert_eq!(table.all_routes().count(), 1);
        table.clear();
        assert_eq!(table.all_routes().count(), 0);
    }
}
