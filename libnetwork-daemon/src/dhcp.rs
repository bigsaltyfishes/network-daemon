use std::net::Ipv4Addr;

#[derive(Debug, Clone)]
pub struct Lease {
    pub assigned_ip: Ipv4Addr,
    pub server_id: Ipv4Addr,
    pub subnet_mask: Ipv4Addr,
    pub router: Option<Ipv4Addr>,
    pub dns_servers: Vec<Ipv4Addr>,
    pub lease_time: u32,     // Seconds
    pub renewal_time: u32,   // T1
    pub rebinding_time: u32, // T2
}

#[derive(Default)]
pub struct LeaseBuilder {
    assigned_ip: Option<Ipv4Addr>,
    server_id: Option<Ipv4Addr>,
    subnet_mask: Option<Ipv4Addr>,
    router: Option<Ipv4Addr>,
    dns_servers: Vec<Ipv4Addr>,
    lease_time: u32,
    renewal_time: Option<u32>,
    rebinding_time: Option<u32>,
}

impl LeaseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_assigned_ip(mut self, ip: Ipv4Addr) -> Self {
        self.assigned_ip = Some(ip);
        self
    }

    pub fn set_server_id(mut self, server_id: Ipv4Addr) -> Self {
        self.server_id = Some(server_id);
        self
    }

    pub fn set_subnet_mask(mut self, mask: Ipv4Addr) -> Self {
        self.subnet_mask = Some(mask);
        self
    }

    pub fn set_router(mut self, router: Ipv4Addr) -> Self {
        self.router = Some(router);
        self
    }

    pub fn add_dns_server(mut self, dns: Ipv4Addr) -> Self {
        self.dns_servers.push(dns);
        self
    }

    pub fn set_lease_time(mut self, lease_time: u32) -> Self {
        self.lease_time = lease_time;
        self
    }

    pub fn set_renewal_time(mut self, renewal_time: u32) -> Self {
        self.renewal_time = Some(renewal_time);
        self
    }

    pub fn set_rebinding_time(mut self, rebinding_time: u32) -> Self {
        self.rebinding_time = Some(rebinding_time);
        self
    }

    pub fn build(self) -> Result<Lease, &'static str> {
        let renewal_time = self.renewal_time.unwrap_or(self.lease_time / 2);
        let rebinding_time = self
            .rebinding_time
            .unwrap_or(((self.lease_time as u64) * 7 / 8) as u32);

        Ok(Lease {
            assigned_ip: self.assigned_ip.ok_or("Assigned IP is required")?,
            server_id: self.server_id.ok_or("Server ID is required")?,
            subnet_mask: self.subnet_mask.ok_or("Subnet mask is required")?,
            router: self.router,
            dns_servers: self.dns_servers,
            lease_time: self.lease_time,
            renewal_time,
            rebinding_time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn base() -> LeaseBuilder {
        LeaseBuilder::new()
            .set_assigned_ip(Ipv4Addr::new(192, 168, 1, 50))
            .set_server_id(Ipv4Addr::new(192, 168, 1, 1))
            .set_subnet_mask(Ipv4Addr::new(255, 255, 255, 0))
            .set_lease_time(3600)
    }

    #[test]
    fn default_timers_are_fractions_of_lease() {
        let lease = base().build().unwrap();
        // T1 defaults to lease/2, T2 defaults to lease*7/8.
        assert_eq!(lease.renewal_time, 1800);
        assert_eq!(lease.rebinding_time, 3150);
    }

    #[test]
    fn explicit_timers_are_preserved() {
        let lease = base()
            .set_renewal_time(100)
            .set_rebinding_time(200)
            .build()
            .unwrap();
        assert_eq!(lease.renewal_time, 100);
        assert_eq!(lease.rebinding_time, 200);
    }

    #[test]
    fn missing_required_fields_is_error() {
        assert!(LeaseBuilder::new().build().is_err());
        assert!(
            LeaseBuilder::new()
                .set_assigned_ip(Ipv4Addr::new(1, 2, 3, 4))
                .build()
                .is_err()
        );
    }

    #[test]
    fn round_trip_dns_and_router() {
        let lease = base()
            .set_router(Ipv4Addr::new(192, 168, 1, 1))
            .add_dns_server(Ipv4Addr::new(8, 8, 8, 8))
            .add_dns_server(Ipv4Addr::new(8, 8, 4, 4))
            .build()
            .unwrap();
        assert_eq!(lease.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(lease.dns_servers.len(), 2);
    }
}
