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
                    self.by_oif.get_mut(&oif).map(|set| set.remove(addr));

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
                    self.by_oif.get_mut(&oif).map(|set| set.remove(addr));

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
